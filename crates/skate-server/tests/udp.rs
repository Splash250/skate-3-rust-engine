use skate_net::{
    Body, Bone, Pose,
    dedicated::{self, Gameplay, PlayerMode},
    lobby::{Info, Session},
    packed::{self, BodyState, Packed, PoseState},
};
use skate_server::{Host, Map, Options};
use std::{
    net::{SocketAddr, UdpSocket},
    thread,
    time::{Duration, Instant},
};

struct Client {
    socket: UdpSocket,
    session: Session,
}

fn client(id: u64, map: u64) -> Client {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    Client {
        socket,
        session: Session::dedicated_client(
            dedicated::SESSION,
            Info {
                id,
                map,
                rig: 22,
                physics: 33,
                appearance: 44,
            },
            1,
        ),
    }
}

fn pump(
    host: &mut Host,
    clients: &mut [Client],
    started: Instant,
    until: impl Fn(&[Client]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(4);
    let address = host.local_addr().unwrap();
    while Instant::now() < deadline {
        let now = started.elapsed().as_millis() as u64;
        for client in clients.iter_mut() {
            for outgoing in client.session.service(now) {
                client.socket.send_to(&outgoing.data, address).unwrap();
            }
        }
        host.step().unwrap();
        for client in clients.iter_mut() {
            let mut bytes = [0; 1500];
            loop {
                match client.socket.recv_from(&mut bytes) {
                    Ok((len, from)) => {
                        assert_eq!(from, address);
                        client.session.receive(1, &bytes[..len], now);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("{error}"),
                }
            }
        }
        if until(clients) {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!(
        "UDP condition did not become true; notices: {:?}",
        clients
            .iter()
            .map(|c| &c.session.notice)
            .collect::<Vec<_>>()
    );
}

#[test]
fn real_udp_discovery_queue_cancellation_and_next_waiter_admission() {
    let mut host = Host::bind(Options {
        accounts: None, operations: None, resources: None,
        bind: "127.0.0.1:0".parse().unwrap(), session: dedicated::SESSION,
        max_players: 1, map: Map::TestWorld,
    }).unwrap();
    let address = host.local_addr().unwrap();
    let query = UdpSocket::bind("127.0.0.1:0").unwrap();
    query.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    query.send_to(&skate_net::discovery::query(77), address).unwrap();
    host.step().unwrap();
    let mut bytes = [0;1500];
    let (len,from) = query.recv_from(&mut bytes).unwrap();
    assert_eq!(from,address);
    let info = skate_net::discovery::parse_response(&bytes[..len],77).unwrap();
    assert!(info.compatible());assert_eq!(info.session,dedicated::SESSION);
    assert_eq!(info.map_fingerprint,host.map_fingerprint());assert_eq!(info.capacity,1);
    assert_eq!(info.players,0);assert!(!info.accounts_required);

    let started=Instant::now();let map=host.map_fingerprint();
    let mut clients=vec![client(501,map)];
    pump(&mut host,&mut clients,started,|clients|clients[0].session.connected());
    clients.push(client(502,map));clients.push(client(503,map));
    pump(&mut host,&mut clients,started,|clients|clients[2].session.join_status.as_ref().is_some_and(|s|s.state=="queued" && s.position==2));
    assert_eq!(host.player_count(),1);
    clients[1].session.cancel_join();
    pump(&mut host,&mut clients,started,|clients|clients[2].session.join_status.as_ref().is_some_and(|s|s.state=="queued" && s.position==1));
    clients[0].session.cancel_join();
    pump(&mut host,&mut clients,started,|clients|clients[2].session.connected());
    assert_eq!(host.player_count(),1);
    assert!(!clients[1].session.connected());
    assert!(clients[2].session.join_status.is_none());
}

#[test]
fn real_udp_incompatible_physics_keeps_terminal_rejection_reason() {
    let mut host=Host::bind(Options {accounts:None,operations:None,resources:None,
        bind:"127.0.0.1:0".parse().unwrap(),session:dedicated::SESSION,max_players:2,map:Map::TestWorld}).unwrap();
    let started=Instant::now();let map=host.map_fingerprint();let mut clients=vec![client(601,map)];
    pump(&mut host,&mut clients,started,|clients|clients[0].session.connected());
    let mut incompatible=client(602,map);
    incompatible.session=Session::dedicated_client(dedicated::SESSION,Info{id:602,map,rig:999,physics:33,appearance:44},1);
    clients.push(incompatible);
    pump(&mut host,&mut clients,started,|clients|clients[1].session.join_status.as_ref().is_some_and(|s|s.terminal()&&s.reason.contains("Physics")));
    assert!(!clients[1].session.connected());assert_eq!(host.player_count(),1);
    let outgoing=clients[1].session.service(10_000);
    assert!(!outgoing.iter().any(|p|packed::envelope(&p.data).is_some_and(|e|e.2==skate_net::lobby::DEDICATED_HELLO)));
}

#[test]
fn real_udp_host_syncs_players_bodies_pose_tricks_and_departure_without_game_assets() {
    let mut host = Host::bind(Options {
        accounts: None,
        operations: None,
        resources: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        session: dedicated::SESSION,
        max_players: 16,
        map: Map::TestWorld,
    })
    .unwrap();
    let started = Instant::now();
    let map = skate_net::hash(b"skate-test-world-v1");
    let mut clients = vec![client(20, map), client(30, map)];
    pump(&mut host, &mut clients, started, |clients| {
        clients.iter().all(|c| c.session.player_ids().len() == 2)
    });
    assert_eq!(host.player_count(), 2);
    let root = Pose {
        p: [5., 2., 0.],
        q: [0., 0., 0., 1.],
    };
    let body = BodyState {
        root,
        enabled: (1 << 33) - 1,
        bodies: vec![
            Body {
                pose: root,
                velocity: [4., 0., 0.],
                angular: [0., 2., 0.]
            };
            33
        ],
    };
    let pose = PoseState {
        root,
        bones: vec![Bone {
            index: 3,
            pose: root,
        }],
    };
    let now = started.elapsed().as_millis() as u64;
    clients[0]
        .session
        .publish(packed::BODY, Packed::body(&body).unwrap(), now);
    clients[0]
        .session
        .publish(packed::POSE, Packed::pose(&pose).unwrap(), now);
    let gameplay = Gameplay {
        mode: PlayerMode::Skating,
        suspended: false,
        trick: "Kickflip".into(),
        trick_seq: 4,
        landed_seq: 2,
        landed_trick: "Ollie".into(),
        bail_seq: 1,
        sequence_score: 300,
        line_score: 900,
    };
    assert!(clients[0].session.publish_application(
        dedicated::GAMEPLAY_KEY,
        serde_json::to_vec(&gameplay).unwrap(),
        now
    ));
    pump(&mut host, &mut clients, started, |clients| {
        clients[1].session.actors.get(&20).is_some_and(|actor| {
            actor.body.latest().is_some()
                && actor.pose.latest().is_some()
                && actor.application.contains_key(dedicated::GAMEPLAY_KEY)
        })
    });
    let remote = &clients[1].session.actors[&20];
    let received = remote.body.latest().unwrap().state.unpack_body().unwrap();
    assert_eq!(received.root.p, [5., 2., 0.]);
    assert_eq!(received.bodies.len(), 33);
    assert_eq!(received.bodies[0].velocity, [4., 0., 0.]);
    assert_eq!(
        remote
            .pose
            .latest()
            .unwrap()
            .state
            .unpack_pose()
            .unwrap()
            .bones[0]
            .index,
        3
    );
    let received: Gameplay =
        serde_json::from_slice(&remote.application[dedicated::GAMEPLAY_KEY].value).unwrap();
    assert_eq!(received.trick, "Kickflip");
    assert_eq!(received.landed_seq, 2);
    for packet in clients[0].session.goodbye() {
        clients[0]
            .socket
            .send_to(&packet.data, host.local_addr().unwrap())
            .unwrap();
    }
    clients.remove(0);
    pump(&mut host, &mut clients, started, |clients| {
        clients[0].session.player_ids() == vec![30]
    });
    assert_eq!(host.player_count(), 1);
}

#[test]
fn oversized_and_unknown_datagrams_do_not_prevent_valid_admission() {
    let mut host = Host::bind(Options {
        accounts: None,
        operations: None,
        resources: None,
        bind: SocketAddr::from(([127, 0, 0, 1], 0)),
        session: dedicated::SESSION,
        max_players: 1,
        map: Map::TestWorld,
    })
    .unwrap();
    let outsider = UdpSocket::bind("127.0.0.1:0").unwrap();
    for bytes in [vec![0; 4096], vec![0; 20], vec![]] {
        outsider
            .send_to(&bytes, host.local_addr().unwrap())
            .unwrap();
    }
    host.step().unwrap();
    assert_eq!(host.player_count(), 0);
    let mut clients = vec![client(20, skate_net::hash(b"skate-test-world-v1"))];
    pump(&mut host, &mut clients, Instant::now(), |clients| {
        clients[0].session.connected()
    });
    assert_eq!(host.player_count(), 1);
}

fn effect_batch(client: &Client) -> Option<dedicated::EffectBatch> {
    let host = client.session.host_actor()?;
    let record = client
        .session
        .actors
        .get(&host)?
        .application
        .get(&dedicated::effects_key(client.session.local))?;
    serde_json::from_slice(&record.value).ok()
}

fn publish_body(client: &mut Client, p: [f32; 3], velocity: [f32; 3], now: u64) {
    let root = Pose {
        p,
        q: [0., 0., 0., 1.],
    };
    let body = BodyState {
        root,
        enabled: (1 << 33) - 1,
        bodies: vec![
            Body {
                pose: root,
                velocity,
                angular: [0.; 3]
            };
            33
        ],
    };
    client
        .session
        .publish(packed::BODY, Packed::body(&body).unwrap(), now);
}

#[test]
fn real_udp_shoves_and_collisions_are_delivered_as_server_effects() {
    let mut host = Host::bind(Options {
        accounts: None,
        operations: None,
        resources: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        session: dedicated::SESSION,
        max_players: 2,
        map: Map::TestWorld,
    })
    .unwrap();
    let started = Instant::now();
    let map = skate_net::hash(b"skate-test-world-v1");
    let mut clients = vec![client(20, map), client(30, map)];
    pump(&mut host, &mut clients, started, |clients| {
        clients
            .iter()
            .all(|c| c.session.player_ids().len() == 2 && effect_batch(c).is_some())
    });
    let mut applied = [
        dedicated::ClientEffects::default(),
        dedicated::ClientEffects::default(),
    ];
    for (client, applied) in clients.iter().zip(&mut applied) {
        applied.consume(&effect_batch(client).unwrap());
    }
    let now = started.elapsed().as_millis() as u64;
    publish_body(&mut clients[0], [0., 2., 0.], [0.; 3], now);
    publish_body(&mut clients[1], [0., 2., 1.5], [0.; 3], now);
    // Let both fresh body samples arrive before the one-shot action request.
    pump(&mut host, &mut clients, started, |clients| {
        clients.iter().all(|c| {
            c.session
                .actors
                .values()
                .filter(|a| a.body.latest().is_some())
                .count()
                == 2
        })
    });
    let request = dedicated::ShoveRequest {
        id: 1,
        target: 30,
        epoch: applied[0].ack().epoch,
    };
    assert!(clients[0].session.publish_application(
        dedicated::SHOVE_KEY,
        serde_json::to_vec(&request).unwrap(),
        started.elapsed().as_millis() as u64
    ));
    pump(&mut host, &mut clients, started, |clients| {
        clients
            .iter()
            .all(|c| effect_batch(c).is_some_and(|b| !b.effects.is_empty()))
    });
    for (index, client) in clients.iter_mut().enumerate() {
        let batch = effect_batch(client).unwrap();
        let effects = applied[index].consume(&batch);
        assert_eq!(effects.len(), 1);
        assert_eq!(
            effects[0].kind,
            if index == 0 {
                dedicated::EffectKind::AttackAccepted
            } else {
                dedicated::EffectKind::Shove
            }
        );
        assert_eq!(effects[0].target, client.session.local);
        assert!(
            applied[index].consume(&batch).is_empty(),
            "Duplicate packet applied a second hit"
        );
        assert!(client.session.publish_application(
            dedicated::EFFECT_ACK_KEY,
            serde_json::to_vec(&applied[index].ack()).unwrap(),
            started.elapsed().as_millis() as u64
        ));
    }
    pump(&mut host, &mut clients, started, |clients| {
        clients
            .iter()
            .all(|c| effect_batch(c).is_some_and(|b| b.effects.is_empty()))
    });
    let now = started.elapsed().as_millis() as u64;
    publish_body(&mut clients[0], [0., 2., 0.], [2., 0., 0.], now);
    publish_body(&mut clients[1], [0.5, 2., 0.], [-2., 0., 0.], now);
    pump(&mut host, &mut clients, started, |clients| {
        clients.iter().all(|c| {
            effect_batch(c).is_some_and(|b| {
                b.effects
                    .iter()
                    .any(|e| e.kind == dedicated::EffectKind::Collision)
            })
        })
    });
    let a = applied[0].consume(&effect_batch(&clients[0]).unwrap());
    let b = applied[1].consume(&effect_batch(&clients[1]).unwrap());
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert!(a[0].delta_velocity.iter().any(|v| *v != 0.));
    for axis in 0..3 {
        assert!((a[0].delta_velocity[axis] + b[0].delta_velocity[axis]).abs() < 0.0001);
    }
}
