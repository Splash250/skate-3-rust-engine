use serde_json::{Value, json};
use skate_mods::resources::{Host as Runtime, InstalledResource, Side};
use skate_net::{
    lobby::{Info, Session},
    resources::{CLIENT_KEY, Client, Kind, ServerRecord, server_key},
};
use skate_server::{Host, Map, Options};
use std::{
    collections::BTreeSet,
    net::UdpSocket,
    path::PathBuf,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skate-settings-integration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(path.join("resources/rules")).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn setup(f: &Fixture) -> PathBuf {
    let manifest = json!({"format":1,"api":1,"id":"rules","version":"1.0.0","language":"lua","client_scripts":["client.lua"],"server_scripts":["server.lua"],"capabilities":["resource.settings","resource.state"],"settings":{
        "duration":{"type":"integer","default":60,"min":10,"max":300,"visibility":"replicated","change":"live"},
        "mode":{"type":"enum","default":"practice","options":["practice","ranked"],"visibility":"public","change":"restart"},
        "private_key":{"type":"string","default":"private-default","visibility":"private"}}});
    std::fs::write(
        f.0.join("resources/rules/resource.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(f.0.join("resources/rules/server.lua"),"return {on_load=function()resource.state.set('loaded_duration',resource.settings.get('duration'))end}").unwrap();
    std::fs::write(f.0.join("resources/rules/client.lua"),"return {on_load=function()sdk.log('initial:'..resource.settings.get('duration'))end,on_settings=function(v)sdk.log('changed:'..v.key)end}").unwrap();
    let config = f.0.join("server.json");
    std::fs::write(&config,serde_json::to_vec(&json!({"root":"resources","storage":"data","ensure":["rules"],"grants":{"rules":["resource.settings","resource.state"]},"settings":{"rules":{"duration":90,"private_key":"operator-secret"}}})).unwrap()).unwrap();
    config
}
fn make_host(path: PathBuf) -> Host {
    Host::bind(Options {
        bind: "127.0.0.1:0".parse().unwrap(),
        session: 7,
        max_players: 2,
        map: Map::TestWorld,
        resources: Some(path),
        accounts: None,
        operations: None,
    })
    .unwrap()
}
#[test]
fn settings_validation_persistence_and_private_projection_use_real_udp() {
    let f = Fixture::new();
    let config = setup(&f);
    assert!(
        skate_server::resources::validate_configuration(&config)
            .unwrap()
            .contains("valid")
    );
    assert!(!f.0.join("data").exists(), "dry-run does not open stores");
    let mut host = make_host(config.clone());
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut session = Session::dedicated_client(
        7,
        Info {
            id: 22,
            map: host.map_fingerprint(),
            rig: 2,
            physics: 3,
            appearance: 4,
        },
        1,
    );
    let mut wire = Client::default();
    let start = Instant::now();
    let mut first = None;
    while start.elapsed() < Duration::from_secs(4) {
        let now = start.elapsed().as_millis() as u64;
        if wire.offer().is_some() {
            wire.set_ready(true);
            session.publish_application(CLIENT_KEY, wire.encode().unwrap(), now);
        }
        for p in session.service(now) {
            socket.send_to(&p.data, host.local_addr().unwrap()).unwrap();
        }
        host.step().unwrap();
        let mut bytes = [0; 2048];
        while let Ok((n, _)) = socket.recv_from(&mut bytes) {
            session.receive(1, &bytes[..n], now);
        }
        if let Some(actor) = session.host_actor() {
            if let Some(record) = session.actors[&actor].application.get(&server_key(22)) {
                wire.receive(&serde_json::from_slice::<ServerRecord>(&record.value).unwrap())
                    .unwrap();
            }
        }
        for m in wire.take_incoming() {
            if m.kind == Kind::State && m.name == "__settings" {
                first = Some(m);
            }
        }
        if first.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let first = first.expect("initial setting snapshot delivered over actual UDP state lane");
    assert_eq!(first.value["duration"], 90);
    assert_eq!(first.value["mode"], "practice");
    assert!(first.value.get("private_key").is_none());
    let manifest = skate_resources::Manifest::read(&f.0.join("resources/rules"))
        .unwrap()
        .client_projection();
    let projection = serde_json::to_string(&manifest).unwrap();
    assert!(!projection.contains("private_key"));
    assert!(!projection.contains("private-default"));
    let mut client = Runtime::new(Side::Client, f.0.join("client"), "settings-test").unwrap();
    client
        .install(vec![InstalledResource {
            manifest,
            root: f.0.join("resources/rules"),
            generation: first.generation,
            grants: BTreeSet::from(["resource.settings".into(), "resource.state".into()]),
        }])
        .unwrap();
    client
        .apply_scoped_state(
            "rules",
            first.generation,
            "__settings",
            first.value,
            json!({"kind":"resource"}),
        )
        .unwrap();
    client.start_all().unwrap();
    assert!(
        client
            .drain_outputs()
            .iter()
            .any(|v| format!("{v:?}").contains("initial:90")),
        "on_load sees validated server override"
    );
    assert!(host.resource_command("setting rules duration 301").is_err());
    assert!(
        host.resource_command("setting rules duration 110")
            .unwrap()
            .contains("true")
    );
    let pending: Value = serde_json::from_str(
        &host
            .resource_command("setting rules mode \"ranked\"")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(pending["restart_required"], true);
    let values: Value =
        serde_json::from_str(&host.resource_command("settings rules").unwrap()).unwrap();
    assert_eq!(values["mode"]["value"], "practice");
    assert_eq!(values["mode"]["pending"], "ranked");
    assert!(
        !host
            .resource_command("profile")
            .unwrap()
            .contains("operator-secret")
    );
    let export = f.0.join("trace.json");
    host.resource_command(&format!("profile-export {}", export.display()))
        .unwrap();
    let trace: Value = serde_json::from_slice(&std::fs::read(export).unwrap()).unwrap();
    assert!(trace["traceEvents"].is_array());
    assert!(!trace.to_string().contains("operator-secret"));
    host.resource_command("restart rules").unwrap();
    let values: Value =
        serde_json::from_str(&host.resource_command("settings rules").unwrap()).unwrap();
    assert_eq!(values["mode"]["value"], "ranked");
    drop(host);
    let mut reopened = make_host(config);
    let values: Value =
        serde_json::from_str(&reopened.resource_command("settings rules").unwrap()).unwrap();
    assert_eq!(values["duration"]["value"], 110);
    assert_eq!(values["mode"]["value"], "ranked");
}
