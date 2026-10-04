//! The real UDP host advertises a content set, HTTP serves it independently,
//! and sandboxed client/server Lua executes across a real connection.
use serde_json::json;
use skate_mods::resources::{Host as LuaHost, InstalledResource, Output, Side};
use skate_net::{
    lobby::{Info, Session},
    resources::{CLIENT_KEY, Client, Kind, ServerRecord, server_key},
};
use skate_resources::{Cache, Limits, download_set};
use skate_server::{Host, Map, Options};
use std::{
    collections::BTreeSet,
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "skate-platform-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(t: &Temp) -> PathBuf {
    let root = t.0.join("resources");
    std::fs::create_dir_all(root.join("challenge")).unwrap();
    std::fs::write(root.join("challenge/resource.json"),serde_json::to_vec(&json!({"format":1,"api":1,"id":"challenge","version":"1.0.0","language":"lua","client_scripts":["client.lua"],"server_scripts":["private.lua"],"files":["title.txt"],"capabilities":["resource.network","resource.state","resource.storage","engine.ui"]})).unwrap()).unwrap();
    std::fs::write(root.join("challenge/title.txt"), "Downloaded challenge").unwrap();
    std::fs::write(
        root.join("challenge/client.lua"),
        "sdk.ui.text('welcome',sdk.read_text('title.txt')); resource.send('join',{action='join'})",
    )
    .unwrap();
    std::fs::write(root.join("challenge/private.lua"),"resource.on_net('join',function(data,sender) if data.action~='join' then return end; local n=(resource.storage.get('visits') or 0)+1; resource.storage.set('visits',n);resource.state.set('joined',{sender=sender,visits=n});end)").unwrap();
    let config = t.0.join("server.json");
    std::fs::write(&config,serde_json::to_vec(&json!({"root":"resources","storage":"store","ensure":["challenge"],"grants":{"challenge":["resource.network","resource.state","resource.storage","engine.ui"]}})).unwrap()).unwrap();
    config
}
fn host(config: PathBuf) -> Host {
    Host::bind(Options {
        accounts: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        session: 7,
        max_players: 16,
        map: Map::TestWorld,
        resources: Some(config),
    })
    .unwrap()
}
struct Guest {
    socket: UdpSocket,
    session: Session,
    wire: Client,
    start: Instant,
    server: SocketAddr,
}
impl Guest {
    fn new(server: SocketAddr, id: u64) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        Self {
            socket,
            session: Session::dedicated_client(
                7,
                Info {
                    id,
                    map: skate_net::hash(b"skate-test-world-v1"),
                    rig: 2,
                    physics: 3,
                    appearance: 4,
                },
                1,
            ),
            wire: Client::default(),
            start: Instant::now(),
            server,
        }
    }
    fn send(&mut self) {
        let now = self.start.elapsed().as_millis() as u64;
        if self.wire.offer().is_some() {
            self.session
                .publish_application(CLIENT_KEY, self.wire.encode().unwrap(), now);
        }
        for p in self.session.service(now) {
            self.socket.send_to(&p.data, self.server).unwrap();
        }
    }
    fn receive(&mut self) {
        let now = self.start.elapsed().as_millis() as u64;
        let mut buf = [0; 2048];
        while let Ok((n, from)) = self.socket.recv_from(&mut buf) {
            assert_eq!(from, self.server);
            self.session.receive(1, &buf[..n], now);
        }
        if let Some(id) = self.session.host_actor() {
            if let Some(r) = self.session.actors[&id]
                .application
                .get(&server_key(self.session.local))
            {
                self.wire
                    .receive(&serde_json::from_slice::<ServerRecord>(&r.value).unwrap())
                    .unwrap();
            }
        }
    }
    fn step(&mut self, host: &mut Host) {
        self.send();
        host.step().unwrap();
        self.receive();
    }
    fn wait_offer(&mut self, host: &mut Host) {
        let until = Instant::now() + Duration::from_secs(3);
        while self.wire.offer().is_none() {
            assert!(Instant::now() < until, "no resource offer");
            self.step(host);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn activate(&mut self, cache: &Cache, scope: &str) -> (LuaHost, u64) {
        let offer = self.wire.offer().unwrap();
        let report = download_set(
            SocketAddr::new(self.server.ip(), offer.port),
            &offer.revision,
            cache,
            scope,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(report.set.resources[0].manifest.server_scripts.is_empty());
        assert!(!report.roots["challenge"].join("private.lua").exists());
        let mut lua = LuaHost::new(Side::Client, cache.root().join("state"), scope).unwrap();
        lua.install(
            report
                .set
                .resources
                .iter()
                .map(|r| InstalledResource {
                    manifest: r.manifest.clone(),
                    root: report.roots[&r.manifest.id].clone(),
                    generation: r.generation,
                    grants: r
                        .manifest
                        .capabilities
                        .iter()
                        .cloned()
                        .collect::<BTreeSet<_>>(),
                })
                .collect(),
        )
        .unwrap();
        lua.start_all().unwrap();
        assert!(
            !lua.drain_commands().is_empty(),
            "downloaded UI script did not execute"
        );
        for out in lua.drain_outputs() {
            if let Output::Event {
                resource,
                generation,
                name,
                payload,
                ..
            } = out
            {
                self.wire
                    .emit(&resource, generation, &name, payload)
                    .unwrap();
            }
        }
        self.wire.set_ready(true);
        (lua, report.downloaded_bytes)
    }
    fn wait_state(&mut self, host: &mut Host, lua: &mut LuaHost) -> serde_json::Value {
        let until = Instant::now() + Duration::from_secs(4);
        loop {
            assert!(Instant::now() < until, "server Lua did not publish state");
            self.step(host);
            for m in self.wire.take_incoming() {
                if m.kind == Kind::State {
                    lua.apply_state(&m.resource, m.generation, &m.name, m.value)
                        .unwrap();
                }
            }
            if let Some(value) = lua.state("challenge", "joined") {
                if value["sender"].as_str() == Some(self.session.local.to_string().as_str()) {
                    return value;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
#[test]
fn real_udp_http_lua_cold_warm_cross_server_and_restart() {
    let temp = Temp::new();
    let config = fixture(&temp);
    let cache_dir = temp.0.join("cache");
    let cache = Cache::open(&cache_dir, Limits::default()).unwrap();
    let mut server = host(config.clone());
    let mut client = Guest::new(server.local_addr().unwrap(), 2);
    client.wait_offer(&mut server);
    assert_eq!(server.player_count(), 0);
    let (mut lua, cold) = client.activate(&cache, "server-a");
    assert!(cold > 0);
    let state = client.wait_state(&mut server, &mut lua);
    assert_eq!(state["sender"], "2");
    assert_eq!(state["visits"], 1);
    assert_eq!(server.player_count(), 1);
    let epoch = client.wire.offer().unwrap().epoch;
    assert!(server.resource_command("restart unknown").is_err());
    for _ in 0..100 {
        client.step(&mut server);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        client.wire.offer().unwrap().epoch,
        epoch,
        "invalid lifecycle command must not interrupt clients"
    );
    lua.disconnect();
    assert!(!lua.running("challenge"));
    assert_eq!(lua.drain_retired(), vec!["challenge"]);
    let mut warm = Guest::new(server.local_addr().unwrap(), 3);
    warm.wait_offer(&mut server);
    let (mut lua, bytes) = warm.activate(&cache, "server-a");
    assert_eq!(bytes, 0);
    assert_eq!(warm.wait_state(&mut server, &mut lua)["visits"], 2);
    drop(cache);
    let cache = Cache::open(&cache_dir, Limits::default()).unwrap();
    let mut second_server = host(config);
    let mut other = Guest::new(second_server.local_addr().unwrap(), u64::MAX - 3);
    other.wait_offer(&mut second_server);
    let (mut lua, bytes) = other.activate(&cache, "server-b");
    assert_eq!(bytes, 0);
    assert_eq!(
        other.wait_state(&mut second_server, &mut lua)["sender"],
        (u64::MAX - 3).to_string()
    );
    let old = warm.wire.offer().unwrap().revision.clone();
    server.resource_command("restart challenge").unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    while warm.wire.offer().unwrap().revision == old {
        assert!(Instant::now() < until);
        warm.step(&mut server);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!warm.wire.ready());
    let (_, bytes) = warm.activate(&cache, "server-a");
    assert_eq!(bytes, 0, "generation change should reuse bytes");
    server.resource_command("stop challenge").unwrap();
    assert!(
        server
            .resource_command("resources")
            .unwrap()
            .contains("stopped")
    );
    server.shutdown();
    second_server.shutdown();
}

#[test]
#[ignore = "subprocess helper invoked by resource_client_restart_reuses_verified_content"]
fn resource_client_subprocess() {
    let server = std::env::var("SKATE_RESOURCE_TEST_SERVER")
        .unwrap()
        .parse()
        .unwrap();
    let id = std::env::var("SKATE_RESOURCE_TEST_ACTOR")
        .unwrap()
        .parse()
        .unwrap();
    let cache = Cache::open(
        std::env::var("SKATE_RESOURCE_TEST_CACHE").unwrap(),
        Limits::default(),
    )
    .unwrap();
    let mut guest = Guest::new(server, id);
    let until = Instant::now() + Duration::from_secs(5);
    while guest.wire.offer().is_none() {
        assert!(Instant::now() < until);
        guest.send();
        guest.receive();
        std::thread::sleep(Duration::from_millis(2));
    }
    let (mut lua, downloaded) = guest.activate(&cache, "process-server");
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            Instant::now() < until,
            "subprocess did not receive server Lua state"
        );
        guest.send();
        guest.receive();
        for m in guest.wire.take_incoming() {
            if m.kind == Kind::State {
                lua.apply_state(&m.resource, m.generation, &m.name, m.value)
                    .unwrap();
            }
        }
        if let Some(value) = lua.state("challenge", "joined") {
            if value["sender"].as_str() == Some(id.to_string().as_str()) {
                std::fs::write(
                    std::env::var("SKATE_RESOURCE_TEST_REPORT").unwrap(),
                    serde_json::to_vec(&json!({"downloaded":downloaded,"visits":value["visits"]}))
                        .unwrap(),
                )
                .unwrap();
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    lua.disconnect();
    for p in guest.session.goodbye() {
        guest.socket.send_to(&p.data, guest.server).unwrap();
    }
}
#[test]
fn resource_client_restart_reuses_verified_content() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    let temp = Temp::new();
    let config = fixture(&temp);
    struct Running(std::process::Child);
    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut server = Running(
        Command::new(
            std::env::var_os("SKATE_SERVER_EXE")
                .unwrap_or_else(|| env!("CARGO_BIN_EXE_skate-server").into()),
        )
        .args([
            "--test-world",
            "--bind",
            "127.0.0.1:0",
            "--session",
            "7",
            "--resources",
        ])
        .arg(config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap(),
    );
    let stdout = server.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if let Ok(line) = line {
                let _ = tx.send(line);
            } else {
                break;
            }
        }
    });
    let line = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let endpoint = line.split_whitespace().nth(4).unwrap();
    let mut downloads = vec![];
    for id in [100, 101] {
        let report = temp.0.join(format!("{id}.json"));
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "resource_client_subprocess", "--ignored"])
            .env("SKATE_RESOURCE_TEST_SERVER", endpoint)
            .env("SKATE_RESOURCE_TEST_ACTOR", id.to_string())
            .env("SKATE_RESOURCE_TEST_CACHE", temp.0.join("process-cache"))
            .env("SKATE_RESOURCE_TEST_REPORT", &report)
            .status()
            .unwrap();
        assert!(status.success());
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(report).unwrap()).unwrap();
        downloads.push(result["downloaded"].as_u64().unwrap());
        assert_eq!(result["visits"].as_u64(), Some(id - 99));
    }
    assert!(downloads[0] > 0);
    assert_eq!(
        downloads[1], 0,
        "second client process must download no unchanged bytes"
    );
    writeln!(server.0.stdin.take().unwrap(), "quit").unwrap();
    assert!(server.0.wait().unwrap().success());
    reader.join().unwrap();
}

#[test]
fn unrelated_lifecycle_does_not_publish_edited_running_content() {
    let temp = Temp::new();
    let config = fixture(&temp);
    let resource = temp.0.join("resources/other");
    std::fs::create_dir_all(&resource).unwrap();
    std::fs::write(resource.join("resource.json"),serde_json::to_vec(&json!({"format":1,"api":1,"id":"other","version":"1","language":"lua","client_scripts":["client.lua"]})).unwrap()).unwrap();
    std::fs::write(
        resource.join("client.lua"),
        "sdk.log('old immutable content')",
    )
    .unwrap();
    let mut settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    settings["ensure"] = json!(["challenge", "other"]);
    std::fs::write(&config, serde_json::to_vec(&settings).unwrap()).unwrap();
    let mut server = host(config);
    std::fs::write(
        resource.join("client.lua"),
        "sdk.log('unstarted changed content')",
    )
    .unwrap();
    server.resource_command("stop challenge").unwrap();
    let mut client = Guest::new(server.local_addr().unwrap(), 10);
    client.wait_offer(&mut server);
    let cache = Cache::open(temp.0.join("cache"), Limits::default()).unwrap();
    let offer = client.wire.offer().unwrap();
    let report = download_set(
        server.resource_address().unwrap(),
        &offer.revision,
        &cache,
        "immutable",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(report.set.resources.len(), 1);
    assert_eq!(
        std::fs::read_to_string(report.roots["other"].join("client.lua")).unwrap(),
        "sdk.log('old immutable content')"
    );
    server.resource_command("restart other").unwrap();
    let old = offer.revision.clone();
    let until = Instant::now() + Duration::from_secs(3);
    while client.wire.offer().unwrap().revision == old {
        assert!(Instant::now() < until);
        client.step(&mut server);
        std::thread::sleep(Duration::from_millis(2));
    }
    let report = download_set(
        server.resource_address().unwrap(),
        &client.wire.offer().unwrap().revision,
        &cache,
        "immutable",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(report.roots["other"].join("client.lua")).unwrap(),
        "sdk.log('unstarted changed content')"
    );
}
#[test]
fn restarting_dependency_rejects_changed_dependent_manifest_before_retiring() {
    let temp = Temp::new();
    let config = fixture(&temp);
    let root = temp.0.join("resources");
    std::fs::create_dir(root.join("base")).unwrap();
    std::fs::write(
        root.join("base/resource.json"),
        serde_json::to_vec(&json!({
            "format": 1, "api": 1, "id": "base", "version": "1",
            "language": "lua"
        }))
        .unwrap(),
    )
    .unwrap();
    let path = root.join("challenge/resource.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["dependencies"] = json!({"base": "1"});
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut server = host(config);
    let mut guest = Guest::new(server.local_addr().unwrap(), 30);
    guest.wait_offer(&mut server);
    let before = guest.wire.offer().unwrap().clone();
    let status = server.resource_command("resources").unwrap();
    manifest["version"] = json!("2");
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let dependency = manifest["dependencies"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap();
    let error = server
        .resource_command(&format!("restart {dependency}"))
        .unwrap_err();
    assert!(
        error.contains("challenge") && error.contains("manifest changed"),
        "{error}"
    );
    assert_eq!(server.resource_command("resources").unwrap(), status);
    for _ in 0..20 {
        guest.step(&mut server);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(guest.wire.offer(), Some(&before));
}

#[test]
fn distinct_server_configurations_isolate_storage_even_in_same_storage_root() {
    let temp = Temp::new();
    let config = fixture(&temp);
    let other_config = temp.0.join("other-server.json");
    std::fs::copy(&config, &other_config).unwrap();
    let cache = Cache::open(temp.0.join("cache"), Limits::default()).unwrap();
    let mut first = host(config);
    let mut guest = Guest::new(first.local_addr().unwrap(), 20);
    guest.wait_offer(&mut first);
    let (mut lua, _) = guest.activate(&cache, "server-a");
    assert_eq!(guest.wait_state(&mut first, &mut lua)["visits"], 1);
    drop(first);
    let mut second = host(other_config);
    let mut guest = Guest::new(second.local_addr().unwrap(), 21);
    guest.wait_offer(&mut second);
    let (mut lua, _) = guest.activate(&cache, "server-b");
    assert_eq!(
        guest.wait_state(&mut second, &mut lua)["visits"],
        1,
        "server config paths must isolate persistent resource data"
    );
}

#[test]
fn resource_database_transactions_and_generation_cleanup_survive_server_restart() {
    let temp = Temp::new();
    let config = fixture(&temp);
    let manifest = temp.0.join("resources/challenge/resource.json");
    let mut data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    data["capabilities"]
        .as_array_mut()
        .unwrap()
        .extend([json!("resource.events"), json!("resource.database")]);
    std::fs::write(manifest, serde_json::to_vec(&data).unwrap()).unwrap();
    let mut data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    data["grants"]["challenge"]
        .as_array_mut()
        .unwrap()
        .extend([json!("resource.events"), json!("resource.database")]);
    std::fs::write(&config, serde_json::to_vec(&data).unwrap()).unwrap();
    std::fs::write(temp.0.join("resources/challenge/private.lua"),r#"
resource.on('service_result', function(event)
    assert(event.result.ok, 'backend operation failed')
    if event.key=='migration' then
        resource.services.submit('increment', {kind='transaction',statements={
            {sql='INSERT INTO progress(id,visits) VALUES (1,1) ON CONFLICT(id) DO UPDATE SET visits=visits+1'},
            {sql='SELECT visits FROM progress WHERE id=?',params={{type='integer',value=1}}}
        }},5000)
    elseif event.key=='increment' then
        resource.state.set('visits',event.result.value.results[2].rows[1][1].value)
    end
end)
resource.services.submit('migration',{kind='migrate',migrations={{version=1,statements={
    {sql='CREATE TABLE progress(id INTEGER PRIMARY KEY, visits INTEGER NOT NULL)'}
}}}},5000)
"#).unwrap();
    for expected in [1, 2] {
        let mut server = host(config.clone());
        let mut guest = Guest::new(server.local_addr().unwrap(), 2);
        guest.wait_offer(&mut server);
        // This client does not run gameplay; it exercises admission and actual state delivery.
        guest.wire.set_ready(true);
        let until = Instant::now() + Duration::from_secs(5);
        let mut seen = false;
        while Instant::now() < until {
            guest.step(&mut server);
            for m in guest.wire.take_incoming() {
                if m.name == "visits" {
                    assert_eq!(m.value, json!(expected));
                    seen = true;
                }
            }
            if seen {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(seen, "database result never reached resource state");
        assert!(
            server
                .resource_command("stop challenge")
                .unwrap()
                .contains("stop")
        );
        server.shutdown();
    }
}

#[test]
fn resource_granted_teleport_resets_without_a_one_second_observation_gap() {
    let temp = Temp::new();
    let config = fixture(&temp);
    let manifest = temp.0.join("resources/challenge/resource.json");
    let mut data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    data["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!("resource.teleport"));
    std::fs::write(manifest, serde_json::to_vec(&data).unwrap()).unwrap();
    let mut data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    data["grants"]["challenge"]
        .as_array_mut()
        .unwrap()
        .push(json!("resource.teleport"));
    std::fs::write(&config, serde_json::to_vec(&data).unwrap()).unwrap();
    std::fs::write(
        temp.0.join("resources/challenge/private.lua"),
        r#"
resource.on_net('travel',function(payload,sender)
    resource.teleport(sender,{position={400,20,100},instance=7})
end)
"#,
    )
    .unwrap();
    let mut server = host(config);
    let mut guest = Guest::new(server.local_addr().unwrap(), 2);
    guest.wait_offer(&mut server);
    guest.wire.set_ready(true);
    guest
        .wire
        .emit("challenge", 1, "travel", json!({}))
        .unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        guest.step(&mut server);
        if let Some(reset) = guest.session.pending_movement_reset() {
            assert_eq!(reset.destination.position, [400., 20., 100.]);
            assert_eq!(reset.destination.instance, 7);
            break;
        }
        assert!(
            Instant::now() < until,
            "resource teleport was not delivered"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn failed_service_callback_retires_already_drained_later_operations() {
    let temp = Temp::new();
    let config = fixture(&temp);
    for path in [
        temp.0.join("resources/challenge/resource.json"),
        config.clone(),
    ] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let caps = if path == config {
            &mut value["grants"]["challenge"]
        } else {
            &mut value["capabilities"]
        };
        caps.as_array_mut()
            .unwrap()
            .extend([json!("resource.events"), json!("resource.database")]);
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    std::fs::write(temp.0.join("resources/challenge/private.lua"),r#"
resource.on('service_result',function() error('retire this resource') end)
resource.services.submit('invalid',{kind='query',statement={sql='SELECT 1'}},120000)
resource.services.submit('must_not_run',{kind='transaction',statements={{sql='CREATE TABLE forbidden(value INTEGER)'}}},5000)
"#).unwrap();
    let mut server = host(config);
    for _ in 0..20 {
        server.step().unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        server
            .resource_command("resources")
            .unwrap()
            .contains("challenge: stopped")
    );
    fn databases(root: &std::path::Path) -> usize {
        std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .map(|e| {
                if e.path().is_dir() {
                    databases(&e.path())
                } else {
                    usize::from(e.path().extension().is_some_and(|ext| ext == "sqlite3"))
                }
            })
            .sum()
    }
    assert_eq!(
        databases(&temp.0.join("store/services")),
        0,
        "retired resource created database"
    );
}

#[test]
fn downloaded_javascript_executes_on_both_sides_over_real_udp_and_http() {
    let temp = Temp::new();
    let config = fixture(&temp);
    let root = temp.0.join("resources/challenge");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("resource.json")).unwrap()).unwrap();
    manifest["language"] = json!("javascript");
    manifest["client_scripts"] = json!(["client.js"]);
    manifest["server_scripts"] = json!(["private.js"]);
    std::fs::write(
        root.join("resource.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("client.js"),
        "sdk.ui.text('welcome', sdk.readText('title.txt')); resource.send('join',{action:'join'});",
    )
    .unwrap();
    std::fs::write(root.join("private.js"),"resource.onNet('join',(data,sender)=>{ if(data.action!=='join')return; const visits=(resource.storage.get('visits')||0)+1; resource.storage.set('visits',visits); resource.state.set('joined',{sender,visits}); });").unwrap();
    let mut server = host(config);
    let cache = Cache::open(temp.0.join("js-cache"), Limits::default()).unwrap();
    let mut guest = Guest::new(server.local_addr().unwrap(), 2);
    guest.wait_offer(&mut server);
    let (mut runtime, bytes) = guest.activate(&cache, "javascript-server");
    assert!(bytes > 0);
    let state = guest.wait_state(&mut server, &mut runtime);
    assert_eq!(state["visits"], 1);
    assert_eq!(state["sender"], "2");
    let old_revision = guest.wire.offer().unwrap().revision.clone();
    server.resource_command("restart challenge").unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    while guest
        .wire
        .offer()
        .is_some_and(|o| o.revision == old_revision)
    {
        guest.step(&mut server);
        assert!(Instant::now() < until);
    }
    assert!(!guest.wire.ready());
    runtime.disconnect();
    assert!(!runtime.running("challenge"));
}

#[test]
fn explicit_bulk_script_handles_report_delivery_cancellation_and_deadlines_over_udp() {
    let temp=Temp::new();let config=fixture(&temp);
    let root=temp.0.join("resources/challenge");
    let mut manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("resource.json")).unwrap()).unwrap();
    manifest["capabilities"].as_array_mut().unwrap().push(json!("resource.events"));
    std::fs::write(root.join("resource.json"),serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut configuration:serde_json::Value=serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    configuration["grants"]["challenge"].as_array_mut().unwrap().push(json!("resource.events"));
    std::fs::write(&config,serde_json::to_vec(&configuration).unwrap()).unwrap();
    std::fs::write(root.join("client.lua"),"sdk.ui.text('bulk','Bulk transfers')").unwrap();
    std::fs::write(root.join("private.lua"),r#"
        resource.on("transfer_progress", function(event)
            resource.state.set("transfer_"..event.key, event)
        end)
        resource.on_net("upload", function(payload,sender)
            if type(payload)~="string" or #payload~=12000 then return end
            resource.transfer.start("delivered", "download", payload, {recipient=sender,timeout_ms=15000})
            resource.transfer.start("cancelled", "never_cancelled", payload, {recipient=sender})
            resource.transfer.cancel("cancelled")
            resource.transfer.start("expired", "never_expired", payload, {recipient=sender,timeout_ms=1})
        end)
    "#).unwrap();
    let mut server=host(config);let mut guest=Guest::new(server.local_addr().unwrap(),2);
    guest.wait_offer(&mut server);
    let cache=Cache::open(temp.0.join("cache"),Limits::default()).unwrap();
    let (_lua,_)=guest.activate(&cache,"bulk-real-udp");
    let ticket=guest.wire.start_large("challenge",1,"upload",json!("x".repeat(12000))).unwrap();
    let mut events=Vec::new();let mut terminal=std::collections::BTreeMap::new();
    let until=Instant::now()+Duration::from_secs(18);
    while Instant::now()<until {
        guest.step(&mut server);
        for message in guest.wire.take_incoming() {
            if message.kind==Kind::Event {events.push(message);}
            else if message.name.starts_with("transfer_") {terminal.insert(message.name,message.value);}
        }
        if terminal.get("transfer_delivered").is_some_and(|v|v["state"]=="delivered")
            && terminal.get("transfer_cancelled").is_some_and(|v|v["state"]=="cancelled")
            && terminal.get("transfer_expired").is_some_and(|v|v["state"]=="timed_out") {break;}
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(events.len(),1,"only completed transfer reaches remote callback");
    assert_eq!(events[0].name,"download");assert_eq!(events[0].value.as_str().unwrap().len(),12000);
    assert!(matches!(guest.wire.large_progress(ticket).unwrap(),skate_net::bulk::Progress::Delivered{..}));
    for (key,state) in [("delivered","delivered"),("cancelled","cancelled"),("expired","timed_out")] {
        assert_eq!(terminal.get(&format!("transfer_{key}")).unwrap()["state"],state);
    }
    assert!(terminal["transfer_delivered"]["acknowledged_bytes"].as_u64().unwrap()>12000);
    let metrics:serde_json::Value=serde_json::from_str(&server.resource_command("metrics challenge").unwrap()).unwrap();
    assert_eq!(metrics["generation"],1);
    assert_eq!(metrics["running"],true);
    assert!(metrics["invocations"].as_u64().unwrap()>0);
    assert!(metrics["lua_heap_bytes"].as_u64().unwrap()>0);
    assert!(server.resource_command("metrics absent").is_err());
    server.shutdown();
}
