//! Real TLS login and authenticated UDP resource requests to the native admin lane.
use serde_json::{Value, json};
use skate_accounts::{ClientCredentials, ClientSession, admin_request, initialize, login_client};
use skate_net::{
    dedicated,
    lobby::{Info, Session},
    resources::{CLIENT_KEY, Client as ResourceClient, ServerRecord, server_key},
};
use skate_server::{Host, Map, Options};
use std::{
    fs,
    net::UdpSocket,
    path::PathBuf,
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
    session: Session,
    crypto: ClientSession,
    wire: ResourceClient,
    inbox: Vec<Value>,
}
impl Guest {
    fn new(config: &ClientCredentials, host: &Host) -> Self {
        let (_, crypto) = login_client(config, Duration::from_secs(5)).unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let session = Session::dedicated_client(
            dedicated::SESSION,
            Info {
                id: crypto.actor,
                map: host.map_fingerprint(),
                rig: 2,
                physics: 3,
                appearance: 4,
            },
            1,
        );
        Self {
            socket,
            session,
            crypto,
            wire: ResourceClient::default(),
            inbox: Vec::new(),
        }
    }
    fn step(&mut self, host: &mut Host, start: Instant) {
        let now = start.elapsed().as_millis() as u64;
        if self.wire.offer().is_some() {
            self.wire.set_ready(true);
            self.session
                .publish_application(CLIENT_KEY, self.wire.encode().unwrap(), now);
        }
        for packet in self.session.service(now) {
            self.socket
                .send_to(
                    &self.crypto.encode(&packet.data).unwrap(),
                    host.local_addr().unwrap(),
                )
                .unwrap();
        }
        host.step().unwrap();
        let mut bytes = [0u8; 2048];
        while let Ok((count, _)) = self.socket.recv_from(&mut bytes) {
            if let Ok(plain) = self.crypto.decode(&bytes[..count]) {
                self.session.receive(1, &plain, now);
            }
        }
        if let Some(actor) = self.session.host_actor() {
            if let Some(record) = self.session.actors[&actor]
                .application
                .get(&server_key(self.session.local))
            {
                self.wire
                    .receive(&serde_json::from_slice::<ServerRecord>(&record.value).unwrap())
                    .unwrap();
            }
        }
        for message in self.wire.take_incoming() {
            if message.name == "__host_admin_result" {
                self.inbox.push(message.value);
            }
        }
    }
    fn ready(&mut self, host: &mut Host, start: Instant) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while self.wire.offer().is_none() {
            self.step(host, start);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..20 {
            self.step(host, start);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn request(&mut self, host: &mut Host, start: Instant, seq: &str, action: Value) -> Value {
        self.wire
            .emit(
                "admin-ui",
                1,
                "__host_admin",
                json!({"seq":seq,"action":action}),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.step(host, start);
            if let Some(index) = self.inbox.iter().position(|value| value["seq"] == seq) {
                return self.inbox.remove(index);
            }
            assert!(Instant::now() < deadline, "no response for {seq}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[test]
fn authenticated_resource_admin_uses_real_actor_and_private_typed_settings() {
    let temp =
        Temp(std::env::temp_dir().join(format!("skate-resource-admin-{}", std::process::id())));
    fs::create_dir(&temp.0).unwrap();
    let auth = temp.0.join("auth");
    initialize(&auth, "administrator", "test-password-12345").unwrap();
    fs::write(temp.0.join("accounts.json"),serde_json::to_vec(&json!({"database":"auth/accounts.sqlite3","bind":"127.0.0.1:0","certificate":"auth/certificate.pem","key":"auth/private-key.pem"})).unwrap()).unwrap();
    let resource = temp.0.join("resources/admin-ui");
    fs::create_dir_all(&resource).unwrap();
    fs::write(resource.join("resource.json"),serde_json::to_vec(&json!({"format":1,"api":1,"id":"admin-ui","version":"1.0.0","language":"lua","client_scripts":["client.lua"],"server_scripts":["server.lua"],"files":[],"capabilities":["resource.network","resource.admin","resource.settings"],"settings":{
        "secret":{"type":"string","default":"classified-local-value","max_bytes":64,"visibility":"private"},
        "count":{"type":"integer","default":1,"min":1,"max":10,"visibility":"private","change":"live"},
        "pending":{"type":"integer","default":2,"min":1,"max":10,"visibility":"private","change":"restart"}
    }})).unwrap()).unwrap();
    fs::write(resource.join("client.lua"), "return {}").unwrap();
    fs::write(resource.join("server.lua"), "return {}").unwrap();
    fs::write(temp.0.join("server.json"),serde_json::to_vec(&json!({"root":"resources","storage":"storage","ensure":["admin-ui"],"grants":{"admin-ui":["resource.network","resource.admin","resource.settings"]}})).unwrap()).unwrap();
    let mut host = Host::bind(Options {
        bind: "127.0.0.1:0".parse().unwrap(),
        session: dedicated::SESSION,
        max_players: 16,
        map: Map::TestWorld, locations:None,
        resources: Some(temp.0.join("server.json")),
        accounts: Some(temp.0.join("accounts.json")),
        operations: None,
    })
    .unwrap();
    let password = auth.join("password");
    fs::write(&password, "test-password-12345").unwrap();
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
    let (authority, _) = login_client(&config, Duration::from_secs(5)).unwrap();
    admin_request(
        &config,
        &authority.token,
        "/v1/admin/accounts",
        Some(json!({"username":"player","password":"test-password-12345"})),
        Duration::from_secs(5),
    )
    .unwrap();
    let mut player_config = config.clone();
    player_config.username = "player".into();
    let start = Instant::now();
    let mut player = Guest::new(&player_config, &host);
    player.ready(&mut host, start);
    let mut admin = Guest::new(&config, &host);
    admin.ready(&mut host, start);
    assert_eq!(
        player.request(
            &mut host,
            start,
            "player-permissions",
            json!({"kind":"permissions","check":["custom.manage"]})
        )["value"]["permissions"],
        json!([])
    );
    let denied = player.request(
        &mut host,
        start,
        "forged-read",
        json!({"kind":"settings_read","resource":"admin-ui"}),
    );
    assert_eq!(denied["ok"], false);
    assert!(!denied.to_string().contains("classified"));
    let permitted = admin.request(
        &mut host,
        start,
        "admin-permissions",
        json!({"kind":"permissions","check":["custom.manage"]}),
    );
    assert!(
        permitted["value"]["permissions"]
            .as_array()
            .unwrap()
            .contains(&json!("settings.read"))
    );
    assert!(
        permitted["value"]["permissions"]
            .as_array()
            .unwrap()
            .contains(&json!("custom.manage")),
        "new plugin permission must work without a native allowlist"
    );
    let status = admin.request(&mut host, start, "status", json!({"kind":"status"}));
    assert_eq!(status["ok"], true);
    assert!(!status.to_string().contains("classified"));
    let read = admin.request(
        &mut host,
        start,
        "private-read",
        json!({"kind":"settings_read","resource":"admin-ui"}),
    );
    assert_eq!(read["ok"], true);
    assert!(read.to_string().contains("classified-local-value"));
    let write = admin.request(
        &mut host,
        start,
        "valid-write",
        json!({"kind":"settings_set","resource":"admin-ui","key":"count","value":5}),
    );
    assert_eq!(write["ok"], true);
    let invalid = admin.request(
        &mut host,
        start,
        "invalid-write",
        json!({"kind":"settings_set","resource":"admin-ui","key":"count","value":50}),
    );
    assert_eq!(invalid["ok"], false);
    let restart = admin.request(
        &mut host,
        start,
        "pending-write",
        json!({"kind":"settings_set","resource":"admin-ui","key":"pending","value":3}),
    );
    assert_eq!(restart["ok"], true);
    assert_eq!(restart["value"]["restart_required"], true);
    for _ in 0..30 {
        player.step(&mut host, start);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        player
            .inbox
            .iter()
            .all(|message| !message.to_string().contains("classified")),
        "private response was delivered to another actor"
    );
}
