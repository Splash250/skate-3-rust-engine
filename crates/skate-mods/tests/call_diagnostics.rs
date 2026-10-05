//! Real plugin diagnostics: host-derived authorization and bounded read-only tests.
use serde_json::{Value, json};
use skate_mods::{
    Command,
    resources::{Host, InstalledResource, Output, Side},
};
use skate_resources::Manifest;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    host: Host,
    store: PathBuf,
}
impl Fixture {
    fn new(side: Side) -> Self {
        let store = std::env::temp_dir().join(format!(
            "skate-call-diagnostics-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources");
        let mut host = Host::new(side, store.clone(), "diagnostics-tests").unwrap();
        let mut installed = Vec::new();
        for name in [
            "interaction-policy",
            "inventory-ui",
            "admin-dashboard",
            "phone-calls",
            "phone",
            "call-diagnostics",
        ] {
            let root = base.join(name);
            let manifest = Manifest::read(&root).unwrap();
            let manifest = if side == Side::Client {
                manifest.client_projection()
            } else {
                manifest
            };
            installed.push(InstalledResource {
                grants: manifest.capabilities.iter().cloned().collect(),
                manifest,
                root,
                generation: 1,
            });
        }
        host.install(installed).unwrap();
        host.start_all().unwrap();
        let mut value = Self { host, store };
        value.tick(0.01);
        value.host.drain_outputs();
        value.host.drain_commands();
        value
    }
    fn tick(&mut self, dt: f64) {
        self.host.tick(dt,json!({"players":[{"id":"1","instance":"0"},{"id":"2","instance":"0"},{"id":"3","instance":"0"}],"voice":{"enabled":false},"permissions":["calls.diagnostics","calls.test"]}));
        assert!(
            self.host.running("call-diagnostics"),
            "{:?}",
            self.host.diagnostics
        );
    }
    fn receive(&mut self, actor: u64, seq: &str, action: &str) -> Value {
        self.host.receive(actor,"call-diagnostics",1,"request",json!({"seq":seq,"action":action,"sender":"2","permissions":["calls.diagnostics","calls.test"]})).unwrap();
        let out = self.host.drain_outputs();
        assert!(
            !out.iter().any(|o| matches!(o, Output::Voice { .. })),
            "diagnostics never modifies voice"
        );
        out.into_iter()
            .find_map(|o| match o {
                Output::Event {
                    resource,
                    name,
                    payload,
                    recipient,
                    scope,
                    ..
                } if resource == "call-diagnostics" && name == "result" => {
                    assert_eq!(recipient, Some(actor));
                    assert_eq!(scope, json!({"kind":"player","id":actor.to_string()}));
                    Some(payload)
                }
                _ => None,
            })
            .expect("private diagnostics response")
    }
    fn page(&mut self, event: Value) {
        self.host.call(
            "call-diagnostics",
            "on_event",
            json!({"type":"browser","key":"diagnostics","event":event}),
        );
    }
    fn update(&mut self, dt: f64) {
        self.host
            .call("call-diagnostics", "on_ui_update", json!({"dt":dt}));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.store);
    }
}
#[test]
fn plugin_diagnostics_authorizes_real_sender_revocation_and_read_only_actual_state() {
    let mut f = Fixture::new(Side::Server);
    let revoked = Arc::new(AtomicBool::new(false));
    let revoked_auth = revoked.clone();
    f.host
        .set_authorizer(Some(Arc::new(move |actor, permission| {
            (actor == 1 && permission == "calls.diagnostics")
                || (actor == 2
                    && !revoked_auth.load(Ordering::SeqCst)
                    && ["calls.diagnostics", "calls.test"].contains(&permission))
        })))
        .unwrap();
    let denied = f.receive(3, "1", "inspect");
    assert_eq!(denied["denied"], true);
    assert!(denied.get("value").is_none());
    let counts = f.receive(1, "1", "inspect");
    assert_eq!(counts["value"]["used"], 0);
    assert_eq!(counts["can_test"], false);
    assert_eq!(
        f.receive(1, "2", "test")["denied"],
        true,
        "read permission never authorizes a test"
    );
    assert_eq!(
        f.receive(1, "3", "inspect")["ok"],
        false,
        "authorized request rate is bounded"
    );
    for actor in [1, 2] {
        f.host
            .receive(
                actor,
                "phone-calls",
                1,
                "request",
                json!({"action":"presence","enabled":true}),
            )
            .unwrap();
    }
    f.host
        .receive(
            1,
            "phone-calls",
            1,
            "request",
            json!({"action":"dial","target":"2"}),
        )
        .unwrap();
    f.host.drain_outputs();
    let counts = f.receive(2, "1", "inspect");
    assert_eq!(counts["value"]["ringing"], 1);
    assert_eq!(counts["value"]["used"], 1);
    assert_eq!(counts["value"]["capacity"], 16);
    f.tick(1.1);
    let test = f.receive(2, "2", "test");
    assert_eq!(test["value"]["ok"], true);
    assert_eq!(test["value"]["checked"], 8);
    assert!(
        !serde_json::to_string(&test).unwrap().contains("g1-1"),
        "no call or participant identities leave diagnostics"
    );
    revoked.store(true, Ordering::SeqCst);
    assert_eq!(f.receive(2, "3", "inspect")["denied"], true);
    assert_eq!(f.receive(2, "4", "test")["denied"], true);
    f.host
        .receive(
            2,
            "call-diagnostics",
            1,
            "request",
            json!({"seq":"9".repeat(33),"action":"inspect"}),
        )
        .unwrap();
    assert!(f.host.drain_outputs().is_empty());
    assert!(f.host.running("call-diagnostics"));
}
#[test]
fn dashboard_discards_closed_expired_and_denied_async_results() {
    let mut f = Fixture::new(Side::Client);
    f.host.call(
        "call-diagnostics",
        "on_event",
        json!({"type":"interface","key":"open"}),
    );
    assert!(
        f.host.drain_commands().iter().any(
            |(_, c)| matches!(c,Command::UiBrowserOpen{options,..}if options.surface.is_some())
        )
    );
    f.page(json!({"kind":"ready"}));
    f.update(0.1);
    let request = f
        .host
        .drain_outputs()
        .into_iter()
        .find_map(|o| match o {
            Output::Event {
                resource,
                name,
                payload,
                ..
            } if resource == "call-diagnostics" && name == "request" => Some(payload),
            _ => None,
        })
        .unwrap();
    f.host
        .receive(
            0,
            "call-diagnostics",
            1,
            "result",
            json!({"seq":request["seq"],"ok":true,"value":{"used":1},"can_test":true}),
        )
        .unwrap();
    assert!(f.host.drain_commands().iter().any(|(_,c)|matches!(c,Command::UiBrowserMessage{value,..}if value["kind"]=="result"&&value["ok"]==true)));
    f.update(2.1);
    assert!(
        f.host.drain_commands().iter().any(
            |(_, c)| matches!(c,Command::UiBrowserMessage{value,..}if value["kind"]=="expired")
        )
    );
    let out = f.host.drain_outputs();
    let pending = out
        .iter()
        .find_map(|o| match o {
            Output::Event {
                resource,
                name,
                payload,
                ..
            } if resource == "call-diagnostics" && name == "request" => {
                Some(payload["seq"].clone())
            }
            _ => None,
        })
        .unwrap();
    f.page(json!({"kind":"message","value":{"action":"close"}}));
    f.host.drain_commands();
    f.host
        .receive(
            0,
            "call-diagnostics",
            1,
            "result",
            json!({"seq":pending,"ok":true,"value":{"used":1}}),
        )
        .unwrap();
    assert!(
        !f.host
            .drain_commands()
            .iter()
            .any(|(_, c)| matches!(c, Command::UiBrowserMessage { .. })),
        "closed pages never receive late private data"
    );
    f.host.call(
        "call-diagnostics",
        "on_event",
        json!({"type":"interface","key":"open"}),
    );
    f.page(json!({"kind":"ready"}));
    f.update(0.1);
    f.host.drain_commands();
    let request = f
        .host
        .drain_outputs()
        .into_iter()
        .find_map(|o| match o {
            Output::Event {
                resource,
                name,
                payload,
                ..
            } if resource == "call-diagnostics" && name == "request" => Some(payload),
            _ => None,
        })
        .unwrap();
    f.host
        .receive(
            0,
            "call-diagnostics",
            1,
            "result",
            json!({"seq":request["seq"],"ok":false,"denied":true,"error":"revoked"}),
        )
        .unwrap();
    assert!(
        f.host
            .drain_commands()
            .iter()
            .any(|(_, c)| matches!(c,Command::UiBrowserMessage{value,..}if value["denied"]==true))
    );
    f.host
        .receive(
            0,
            "call-diagnostics",
            1,
            "result",
            json!({"seq":request["seq"],"ok":true,"value":{"used":1}}),
        )
        .unwrap();
    assert!(f.host.drain_commands().is_empty());
}

#[test]
fn diagnostics_discards_results_if_permission_changes_during_export() {
    let mut f = Fixture::new(Side::Server);
    let checks = Arc::new(AtomicU64::new(0));
    let auth_checks = checks.clone();
    f.host
        .set_authorizer(Some(Arc::new(move |actor, permission| {
            actor == 1
                && (permission == "calls.test"
                    || (permission == "calls.diagnostics"
                        && auth_checks.fetch_add(1, Ordering::SeqCst) == 0))
        })))
        .unwrap();
    let result = f.receive(1, "1", "inspect");
    assert!(checks.load(Ordering::SeqCst) >= 2);
    assert_eq!(result["denied"], true);
    assert!(
        result.get("value").is_none(),
        "revoked results must be discarded after the export"
    );
}
