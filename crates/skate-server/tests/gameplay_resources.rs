//! Shipped gameplay scripts with real Lua VMs and asynchronous SQLite workers.
//! Account snapshots here are trusted host fixtures, not authentication evidence.
use serde_json::{Value, json};
use skate_mods::resources::{Host, InstalledResource, Output, Side};
use skate_resources::Manifest;
use skate_services::{Grants, Limits, Owner, Services};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

const RESOURCES: &[&str] = &[
    "platform-profiles",
    "platform-crews",
    "platform-rounds",
    "platform-map-vote",
    "platform-leaderboards",
    "platform-tournaments",
];
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "skate-gameplay-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Runtime {
    host: Host,
    services: Services,
    owners: BTreeMap<String, (u64, Owner)>,
    pending: BTreeMap<u64, (String, u64, String)>,
    players: Value,
    operations: Vec<Output>,
    hold_commits: bool,
    lose_commit_completion: bool,
    committed_without_completion: bool,
}
impl Runtime {
    fn new(root: &Path) -> Self {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
        let mut installed: Vec<_> = RESOURCES
            .iter()
            .map(|id| {
                let folder = source.join(id);
                let manifest = Manifest::read(&folder).unwrap();
                InstalledResource {
                    grants: manifest.capabilities.iter().cloned().collect(),
                    manifest,
                    root: folder,
                    generation: 1,
                }
            })
            .collect();
        let driver = root.join("driver");
        std::fs::create_dir_all(&driver).unwrap();
        std::fs::write(driver.join("server.lua"),"resource.on('invoke',function(p) resource.state.set('response',resource.call(p.resource,p.name,p.value) or false) end)").unwrap();
        let manifest:Manifest=serde_json::from_value(json!({"format":1,"api":1,"id":"driver","version":"1.0.0","language":"lua","server_scripts":["server.lua"],"capabilities":["resource.events","resource.state","resource.exports"],"dependencies":RESOURCES.iter().map(|id|(id.to_string(),"1.0.0")).collect::<BTreeMap<_,_>>() })).unwrap();
        installed.push(InstalledResource {
            grants: manifest.capabilities.iter().cloned().collect(),
            manifest,
            root: driver,
            generation: 1,
        });
        let mut host = Host::new(Side::Server, root.join("storage"), "gameplay-fixture").unwrap();
        host.install(installed).unwrap();
        host.start_all().unwrap();
        Self {
            host,
            services: Services::new(root.join("databases"), Limits::default()).unwrap(),
            owners: BTreeMap::new(),
            pending: BTreeMap::new(),
            players: json!([]),
            operations: Vec::new(),
            hold_commits: false,
            lose_commit_completion: false,
            committed_without_completion: false,
        }
    }
    fn flush(&mut self) {
        for (id, installed) in self.host.installed() {
            let generation = self.host.generation(id).unwrap();
            if self.host.running(id)
                && self
                    .owners
                    .get(id)
                    .is_none_or(|(old, _)| *old != generation)
            {
                let owner = self
                    .services
                    .activate(
                        id,
                        generation,
                        Grants {
                            database: installed.grants.contains("resource.database"),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                self.owners.insert(id.clone(), (generation, owner));
            }
        }
        for output in self.host.drain_outputs() {
            match output {
                Output::Service {
                    resource,
                    generation,
                    key,
                    operation,
                    timeout_ms,
                } => {
                    if self.hold_commits && key == "commit" {
                        continue;
                    }
                    let request = serde_json::from_value(operation).unwrap();
                    let ticket = self
                        .services
                        .submit(
                            &self.owners[&resource].1,
                            request,
                            Duration::from_millis(timeout_ms),
                        )
                        .unwrap();
                    self.pending.insert(ticket, (resource, generation, key));
                }
                Output::State { .. } | Output::Log { .. } => {}
                other => self.operations.push(other),
            }
        }
        while let Some(completion) = self.services.poll() {
            let Some((resource, generation, key)) = self.pending.remove(&completion.id) else {
                continue;
            };
            if key == "commit" && self.lose_commit_completion {
                assert!(
                    completion.result.is_ok(),
                    "commit must really reach SQLite before simulating lost acknowledgement"
                );
                self.committed_without_completion = true;
                continue;
            }
            let result = match completion.result {
                Ok(value) => json!({"ok":true,"value":value}),
                Err(error) => json!({"ok":false,"error":error}),
            };
            if self.host.generation(&resource) == Some(generation) {
                self.host
                    .service_result(&resource, generation, &key, result)
                    .unwrap();
            }
        }
        assert!(
            self.host.diagnostics.is_empty(),
            "runtime failure: {:?}",
            self.host.diagnostics
        );
    }
    fn tick(&mut self, dt: f64) {
        self.host.tick(
            dt,
            json!({"players":self.players,"network":{"active":true,"is_host":true}}),
        );
        self.flush();
    }
    fn until(&mut self, mut condition: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.tick(0.01);
            if condition(self) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "condition timed out; states={:?}",
                self.host.scoped_states()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn invoke(&mut self, resource: &str, name: &str, value: Value) -> Value {
        self.host
            .host_event(
                "driver",
                self.host.generation("driver").unwrap(),
                "invoke",
                json!({"resource":resource,"name":name,"value":value}),
            )
            .unwrap();
        self.host.state("driver", "response").unwrap()
    }
    fn event(&mut self, actor: u64, resource: &str, name: &str, payload: Value) {
        self.host
            .receive(
                actor,
                resource,
                self.host.generation(resource).unwrap(),
                name,
                payload,
            )
            .unwrap();
        self.flush();
    }
    fn private(&self, resource: &str, actor: u64, key: &str) -> Option<Value> {
        self.host
            .scoped_state(
                resource,
                key,
                &json!({"kind":"player","id":actor.to_string()}),
            )
            .unwrap()
    }
    fn ready(&mut self) {
        self.until(|runtime| runtime.host.state("platform-leaderboards", "top").is_some());
    }
}
fn players() -> Value {
    json!([{"id":"10","account_id":"verified-a","instance":"0","position":[-8,1,0]},{"id":"20","account_id":"verified-b","instance":"0","position":[-10,1,0]},{"id":"30","instance":"0","position":[-12,1,0]}])
}

#[test]
fn profiles_crews_reconnect_restart_and_actor_spoof_denial() {
    let temp = Temp::new();
    let mut runtime = Runtime::new(&temp.0);
    runtime.players = players();
    runtime.ready();
    runtime.until(|r| {
        r.private("platform-profiles", 10, "profile").is_some()
            && r.private("platform-profiles", 20, "profile").is_some()
    });
    runtime.event(
        10,
        "platform-profiles",
        "rename",
        json!({"name":"Alice","account_id":"verified-b","actor":"20"}),
    );
    runtime.until(|r| {
        r.private("platform-profiles", 10, "profile")
            .is_some_and(|p| p["name"] == "Alice")
    });
    assert_eq!(
        runtime.private("platform-profiles", 20, "profile").unwrap()["name"],
        "Skater"
    );
    runtime.event(
        30,
        "platform-profiles",
        "rename",
        json!({"name":"Imposter","account_id":"verified-a"}),
    );
    assert!(
        runtime
            .private("platform-profiles", 30, "profile")
            .is_none()
    );
    runtime.event(
        10,
        "platform-crews",
        "crew",
        json!({"action":"create","name":"Courtyard"}),
    );
    runtime.event(
        20,
        "platform-crews",
        "crew",
        json!({"action":"invite","actor":"30","account_id":"verified-a"}),
    );
    assert_eq!(
        runtime.private("platform-crews", 20, "crew"),
        Some(json!(false))
    );
    runtime.event(
        10,
        "platform-crews",
        "crew",
        json!({"action":"invite","actor":"20"}),
    );
    runtime.event(20, "platform-crews", "crew", json!({"action":"accept"}));
    assert_eq!(
        runtime.private("platform-crews", 20, "crew").unwrap()["leader"],
        "verified-a"
    );
    runtime.players = json!([]);
    runtime.tick(0.01);
    runtime.players =
        json!([{"id":"40","account_id":"verified-a","instance":"7","position":[0,1,0]}]);
    runtime.until(|r| {
        r.private("platform-profiles", 40, "profile")
            .is_some_and(|p| p["name"] == "Alice" && p["visits"] == 2)
    });
    runtime.tick(1.1);
    assert_eq!(
        runtime.private("platform-crews", 40, "crew").unwrap()["name"],
        "Courtyard"
    );
    runtime.host.restart("platform-profiles").unwrap();
    runtime.ready();
    runtime.until(|r| {
        r.private("platform-profiles", 40, "profile")
            .is_some_and(|p| p["name"] == "Alice")
    });
    runtime.tick(1.1);
    assert_eq!(
        runtime.private("platform-crews", 40, "crew").unwrap()["members"]["verified-b"],
        true
    );
    assert!(
        runtime
            .host
            .receive(
                10,
                "platform-profiles",
                1,
                "rename",
                json!({"name":"Stale"})
            )
            .is_err()
    );
}

#[test]
fn leaderboard_rejects_network_scores_and_replays_durable_outbox_after_restart() {
    let temp = Temp::new();
    let mut runtime = Runtime::new(&temp.0);
    runtime.players = players();
    runtime.ready();
    assert_eq!(
        runtime.invoke(
            "platform-leaderboards",
            "start",
            json!({"player":"30","rules":"course-v1"})
        )["ok"],
        false
    );
    let attempt = runtime.invoke(
        "platform-leaderboards",
        "start",
        json!({"player":"10","rules":"course-v1","tournament":"test"}),
    );
    assert_eq!(attempt["ok"], true);
    assert!(runtime.host.receive(10,"platform-leaderboards",1,"competition_result",json!({"kind":"completed","player":"10","score":999999,"verified_rules":"course-v1"})).is_err());
    assert!(
        runtime
            .host
            .receive(
                10,
                "platform-leaderboards",
                1,
                "service_result",
                json!({"key":"commit","result":{"ok":true}})
            )
            .is_err()
    );
    runtime.hold_commits = true;
    runtime
        .host
        .host_event(
            "platform-leaderboards",
            1,
            "competition_result",
            json!({"kind":"completed","player":"10","score":175,"verified_rules":"course-v1"}),
        )
        .unwrap();
    runtime.flush();
    assert!(
        runtime
            .host
            .state("platform-leaderboards", "last_result")
            .is_none()
    );
    drop(runtime);
    let mut recovered = Runtime::new(&temp.0);
    recovered.players = players();
    recovered.until(|r| {
        r.host
            .state("platform-leaderboards", "last_result")
            .is_some_and(|p| p["status"] == "committed")
    });
    assert_eq!(
        recovered
            .host
            .state("platform-leaderboards", "last_result")
            .unwrap()["id"],
        attempt["id"]
    );
    assert_eq!(
        recovered
            .host
            .state("platform-leaderboards", "top")
            .unwrap()[0]["score"],
        175.0
    );
    drop(recovered);
    let mut reopened = Runtime::new(&temp.0);
    reopened.players = players();
    reopened.ready();
    let top = reopened.host.state("platform-leaderboards", "top").unwrap();
    assert_eq!(top.as_array().unwrap().len(), 1);
    assert_eq!(top[0]["account"], "verified-a");
}

#[test]
fn rounds_and_votes_keep_instance_membership_permissions_and_account_fairness() {
    let temp = Temp::new();
    let mut runtime = Runtime::new(&temp.0);
    runtime.players = players();
    runtime.ready();
    assert!(
        runtime
            .host
            .command(10, "map_vote", vec![], &Default::default())
            .is_err()
    );
    assert!(
        runtime
            .host
            .command(10, "tournament_start", vec![], &Default::default())
            .is_err()
    );
    runtime
        .host
        .command(0, "map_vote", vec![], &["*".into()].into())
        .unwrap();
    runtime.event(
        10,
        "platform-map-vote",
        "vote",
        json!({"map":"creator-park"}),
    );
    runtime.players[0]["id"] = json!("40");
    runtime.tick(1.1);
    runtime.event(
        40,
        "platform-map-vote",
        "vote",
        json!({"map":"creator-park"}),
    );
    assert_eq!(
        runtime.host.state("platform-map-vote", "vote").unwrap()["counts"]["creator-park"],
        1
    );
    runtime.event(
        999,
        "platform-map-vote",
        "vote",
        json!({"map":"community-park"}),
    );
    assert_eq!(
        runtime.host.state("platform-map-vote", "vote").unwrap()["counts"]["community-park"],
        0
    );
    runtime.players[0]["instance"] = json!("7");
    runtime.tick(1.1);
    let round = runtime
        .host
        .scoped_state(
            "platform-rounds",
            "round",
            &json!({"kind":"instance","id":"0"}),
        )
        .unwrap()
        .unwrap();
    assert!(round["participants"].get("40").is_none());
    for _ in 0..3 {
        runtime.tick(10.0);
    }
    assert!(runtime.operations.iter().any(|o|matches!(o,Output::World{operation,..} if operation["op"]=="select"&&operation["resource"]=="creator-park")));
    runtime.event(
        30,
        "platform-tournaments",
        "enroll",
        json!({"account_id":"verified-a"}),
    );
    assert_eq!(
        runtime.invoke("platform-tournaments", "inspect", Value::Null)["entrants"],
        json!({})
    );
}

/// Drives the shipped ranked course using the actual network movement validator
/// and swept course verifier. Only the resulting host event reaches the scripts.
#[test]
fn actual_course_verifier_completion_persists_the_ranked_result() {
    use skate_net::{
        Body, Pose,
        dedicated::{Config, Server},
        lobby::{Info, Session},
        packed::{self, BodyState, Packed},
    };
    use skate_server::{competition::Competition, world::Terrain};
    use std::sync::Arc;
    let temp = Temp::new();
    let mut runtime = Runtime::new(&temp.0);
    runtime.players = players();
    runtime.ready();
    let mut server = Server::new(Config {
        session: 7,
        server_id: 99,
        map: 1,
        max_players: 4,
    })
    .unwrap();
    let mut client = Session::dedicated_client(
        7,
        Info {
            id: 10,
            map: 1,
            rig: 3,
            physics: 4,
            appearance: 5,
        },
        99,
    );
    client.set_loopback(true);
    let terrain = Terrain {
        revision: "ranked-fixture".into(),
        triangles: Arc::new(vec![
            [[-30., 0., -20.], [30., 0., 20.], [30., 0., -20.]],
            [[-30., 0., -20.], [-30., 0., 20.], [30., 0., 20.]],
        ]),
        spawn: [-8., 1., 0.],
        heading: 0.,
    };
    let mut competition = Competition::default();
    competition.set_terrain(Some(&terrain)).unwrap();
    competition.sync_resources(BTreeMap::from([("platform-leaderboards".into(), 1)]));
    let mut now = 0;
    fn tick(
        server: &mut Server,
        client: &mut Session,
        competition: &mut Competition,
        now: &mut u64,
        x: f32,
    ) -> Vec<skate_server::competition::OwnedEvent> {
        *now += 20;
        if let Some(reset) = client.pending_movement_reset() {
            client.complete_movement_reset(reset.epoch);
        }
        let pose = Pose {
            p: [x, 1., 0.],
            q: [0., 0., 0., 1.],
        };
        client.publish(
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
            *now,
        );
        for packet in client.service(*now) {
            server.receive(11, &packet.data, *now);
        }
        for packet in server.service(*now) {
            client.receive(99, &packet.data, *now);
        }
        competition.step(server, &[])
    }
    for _ in 0..15 {
        tick(&mut server, &mut client, &mut competition, &mut now, -8.);
    }
    assert_eq!(server.player_count(), 1);
    let attempt = runtime.invoke(
        "platform-leaderboards",
        "start",
        json!({"player":"10","rules":"course-v1"}),
    );
    assert_eq!(attempt["ok"], true);
    runtime.flush();
    for output in std::mem::take(&mut runtime.operations) {
        if let Output::Competition {
            resource,
            generation,
            operation,
        } = output
        {
            competition
                .command(&resource, generation, operation, &mut server)
                .unwrap();
        }
    }
    for packet in server.service(now) {
        client.receive(99, &packet.data, now);
    }
    let mut completed = Vec::new();
    for step in 0..=225 {
        completed.extend(tick(
            &mut server,
            &mut client,
            &mut competition,
            &mut now,
            -8. + step as f32 * 0.04,
        ));
    }
    let event = completed
        .iter()
        .find(|event| event.value["kind"] == "completed")
        .unwrap_or_else(|| panic!("actual verifier did not complete: {completed:?}"));
    assert_eq!(event.value["verified_rules"], "course-v1");
    assert_eq!(event.value["score"], 175);
    runtime
        .host
        .host_event(
            &event.resource,
            event.generation,
            "competition_result",
            event.value.clone(),
        )
        .unwrap();
    runtime.flush();
    runtime.until(|r| {
        r.host
            .state("platform-leaderboards", "last_result")
            .is_some_and(|p| p["status"] == "committed")
    });
    assert_eq!(
        runtime.host.state("platform-leaderboards", "top").unwrap()[0]["score"],
        175.0
    );
}

#[test]
fn creator_park_scripts_read_exported_markers_and_publish_private_guidance() {
    let temp = Temp::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/creator-park");
    let manifest = Manifest::read(&root).unwrap();
    let mut host = Host::new(Side::Server, temp.0.join("server"), "creator-server").unwrap();
    host.install(vec![InstalledResource {
        grants: manifest.capabilities.iter().cloned().collect(),
        manifest: manifest.clone(),
        root: root.clone(),
        generation: 1,
    }])
    .unwrap();
    host.start_all().unwrap();
    host.tick(
        0.02,
        json!({"players":[{"id":"10","instance":"0","position":[4,1,0]}]}),
    );
    host.dispatch("on_fixed_update", json!({"dt":0.02}));
    assert!(
        host.diagnostics.is_empty(),
        "creator server diagnostics: {:?}",
        host.diagnostics
    );
    let marker = host
        .scoped_state(
            "creator-park",
            "marker",
            &json!({"kind":"player","id":"10"}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(marker["type"], "checkpoint");
    assert_eq!(marker["order"], 1);
    let mut client = Host::new(Side::Client, temp.0.join("client"), "creator-client").unwrap();
    client
        .install(vec![InstalledResource {
            grants: manifest.capabilities.iter().cloned().collect(),
            manifest: manifest.client_projection(),
            root,
            generation: 1,
        }])
        .unwrap();
    client.start_all().unwrap();
    client
        .apply_scoped_state(
            "creator-park",
            1,
            "marker",
            marker,
            json!({"kind":"player","id":"10"}),
        )
        .unwrap();
    client.tick(0.02, json!({"players":[{"id":"10","local":true}]}));
    client.dispatch("on_ui_update", json!({"dt":0.02}));
    assert!(
        client.diagnostics.is_empty(),
        "creator client diagnostics: {:?}",
        client.diagnostics
    );
    assert!(
        !client.drain_commands().is_empty(),
        "creator HUD did not reach real engine command bridge"
    );
}

#[test]
fn leaderboard_recovery_after_database_commit_before_completion_is_idempotent() {
    let temp = Temp::new();
    let mut runtime = Runtime::new(&temp.0);
    runtime.players = players();
    runtime.ready();
    let attempt = runtime.invoke(
        "platform-leaderboards",
        "start",
        json!({"player":"10","rules":"native-input-v1","ticks":60}),
    );
    assert_eq!(attempt["ok"], true);
    runtime.lose_commit_completion = true;
    // Runtime contract test: only trusted host completions may enter this path;
    // native solver correctness and its admission guards have separate tests.
    runtime.host.host_event("platform-leaderboards",1,"competition_result",json!({"kind":"completed","player":"10","score":{"awarded":450},"verified_rules":"native-input-v1"})).unwrap();
    runtime.until(|r| r.committed_without_completion);
    assert!(
        runtime
            .host
            .state("platform-leaderboards", "last_result")
            .is_none()
    );
    drop(runtime);
    let mut recovered = Runtime::new(&temp.0);
    recovered.players = players();
    recovered.until(|r| {
        r.host
            .state("platform-leaderboards", "last_result")
            .is_some_and(|p| p["status"] == "committed")
    });
    assert_eq!(
        recovered
            .host
            .state("platform-leaderboards", "last_result")
            .unwrap()["id"],
        attempt["id"]
    );
    assert_eq!(
        recovered
            .host
            .state("platform-leaderboards", "top")
            .unwrap()[0]["score"],
        450.0
    );
    let owner = &recovered.owners["platform-leaderboards"].1;
    let ticket = recovered
        .services
        .submit(
            owner,
            serde_json::from_value(
                json!({"kind":"query","statement":{"sql":"SELECT COUNT(*) FROM results"}}),
            )
            .unwrap(),
            Duration::from_secs(2),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(completion) = recovered.services.poll() {
            if completion.id == ticket {
                let value = serde_json::to_value(completion.result.unwrap()).unwrap();
                assert_eq!(value["results"][0]["rows"][0][0]["value"], 1);
                break;
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn resource_stop_and_restart_restore_temporary_travel_through_real_udp_readmission() {
    use skate_net::{
        Body, Pose,
        lobby::{Info, Session},
        packed::{self, BodyState, Packed},
        resources::{CLIENT_KEY, Client, ServerRecord, server_key},
    };
    use skate_server::{Host as ServerHost, Map, Options};
    use std::net::UdpSocket;
    struct Guest {
        socket: UdpSocket,
        session: Session,
        wire: Client,
        start: Instant,
        position: [f32; 3],
        instance: u64,
        ready: bool,
        observed: bool,
    }
    impl Guest {
        fn step(&mut self, host: &mut ServerHost) {
            let now = self.start.elapsed().as_millis() as u64;
            if let Some(reset) = self.session.pending_movement_reset() {
                self.position = reset.destination.position;
                self.instance = reset.destination.instance;
                self.session.complete_movement_reset(reset.epoch);
            }
            let pose = Pose {
                p: self.position,
                q: [0., 0., 0., 1.],
            };
            self.session.publish(
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
                now,
            );
            if self.wire.offer().is_some() {
                // A new offer resets Client readiness itself. Pausing download
                // acceptance must not revoke the old epoch before its offer arrives.
                if self.ready {
                    self.wire.set_ready(true);
                }
                self.session
                    .publish_application(CLIENT_KEY, self.wire.encode().unwrap(), now);
            }
            for packet in self.session.service(now) {
                self.socket
                    .send_to(&packet.data, host.local_addr().unwrap())
                    .unwrap();
            }
            host.step().unwrap();
            let mut buf = [0; 2048];
            while let Ok((n, _)) = self.socket.recv_from(&mut buf) {
                self.session.receive(1, &buf[..n], now);
            }
            if let Some(actor) = self.session.host_actor() {
                if let Some(record) = self.session.actors[&actor]
                    .application
                    .get(&server_key(self.session.local))
                {
                    self.wire
                        .receive(&serde_json::from_slice::<ServerRecord>(&record.value).unwrap())
                        .unwrap();
                    for message in self.wire.take_incoming() {
                        if message.name == "observed" && message.value.is_array() {
                            self.observed = true;
                        }
                    }
                }
            }
        }
        fn wait(
            &mut self,
            host: &mut ServerHost,
            mut condition: impl FnMut(&Self, &ServerHost) -> bool,
        ) {
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                self.step(host);
                if condition(self, host) {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "UDP condition timed out, instance {}",
                    self.instance
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    for action in [
        "stop",
        "restart",
        "unrelated_restart_then_stop",
        "return_while_unready",
    ] {
        let temp = Temp::new();
        let resource = temp.0.join("resources/travel");
        std::fs::create_dir_all(&resource).unwrap();
        std::fs::write(resource.join("resource.json"),serde_json::to_vec(&json!({"format":1,"api":1,"id":"travel","version":"1.0.0","language":"lua","server_scripts":["server.lua"],"capabilities":["resource.teleport","resource.commands","resource.state"]})).unwrap()).unwrap();
        std::fs::write(resource.join("server.lua"),"resource.command('visit','travel.manage',function(args) resource.teleport(args[1],{position={5,1,0},instance=1000,restore_on_stop=true}) end) resource.command('return','travel.manage',function(args) resource.teleport(args[1],{restore_previous=true}) end) return {on_update=function(p) for _,player in ipairs(resource.players()) do if player.position then resource.state.set('observed',player.position) end end end}").unwrap();
        let world = temp.0.join("resources/static-world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/creator-park/park.skate"),
            world.join("park.skate"),
        )
        .unwrap();
        std::fs::write(world.join("resource.json"),serde_json::to_vec(&json!({"format":1,"api":1,"id":"static-world","version":"1.0.0","language":"lua","files":["park.skate"],"world":{"map":"park.skate","required":true}})).unwrap()).unwrap();
        let other = temp.0.join("resources/other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            other.join("resource.json"),
            serde_json::to_vec(
                &json!({"format":1,"api":1,"id":"other","version":"1.0.0","language":"lua"}),
            )
            .unwrap(),
        )
        .unwrap();
        let config = temp.0.join("server.json");
        std::fs::write(&config,serde_json::to_vec(&json!({"root":"resources","storage":"store","ensure":["travel","static-world","other"],"grants":{"travel":["resource.teleport","resource.commands","resource.state"]}})).unwrap()).unwrap();
        let mut host = ServerHost::bind(Options {
            bind: "127.0.0.1:0".parse().unwrap(),
            session: 7,
            max_players: 4,
            map: Map::TestWorld,
            resources: Some(config),
            accounts: None,
            operations: None,
        })
        .unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut session = Session::dedicated_client(
            7,
            Info {
                id: 2,
                map: skate_net::hash(b"skate-test-world-v1"),
                rig: 2,
                physics: 3,
                appearance: 4,
            },
            1,
        );
        session.set_loopback(true);
        let mut guest = Guest {
            socket,
            session,
            wire: Client::default(),
            start: Instant::now(),
            position: [-8., 1., 0.],
            instance: 0,
            ready: true,
            observed: false,
        };
        guest.wait(&mut host, |guest, host| {
            guest.observed && host.player_count() == 1
        });
        for _ in 0..10 {
            guest.step(&mut host);
            std::thread::sleep(Duration::from_millis(5));
        }
        let original = guest.position;
        host.resource_command("command visit 2").unwrap();
        guest.ready = false;
        guest.wait(&mut host, |guest, _| guest.instance == 1000);
        if action == "unrelated_restart_then_stop" {
            guest.ready = true;
            guest.wait(&mut host, |_, host| host.player_count() == 1);
            host.resource_command("restart other").unwrap();
            guest.wait(&mut host, |_, host| host.player_count() == 1);
            // Flush the resulting trusted world-spawn epoch before retirement.
            for _ in 0..5 {
                guest.step(&mut host);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(guest.instance, 1000);
            host.resource_command("stop travel").unwrap();
        } else if action == "return_while_unready" {
            host.resource_command("command return 2").unwrap();
        } else {
            host.resource_command(&format!("{action} travel")).unwrap();
        }
        for _ in 0..3 {
            guest.step(&mut host);
        }
        if action != "unrelated_restart_then_stop" {
            assert_eq!(
                guest.instance, 1000,
                "lease restored before content readiness"
            );
        }
        // Complete the new offer normally; restoration never bypasses readiness.
        guest.ready = true;
        guest.wait(&mut host, |guest, host| {
            guest.instance == 0 && host.player_count() == 1
        });
        for i in 0..3 {
            assert!(
                (guest.position[i] - original[i]).abs() < 0.01,
                "return position changed {original:?} -> {:?}",
                guest.position
            );
        }
    }
}

#[test]
fn all_shipped_gameplay_sides_survive_empty_initial_snapshot_without_state() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
    let temp = Temp::new();
    for side in [Side::Server, Side::Client] {
        let label = match side {
            Side::Server => "server",
            Side::Client => "client",
        };
        let installed: Vec<_> = RESOURCES
            .iter()
            .copied()
            .chain(["creator-park"])
            .map(|id| {
                let root = source.join(id);
                let manifest = Manifest::read(&root).unwrap();
                InstalledResource {
                    grants: manifest.capabilities.iter().cloned().collect(),
                    manifest: if side == Side::Client {
                        manifest.client_projection()
                    } else {
                        manifest
                    },
                    root,
                    generation: 1,
                }
            })
            .collect();
        let mut host = Host::new(side, temp.0.join(label), label).unwrap();
        host.install(installed).unwrap();
        host.start_all().unwrap();
        for _ in 0..3 {
            host.tick(0.02,json!({"players":[],"entities":[],"network":{"active":true,"is_host":side==Side::Server}}));
            host.dispatch("on_fixed_update", json!({"dt":0.02}));
            host.dispatch("on_ui_update", json!({"dt":0.02}));
        }
        assert!(
            host.diagnostics.is_empty(),
            "{label} empty startup: {:?}",
            host.diagnostics
        );
        for id in RESOURCES.iter().copied().chain(["creator-park"]) {
            assert!(
                host.running(id),
                "{label} resource {id} retired during empty startup"
            );
        }
    }
}

#[test]
fn leaderboard_review_keeps_newest_result_across_decimal_id_rollover() {
    let temp = Temp::new();
    let mut runtime = Runtime::new(&temp.0);
    runtime.players = players();
    runtime.ready();
    let mut first = Value::Null;
    let mut latest = Value::Null;
    for index in 1..=100 {
        let attempt = runtime.invoke(
            "platform-leaderboards",
            "start",
            json!({"player":"10","rules":"course-v1"}),
        );
        assert_eq!(attempt["ok"], true, "attempt {index}: {attempt}");
        let id = attempt["id"].clone();
        if index == 1 {
            first = id.clone();
        }
        latest = id.clone();
        // This trusted host fixture checks cache/persistence contracts, not the
        // motion verifier (which has a separate actual UDP completion test).
        runtime
            .host
            .host_event(
                "platform-leaderboards",
                1,
                "competition_result",
                json!({"kind":"completed","player":"10","score":175,"verified_rules":"course-v1"}),
            )
            .unwrap();
        runtime.until(|r| {
            r.host
                .state("platform-leaderboards", "last_result")
                .is_some_and(|result| result["id"] == id && result["status"] == "committed")
        });
        assert_eq!(
            runtime.invoke("platform-leaderboards", "result", id)["status"],
            "committed",
            "newly committed result evicted at {index}"
        );
    }
    assert_eq!(latest, "local-1:100");
    assert_eq!(
        runtime.invoke("platform-leaderboards", "result", first),
        false,
        "old result must expire from the bounded cache"
    );
    assert_eq!(
        runtime.invoke("platform-leaderboards", "result", latest)["status"],
        "committed"
    );
}

#[test]
fn leaderboard_review_retains_both_rule_categories_after_commit_and_restart() {
    let temp = Temp::new();
    let mut runtime = Runtime::new(&temp.0);
    runtime.ready();
    for (rules, count) in [("course-v1", 20), ("native-input-v1", 11)] {
        for index in 1..=count {
            runtime.players =
                json!([{"id":"10","account_id":format!("{rules}-{index:02}"),"instance":"0"}]);
            runtime.tick(0.01);
            let attempt = runtime.invoke(
                "platform-leaderboards",
                "start",
                json!({"player":"10","rules":rules,"ticks":60}),
            );
            assert_eq!(attempt["ok"], true, "{attempt}");
            let score = if rules == "course-v1" {
                json!(index)
            } else {
                json!({"awarded":1000+index})
            };
            // Trusted host result fixtures exercise actual Lua/SQLite ranking;
            // they are not evidence of native motion or score verification.
            runtime
                .host
                .host_event(
                    "platform-leaderboards",
                    1,
                    "competition_result",
                    json!({"kind":"completed","player":"10","score":score,"verified_rules":rules}),
                )
                .unwrap();
            runtime.until(|r| {
                r.host
                    .state("platform-leaderboards", "last_result")
                    .is_some_and(|p| p["id"] == attempt["id"] && p["status"] == "committed")
            });
        }
    }
    let top = runtime.invoke("platform-leaderboards", "top", Value::Null);
    let check = |top: &Value| {
        let rows = top.as_array().unwrap();
        assert_eq!(rows.len(), 20, "standings remain bounded");
        for rules in ["course-v1", "native-input-v1"] {
            let category: Vec<_> = rows.iter().filter(|row| row["rules"] == rules).collect();
            assert_eq!(category.len(), 10, "category {rules} was crowded out");
            assert!(
                category
                    .windows(2)
                    .all(|pair| pair[0]["score"].as_f64() >= pair[1]["score"].as_f64())
            );
        }
        assert_eq!(
            rows.iter()
                .find(|r| r["rules"] == "native-input-v1")
                .unwrap()["score"],
            1011.0
        );
    };
    check(&top);
    drop(runtime);
    let mut recovered = Runtime::new(&temp.0);
    recovered.ready();
    let restored = recovered.invoke("platform-leaderboards", "top", Value::Null);
    check(&restored);
    assert_eq!(
        restored, top,
        "startup query must preserve the commit ranking contract"
    );
}

#[test]
fn leaderboard_review_client_displays_each_rule_category() {
    let temp = Temp::new();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
    let installed = ["platform-profiles", "platform-leaderboards"]
        .into_iter()
        .map(|id| {
            let root = source.join(id);
            let manifest = Manifest::read(&root).unwrap().client_projection();
            InstalledResource {
                grants: manifest.capabilities.iter().cloned().collect(),
                manifest,
                root,
                generation: 1,
            }
        })
        .collect();
    let mut client = Host::new(Side::Client, temp.0.join("client"), "leaderboard-hud").unwrap();
    client.install(installed).unwrap();
    client.start_all().unwrap();
    let rows: Vec<_> = ["course-v1", "native-input-v1"].into_iter().flat_map(|rules| {
        (1..=10).rev().map(move |rank| json!({"account":format!("{rules}-{rank}"),"rules":rules,"score":if rules == "course-v1" {100+rank} else {1000+rank}}))
    }).collect();
    client
        .apply_state("platform-leaderboards", 1, "top", json!(rows))
        .unwrap();
    client.dispatch("on_ui_update", json!({"dt":0.02}));
    assert!(client.diagnostics.is_empty(), "{:?}", client.diagnostics);
    let overlay = client
        .drain_commands()
        .into_iter()
        .find_map(|(_, command)| match command {
            skate_mods::Command::Overlay { text, .. } if text.contains("VERIFIED STANDINGS") => {
                Some(text)
            }
            _ => None,
        })
        .expect("leaderboard overlay");
    assert!(
        overlay.contains("course-v1") && overlay.contains("native-input-v1"),
        "missing ranking category: {overlay}"
    );
    assert!(
        overlay.contains("110") && overlay.contains("1010"),
        "each category leader must be visible: {overlay}"
    );
    assert!(
        !overlay.contains("1001"),
        "overlay should show a bounded five places per category"
    );
}

#[test]
fn profiles_review_retries_fresh_actor_after_pending_capacity_drains() {
    let temp = Temp::new();
    let mut runtime = Runtime::new(&temp.0);
    runtime.ready();
    let old:Vec<_>=(1..=64).map(|id|json!({"id":id.to_string(),"account_id":format!("old-account-{id}"),"instance":"0"})).collect();
    // Hold these submitted requests at the backend boundary while all actors
    // disconnect. No new profile may be marked loaded merely because it waited.
    runtime.host.tick(0.01, json!({"players":old}));
    let pending: Vec<_> = runtime
        .host
        .drain_outputs()
        .into_iter()
        .filter_map(|output| match output {
            Output::Service {
                resource,
                generation,
                key,
                ..
            } if resource == "platform-profiles" => Some((generation, key)),
            _ => None,
        })
        .collect();
    assert_eq!(pending.len(), 64);
    runtime.players = json!([{"id":"65","account_id":"fresh-account","instance":"0"}]);
    runtime.host.tick(0.01, json!({"players":runtime.players}));
    assert!(
        !runtime.host.drain_outputs().iter().any(
            |output| matches!(output,Output::Service{resource,..} if resource=="platform-profiles")
        ),
        "capacity must stay bounded"
    );
    for (generation, key) in pending {
        runtime.host.service_result("platform-profiles",generation,&key,json!({"ok":false,"error":{"code":"cancelled","message":"old connection backend work retired"}})).unwrap();
    }
    // The fresh request now reaches the real SQLite worker and publishes its
    // account-keyed row without requiring another reconnect.
    runtime.until(|r| r.private("platform-profiles", 65, "profile").is_some());
    let profile = runtime.private("platform-profiles", 65, "profile").unwrap();
    assert_eq!(profile["name"], "Skater");
    assert_eq!(profile["visits"], 1);
    drop(runtime);
    let mut reopened = Runtime::new(&temp.0);
    reopened.players = json!([{"id":"66","account_id":"fresh-account","instance":"0"}]);
    reopened.until(|r| r.private("platform-profiles", 66, "profile").is_some());
    assert_eq!(
        reopened
            .private("platform-profiles", 66, "profile")
            .unwrap()["visits"],
        2
    );
}

#[test]
fn tournament_tolerates_content_gap_and_returns_lease_on_cancel_or_timeout() {
    for outcome in ["timeout", "cancel", "resume"] {
        let temp = Temp::new();
        let mut runtime = Runtime::new(&temp.0);
        runtime.players = players();
        runtime.ready();
        runtime.event(10, "platform-tournaments", "enroll", json!({}));
        runtime
            .host
            .command(0, "tournament_start", vec![], &["*".into()].into())
            .unwrap();
        runtime.tick(0.1);
        assert!(runtime.operations.iter().any(|output| matches!(
            output,
            Output::Teleport {
                player: 10,
                restore_on_stop: true,
                ..
            }
        )));
        runtime.operations.clear();
        runtime.players = json!([]);
        for _ in 0..10 {
            runtime.tick(1.0);
        }
        let state = runtime.invoke("platform-tournaments", "inspect", Value::Null);
        assert_eq!(state["phase"], "running");
        assert!(
            state["results"].get("verified-a").is_none(),
            "normal readiness gap became a forfeit"
        );
        if outcome != "timeout" {
            if outcome == "resume" {
                runtime.players = players();
                runtime.players[0]["instance"] = json!("1000");
                runtime.tick(0.1);
                assert!(runtime.operations.iter().any(|output|matches!(output,Output::Competition{operation,..} if operation["kind"]=="start")),"ready original entrant did not start verified round");
            }
            runtime
                .host
                .command(0, "tournament_cancel", vec![], &["*".into()].into())
                .unwrap();
            runtime.flush();
        } else {
            for _ in 0..7 {
                runtime.tick(1.0);
            }
        }
        assert!(
            runtime.operations.iter().any(|output| matches!(
                output,
                Output::Teleport {
                    player: 10,
                    restore_previous: true,
                    ..
                }
            )),
            "absent original actor was not returned through its lease"
        );
        if outcome == "timeout" {
            let state = runtime.invoke("platform-tournaments", "inspect", Value::Null);
            assert_eq!(state["results"]["verified-a"]["status"], "forfeit");
        }
    }
}

#[test]
fn tournament_publishes_initial_phase_and_withdraws_waiting_current_and_active_entrants() {
    for active in [false, true] {
        let temp = Temp::new();
        let mut runtime = Runtime::new(&temp.0);
        assert_eq!(
            runtime
                .host
                .state("platform-tournaments", "tournament")
                .unwrap()["phase"],
            "enrollment"
        );
        runtime.players = players();
        runtime.ready();
        runtime.event(10, "platform-tournaments", "enroll", json!({}));
        runtime.event(20, "platform-tournaments", "enroll", json!({}));
        runtime
            .host
            .command(0, "tournament_start", vec![], &["*".into()].into())
            .unwrap();
        runtime.tick(0.1);
        runtime.operations.clear();
        runtime.event(20, "platform-tournaments", "leave", json!({"player":"10"}));
        let result = runtime.invoke("platform-tournaments", "inspect", Value::Null);
        assert_eq!(result["results"]["verified-b"]["reason"], "withdrawn");
        assert!(
            result["results"].get("verified-a").is_none(),
            "payload actor spoof affected current entrant"
        );
        if active {
            runtime.players[0]["instance"] = json!("1000");
            runtime.tick(0.1);
            runtime.operations.clear();
        }
        runtime.event(10, "platform-tournaments", "leave", json!({}));
        let result = runtime.invoke("platform-tournaments", "inspect", Value::Null);
        assert_eq!(result["results"]["verified-a"]["reason"], "withdrawn");
        assert!(runtime.operations.iter().any(|output| matches!(
            output,
            Output::Teleport {
                player: 10,
                restore_previous: true,
                ..
            }
        )));
        assert_eq!(runtime.operations.iter().any(|output|matches!(output,Output::Competition{operation,..} if operation["kind"]=="cancel")),active);
        runtime.tick(0.1);
        assert_eq!(
            runtime.invoke("platform-tournaments", "inspect", Value::Null)["phase"],
            "finished"
        );
    }
}
