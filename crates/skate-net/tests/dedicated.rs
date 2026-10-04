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
fn shared_object(id:u64,instance:u64)->skate_net::entities::Entity {
    use skate_net::entities::{Entity,Definition,Shape,BodyType};
    Entity {id,resource:"objects".into(),generation:1,key:format!("box{id}"),instance,controller:None,
        definition:Definition {shape:Shape::Box {half_extents:[0.5;3]},body_type:BodyType::Dynamic,mass:5.,friction:0.7,color:[0.2,0.6,0.9,1.]},
        tick:1,epoch:1,position:[0.,1.,0.],rotation:[0.,0.,0.,1.],velocity:[1.,0.,0.],angular:[0.;3]}
}
#[test]
fn shared_object_snapshots_recover_loss_late_join_and_instance_change_without_spoofing() {
    let mut s=server();let mut gs=clients();
    for now in (0..200).step_by(10) {pump(&mut s,&mut gs,now,false);}
    s.publish_entities(vec![shared_object(1,0),shared_object(2,9)]).unwrap();
    for now in (200..1000).step_by(10) {pump(&mut s,&mut gs,now,true);}
    assert!(gs.iter().all(|g|g.entities.entities().keys().copied().collect::<Vec<_>>()==vec![1]),"replicas: {:?}",gs.iter().map(|g|(g.connected(),g.movement_epoch(),g.entities.entities().keys().copied().collect::<Vec<_>>())).collect::<Vec<_>>());
    gs.push(Session::dedicated_client(7,info(4),1));
    for now in (1000..2800).step_by(10) {pump(&mut s,&mut gs,now,true);}
    assert!(gs[2].entities.entities().contains_key(&1),"late connected={} epoch={} keys={:?}",gs[2].connected(),gs[2].movement_epoch(),gs[2].entities.entities().keys());
    assert_eq!(gs[2].entities.entities()[&1].position,[0.,1.,0.]);
    let old_epoch=gs[0].movement_epoch();
    let forged=skate_net::entities::Frame::Upsert {epoch:old_epoch,instance:0,revision:1,entity:{let mut e=shared_object(1,0);e.position=[900.,1.,0.];e.tick=500;e}};
    let mut packet=packed::header(7,99,skate_net::lobby::SHARED_ENTITY,0);packet.extend(serde_json::to_vec(&forged).unwrap());
    gs[0].receive(123,&packet,2801);s.receive(10,&packet,2801);
    assert_eq!(gs[0].entities.entities()[&1].position,[0.,1.,0.]);
    s.teleport(2,dedicated::TeleportDestination {position:[0.,1.,0.],heading:0.,velocity:[0.;3],instance:9},2802).unwrap();
    for now in (2810..4000).step_by(10) {pump(&mut s,&mut gs,now,true);}
    assert_eq!(gs[0].entities.entities().keys().copied().collect::<Vec<_>>(),vec![2]);
    gs[0].receive(1,&packet,4001);
    assert_eq!(gs[0].entities.entities().keys().copied().collect::<Vec<_>>(),vec![2]);
    assert_eq!(gs[1].entities.entities().keys().copied().collect::<Vec<_>>(),vec![1]);
    s.publish_entities(Vec::new()).unwrap();
    for now in (4010..5000).step_by(10) {pump(&mut s,&mut gs,now,true);}
    assert!(gs.iter().all(|g|g.entities.entities().is_empty()));
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
#[test]
fn peer_metadata_replays_after_instance_return_despite_delayed_old_acknowledgements() {
    let mut server=server();let mut guests=clients();
    for now in (0..200).step_by(10) {pump(&mut server,&mut guests,now,false);}
    guests[0].publish_application("mp:name",b"Alice".to_vec(),201);
    guests[1].publish_application("mp:name",b"Bob".to_vec(),201);
    let mut old_acks=Vec::new();
    for now in (210..700).step_by(10) {
        for (index,guest) in guests.iter_mut().enumerate() {
            for packet in guest.service(now) {
                if packed::envelope(&packet.data).unwrap().2==skate_net::lobby::APPLICATION_ACK
                    && packet.data.ends_with(b"mp:name") {
                    old_acks.push((10+index as u64,packet.data.clone()));
                }
                server.receive(10+index as u64,&packet.data,now);
            }
        }
        for packet in server.service(now) {guests[(packet.peer-10) as usize].receive(1,&packet.data,now);}
    }
    assert!(!old_acks.is_empty());
    assert_eq!(guests[0].actors[&3].application["mp:name"].value,b"Bob");
    assert_eq!(guests[1].actors[&2].application["mp:name"].value,b"Alice");
    let destination=|instance|dedicated::TeleportDestination {position:[0.,1.,0.],heading:0.,velocity:[0.;3],instance};
    server.teleport(2,destination(7),701).unwrap();
    for now in (710..1200).step_by(10) {pump(&mut server,&mut guests,now,false);}
    assert!(!guests[0].actors.contains_key(&3));assert!(!guests[1].actors.contains_key(&2));
    for (peer,ack) in &old_acks {server.receive(*peer,ack,1201);}
    server.teleport(2,destination(0),1202).unwrap();
    for now in (1210..2100).step_by(10) {
        for (index,guest) in guests.iter_mut().enumerate() {
            for packet in guest.service(now) {server.receive(10+index as u64,&packet.data,now);}
        }
        // Reordered pre-departure ACKs must not confirm a lost replay after
        // either endpoint crosses a visibility boundary.
        for (peer,ack) in &old_acks {server.receive(*peer,ack,now);}
        for packet in server.service(now) {
            let (_,actor,kind,_)=packed::envelope(&packet.data).unwrap();
            if now<1410 && actor!=99 && kind==skate_net::lobby::APPLICATION {continue;}
            guests[(packet.peer-10) as usize].receive(1,&packet.data,now);
        }
    }
    assert_eq!(guests[0].actors[&3].application.get("mp:name").map(|r|r.value.as_slice()),Some(b"Bob".as_slice()),"returning observer lost the unchanged public name");
    assert_eq!(guests[1].actors[&2].application.get("mp:name").map(|r|r.value.as_slice()),Some(b"Alice".as_slice()),"remaining observer lost the returning peer's name");
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
fn entity_sample(server: &mut Server, guests: &mut [Session], x: f32, wheel_velocity: f32,
    rig_velocity: f32, captured: u64, received: u64) {
    let mut state=body(x,0.,rig_velocity).unpack_body().unwrap();
    state.bodies[0].velocity=[wheel_velocity,0.,0.];
    guests[0].publish(packed::BODY,Packed::body(&state).unwrap(),captured);
    let revision=guests[0].actors[&2].body.latest().unwrap();
    let mut packet=packed::delta(7,2,packed::BODY,revision.seq,&revision.state,None);
    packet.extend(guests[0].movement_epoch().to_le_bytes());
    let before=server.stats().decoded;
    server.receive(10,&packet,received);
    assert_eq!(server.stats().decoded,before+1,"fixture must enter accepted BODY history");
    for packet in server.service(received) {guests[(packet.peer-10) as usize].receive(1,&packet.data,received);}
}
fn entity_velocity(server: &Server) -> [f32;3] {
    server.entity_players().into_iter().find(|p|p.actor==2).unwrap().velocity
}
#[test]
fn entity_contact_velocity_tracks_root_motion_instead_of_rebounding_wheel() {
    let mut server=server();let mut guests=clients();
    for now in (0..200).step_by(10) {pump(&mut server,&mut guests,now,false);}
    entity_sample(&mut server,&mut guests,0.,4.,4.,200,200);
    // Accepted root samples still enter the object while an articulated wheel
    // has already bounced. The capsule is anchored to the root, not that wheel.
    entity_sample(&mut server,&mut guests,0.2,-1.,4.,250,250);
    assert!((entity_velocity(&server)[0]-4.).abs()<0.001,
        "incoming root motion must not acquire the wheel's outgoing velocity: {:?}",entity_velocity(&server));
}
#[test]
fn entity_contact_root_velocity_is_bounded_under_clock_jitter_and_discontinuities() {
    for (captured,received,x,rig,expected) in [
        (250,250,0.2,4.,4.), // ordinary 50 ms sample interval
        (201,250,0.2,4.,4.), // tiny source interval cannot beat arrival time
        (250,200,0.2,4.,4.), // receive bunching retains source interval
        (201,200,0.2,4.,4.), // both clocks compressed: bounded by observed rig speed
        (200,250,0.2,4.,0.), // duplicate source capture time has no derivative
        (200,200,0.2,4.,0.), // duplicate capture and receive times
        (451,250,0.2,4.,0.), // old source baseline
        (451,451,0.2,4.,0.), // old receive baseline
        (250,250,5.,4.,0.), // tolerated movement offset is not a kinematic shove
        (250,250,0.,4.,0.), // wheel motion alone does not move the root capsule
        (201,200,0.5,400.,200.), // conservative proxy speed cap
    ] {
        let mut server=server();let mut guests=clients();
        for now in (0..200).step_by(10) {pump(&mut server,&mut guests,now,false);}
        entity_sample(&mut server,&mut guests,0.,rig,rig,200,200);
        entity_sample(&mut server,&mut guests,x,-rig,rig,captured,received);
        let actual=entity_velocity(&server);
        assert!((actual[0]-expected).abs()<0.001 && actual[1]==0. && actual[2]==0.,
            "capture={captured} receive={received} x={x} rig={rig}: {actual:?} != {expected}");
    }
}
#[test]
fn entity_contact_root_history_retires_on_teleport_staleness_and_disconnect() {
    let mut server=server();let mut guests=clients();
    for now in (0..200).step_by(10) {pump(&mut server,&mut guests,now,false);}
    entity_sample(&mut server,&mut guests,0.,4.,4.,200,200);
    assert_eq!(entity_velocity(&server),[0.;3],"one owner sample is not root motion evidence");
    entity_sample(&mut server,&mut guests,0.2,-1.,4.,250,250);
    for (instance,now) in [(0,300),(7,600)] {
        let epoch=server.teleport(2,dedicated::TeleportDestination {
            position:[40.,1.,0.],heading:0.,velocity:[12.,0.,0.],instance},now).unwrap();
        pump(&mut server,&mut guests,now,false);
        assert_eq!(entity_velocity(&server),[0.;3],"reset placement must not inherit old root travel");
        assert_eq!(server.entity_players()[0].instance,instance);
        assert_eq!(server.entity_players()[0].epoch,epoch);
        guests[0].complete_movement_reset(epoch);
        entity_sample(&mut server,&mut guests,40.05,4.,4.,now+50,now+50);
        assert_eq!(entity_velocity(&server),[0.;3],"synthetic reset sample is not a derivative baseline");
        entity_sample(&mut server,&mut guests,40.25,-1.,4.,now+100,now+100);
        assert!((entity_velocity(&server)[0]-4.).abs()<0.001);
    }
    server.service(951);
    assert!(server.entity_players().is_empty(),"stale root history cannot remain a contact proxy");
    for packet in guests[0].goodbye() {server.receive(10,&packet.data,952);}
    server.service(952);
    assert!(!server.entity_players().iter().any(|p|p.actor==2));
    guests[0]=Session::dedicated_client(7,info(2),1);
    for now in (1000..1200).step_by(10) {pump(&mut server,&mut guests,now,false);}
    entity_sample(&mut server,&mut guests,100.,4.,4.,1200,1200);
    assert_eq!(entity_velocity(&server),[0.;3],"reconnected identity must establish new root history");
}
#[test]
fn entity_contact_root_history_does_not_cross_resource_readmission() {
    let mut server=server();let mut guests=clients();
    for now in (0..200).step_by(10) {pump(&mut server,&mut guests,now,false);}
    entity_sample(&mut server,&mut guests,0.,4.,4.,200,200);
    entity_sample(&mut server,&mut guests,0.2,4.,4.,250,250);
    assert!((entity_velocity(&server)[0]-4.).abs()<0.001);
    server.configure_resources("a".repeat(64),31031,
        std::collections::BTreeMap::from([("app".into(),1)])).unwrap();
    assert!(server.entity_players().is_empty());
    let mut channels=[skate_net::resources::Client::default(),skate_net::resources::Client::default()];
    for now in (300..700).step_by(10) {
        pump(&mut server,&mut guests,now,false);
        for (guest,channel) in guests.iter_mut().zip(&mut channels) {
            let host=guest.host_actor().unwrap();
            let Some(record)=guest.actors[&host].application.get(&skate_net::resources::server_key(guest.local)) else {continue};
            channel.receive(&serde_json::from_slice(&record.value).unwrap()).unwrap();
            channel.set_ready(true);
            guest.publish_application(skate_net::resources::CLIENT_KEY,channel.encode().unwrap(),now);
        }
    }
    assert!(server.resource_ready(2));
    entity_sample(&mut server,&mut guests,0.4,4.,4.,700,700);
    assert_eq!(entity_velocity(&server),[0.;3],"old resource generation must not supply a derivative baseline");
    entity_sample(&mut server,&mut guests,0.6,-1.,4.,750,750);
    assert!((entity_velocity(&server)[0]-4.).abs()<0.001);
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
fn host_kick_retires_actor_and_old_endpoint_cannot_immediately_readmit() {
    let mut s=server();let mut gs=clients();
    for now in (0..200).step_by(10) {pump(&mut s,&mut gs,now,false);}
    assert_eq!(s.peer_for_actor(2),Some(10));
    assert!(!s.kick(900,201));assert!(s.kick(2,201));assert!(!s.kick(2,201));
    assert_eq!(s.peer_for_actor(2),None);
    for now in (210..1200).step_by(10) {pump(&mut s,&mut gs,now,false);}
    assert_eq!(s.player_count(),1);assert!(!gs[1].player_ids().contains(&2));
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
fn unauthorized_respawn_remains_rejected_after_a_gap() {
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
        0.
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

fn destination(x: f32, instance: u64) -> dedicated::TeleportDestination {
    dedicated::TeleportDestination {
        position: [x, 1., 0.],
        heading: 0.,
        velocity: [0.; 3],
        instance,
    }
}

#[test]
fn authorized_teleport_is_immediate_resets_history_and_rejects_inflight_movement() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..300).step_by(10) {
        if now % 50 == 0 {
            frames(&mut gs, now, 20.);
        }
        pump(&mut s, &mut gs, now, false);
    }
    gs[0].publish(packed::BODY, body(1., 0., 0.), 300);
    gs[0].publish(
        packed::POSE,
        Packed::pose(&packed::PoseState {
            root: Pose {
                p: [1., 1., 0.],
                q: [0., 0., 0., 1.],
            },
            bones: vec![skate_net::Bone {
                index: 1,
                pose: Pose {
                    p: [1., 1., 0.],
                    q: [0., 0., 0., 1.],
                },
            }],
        })
        .unwrap(),
        300,
    );
    let stale: Vec<_> = gs[0]
        .service(310)
        .into_iter()
        .filter(|p| {
            matches!(
                packed::envelope(&p.data).unwrap().2,
                packed::BODY | packed::POSE
            )
        })
        .collect();
    assert!(!stale.is_empty());
    let before = gs[0].movement_epoch();
    let epoch = s.teleport(2, destination(40., 0), 311).unwrap();
    assert!(epoch > before);
    // Same-tick control delivery resets both the owner and observer. No one
    // second silence and no new owner snapshot is needed to display the target.
    pump(&mut s, &mut gs, 311, false);
    assert_eq!(gs[0].pending_movement_reset().unwrap().epoch, epoch);
    assert_eq!(
        gs[1].actors[&2].body.latest().unwrap().state.position()[0],
        40.
    );
    assert_eq!(gs[1].actors[&2].body.history.len(), 1);
    assert!(batch(&gs[1]).unwrap().effects.is_empty());
    for packet in &stale {
        s.receive(10, &packet.data, 312);
        gs[1].receive(1, &packet.data, 312);
    }
    gs[0].complete_movement_reset(epoch);
    gs[0].publish(packed::BODY, body(40., 0., 0.), 313);
    for now in (313..600).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        gs[1].actors[&2].body.latest().unwrap().state.position()[0],
        40.
    );
    assert!(
        gs[1].actors[&2]
            .body
            .history
            .iter()
            .all(|r| r.state.position()[0] == 40.)
    );
    assert!(s.stats().late > 0);
    assert!(batch(&gs[1]).unwrap().effects.is_empty());
}

#[test]
fn teleport_retry_duplicates_and_old_resets_cannot_roll_back_owner() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..200).step_by(10) {
        frames(&mut gs, now, 20.);
        pump(&mut s, &mut gs, now, false);
    }
    let first = s.teleport(2, destination(40., 0), 200).unwrap();
    // Drop initial reset delivery; movement packets carry no implicit reset.
    let dropped = s.service(200);
    let old = dropped
        .iter()
        .find(|p| {
            p.peer == 10 && packed::envelope(&p.data).unwrap().2 == skate_net::lobby::MOVEMENT_RESET
        })
        .unwrap()
        .data
        .clone();
    assert!(gs[0].pending_movement_reset().is_none());
    pump(&mut s, &mut gs, 300, false);
    assert_eq!(gs[0].pending_movement_reset().unwrap().epoch, first);
    gs[0].complete_movement_reset(first);
    gs[0].receive(1, &old, 301);
    assert!(
        gs[0].pending_movement_reset().is_none(),
        "Duplicate reset reapplied"
    );
    let second = s.teleport(2, destination(80., 0), 302).unwrap();
    pump(&mut s, &mut gs, 402, false);
    assert_eq!(gs[0].pending_movement_reset().unwrap().epoch, second);
    gs[0].complete_movement_reset(second);
    gs[0].receive(1, &old, 403);
    assert_eq!(gs[0].movement_epoch(), second);
    assert_eq!(
        gs[0].actors[&2].body.latest().unwrap().state.position()[0],
        80.
    );
    assert!(gs[0].pending_movement_reset().is_none());
}

#[test]
fn teleport_requests_require_explicit_server_allowlist_and_current_epoch() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..200).step_by(10) {
        frames(&mut gs, now, 20.);
        pump(&mut s, &mut gs, now, false);
    }
    let epoch = gs[0].movement_epoch();
    let request = |id, epoch| {
        serde_json::to_vec(&dedicated::TeleportRequest {
            epoch,
            id,
            destination: "spawn".into(),
        })
        .unwrap()
    };
    assert!(gs[0].publish_application(dedicated::TELEPORT_KEY, request(1, epoch), 200));
    for now in (200..400).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(gs[0].movement_epoch(), epoch);
    s.allow_teleport_destination("spawn".into(), destination(50., 0))
        .unwrap();
    assert!(gs[0].publish_application(dedicated::TELEPORT_KEY, request(2, epoch), 400));
    for now in (400..600).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    let new_epoch = gs[0].movement_epoch();
    assert!(new_epoch > epoch);
    gs[0].complete_movement_reset(new_epoch);
    assert!(gs[0].publish_application(dedicated::TELEPORT_KEY, request(3, epoch), 600));
    for now in (600..800).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(gs[0].movement_epoch(), new_epoch);
    let mut forged = packed::header(7, 99, skate_net::lobby::MOVEMENT_RESET, 0);
    forged.extend(
        serde_json::to_vec(&dedicated::MovementReset {
            actor: 2,
            epoch: new_epoch + 1,
            destination: destination(90., 0),
        })
        .unwrap(),
    );
    gs[0].receive(999, &forged, 800);
    s.receive(10, &forged, 800);
    assert_eq!(gs[0].movement_epoch(), new_epoch);
    assert!(s.teleport(2, destination(f32::NAN, 0), 800).is_err());
    assert!(s.teleport(500, destination(0., 0), 800).is_err());
}

#[test]
fn teleport_instance_transition_cleans_visibility_and_prevents_shared_collisions() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..200).step_by(10) {
        frames(&mut gs, now, 20.);
        pump(&mut s, &mut gs, now, false);
    }
    let epoch = s.teleport(2, destination(20., 5), 200).unwrap();
    pump(&mut s, &mut gs, 201, false);
    assert_eq!(s.instance_of(2), Some(5));
    assert_eq!(s.instance_of(3), Some(0));
    assert!(!gs[0].actors.contains_key(&3));
    assert!(!gs[1].actors.contains_key(&2));
    gs[0].complete_movement_reset(epoch);
    for now in (250..700).step_by(50) {
        gs[0].publish(packed::BODY, body(20., 0., 0.), now);
        gs[1].publish(packed::BODY, body(20., 0., 0.), now);
        pump(&mut s, &mut gs, now, false);
    }
    assert!(batch(&gs[0]).unwrap().effects.is_empty());
    assert!(batch(&gs[1]).unwrap().effects.is_empty());
    s.teleport(2, destination(40., 0), 701).unwrap();
    pump(&mut s, &mut gs, 702, false);
    assert!(gs[0].actors.contains_key(&3));
    assert!(gs[1].actors.contains_key(&2));
}

#[test]
fn sixty_four_player_rosters_reassemble_under_loss_with_every_datagram_bounded() {
    let mut s = Server::new(Config {
        session: 7,
        server_id: 99,
        map: 1,
        max_players: 64,
    })
    .unwrap();
    let mut gs: Vec<_> = (2..66)
        .map(|id| Session::dedicated_client(7, info(id), 1))
        .collect();
    for now in (0..4000).step_by(10) {
        pump(&mut s, &mut gs, now, true);
    }
    assert_eq!(s.player_count(), 64);
    assert!(
        gs.iter()
            .all(|g| g.connected() && g.player_ids().len() == 64)
    );
}

#[test]
fn teleport_before_first_body_enforces_destination_and_applies_heading() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..100).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    let epoch = s.teleport(2, destination(40., 0), 100).unwrap();
    pump(&mut s, &mut gs, 100, false);
    gs[0].complete_movement_reset(epoch);
    gs[0].publish(packed::BODY, body(1000., 0., 0.), 110);
    for now in (110..200).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert!(gs[1].actors[&2].body.latest().is_none());
    gs[0].publish(packed::BODY, body(40., 0., 0.), 200);
    for now in (200..400).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    let mut target = destination(50., 0);
    target.heading = std::f32::consts::FRAC_PI_2;
    s.teleport(2, target, 400).unwrap();
    pump(&mut s, &mut gs, 400, false);
    let body = gs[1].actors[&2]
        .body
        .latest()
        .unwrap()
        .state
        .unpack_body()
        .unwrap();
    assert!((body.root.q[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
    assert!((body.bodies[0].pose.q[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
}

#[test]
fn authorized_teleport_before_first_roster_still_relocates_owner() {
    let mut s = server();
    let mut gs = clients();
    for (index, guest) in gs.iter_mut().enumerate() {
        for packet in guest.service(0) {
            s.receive(10 + index as u64, &packet.data, 0);
        }
    }
    let epoch = s.teleport(2, destination(50., 0), 1).unwrap();
    pump(&mut s, &mut gs, 1, false);
    assert!(gs[0].connected());
    assert_eq!(gs[0].pending_movement_reset().unwrap().epoch, epoch);
    gs[0].complete_movement_reset(epoch);
    pump(&mut s, &mut gs, 101, false);
    assert!(gs[0].pending_movement_reset().is_none());
}

#[test]
fn teleport_resets_shove_sequence_without_replaying_preteleport_requests() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..200).step_by(10) {
        frames(&mut gs, now, 20.);
        pump(&mut s, &mut gs, now, false);
    }
    let old_epoch = batch(&gs[0]).unwrap().epoch;
    gs[0].publish_application(
        dedicated::SHOVE_KEY,
        serde_json::to_vec(&ShoveRequest {
            epoch: old_epoch,
            id: 99,
            target: 3,
        })
        .unwrap(),
        200,
    );
    for now in (200..400).step_by(10) {
        frames(&mut gs, now, 20.);
        pump(&mut s, &mut gs, now, false);
    }
    let mut target = destination(20., 0);
    target.position[2] = -1.2;
    let epoch = s.teleport(2, target, 400).unwrap();
    pump(&mut s, &mut gs, 400, false);
    assert!(batch(&gs[0]).unwrap().effects.is_empty());
    gs[0].complete_movement_reset(epoch);
    gs[0].publish_application(
        dedicated::SHOVE_KEY,
        serde_json::to_vec(&ShoveRequest {
            epoch,
            id: 1,
            target: 3,
        })
        .unwrap(),
        401,
    );
    for now in (410..600).step_by(10) {
        gs[0].publish(packed::BODY, body(20., -1.2, 0.), now);
        gs[1].publish(packed::BODY, body(20., 0., 0.), now);
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
}

#[test]
fn native_respawn_returns_to_server_retained_spawn_and_tracks_trusted_instances() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..200).step_by(10) {
        frames(&mut gs, now, 20.);
        pump(&mut s, &mut gs, now, false);
    }
    let request = |id, epoch| {
        serde_json::to_vec(&dedicated::TeleportRequest {
            epoch,
            id,
            destination: dedicated::RESPAWN_DESTINATION.into(),
        })
        .unwrap()
    };
    let old = gs[0].movement_epoch();
    gs[0].publish_application(dedicated::TELEPORT_KEY, request(1, old), 201);
    for now in (201..400).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    let reset = gs[0].pending_movement_reset().unwrap();
    assert!(reset.epoch > old);
    assert_eq!(reset.destination.position, [0., 1., 0.]);
    gs[0].complete_movement_reset(reset.epoch);
    let epoch = s.teleport(2, destination(80., 9), 500).unwrap();
    pump(&mut s, &mut gs, 500, false);
    gs[0].complete_movement_reset(epoch);
    // Advertising ordinary resources must not forget the trusted recovery
    // point when the server has never mounted a required world.
    s.set_world_spawn(None).unwrap();
    gs[0].publish_application(dedicated::TELEPORT_KEY, request(2, epoch), 800);
    for now in (800..1000).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    let reset = gs[0].pending_movement_reset().unwrap();
    assert_eq!(reset.destination.position, [80., 1., 0.]);
    assert_eq!(reset.destination.instance, 9);
    assert!(
        s.allow_teleport_destination(dedicated::RESPAWN_DESTINATION.into(), destination(999., 0))
            .is_err()
    );
}

#[test]
fn lost_initial_body_is_retried_before_native_recovery_is_authorized() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..100).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    gs[0].publish(packed::BODY, body(10., 0., 0.), 100);
    let dropped = gs[0].service(100);
    assert!(
        dropped
            .iter()
            .any(|p| packed::envelope(&p.data).unwrap().2 == packed::BODY)
    );
    let epoch = gs[0].movement_epoch();
    gs[0].publish_application(
        dedicated::TELEPORT_KEY,
        serde_json::to_vec(&dedicated::TeleportRequest {
            epoch,
            id: 1,
            destination: dedicated::RESPAWN_DESTINATION.into(),
        })
        .unwrap(),
        101,
    );
    pump(&mut s, &mut gs, 101, false);
    assert!(gs[0].pending_movement_reset().is_none());
    gs[0].retry_owner_body();
    for now in (150..300).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        gs[0].pending_movement_reset().unwrap().destination.position,
        [10., 1., 0.]
    );
}

#[test]
fn resource_reload_rotates_movement_epoch_and_rejects_previous_incarnation_body() {
    let mut s = server();
    let mut gs = clients();
    for now in (0..200).step_by(10) {
        frames(&mut gs, now, 20.);
        pump(&mut s, &mut gs, now, false);
    }
    let old_epoch = gs[0].movement_epoch();
    gs[0].publish(packed::BODY, body(5., 0., 0.), 200);
    let stale: Vec<_> = gs[0]
        .service(250)
        .into_iter()
        .filter(|p| packed::envelope(&p.data).unwrap().2 == packed::BODY)
        .collect();
    assert!(!stale.is_empty());
    s.configure_resources(
        "a".repeat(64),
        31031,
        std::collections::BTreeMap::from([("app".into(), 1)]),
    )
    .unwrap();
    let mut resource_clients = [
        skate_net::resources::Client::default(),
        skate_net::resources::Client::default(),
    ];
    for now in (251..900).step_by(10) {
        pump(&mut s, &mut gs, now, false);
        for (guest, wire) in gs.iter_mut().zip(&mut resource_clients) {
            let Some(host) = guest.host_actor() else {
                continue;
            };
            let Some(record) = guest.actors[&host]
                .application
                .get(&skate_net::resources::server_key(guest.local))
            else {
                continue;
            };
            wire.receive(&serde_json::from_slice(&record.value).unwrap())
                .unwrap();
            wire.set_ready(true);
            guest.publish_application(
                skate_net::resources::CLIENT_KEY,
                wire.encode().unwrap(),
                now,
            );
        }
    }
    assert_eq!(s.player_count(), 2);
    assert!(gs[0].movement_epoch() > old_epoch);
    for packet in stale {
        s.receive(10, &packet.data, 900);
    }
    pump(&mut s, &mut gs, 901, false);
    assert!(gs[1].actors[&2].body.latest().is_none());
    gs[0].publish(packed::BODY, body(900., 0., 0.), 910);
    for now in (910..1100).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert!(gs[1].actors[&2].body.latest().is_none(),"A resource reload must retain the accepted movement bound");
    gs[0].publish(packed::BODY, body(0., 0., 0.), 1110);
    for now in (1110..1400).step_by(10) {
        pump(&mut s, &mut gs, now, false);
    }
    assert_eq!(
        gs[1].actors[&2].body.latest().unwrap().state.position()[0],
        0.
    );
}

#[test]
fn required_world_readiness_issues_one_trusted_spawn_and_rejects_other_initial_baselines() {
    let mut s=server();s.set_world_spawn(Some(destination(40.,0))).unwrap();
    s.configure_resources("a".repeat(64),31030,std::collections::BTreeMap::from([("app".into(),1)])).unwrap();
    let mut guests=clients();
    let mut channels=[skate_net::resources::Client::default(),skate_net::resources::Client::default()];
    for now in (0..1200).step_by(10) {
        pump(&mut s,&mut guests,now,false);
        for (guest,channel) in guests.iter_mut().zip(&mut channels) {
            let Some(record)=guest.actors.get(&99).and_then(|a|a.application.get(&skate_net::resources::server_key(guest.local))) else {continue;};
            channel.receive(&serde_json::from_slice(&record.value).unwrap()).unwrap();channel.set_ready(true);
            guest.publish_application(skate_net::resources::CLIENT_KEY,channel.encode().unwrap(),now);
        }
    }
    assert!(s.resource_ready(2));
    let reset=guests[0].pending_movement_reset().expect("validated world must authorize spawn");
    assert_eq!(reset.destination.position,[40.,1.,0.]);
    guests[0].complete_movement_reset(reset.epoch);
    guests[0].publish(packed::BODY,body(900.,0.,0.),1200);
    for now in (1200..1400).step_by(10) {pump(&mut s,&mut guests,now,false);}
    assert!(guests[1].actors[&2].body.latest().is_none());
    assert_eq!(guests[0].movement_epoch(),reset.epoch,"readiness must not repeatedly relocate");
    guests[0].publish(packed::BODY,body(40.,0.,0.),1400);
    for now in (1400..1700).step_by(10) {pump(&mut s,&mut guests,now,false);}
    assert_eq!(guests[1].actors[&2].body.latest().unwrap().state.position(),[40.,1.,0.]);
}

#[test]
fn world_removal_requires_trusted_base_metadata_and_rejects_invalid_fallback() {
    let mut s = server();
    s.set_world_spawn(Some(destination(900., 0))).unwrap();
    assert!(s.set_world_spawn(None).unwrap_err().contains("trusted base-world"));
    assert!(s.set_base_world_spawn(destination(f32::NAN, 0)).is_err());
    assert!(s.set_world_spawn(None).is_err(), "failed fallback configuration must not retire the current world");
    s.set_base_world_spawn(destination(12., 0)).unwrap();
    s.set_world_spawn(None).unwrap();
}

#[test]
fn required_world_removal_approves_base_spawn_after_readiness_without_accepting_old_world_body() {
    fn ready(s: &mut Server, guests: &mut [Session], channels: &mut [skate_net::resources::Client], now: u64) {
        pump(s, guests, now, false);
        for (guest, channel) in guests.iter_mut().zip(channels) {
            let Some(record) = guest.actors.get(&99).and_then(|a| a.application.get(&skate_net::resources::server_key(guest.local))) else { continue; };
            channel.receive(&serde_json::from_slice(&record.value).unwrap()).unwrap();
            channel.set_ready(true);
            guest.publish_application(skate_net::resources::CLIENT_KEY, channel.encode().unwrap(), now);
        }
    }
    let mut s = server();
    s.set_base_world_spawn(destination(12., 0)).unwrap();
    s.set_world_spawn(Some(destination(900., 0))).unwrap();
    s.configure_resources("a".repeat(64), 31030, std::collections::BTreeMap::from([("park".into(), 1)])).unwrap();
    let mut guests = clients();
    let mut channels = [skate_net::resources::Client::default(), skate_net::resources::Client::default()];
    for now in (0..400).step_by(10) { ready(&mut s, &mut guests, &mut channels, now); }
    let initial = guests[0].pending_movement_reset().unwrap();
    assert_eq!(initial.destination.position, [900., 1., 0.]);
    guests[0].complete_movement_reset(initial.epoch);
    guests[0].publish(packed::BODY, body(900., 0., 0.), 400);
    for now in (400..700).step_by(10) { ready(&mut s, &mut guests, &mut channels, now); }
    assert_eq!(s.player_observations().as_array().unwrap().iter().find(|p| p["id"] == "2").unwrap()["position"], serde_json::json!([900., 1., 0.]));
    let private = s.teleport(2, destination(900., 7), 700).unwrap();
    for now in (700..1000).step_by(10) { ready(&mut s, &mut guests, &mut channels, now); }
    guests[0].complete_movement_reset(private);
    guests[0].publish(packed::BODY, body(900., 0., 0.), 1000);
    ready(&mut s, &mut guests, &mut channels, 1000);
    s.set_world_spawn(None).unwrap();
    s.set_world_spawn(None).unwrap(); // Re-advertisement cannot erase pending fallback.
    s.configure_resources("b".repeat(64), 31030, std::collections::BTreeMap::new()).unwrap();
    assert!(!s.resource_ready(2));
    for now in (1010..1500).step_by(10) { ready(&mut s, &mut guests, &mut channels, now); }
    let reset = guests[0].pending_movement_reset().expect("world removal must approve a trusted base-world reset");
    assert!(reset.epoch > private);
    assert_eq!(reset.destination.position, [12., 1., 0.]);
    assert_eq!(reset.destination.instance, 7);
    guests[0].complete_movement_reset(reset.epoch);
    guests[0].publish(packed::BODY, body(900., 0., 0.), 1500);
    for now in (1500..1700).step_by(10) { ready(&mut s, &mut guests, &mut channels, now); }
    assert!(s.competition_players().iter().all(|p| p.actor != 2), "old-world body must not bypass the new baseline guard");
    guests[0].publish(packed::BODY, body(12., 0., 0.), 1700);
    for now in (1700..1900).step_by(10) { ready(&mut s, &mut guests, &mut channels, now); }
    assert_eq!(s.player_observations().as_array().unwrap().iter().find(|p| p["id"] == "2").unwrap()["position"], serde_json::json!([12., 1., 0.]));
    assert_eq!(guests[0].movement_epoch(), reset.epoch, "fallback reset occurs once");
}
