use skate_net::{
    Body, Pose,
    dedicated::{
        self, ClientEffects, Config, EffectAck, EffectBatch, EffectKind, Gameplay, PlayerMode,
        Server, ShoveRequest,
    },
    lobby::{Info, Session},
    packed::{self, BodyState, Packed},
};
fn info(id: u64) -> Info {
    Info {
        id,
        map: 1,
        rig: 2,
        physics: 3,
        appearance: 4,
    }
}
fn server() -> Server {
    Server::new(Config {
        session: 7,
        server_id: 99,
        map: 1,
        max_players: 16,
    })
    .unwrap()
}
fn clients() -> Vec<Session> {
    (2..=3)
        .map(|id| Session::dedicated_client(7, info(id), 1))
        .collect()
}
fn pump(server: &mut Server, guests: &mut [Session], now: u64, lose: bool) {
    for (i, g) in guests.iter_mut().enumerate() {
        for (n, p) in g.service(now).into_iter().enumerate() {
            if !lose || (n as u64 + now / 10) % 3 != 0 {
                server.receive(10 + i as u64, &p.data, now);
            }
        }
    }
    for (n, p) in server.service(now).into_iter().enumerate() {
        assert!(p.data.len() <= packed::MTU);
        if !lose || (n as u64 + now / 10) % 4 != 0 {
            if let Some(g) = guests.get_mut((p.peer - 10) as usize) {
                g.receive(1, &p.data, now);
            }
        }
    }
}
fn body(x: f32, z: f32, vx: f32) -> Packed {
    let root = Pose {
        p: [x, 1., z],
        q: [0., 0., 0., 1.],
    };
    Packed::body(&BodyState {
        root,
        enabled: (1 << 33) - 1,
        bodies: vec![
            Body {
                pose: root,
                velocity: [vx, 0., 0.],
                angular: [0.; 3]
            };
            33
        ],
    })
    .unwrap()
}
fn frames(guests: &mut [Session], now: u64, distance: f32) {
    guests[0].publish(packed::BODY, body(0., 0., 2.), now);
    guests[1].publish(packed::BODY, body(distance, 0., -2.), now);
}
fn batch(guest: &Session) -> Option<EffectBatch> {
    let server = guest.host_actor()?;
    let record = guest
        .actors
        .get(&server)?
        .application
        .get(&dedicated::effects_key(guest.local))?;
    serde_json::from_slice(&record.value).ok()
}
#[test]
fn dedicated_membership_excludes_server_and_preserves_sixteen_client_capacity() {
    let mut s = server();
    let mut gs: Vec<_> = (2..=17)
        .map(|id| Session::dedicated_client(7, info(id), 1))
        .collect();
    for now in (0..2000).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(s.player_count(), 16);
    assert!(
        gs.iter()
            .all(|g| g.connected() && g.is_dedicated() && g.player_ids().len() == 16)
    );
    assert!(gs.iter().all(|g| !g.player_ids().contains(&99)));
}
#[test]
fn legacy_and_incompatible_clients_cannot_join_a_dedicated_server() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..100).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    let mut legacy = Session::new(7, info(4), Some(1));
    for p in legacy.service(100) {
        s.receive(20, &p.data, 100);
    }
    let mut wrong = info(5);
    wrong.physics = 44;
    let mut wrong = Session::dedicated_client(7, wrong, 1);
    for p in wrong.service(100) {
        s.receive(21, &p.data, 100);
    }
    let mut wrong_map = info(6);
    wrong_map.map = 55;
    let mut wrong_map = Session::dedicated_client(7, wrong_map, 1);
    for p in wrong_map.service(100) {
        s.receive(22, &p.data, 100);
    }
    s.service(100);
    assert_eq!(s.player_count(), 2);
}
#[test]
fn dedicated_body_gameplay_and_names_replicate_but_mod_records_do_not() {
    let mut s = server();
    let mut gs = clients();
    gs[0].publish_application("mp:name", b"Skater".to_vec(), 0);
    let gameplay = Gameplay {
        mode: PlayerMode::Skating,
        trick_seq: 2,
        trick: "Kickflip".into(),
        ..Default::default()
    };
    gs[0].publish_application(
        dedicated::GAMEPLAY_KEY,
        serde_json::to_vec(&gameplay).unwrap(),
        0,
    );
    assert!(!gs[0].publish_application("net:mod:state", b"forbidden".to_vec(), 0));
    for now in (0..3000).step_by(10) {
        if now % 50 == 0 {
            frames(&mut gs, now, 4.);
        }
        pump(&mut s, &mut gs, now, true);
    }
    assert!(gs[1].actors[&2].body.latest().is_some());
    assert_eq!(gs[1].actors[&2].application["mp:name"].value, b"Skater");
    let received: Gameplay =
        serde_json::from_slice(&gs[1].actors[&2].application[dedicated::GAMEPLAY_KEY].value)
            .unwrap();
    assert_eq!(received.trick, "Kickflip");
}
#[test]
fn opposite_collision_effects_survive_loss_and_are_consumed_once_without_overlap_pumping() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..3000).step_by(10) {
        if now % 50 == 0 {
            frames(&mut gs, now, 0.5);
        }
        pump(&mut s, &mut gs, now, true);
    }
    let a = batch(&gs[0]).unwrap();
    let b = batch(&gs[1]).unwrap();
    assert_eq!(a.effects.len(), 1);
    assert_eq!(b.effects.len(), 1);
    assert_eq!(a.effects[0].kind, EffectKind::Collision);
    assert!(a.effects[0].delta_velocity[0] < 0.);
    assert!(b.effects[0].delta_velocity[0] > 0.);
    assert!((a.effects[0].delta_velocity[0] + b.effects[0].delta_velocity[0]).abs() < 0.001);
    let mut consumer = ClientEffects::default();
    assert_eq!(consumer.consume(&a).len(), 1);
    assert!(consumer.consume(&a).is_empty());
    let ack = consumer.ack();
    assert_eq!(ack.through, 1);
    gs[0].publish_application(
        dedicated::EFFECT_ACK_KEY,
        serde_json::to_vec(&ack).unwrap(),
        3000,
    );
    for now in (3000..4500).step_by(10) {
        pump(&mut s, &mut gs, now, true);
    }
    assert!(batch(&gs[0]).unwrap().effects.is_empty());
}
#[test]
fn shoves_validate_reach_freshness_cooldown_and_issue_attacker_animation_once() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..100).step_by(10) {
        gs[0].publish(packed::BODY, body(0., 0., 0.), now);
        gs[1].publish(packed::BODY, body(0., 1.2, 0.), now);
        pump(&mut s, &mut gs, now, false);
    }
    let epoch = batch(&gs[0]).unwrap().epoch;
    gs[0].publish_application(
        dedicated::SHOVE_KEY,
        serde_json::to_vec(&ShoveRequest {
            epoch,
            id: 1,
            target: 3,
        })
        .unwrap(),
        100,
    );
    for now in (100..300).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        batch(&gs[0])
            .unwrap()
            .effects
            .iter()
            .filter(|e| e.kind == EffectKind::AttackAccepted)
            .count(),
        1
    );
    assert_eq!(
        batch(&gs[1])
            .unwrap()
            .effects
            .iter()
            .filter(|e| e.kind == EffectKind::Shove)
            .count(),
        1
    );
    let epoch = batch(&gs[0]).unwrap().epoch;
    gs[0].publish_application(
        dedicated::SHOVE_KEY,
        serde_json::to_vec(&ShoveRequest {
            epoch,
            id: 2,
            target: 3,
        })
        .unwrap(),
        300,
    );
    for now in (300..1000).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        batch(&gs[1])
            .unwrap()
            .effects
            .iter()
            .filter(|e| e.kind == EffectKind::Shove)
            .count(),
        1
    );
}
#[test]
fn effects_do_not_accept_future_ack_and_departure_removes_player() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..500).step_by(10) {
        if now % 50 == 0 {
            frames(&mut gs, now, 0.5);
        }
        pump(&mut s, &mut gs, now, false);
    }
    let initial = batch(&gs[0]).unwrap();
    gs[0].publish_application(
        dedicated::EFFECT_ACK_KEY,
        serde_json::to_vec(&EffectAck {
            epoch: initial.epoch,
            through: 100,
        })
        .unwrap(),
        500,
    );
    for now in (500..800).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(batch(&gs[0]).unwrap().effects.len(), 1);
    for p in gs[0].goodbye() {
        s.receive(10, &p.data, 800);
    }
    s.service(800);
    assert_eq!(s.player_count(), 1);
}

#[test]
fn ping_metadata_is_allowed_without_opening_arbitrary_application_records() {
    let mut s = server();
    let mut gs = clients();
    assert!(gs[0].publish_application("mp:ping", 42u64.to_le_bytes().to_vec(), 0));
    assert!(!gs[0].publish_application("mp:ping", vec![1; 9], 0));
    for now in (0..1000).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        gs[1].actors[&2].application["mp:ping"].value,
        42u64.to_le_bytes()
    );
}

#[test]
fn multiple_unacked_effects_remain_ordered_across_batches() {
    let mut s = server();
    let mut gs = clients();
    // Alternating separation and collision produces six distinct contacts; no
    // recipient acknowledges anything while the reliable effect queue grows.
    for now in (0..6000).step_by(10) {
        if now % 50 == 0 {
            frames(&mut gs, now, if now % 1000 < 400 { 0.5 } else { 4. });
        }
        pump(&mut s, &mut gs, now, false);
    }
    let mut consumer = ClientEffects::default();
    let mut ids = vec![];
    for now in (6000..9000).step_by(10) {
        if let Some(batch) = batch(&gs[0]) {
            ids.extend(consumer.consume(&batch).into_iter().map(|effect| effect.id));
            gs[0].publish_application(
                dedicated::EFFECT_ACK_KEY,
                serde_json::to_vec(&consumer.ack()).unwrap(),
                now,
            );
        }
        pump(&mut s, &mut gs, now, true);
    }
    assert_eq!(ids, vec![1, 2, 3, 4, 5, 6]);
    assert!(batch(&gs[0]).unwrap().effects.is_empty());
}

#[test]
fn stale_wrong_map_out_of_reach_and_rear_attacks_produce_no_hit() {
    for (target_x, target_z, wait, ragdoll) in [
        (0., 4., 0, false),
        (0., -1.2, 0, false),
        (0., 1.2, 600, false),
        (0., 1.2, 0, true),
    ] {
        let mut s = server();
        let mut gs = clients();
        for now in (0..100).step_by(10) {
            gs[0].publish(packed::BODY, body(0., 0., 0.), now);
            gs[1].publish(packed::BODY, body(target_x, target_z, 0.), now);
            if ragdoll {
                gs[0].publish_application(
                    dedicated::GAMEPLAY_KEY,
                    serde_json::to_vec(&Gameplay {
                        mode: PlayerMode::Ragdoll,
                        ..Default::default()
                    })
                    .unwrap(),
                    now,
                );
            }
            pump(&mut s, &mut gs, now, false);
        }
        let epoch = batch(&gs[0]).unwrap().epoch;
        gs[0].publish_application(
            dedicated::SHOVE_KEY,
            serde_json::to_vec(&ShoveRequest {
                epoch,
                id: 1,
                target: 3,
            })
            .unwrap(),
            100 + wait,
        );
        for now in (100 + wait..500 + wait).step_by(10) {
            pump(&mut s, &mut gs, now, false);
        }
        assert!(
            batch(&gs[1])
                .unwrap()
                .effects
                .iter()
                .all(|effect| effect.kind != EffectKind::Shove),
            "{target_z}, wait={wait}, ragdoll={ragdoll}"
        );
    }
}

#[test]
fn server_rejects_spoofed_builtin_effects_and_blob_downloads() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..100).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    let mut spoof = packed::header(7, 99, skate_net::lobby::APPLICATION, 50);
    let key = dedicated::effects_key(3);
    spoof.push(key.len() as u8);
    spoof.extend(key.as_bytes());
    spoof.extend(b"forged");
    s.receive(10, &spoof, 100);
    let mut blob = packed::header(7, 2, skate_net::blob::META, 1);
    blob.extend([0; 36]);
    s.receive(10, &blob, 100);
    let mut arbitrary = packed::header(7, 2, skate_net::lobby::APPLICATION, 50);
    arbitrary.push(8);
    arbitrary.extend(b"mod:evil");
    arbitrary.extend(b"not allowed");
    s.receive(10, &arbitrary, 100);
    for now in (100..500).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert!(batch(&gs[1]).unwrap().effects.is_empty());
    assert!(!gs[1].actors[&2].application.contains_key("mod:evil"));
    assert!(gs.iter().all(|guest| guest.blobs.ready(2).is_none()));
}

#[test]
fn movement_plausibility_rejects_stationary_teleport_without_capping_high_speed() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..300).step_by(10) {
        if now % 50 == 0 {
            gs[0].publish(packed::BODY, body(0., 0., 0.), now);
        }
        pump(&mut s, &mut gs, now, false);
    }
    gs[0].publish(packed::BODY, body(1000., 0., 0.), 300);
    for now in (300..500).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        gs[1].actors[&2].body.latest().unwrap().state.position()[0],
        0.
    );
    gs[0].publish(packed::BODY, body(1000., 0., 4000.), 500);
    for now in (500..900).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        gs[1].actors[&2].body.latest().unwrap().state.position()[0],
        1000.
    );
}

#[test]
fn old_epoch_and_out_of_order_batches_cannot_reapply_effects() {
    let effect = |id| dedicated::Effect {
        id,
        source: 2,
        target: 3,
        delta_velocity: [1., 0., 0.],
        position: [0.; 3],
        kind: EffectKind::Collision,
    };
    let mut client = ClientEffects::default();
    assert!(
        client
            .consume(&EffectBatch {
                epoch: 10,
                effects: vec![effect(2)]
            })
            .is_empty()
    );
    assert_eq!(
        client
            .consume(&EffectBatch {
                epoch: 10,
                effects: vec![effect(1), effect(2)]
            })
            .len(),
        2
    );
    assert!(
        client
            .consume(&EffectBatch {
                epoch: 10,
                effects: vec![effect(4)]
            })
            .is_empty()
    );
    assert_eq!(
        client
            .consume(&EffectBatch {
                epoch: 10,
                effects: vec![effect(2), effect(3), effect(4)]
            })
            .len(),
        2
    );
    assert_eq!(
        client
            .consume(&EffectBatch {
                epoch: 11,
                effects: vec![effect(1)]
            })
            .len(),
        1
    );
    assert!(
        client
            .consume(&EffectBatch {
                epoch: 10,
                effects: vec![effect(1)]
            })
            .is_empty()
    );
    assert_eq!(
        client.ack(),
        EffectAck {
            epoch: 11,
            through: 1
        }
    );
}

#[test]
fn unacknowledged_overflow_disconnects_instead_of_dropping_impulses() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..64_000).step_by(10) {
        if now % 50 == 0 {
            frames(&mut gs, now, if now % 1000 < 400 { 0.5 } else { 4. });
        }
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(s.player_count(), 2);
    for now in (64_000..64_500).step_by(10) {
        if now % 50 == 0 {
            frames(&mut gs, now, 0.5);
        }
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(s.player_count(), 0);
}

#[test]
fn respawn_rebaselines_after_a_bounded_gap_and_does_not_make_a_swept_hit() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..300).step_by(10) {
        if now % 50 == 0 {
            frames(&mut gs, now, 20.);
        }
        pump(&mut s, &mut gs, now, false);
    }
    for now in (300..1800).step_by(10) {
        if now % 50 == 0 {
            gs[0].publish(packed::BODY, body(40., 0., 0.), now);
            gs[1].publish(packed::BODY, body(20., 0., 0.), now);
        }
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        gs[1].actors[&2].body.latest().unwrap().state.position()[0],
        40.
    );
    assert!(batch(&gs[1]).unwrap().effects.is_empty());
}

#[test]
fn crossing_players_collide_between_twenty_hz_snapshots() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..200).step_by(10) {
        if now % 50 == 0 {
            gs[0].publish(packed::BODY, body(-2., 0., 80.), now);
            gs[1].publish(packed::BODY, body(2., 0., -80.), now);
        }
        pump(&mut s, &mut gs, now, false);
    }
    gs[0].publish(packed::BODY, body(2., 0., 80.), 200);
    gs[1].publish(packed::BODY, body(-2., 0., -80.), 200);
    for now in (200..600).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    let a = batch(&gs[0]).unwrap();
    let b = batch(&gs[1]).unwrap();
    assert_eq!(a.effects.len(), 1);
    assert_eq!(b.effects.len(), 1);
    assert!(a.effects[0].delta_velocity[0] < 0.);
    assert!(b.effects[0].delta_velocity[0] > 0.);
}

#[test]
fn server_readmission_recovers_unchanged_metadata_and_does_not_replay_shove() {
    let mut s = server();
    let mut gs = clients();
    gs[0].publish_application("mp:name", b"Returner".to_vec(), 0);
    for now in (0..200).step_by(10) {
        if now % 50 == 0 {
            gs[0].publish(packed::BODY, body(0., 0., 0.), now);
            gs[1].publish(packed::BODY, body(0., 1.2, 0.), now);
        }
        pump(&mut s, &mut gs, now, false);
    }
    let old_epoch = batch(&gs[0]).unwrap().epoch;
    // Simulate asymmetric packet loss: server-to-client packets keep clients
    // alive, while every client-to-server datagram is lost for over 5 seconds.
    for now in (200..6200).step_by(10) {
        for guest in &mut gs {
            let _ = guest.service(now);
        }
        for packet in s.service(now) {
            if let Some(guest) = gs.get_mut((packet.peer - 10) as usize) {
                guest.receive(1, &packet.data, now);
            }
        }
    }
    assert_eq!(s.player_count(), 0);
    for now in (6200..7600).step_by(10) {
        if now % 50 == 0 {
            gs[0].publish(packed::BODY, body(0., 0., 0.), now);
            gs[1].publish(packed::BODY, body(0., 1.2, 0.), now);
        }
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(s.player_count(), 2);
    assert_eq!(gs[1].actors[&2].application["mp:name"].value, b"Returner");
    let new_epoch = batch(&gs[0]).unwrap().epoch;
    assert_ne!(new_epoch, old_epoch);
    assert!(batch(&gs[1]).unwrap().effects.is_empty());
    gs[0].publish_application(
        dedicated::SHOVE_KEY,
        serde_json::to_vec(&ShoveRequest {
            epoch: old_epoch,
            id: 1,
            target: 3,
        })
        .unwrap(),
        7600,
    );
    for now in (7600..8000).step_by(10) {
        if now % 50 == 0 {
            gs[0].publish(packed::BODY, body(0., 0., 0.), now);
            gs[1].publish(packed::BODY, body(0., 1.2, 0.), now);
        }
        pump(&mut s, &mut gs, now, false);
    }
    assert!(
        batch(&gs[1]).unwrap().effects.is_empty(),
        "Old incarnation must not attack again"
    );
    gs[0].publish_application(
        dedicated::SHOVE_KEY,
        serde_json::to_vec(&ShoveRequest {
            epoch: new_epoch,
            id: 1,
            target: 3,
        })
        .unwrap(),
        8000,
    );
    for now in (8000..8200).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        batch(&gs[1])
            .unwrap()
            .effects
            .iter()
            .filter(|e| e.kind == EffectKind::Shove)
            .count(),
        1
    );
}

#[test]
fn fast_server_restart_republishes_unchanged_metadata_and_recovers_effect_epoch() {
    let mut s = server();
    let mut gs = clients();
    gs[0].publish_application("mp:name", b"Survivor".to_vec(), 0);
    for now in (0..2000).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    let old_epoch = batch(&gs[0]).unwrap().epoch;
    s = Server::new(Config {
        session: 7,
        server_id: 100,
        map: 1,
        max_players: 16,
    })
    .unwrap();
    for now in (2000..4000).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(s.player_count(), 2);
    assert_eq!(gs[0].host_actor(), Some(100));
    assert_eq!(gs[1].actors[&2].application["mp:name"].value, b"Survivor");
    assert_ne!(batch(&gs[0]).unwrap().epoch, old_epoch);
    gs.push(Session::dedicated_client(7, info(4), 1));
    for now in (4000..5000).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(gs[2].actors[&2].application["mp:name"].value, b"Survivor");
}
