//! The shipped inventory runs against the real authenticated UDP host, content
//! HTTP server and SQLite backend. No client-supplied account selector is trusted.
use serde_json::{Value, json};
use skate_accounts::{ClientCredentials, ClientSession, initialize, login_client};
use skate_net::{
    dedicated,
    lobby::{Info, Session},
    resources::{CLIENT_KEY, Client, ServerRecord, server_key},
};
use skate_resources::{Cache, Limits, download_set};
use skate_server::{Host, Map, Options};
use std::{
    fs,
    net::UdpSocket,
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("skate-inventory-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        initialize(
            &root.join("auth"),
            "inventory-user",
            "inventory-test-password",
        )
        .unwrap();
        let resource = root.join("resources/inventory-ui");
        fs::create_dir_all(&resource).unwrap();
        for (name, bytes) in [
            (
                "resource.json",
                include_str!("../../../resources/inventory-ui/resource.json"),
            ),
            (
                "client.lua",
                include_str!("../../../resources/inventory-ui/client.lua"),
            ),
            (
                "server.lua",
                include_str!("../../../resources/inventory-ui/server.lua"),
            ),
            (
                "index.html",
                include_str!("../../../resources/inventory-ui/index.html"),
            ),
            (
                "inventory.css",
                include_str!("../../../resources/inventory-ui/inventory.css"),
            ),
            (
                "inventory.js",
                include_str!("../../../resources/inventory-ui/inventory.js"),
            ),
        ] {
            fs::write(resource.join(name), bytes).unwrap();
        }
        fs::write(root.join("accounts.json"),serde_json::to_vec(&json!({"database":"auth/accounts.sqlite3","bind":"127.0.0.1:0","certificate":"auth/certificate.pem","key":"auth/private-key.pem"})).unwrap()).unwrap();
        fs::write(root.join("resources.json"),serde_json::to_vec(&json!({"root":"resources","storage":"store","ensure":["inventory-ui"],"grants":{"inventory-ui":["resource.network","resource.events","resource.database","engine.ui"]}})).unwrap()).unwrap();
        fs::write(root.join("password"), "inventory-test-password").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root.join("password"), fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self(root)
    }
    fn host(&self) -> Host {
        Host::bind(Options {
            bind: "127.0.0.1:0".parse().unwrap(),
            session: dedicated::SESSION,
            max_players: 16,
            map: Map::TestWorld,
            resources: Some(self.0.join("resources.json")),
            accounts: Some(self.0.join("accounts.json")),
        })
        .unwrap()
    }
    fn credentials(&self, host: &Host) -> ClientCredentials {
        ClientCredentials {
            endpoint: format!(
                "https://localhost:{}",
                host.account_address().unwrap().port()
            ),
            ca_certificate: self.0.join("auth/certificate.pem"),
            username: "inventory-user".into(),
            password_file: self.0.join("password"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Guest {
    socket: UdpSocket,
    session: Session,
    crypto: ClientSession,
    wire: Client,
    start: Instant,
    generation: u64,
}
impl Guest {
    fn connect(host: &mut Host, config: &ClientCredentials, cache: &Cache) -> Self {
        let (_, crypto) = login_client(config, Duration::from_secs(5)).unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut guest = Self {
            socket,
            session: Session::dedicated_client(
                dedicated::SESSION,
                Info {
                    id: crypto.actor,
                    map: host.map_fingerprint(),
                    rig: 2,
                    physics: 3,
                    appearance: 4,
                },
                1,
            ),
            crypto,
            wire: Client::default(),
            start: Instant::now(),
            generation: 0,
        };
        let deadline = Instant::now() + Duration::from_secs(4);
        while guest.wire.offer().is_none() {
            guest.step(host);
            assert!(Instant::now() < deadline, "resource offer timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
        let offer = guest.wire.offer().unwrap();
        let downloaded = download_set(
            (host.local_addr().unwrap().ip(), offer.port).into(),
            &offer.revision,
            cache,
            "inventory-test",
            &AtomicBool::new(false),
        )
        .unwrap();
        let root = &downloaded.roots["inventory-ui"];
        assert!(root.join("index.html").is_file());
        assert!(root.join("inventory.js").is_file());
        assert!(!root.join("server.lua").exists());
        guest.generation = downloaded
            .set
            .resources
            .iter()
            .find(|r| r.manifest.id == "inventory-ui")
            .unwrap()
            .generation;
        guest.wire.set_ready(true);
        guest
    }
    fn step(&mut self, host: &mut Host) {
        let now = self.start.elapsed().as_millis() as u64;
        if self.wire.offer().is_some() {
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
        let mut bytes = [0; 2048];
        while let Ok((n, _)) = self.socket.recv_from(&mut bytes) {
            if let Ok(plain) = self.crypto.decode(&bytes[..n]) {
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
    }
    fn request(&mut self, value: Value) {
        self.wire
            .emit("inventory-ui", self.generation, "inventory_request", value)
            .unwrap();
    }
    fn result(&mut self, host: &mut Host) -> Value {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            self.step(host);
            for message in self.wire.take_incoming() {
                if message.resource == "inventory-ui" && message.name == "inventory" {
                    return message.value;
                }
            }
            assert!(Instant::now() < deadline, "inventory response timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn inspect(&mut self, host: &mut Host) -> Value {
        self.request(json!({"action":"inspect","account_id":"forged-other-account"}));
        self.result(host)
    }
}
#[test]
fn authenticated_inventory_transactions_cannot_overspend_and_survive_server_restart() {
    let fixture = Fixture::new();
    let cache = Cache::open(fixture.0.join("cache"), Limits::default()).unwrap();
    let mut host = fixture.host();
    let config = fixture.credentials(&host);
    let mut first = Guest::connect(&mut host, &config, &cache);
    let identity = first.crypto.account_id.clone();
    let initial = first.inspect(&mut host);
    assert_eq!(initial["ok"], true, "{initial}");
    assert_eq!(initial["coins"], 100);
    let mut second = Guest::connect(&mut host, &config, &cache);
    assert_eq!(second.crypto.account_id, identity);
    assert_ne!(second.crypto.actor, first.crypto.actor);
    // Concurrent sessions share one stable account and serialize the SQL updates.
    for _ in 0..2 {
        first.request(
            json!({"action":"buy","item":"deck_blue","account_id":"forged-other-account"}),
        );
        second.request(json!({"action":"buy","item":"deck_blue"}));
        for _ in 0..30 {
            first.step(&mut host);
            second.step(&mut host);
            std::thread::sleep(Duration::from_millis(2));
        }
        let a = first.result(&mut host);
        let b = second.result(&mut host);
        assert_eq!(a["ok"], true, "{a}");
        assert_eq!(b["ok"], true, "{b}");
    }
    first.request(json!({"action":"buy","item":"deck_blue"}));
    let failed = first.result(&mut host);
    assert_eq!(failed["ok"], false);
    let empty = first.inspect(&mut host);
    assert_eq!(empty["coins"], 0);
    assert_eq!(empty["owned"]["deck_blue"], 4);
    host.shutdown();
    drop(first);
    drop(second);
    drop(host);
    let mut reopened = fixture.host();
    let config = fixture.credentials(&reopened);
    let mut again = Guest::connect(&mut reopened, &config, &cache);
    assert_eq!(again.crypto.account_id, identity);
    let saved = again.inspect(&mut reopened);
    assert_eq!(saved["coins"], 0);
    assert_eq!(saved["owned"]["deck_blue"], 4);
    reopened.shutdown();
}
