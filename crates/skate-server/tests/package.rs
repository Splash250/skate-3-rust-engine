//! Exercise the shipped executable, not an in-process Host. CI can point this
//! at an extracted package with SKATE_SERVER_EXE; ordinary tests use Cargo's bin.
use skate_net::{
    Body, Bone, Pose,
    dedicated::{self, ClientEffects, EffectBatch, EffectKind, Gameplay, ShoveRequest},
    lobby::{Info, Session},
    packed::{self, BodyState, Packed, PoseState},
};
use std::{
    io::{BufRead, BufReader},
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Running {
    child: Child,
    reader: Option<JoinHandle<()>>,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skate-server-package-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn launch(directory: &Scratch) -> (Running, SocketAddr) {
    let executable = std::env::var_os("SKATE_SERVER_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_skate-server")))
        .canonicalize()
        .expect("SKATE_SERVER_EXE must name an existing executable");
    let mut child = Command::new(executable)
        .args([
            "--test-world",
            "--bind",
            "127.0.0.1:0",
            "--max-players",
            "2",
        ])
        .current_dir(&directory.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Packaged server did not launch from an empty working directory");
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let result = reader.read_line(&mut line);
        let _ = send.send((result, line));
        // Drain status logs throughout the test, including during shutdown.
        for line in reader.lines() {
            if line.is_err() {
                break;
            }
        }
    });
    let running = Running {
        child,
        reader: Some(reader),
    };
    let (result, line) = receive
        .recv_timeout(Duration::from_secs(10))
        .expect("Packaged server did not announce a listening address");
    result.unwrap();
    let address = line
        .strip_prefix("Dedicated server listening on ")
        .and_then(|line| line.split_once(" | "))
        .expect("Unexpected packaged-server startup message")
        .0
        .parse::<SocketAddr>()
        .unwrap();
    assert!(address.ip().is_loopback());
    assert_ne!(address.port(), 0);
    (running, address)
}

struct Client {
    socket: UdpSocket,
    session: Session,
    position: [f32; 3],
    velocity: [f32; 3],
    published: Option<u64>,
}
impl Client {
    fn new(id: u64, position: [f32; 3]) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        Self {
            socket,
            session: Session::dedicated_client(
                dedicated::SESSION,
                Info {
                    id,
                    map: skate_net::hash(b"skate-test-world-v1"),
                    rig: 22,
                    physics: 33,
                    appearance: 0,
                },
                1,
            ),
            position,
            velocity: [0.; 3],
            published: None,
        }
    }

    fn publish(&mut self, now: u64) {
        if self.published.is_some_and(|at| now.saturating_sub(at) < 50) {
            return;
        }
        let root = Pose {
            p: self.position,
            q: [0., 0., 0., 1.],
        };
        let body = BodyState {
            root,
            enabled: (1 << 33) - 1,
            bodies: vec![
                Body {
                    pose: root,
                    velocity: self.velocity,
                    angular: [0.; 3]
                };
                33
            ],
        };
        let pose = PoseState {
            root,
            bones: vec![Bone {
                index: 7,
                pose: root,
            }],
        };
        self.session
            .publish(packed::BODY, Packed::body(&body).unwrap(), now);
        self.session
            .publish(packed::POSE, Packed::pose(&pose).unwrap(), now);
        self.published = Some(now);
    }

    fn effects(&self) -> Option<EffectBatch> {
        let server = self.session.host_actor()?;
        let record = self
            .session
            .actors
            .get(&server)?
            .application
            .get(&dedicated::effects_key(self.session.local))?;
        serde_json::from_slice(&record.value).ok()
    }
}

fn pump(
    server: &mut Running,
    address: SocketAddr,
    clients: &mut [Client],
    start: Instant,
    condition: impl Fn(&[Client]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        assert!(
            server.child.try_wait().unwrap().is_none(),
            "Packaged server exited during UDP smoke test"
        );
        let now = start.elapsed().as_millis() as u64;
        for client in clients.iter_mut() {
            client.publish(now);
            for packet in client.session.service(now) {
                client.socket.send_to(&packet.data, address).unwrap();
            }
            let mut bytes = [0; 1500];
            for _ in 0..4096 {
                match client.socket.recv_from(&mut bytes) {
                    Ok((len, from)) => {
                        assert_eq!(from, address);
                        client.session.receive(1, &bytes[..len], now);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("Package smoke UDP receive: {error}"),
                }
            }
        }
        if condition(clients) {
            return;
        }
        thread::sleep(Duration::from_millis(2));
    }
    panic!(
        "Packaged-server UDP condition timed out; clients: {:?}",
        clients
            .iter()
            .map(|c| (&c.session.notice, c.session.player_ids()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn packaged_server_routes_two_clients_and_authorizes_shared_effects_from_empty_cwd() {
    let directory = Scratch::new();
    let (mut server, address) = launch(&directory);
    let start = Instant::now();
    let mut clients = vec![
        Client::new(20, [0., 2., 0.]),
        Client::new(30, [0., 2., 1.5]),
    ];
    for client in &mut clients {
        let gameplay = Gameplay {
            trick: "Kickflip".into(),
            trick_seq: 4,
            landed_seq: 2,
            landed_trick: "Ollie".into(),
            sequence_score: 300,
            line_score: 900,
            ..Default::default()
        };
        assert!(client.session.publish_application(
            dedicated::GAMEPLAY_KEY,
            serde_json::to_vec(&gameplay).unwrap(),
            0
        ));
    }
    pump(&mut server, address, &mut clients, start, |clients| {
        clients.iter().all(|client| {
            let remote = if client.session.local == 20 { 30 } else { 20 };
            client.session.player_ids() == vec![20, 30]
                && client.effects().is_some()
                && client.session.actors.get(&remote).is_some_and(|actor| {
                    actor.body.latest().is_some()
                        && actor.pose.latest().is_some()
                        && actor.application.contains_key(dedicated::GAMEPLAY_KEY)
                })
        })
    });
    for (client, remote_id, position) in [
        (&clients[0], 30, [0., 2., 1.5]),
        (&clients[1], 20, [0., 2., 0.]),
    ] {
        let remote = &client.session.actors[&remote_id];
        let body = remote.body.latest().unwrap().state.unpack_body().unwrap();
        assert_eq!(body.bodies.len(), 33);
        assert_eq!(body.root.p, position);
        assert_eq!(body.bodies[32].pose.p, position);
        let pose = remote.pose.latest().unwrap().state.unpack_pose().unwrap();
        assert_eq!(pose.bones[0].index, 7);
        let gameplay: Gameplay =
            serde_json::from_slice(&remote.application[dedicated::GAMEPLAY_KEY].value).unwrap();
        assert_eq!(gameplay.trick, "Kickflip");
        assert_eq!(gameplay.landed_seq, 2);
        assert_eq!(gameplay.line_score, 900);
    }

    let mut applied = [ClientEffects::default(), ClientEffects::default()];
    for (client, effects) in clients.iter().zip(&mut applied) {
        effects.consume(&client.effects().unwrap());
    }
    let request = ShoveRequest {
        epoch: applied[0].ack().epoch,
        id: 1,
        target: 30,
    };
    assert!(clients[0].session.publish_application(
        dedicated::SHOVE_KEY,
        serde_json::to_vec(&request).unwrap(),
        start.elapsed().as_millis() as u64
    ));
    pump(&mut server, address, &mut clients, start, |clients| {
        clients.iter().all(|client| {
            client
                .effects()
                .is_some_and(|batch| !batch.effects.is_empty())
        })
    });
    for (index, client) in clients.iter_mut().enumerate() {
        let batch = client.effects().unwrap();
        let effects = applied[index].consume(&batch);
        assert_eq!(effects.len(), 1);
        assert_eq!(
            effects[0].kind,
            if index == 0 {
                EffectKind::AttackAccepted
            } else {
                EffectKind::Shove
            }
        );
        assert_eq!(effects[0].target, client.session.local);
        assert!(applied[index].consume(&batch).is_empty());
        assert!(client.session.publish_application(
            dedicated::EFFECT_ACK_KEY,
            serde_json::to_vec(&applied[index].ack()).unwrap(),
            start.elapsed().as_millis() as u64
        ));
    }
    pump(&mut server, address, &mut clients, start, |clients| {
        clients.iter().all(|client| {
            client
                .effects()
                .is_some_and(|batch| batch.effects.is_empty())
        })
    });

    clients[0].velocity = [2., 0., 0.];
    clients[1].velocity = [-2., 0., 0.];
    clients[1].position = [0.5, 2., 0.];
    for client in &mut clients {
        client.published = None;
    }
    pump(&mut server, address, &mut clients, start, |clients| {
        clients.iter().all(|client| {
            client.effects().is_some_and(|batch| {
                batch
                    .effects
                    .iter()
                    .any(|effect| effect.kind == EffectKind::Collision)
            })
        })
    });
    let a = applied[0].consume(&clients[0].effects().unwrap());
    let b = applied[1].consume(&clients[1].effects().unwrap());
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert!(a[0].delta_velocity.iter().any(|v| *v != 0.));
    for axis in 0..3 {
        assert!((a[0].delta_velocity[axis] + b[0].delta_velocity[axis]).abs() < 0.0001);
    }

    for packet in clients[0].session.goodbye() {
        clients[0].socket.send_to(&packet.data, address).unwrap();
    }
    clients.remove(0);
    pump(&mut server, address, &mut clients, start, |clients| {
        clients[0].session.player_ids() == vec![30]
    });
    assert!(server.child.try_wait().unwrap().is_none());
}
