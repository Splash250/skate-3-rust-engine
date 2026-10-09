//! Real TLS identities, encrypted UDP, authority positions and server Lua policy.
use serde_json::json;
use skate_accounts::{ClientCredentials, ClientSession, initialize, login_client};
use skate_net::{
    Body, Pose,
    lobby::{Info, Session},
    packed::{self, BodyState, Packed},
    resources::{CLIENT_KEY, Client as ResourceClient, ServerRecord, server_key},
};
use skate_server::{Host, Map, Options};
use skate_voice::{
    ClientState, Context, Incoming,
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
        }
    }
    fn send_voice(&mut self, host: &Host, actor: u64, sequence: u64, channel: &str) -> Vec<u8> {
        let frame = Frame {
            session: 7,
            actor,
            epoch: self.session.movement_epoch(),
            revision: self.voice.revision(),
            sequence,
            channel: channel.into(),
            data: vec![0xf8, 0xff, 0xfe],
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
    for guest in guests {
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
    thread::sleep(Duration::from_millis(5));
}
fn settle(host: &mut Host, guests: &mut [Guest], start: Instant) {
    for _ in 0..25 {
        pump(host, guests, start);
    }
}

#[test]
fn authenticated_udp_voice_obeys_radio_mute_instance_and_resource_retirement() {
    let temp = Temp(std::env::temp_dir().join(format!("skate-host-voice-{}", std::process::id())));
    fs::create_dir_all(&temp.0).unwrap();
    let auth = temp.0.join("auth");
    initialize(&auth, "administrator", "voice-test-password-123").unwrap();
    fs::write(temp.0.join("accounts.json"),serde_json::to_vec(&json!({"database":"auth/accounts.sqlite3","bind":"127.0.0.1:0","certificate":"auth/certificate.pem","key":"auth/private-key.pem"})).unwrap()).unwrap();
    let resource = temp.0.join("resources/radio");
    fs::create_dir_all(&resource).unwrap();
    let grants = json!(["resource.voice", "resource.commands", "resource.teleport"]);
    fs::write(resource.join("resource.json"),serde_json::to_vec(&json!({"format":1,"api":1,"id":"radio","version":"1.0.0","language":"lua","server_scripts":["server.lua"],"capabilities":grants})).unwrap()).unwrap();
    fs::write(
        resource.join("server.lua"),
        r#"
resource.command('radio','voice.admin',function(args)
 resource.voice.submit({kind='channel',name='crew',members={args[1],args[2]}})
end)
resource.command('mute','voice.admin',function(args)
 resource.voice.submit({kind='mute',player=args[1],muted=args[2]=='true'})
end)
resource.command('room','voice.admin',function(args)
 resource.teleport(args[1],{position={80,0,0},instance=9})
end)
"#,
    )
    .unwrap();
    fs::write(temp.0.join("server.json"),serde_json::to_vec(&json!({"root":"resources","storage":"state","ensure":["radio"],"grants":{"radio":grants}})).unwrap()).unwrap();
    let mut host = Host::bind(Options {
        bind: "127.0.0.1:0".parse().unwrap(),
        session: 7,
        max_players: 16,
        map: Map::TestWorld, locations:None,
        resources: Some(temp.0.join("server.json")),
        accounts: Some(temp.0.join("accounts.json")),
        operations: None,
    })
    .unwrap();
    let password = auth.join("password");
    fs::write(&password, "voice-test-password-123").unwrap();
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
        username: "administrator".into(),
        password_file: password,
    };
    let mut guests = vec![
        Guest::new(&config, host.map_fingerprint(), [0.; 3]),
        Guest::new(&config, host.map_fingerprint(), [3., 0., 0.]),
        Guest::new(&config, host.map_fingerprint(), [80., 0., 0.]),
    ];
    let start = Instant::now();
    let deadline = start + Duration::from_secs(5);
    while host.player_count() != 3 || guests.iter().any(|g| g.voice.revision() == 0) {
        pump(&mut host, &mut guests, start);
        assert!(Instant::now() < deadline, "voice admission timeout");
    }
    settle(&mut host, &mut guests, start);
    let actor = guests[0].crypto.actor;
    let far = guests[2].crypto.actor;
    let replay = guests[0].send_voice(&host, actor, 1, "");
    settle(&mut host, &mut guests, start);
    assert_eq!(
        guests[1].incoming.len(),
        1,
        "nearby peer receives encrypted speech"
    );
    assert!(guests[2].incoming.is_empty(), "proximity excludes far peer");
    guests[0]
        .socket
        .send_to(&replay, host.local_addr().unwrap())
        .unwrap();
    let other_actor = guests[1].crypto.actor;
    guests[0].send_voice(&host, other_actor, 2, "");
    settle(&mut host, &mut guests, start);
    assert_eq!(
        guests[1].incoming.len(),
        1,
        "AEAD replay and forged actor rejected"
    );
    host.resource_command(&format!("command radio {actor} {far}"))
        .unwrap();
    settle(&mut host, &mut guests, start);
    guests[0].send_voice(&host, actor, 3, "radio/crew");
    settle(&mut host, &mut guests, start);
    assert_eq!(
        guests[2].incoming.len(),
        1,
        "real Lua channel reaches distant member"
    );
    assert_eq!(guests[1].incoming.len(), 1, "radio excludes nonmember");
    host.resource_command(&format!("command mute {actor} true"))
        .unwrap();
    settle(&mut host, &mut guests, start);
    guests[0].send_voice(&host, actor, 4, "radio/crew");
    settle(&mut host, &mut guests, start);
    assert_eq!(guests[2].incoming.len(), 1);
    host.resource_command(&format!("command mute {actor} false"))
        .unwrap();
    host.resource_command(&format!("command room {far}"))
        .unwrap();
    settle(&mut host, &mut guests, start);
    assert_eq!(guests[2].session.actors[&far].instance, 9);
    guests[0].send_voice(&host, actor, 5, "radio/crew");
    settle(&mut host, &mut guests, start);
    assert_eq!(
        guests[2].incoming.len(),
        1,
        "radio remains instance-isolated"
    );
    host.resource_command("stop radio").unwrap();
    settle(&mut host, &mut guests, start);
    guests[0].send_voice(&host, actor, 6, "radio/crew");
    settle(&mut host, &mut guests, start);
    assert_eq!(guests[2].incoming.len(), 1, "retired resource cannot route");
    host.shutdown();
}
