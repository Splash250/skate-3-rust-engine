//! Shipped call resource over authenticated UDP, with real Opus encoding and decoding.
use serde_json::{Value, json};
use skate_accounts::{ClientCredentials, ClientSession, initialize, login_client};
use skate_net::{
    Body, Pose,
    lobby::{Info, Session},
    packed::{self, BodyState, Packed},
    resources::{CLIENT_KEY, Client as ResourceClient, ServerRecord, server_key},
};
use skate_server::{Host, Map, Options};
use skate_voice::{
    ClientState, Context, Encoder, FRAME_SAMPLES, Incoming, Mixer, SAMPLE_RATE,
    wire::{self, Frame, Playback},
};
use std::{
    fs,
    net::UdpSocket,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Guest {
    socket: UdpSocket,
    crypto: ClientSession,
    session: Session,
    resources: ResourceClient,
    voice: ClientState,
    incoming: Vec<Playback>,
    position: [f32; 3],
    events: Vec<(String, Value)>,
}
impl Guest {
    fn new(config: &ClientCredentials, map: u64, position: [f32; 3]) -> Self {
        let (_, crypto) = login_client(config, Duration::from_secs(5)).unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut session = Session::dedicated_client(
            7,
            Info {
                id: crypto.actor,
                map,
                rig: 2,
                physics: 3,
                appearance: 4,
            },
            1,
        );
        session.set_loopback(true);
        Self {
            socket,
            crypto,
            session,
            resources: ResourceClient::default(),
            voice: ClientState::default(),
            incoming: vec![],
            position,
            events: vec![],
        }
    }
    fn request(&mut self, value: Value) {
        self.resources
            .emit("phone-calls", 1, "request", value)
            .unwrap();
    }
    fn call(&self) -> Option<Value> {
        self.events
            .iter()
            .rev()
            .find(|(name, _)| name == "call")
            .map(|(_, v)| v.clone())
    }
    fn send_voice(
        &mut self,
        host: &Host,
        actor: u64,
        sequence: u64,
        channel: &str,
        data: Vec<u8>,
    ) -> Vec<u8> {
        let frame = Frame {
            session: 7,
            actor,
            epoch: self.session.movement_epoch(),
            revision: self.voice.revision(),
            sequence,
            channel: channel.into(),
            data,
        }
        .encode()
        .unwrap();
        let encrypted = self.crypto.encode(&frame).unwrap();
        self.socket
            .send_to(&encrypted, host.local_addr().unwrap())
            .unwrap();
        encrypted
    }
}
fn pump(host: &mut Host, guests: &mut [Guest], start: Instant) {
    let now = start.elapsed().as_millis() as u64;
    for guest in guests.iter_mut() {
        let root = Pose {
            p: guest.position,
            q: [0., 0., 0., 1.],
        };
        guest.session.publish(
            packed::BODY,
            Packed::body(&BodyState {
                root,
                enabled: (1 << 33) - 1,
                bodies: vec![
                    Body {
                        pose: root,
                        velocity: [0.; 3],
                        angular: [0.; 3]
                    };
                    33
                ],
            })
            .unwrap(),
            now,
        );
        if guest.resources.offer().is_some() {
            guest.resources.set_ready(true);
            guest
                .session
                .publish_application(CLIENT_KEY, guest.resources.encode().unwrap(), now);
        }
        for packet in guest.session.service(now) {
            guest
                .socket
                .send_to(
                    &guest.crypto.encode(&packet.data).unwrap(),
                    host.local_addr().unwrap(),
                )
                .unwrap();
        }
    }
    host.step().unwrap();
    for guest in guests.iter_mut() {
        let mut bytes = [0; 2048];
        while let Ok((n, _)) = guest.socket.recv_from(&mut bytes) {
            let plain = guest.crypto.decode(&bytes[..n]).unwrap();
            if wire::is_voice(&plain) {
                if let Some(Incoming::Playback { packet, .. }) = guest.voice.accept(1, &plain) {
                    guest.incoming.push(packet);
                }
            } else {
                guest.session.receive(1, &plain, now);
                if guest.session.connected() {
                    guest.voice.synchronize(Some(Context {
                        session: 7,
                        host_peer: 1,
                        actor: guest.session.local,
                        epoch: guest.session.movement_epoch(),
                        instance: guest.session.actors[&guest.session.local].instance,
                    }));
                }
            }
        }
        if let Some(id) = guest.session.host_actor() {
            if let Some(record) = guest.session.actors[&id]
                .application
                .get(&server_key(guest.session.local))
            {
                guest
                    .resources
                    .receive(&serde_json::from_slice::<ServerRecord>(&record.value).unwrap())
                    .unwrap();
            }
        }
    }
    for guest in guests {
        for message in guest.resources.take_incoming() {
            guest.events.push((message.name, message.value));
        }
    }
    thread::sleep(Duration::from_millis(5));
}
fn settle(host: &mut Host, guests: &mut [Guest], start: Instant) {
    for _ in 0..25 {
        pump(host, guests, start);
    }
}

#[test]
fn shipped_calls_create_private_channels_only_after_acceptance_and_route_encoded_audio() {
    let temp = Temp(std::env::temp_dir().join(format!("skate-call-udp-{}", std::process::id())));
    fs::create_dir(&temp.0).unwrap();
    let auth = temp.0.join("auth");
    initialize(&auth, "caller", "phone-test-password-123").unwrap();
    fs::write(temp.0.join("accounts.json"),serde_json::to_vec(&json!({"database":"auth/accounts.sqlite3","bind":"127.0.0.1:0","certificate":"auth/certificate.pem","key":"auth/private-key.pem"})).unwrap()).unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/phone-calls");
    let package = temp.0.join("resources/phone-calls");
    fs::create_dir_all(&package).unwrap();
    for name in ["resource.json", "server.lua", "client.lua"] {
        fs::copy(source.join(name), package.join(name)).unwrap();
    }
    let manifest: Value =
        serde_json::from_slice(&fs::read(package.join("resource.json")).unwrap()).unwrap();
    fs::write(temp.0.join("server.json"),serde_json::to_vec(&json!({"root":"resources","storage":"state","ensure":["phone-calls"],"grants":{"phone-calls":manifest["capabilities"]}})).unwrap()).unwrap();
    let mut host = Host::bind(Options {
        bind: "127.0.0.1:0".parse().unwrap(),
        session: 7,
        max_players: 16,
        map: Map::TestWorld,
        resources: Some(temp.0.join("server.json")),
        accounts: Some(temp.0.join("accounts.json")),
        operations: None,
    })
    .unwrap();
    let password = auth.join("password");
    fs::write(&password, "phone-test-password-123").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&password, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let config = ClientCredentials {
        endpoint: format!(
            "https://localhost:{}",
            host.account_address().unwrap().port()
        ),
        ca_certificate: auth.join("certificate.pem"),
        username: "caller".into(),
        password_file: password,
    };
    let mut guests = vec![
        Guest::new(&config, host.map_fingerprint(), [0.; 3]),
        Guest::new(&config, host.map_fingerprint(), [80., 0., 0.]),
        Guest::new(&config, host.map_fingerprint(), [2., 0., 0.]),
    ];
    let start = Instant::now();
    let deadline = start + Duration::from_secs(8);
    while host.player_count() != 3
        || guests
            .iter()
            .any(|g| g.voice.revision() == 0 || g.resources.offer().is_none())
    {
        pump(&mut host, &mut guests, start);
        assert!(Instant::now() < deadline);
    }
    settle(&mut host, &mut guests, start);
    for guest in &mut guests {
        guest.request(json!({"action":"presence","enabled":true}));
    }
    settle(&mut host, &mut guests, start);
    let caller = guests[0].crypto.actor;
    let callee = guests[1].crypto.actor;
    guests[0].request(json!({"action":"dial","target":callee.to_string(),"sender":"forged"}));
    settle(&mut host, &mut guests, start);
    let ring = guests[1].call().expect("callee rings");
    assert_eq!(ring["state"], "ringing");
    let id = ring["id"].as_str().unwrap();
    let channel = format!("phone-calls/{id}");
    assert!(
        guests[2].call().is_none(),
        "third party never gets private signaling"
    );
    let mut encoder = Encoder::new().unwrap();
    let pcm = std::array::from_fn(|i| {
        (i as f32 * 440. * std::f32::consts::TAU / SAMPLE_RATE as f32).sin() * 0.25
    });
    guests[0].send_voice(&host, caller, 1, &channel, encoder.encode(&pcm).unwrap());
    settle(&mut host, &mut guests, start);
    assert!(
        guests.iter().all(|g| g.incoming.is_empty()),
        "ringing does not grant voice membership"
    );
    guests[2].request(json!({"action":"accept","id":id,"sender":callee.to_string()}));
    settle(&mut host, &mut guests, start);
    assert_eq!(guests[1].call().unwrap()["state"], "ringing");
    guests[1].request(json!({"action":"accept","id":id}));
    settle(&mut host, &mut guests, start);
    assert_eq!(guests[0].call().unwrap()["state"], "active");
    assert_eq!(guests[1].call().unwrap()["state"], "active");
    let mut mixer = Mixer::new();
    let mut energy = 0.;
    let mut count = 0;
    for n in 0..12u64 {
        let pcm = std::array::from_fn(|i| {
            ((n as usize * FRAME_SAMPLES + i) as f32 * 440. * std::f32::consts::TAU
                / SAMPLE_RATE as f32)
                .sin()
                * 0.25
        });
        guests[0].send_voice(
            &host,
            caller,
            n + 2,
            &channel,
            encoder.encode(&pcm).unwrap(),
        );
        for _ in 0..5 {
            pump(&mut host, &mut guests, start);
        }
        for packet in guests[1].incoming.drain(..) {
            mixer.push(packet).unwrap();
            count += 1;
        }
        energy += mixer.render().iter().map(|x| x * x).sum::<f32>();
    }
    assert!(count >= 10, "accepted Opus packet count: {count}");
    assert!(energy > 1., "actual decoded sine wave energy: {energy}");
    assert!(
        guests[2].incoming.is_empty(),
        "nearby third party receives no call audio"
    );
    assert!(mixer.pending() <= 8 * 4);
    guests[1].send_voice(&host, callee, 1, &channel, encoder.encode(&pcm).unwrap());
    settle(&mut host, &mut guests, start);
    assert!(
        !guests[0].incoming.is_empty(),
        "audio routes in both directions"
    );
    guests[0].incoming.clear();
    guests[0].request(json!({"action":"hangup","id":id}));
    settle(&mut host, &mut guests, start);
    assert_eq!(guests[1].call().unwrap()["state"], "ended");
    guests[0].send_voice(&host, caller, 100, &channel, encoder.encode(&pcm).unwrap());
    settle(&mut host, &mut guests, start);
    assert!(
        guests.iter().all(|g| g.incoming.is_empty()),
        "hangup revokes membership even for modified clients"
    );
    host.resource_command("restart phone-calls").unwrap();
    settle(&mut host, &mut guests, start);
    guests[0].send_voice(&host, caller, 101, &channel, encoder.encode(&pcm).unwrap());
    settle(&mut host, &mut guests, start);
    assert!(guests.iter().all(|g| g.incoming.is_empty()));
    eprintln!(
        "Shipped phone calls: {count} real Opus frames decoded, energy={energy}, third-party frames=0; encrypted loopback UDP, no physical audio devices."
    );
}
