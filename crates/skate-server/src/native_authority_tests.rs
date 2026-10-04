use super::*;

struct Run {
    server: Server,
    client: skate_net::lobby::Session,
    native: NativeAuthority,
    now: u64,
}
impl Run {
    fn new(config: Config) -> Self {
        use skate_net::lobby::{Info, Session};
        let server = Server::new(skate_net::dedicated::Config {
            session: 7,
            server_id: 99,
            map: 1,
            max_players: 4,
        })
        .unwrap();
        let mut client = Session::dedicated_client(
            7,
            Info {
                id: 2,
                map: 1,
                rig: 3,
                physics: 4,
                appearance: 5,
            },
            99,
        );
        client.set_loopback(true);
        let mut native = NativeAuthority::new(Some(config));
        let bytes = include_bytes!("../../../resources/community-park/park.skate").to_vec();
        let map = skate_data::skate_map::SkateMap::parse(&bytes).unwrap();
        native.world = Some(World {
            revision: "test-world-identity".into(),
            bytes: Arc::new(bytes),
            spawn: map.spawn,
            heading: map.heading,
        });
        native.sync_resources(BTreeMap::from([("rules".into(), 1)]));
        let mut run = Self {
            server,
            client,
            native,
            now: 0,
        };
        for _ in 0..15 {
            run.tick();
        }
        assert_eq!(run.server.player_count(), 1);
        run
    }
    fn tick(&mut self) -> Vec<OwnedEvent> {
        self.pump();
        self.native.step(&mut self.server, &Default::default())
    }
    fn pump(&mut self) {
        use skate_net::{
            Body, Pose,
            packed::{self, BodyState, Packed},
        };
        self.now += 20;
        if let Some(reset) = self.client.pending_movement_reset() {
            self.client.complete_movement_reset(reset.epoch);
        }
        let pose = Pose {
            p: [0., 1., 0.],
            q: [0., 0., 0., 1.],
        };
        self.client.publish(
            packed::BODY,
            Packed::body(&BodyState {
                root: pose,
                enabled: (1 << 33) - 1,
                bodies: vec![
                    Body {
                        pose,
                        velocity: [0.; 3],
                        angular: [0.; 3]
                    };
                    33
                ],
            })
            .unwrap(),
            self.now,
        );
        for packet in self.client.service(self.now) {
            self.server.receive(11, &packet.data, self.now);
        }
        for packet in self.server.service(self.now) {
            self.client.receive(99, &packet.data, self.now);
        }
    }
    fn start(&mut self, ticks: u64) {
        self.native
            .command(
                "rules",
                1,
                json!({"kind":"native_start","player":"2","ticks":ticks}),
                &mut self.server,
                &Default::default(),
            )
            .unwrap();
    }
    fn wait_running(&mut self) {
        let until = Instant::now() + Duration::from_secs(30);
        while self.native.attempts.get(&2).unwrap().started_ms.is_none() {
            let events = self.tick();
            assert!(events.is_empty(), "native startup failed: {:?}", events);
            assert!(Instant::now() < until, "native startup timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(unix)]
#[test]
fn retirement_disconnect_and_instance_intrusion_cancel_before_worker_results() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::create().unwrap();
    let executable = scratch.0.join("waiting-worker");
    std::fs::write(
        &executable,
        b"#!/usr/bin/env python3\nimport sys,time\nsys.stdin.readline()\ntime.sleep(30)\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    for reason in [
        "resource_retired",
        "world_changed",
        "disconnect",
        "shared_geometry",
        "peer_join",
        "slow_drip",
        "late_first_input",
    ] {
        let mut run = Run::new(Config {
            executable: executable.clone(),
            assets: scratch.0.clone(),
            max_workers: 1,
        });
        run.start(60);
        let mut blocked = std::collections::BTreeSet::new();
        match reason {
            "resource_retired" => run
                .native
                .sync_resources(BTreeMap::from([("rules".into(), 2)])),
            "world_changed" => run.native.world.as_mut().unwrap().revision = "new-world".into(),
            "disconnect" => assert!(run.server.kick(2, run.now)),
            "shared_geometry" => {
                blocked.insert(0);
            }
            "slow_drip" => {
                // A recent input cannot extend a one-second attempt indefinitely.
                // No actual scoring is substituted: the silent worker is only
                // a probe of the authority's host-clock/retirement boundary.
                while run.now <= 21_000 {
                    run.pump();
                }
                let attempt = run.native.attempts.get_mut(&2).unwrap();
                attempt.started_ms = Some(0);
                attempt.last_input = run.now;
            }
            "late_first_input" => {
                while run.now <= 1_000 {
                    run.pump();
                }
                let attempt = run.native.attempts.get_mut(&2).unwrap();
                attempt.started_ms = Some(0);
                attempt.last_reply = Instant::now() - Duration::from_secs(3);
                let epoch = attempt.log.admission().epoch;
                assert!(
                    run.client.publish_application(
                        skate_net::native_authority::INPUT_KEY,
                        skate_net::native_authority::InputPacket {
                            inputs: (1..=32)
                                .map(|tick| skate_net::native_authority::Input {
                                    epoch,
                                    tick,
                                    actions: [0.; 18],
                                })
                                .collect(),
                        }
                        .encode()
                        .unwrap(),
                        run.now,
                    )
                );
                run.pump();
                assert!(run.native.step(&mut run.server, &blocked).is_empty());
                assert_eq!(run.native.attempts[&2].log.len(), 32);
                assert_eq!(
                    run.native.attempts[&2].sent, 4,
                    "in-flight work must fit reply capacity even with a fast worker"
                );
                assert!(
                    run.native.step(&mut run.server, &blocked).is_empty(),
                    "new work deserves a fresh processing deadline after an idle worker"
                );
                assert!(run.native.active(2));
                // Then retire by generation so the common reap checks still run.
                run.native.sync_resources(BTreeMap::new());
            }
            "peer_join" => {
                let mut peer = skate_net::lobby::Session::dedicated_client(
                    7,
                    skate_net::lobby::Info {
                        id: 3,
                        map: 1,
                        rig: 3,
                        physics: 4,
                        appearance: 5,
                    },
                    99,
                );
                peer.set_loopback(true);
                for _ in 0..15 {
                    run.now += 20;
                    for packet in peer.service(run.now) {
                        run.server.receive(12, &packet.data, run.now);
                    }
                    for packet in run.server.service(run.now) {
                        peer.receive(99, &packet.data, run.now);
                    }
                }
                assert_eq!(run.server.player_count(), 2);
            }
            _ => unreachable!(),
        }
        let results = run.native.step(&mut run.server, &blocked);
        assert!(!run.native.active(2), "{reason}");
        if matches!(reason, "resource_retired" | "late_first_input") {
            assert!(
                results.is_empty(),
                "retired generation must not receive callbacks"
            );
        } else {
            assert_eq!(results.len(), 1, "{reason}");
            assert_eq!(
                results[0].value["kind"],
                if reason == "slow_drip" {
                    "rejected"
                } else {
                    "cancelled"
                },
                "{reason}"
            );
            assert_eq!(results[0].value["score"], 0, "{reason}");
            if reason == "slow_drip" {
                assert_eq!(results[0].value["reason"], "native_attempt_deadline");
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while run.native.workers.load(Ordering::Acquire) != 0 {
            assert!(Instant::now() < deadline, "worker leaked after {reason}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
#[ignore = "requires freshly built SKATE_NATIVE_EXE and owned SKATE3_ASSET_ROOT"]
fn actual_native_worker_ignores_reported_scores_and_retires_epochs() {
    use skate_net::native_authority::{INPUT_KEY, Input, InputPacket};
    let config = Config {
        executable: PathBuf::from(std::env::var_os("SKATE_NATIVE_EXE").expect("SKATE_NATIVE_EXE")),
        assets: PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT")),
        max_workers: 1,
    };
    let mut run = Run::new(config);
    run.start(300);
    run.wait_running();
    let epoch = run.native.attempts[&2].log.admission().epoch;
    let mut forged = skate_net::dedicated::Gameplay::default();
    forged.line_score = 1_000_000;
    forged.sequence_score = 1_000_000;
    forged.landed_seq = 999;
    run.client.publish_application(
        skate_net::dedicated::GAMEPLAY_KEY,
        serde_json::to_vec(&forged).unwrap(),
        run.now,
    );
    let until = Instant::now() + Duration::from_secs(20);
    let outcome = loop {
        let Some(attempt) = run.native.attempts.get(&2) else {
            panic!("attempt ended without result");
        };
        let tick = attempt.log.len() as u64 + 1;
        if tick <= 300 {
            let mut actions = [0.; 18];
            if (120..150).contains(&tick) {
                actions[4] = -1.;
            }
            if (150..153).contains(&tick) {
                actions[3] = -0.89442;
                actions[4] = 0.44722;
            }
            let input = Input {
                epoch,
                tick,
                actions,
            };
            run.client.publish_application(
                INPUT_KEY,
                InputPacket {
                    inputs: vec![input],
                }
                .encode()
                .unwrap(),
                run.now,
            );
        }
        let results = run.tick();
        if let Some(result) = results.into_iter().next() {
            break result;
        }
        assert!(Instant::now() < until, "native result timeout");
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(outcome.value["kind"], "completed", "{}", outcome.value);
    assert_eq!(outcome.value["ticks"], 300);
    assert_eq!(outcome.value["verified_rules"], "native-input-v1");
    assert_eq!(outcome.value["score"]["awarded"], 22.0);
    // The map's initial native reset publishes a zero reward before the
    // heelflip. Publication count includes that event; the awarded sum is22.
    assert_eq!(outcome.value["score"]["publications"], 2);
    assert_ne!(outcome.value["score"]["line"], json!(1_000_000.0));
    assert_ne!(outcome.value["score"]["landing_seq"], 999);
    assert_eq!(
        outcome.value["score"]["landing_seq"], 1,
        "{}",
        outcome.value
    );
    assert_eq!(
        outcome.value["score"]["sequence"], 22.0,
        "{}",
        outcome.value
    );
    while run.native.workers.load(Ordering::Acquire) != 0 {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    run.start(90);
    run.wait_running();
    let before = run.server.movement_epoch_of(2).unwrap();
    let new = run
        .server
        .teleport_now(
            2,
            TeleportDestination {
                position: [5., 1., 0.],
                heading: 0.,
                velocity: [0.; 3],
                instance: 7,
            },
        )
        .unwrap();
    assert_ne!(before, new);
    let results = run.tick();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].value["kind"], "cancelled");
    assert_eq!(run.server.movement_epoch_of(2), Some(new));
    assert!(!run.native.active(2));
}

#[test]
#[ignore = "requires freshly built SKATE_NATIVE_EXE and owned SKATE3_ASSET_ROOT"]
fn actual_native_worker_completes_full_length_with_delayed_lossy_input_windows() {
    use skate_net::native_authority::{
        INPUT_KEY, Input, InputPacket, MAX_PACKET_INPUTS, state_key,
    };
    let mut run = Run::new(Config {
        executable: PathBuf::from(std::env::var_os("SKATE_NATIVE_EXE").expect("SKATE_NATIVE_EXE")),
        assets: PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT")),
        max_workers: 1,
    });
    run.start(MAX_TICKS);
    run.wait_running();
    let admission = run.native.attempts[&2].log.admission();
    let started = run.native.attempts[&2].started_ms.unwrap();
    let mut inputs = Vec::new();
    let mut acknowledged = 0;
    // Deterministic simulated-time delivery of real session datagrams. This is
    // an in-process link model, not a WAN or physical-device measurement.
    let mut packets: Vec<(bool, u64, u64, Vec<u8>)> = Vec::new();
    let mut serial = 0u64;
    let mut dropped = 0;
    let mut reordered = 0;
    let mut newest = [0; 2];
    let mut peak_packets = 0;
    let mut peak_lead = 0;
    let wall_deadline = Instant::now() + Duration::from_secs(60);
    let outcome = loop {
        run.now += 10;
        if let Some(record) = run.client.actors[&99].application.get(&state_key(2)) {
            let state: State = serde_json::from_slice(&record.value).unwrap();
            if state.admission == admission {
                acknowledged = acknowledged.max(state.tick);
            }
        }
        let desired = ((run.now - started) * 60 / 1_000)
            .min(MAX_TICKS)
            .min(acknowledged + 120);
        while (inputs.len() as u64) < desired {
            let tick = inputs.len() as u64 + 1;
            let mut actions = [0.; 18];
            if (120..150).contains(&tick) {
                actions[4] = -1.;
            }
            if (150..153).contains(&tick) {
                actions[3] = -0.89442;
                actions[4] = 0.44722;
            }
            inputs.push(Input {
                epoch: admission.epoch,
                tick,
                actions,
            });
        }
        peak_lead = peak_lead.max(inputs.len() as u64 - acknowledged);
        let first = acknowledged as usize;
        let end = inputs.len().min(first + MAX_PACKET_INPUTS);
        if first < end {
            assert!(
                run.client.publish_application(
                    INPUT_KEY,
                    InputPacket {
                        inputs: inputs[first..end].to_vec()
                    }
                    .encode()
                    .unwrap(),
                    run.now
                )
            );
        }
        let mut enqueue = |to_server: bool, data: Vec<u8>, packets: &mut Vec<_>| {
            serial += 1;
            if serial % 97 == 0 {
                dropped += 1;
                return;
            }
            let delay = 25 + serial * 17 % 51;
            packets.push((to_server, run.now + delay, serial, data));
        };
        for packet in run.client.service(run.now) {
            enqueue(true, packet.data, &mut packets);
        }
        for (to_server, due, sequence, data) in std::mem::take(&mut packets) {
            if due > run.now {
                packets.push((to_server, due, sequence, data));
                continue;
            }
            let side = usize::from(to_server);
            if sequence < newest[side] {
                reordered += 1;
            }
            newest[side] = newest[side].max(sequence);
            if to_server {
                run.server.receive(11, &data, run.now);
            } else {
                run.client.receive(99, &data, run.now);
            }
        }
        for packet in run.server.service(run.now) {
            enqueue(false, packet.data, &mut packets);
        }
        peak_packets = peak_packets.max(packets.len());
        assert!(packets.len() < 512);
        let events = run.native.step(&mut run.server, &Default::default());
        if let Some(event) = events.into_iter().next() {
            break event;
        }
        assert!(
            run.now - started < 81_000,
            "native transport failed to make progress"
        );
        assert!(Instant::now() < wall_deadline, "worker wall-clock timeout");
        std::thread::sleep(Duration::from_millis(4));
    };
    assert_eq!(outcome.value["kind"], "completed", "{}", outcome.value);
    assert_eq!(outcome.value["ticks"], MAX_TICKS);
    assert_eq!(outcome.value["score"]["awarded"], 22.0);
    assert!(dropped > 0 && reordered > 0);
    assert!(peak_lead <= 120);
    eprintln!(
        "NATIVE_IMPAIRED ticks={} simulated_elapsed_ms={} dropped={} reordered={} peak_packets={} peak_prediction_lead={}",
        MAX_TICKS,
        run.now - started,
        dropped,
        reordered,
        peak_packets,
        peak_lead
    );
}
