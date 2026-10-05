//! Exercise the shipped call resource in real Lua VMs; event identities are host supplied.
use serde_json::{Value, json};
use skate_mods::resources::{Host, InstalledResource, Output, Side};
use skate_resources::Manifest;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Calls {
    host: Host,
    root: PathBuf,
    players: Value,
}
impl Calls {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "skate-phone-calls-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/phone-calls");
        let manifest: Manifest = serde_json::from_slice(
            &std::fs::read(package.join("resource.json")).expect("shipped phone-calls manifest"),
        )
        .unwrap();
        let grants = manifest.capabilities.iter().cloned().collect();
        let mut host = Host::new(Side::Server, root.clone(), "phone-tests").unwrap();
        host.install(vec![InstalledResource {
            manifest,
            root: package,
            generation: 1,
            grants,
        }])
        .unwrap();
        host.start_all().unwrap();
        let mut value = Self {
            host,
            root,
            players: json!([{ "id":"1","instance":"0"},{"id":"2","instance":"0"},{"id":"3","instance":"0"}]),
        };
        value.tick(0.01);
        for actor in 1..=3 {
            value.send(actor, json!({"action":"presence","enabled":true}));
        }
        value.drain();
        value
    }
    fn tick(&mut self, dt: f64) {
        self.host.tick(dt, json!({"players":self.players}));
        assert!(
            self.host.running("phone-calls"),
            "{:?}",
            self.host.diagnostics
        );
    }
    fn send(&mut self, actor: u64, payload: Value) {
        let g = self.host.generation("phone-calls").unwrap();
        self.host
            .receive(actor, "phone-calls", g, "request", payload)
            .unwrap();
    }
    fn drain(&mut self) -> Vec<Output> {
        self.host.drain_outputs()
    }
    fn event(outputs: &[Output], actor: u64, kind: &str) -> Option<Value> {
        outputs.iter().rev().find_map(|o| match o {
            Output::Event {
                recipient: Some(a),
                name,
                payload,
                ..
            } if *a == actor && name == kind => Some(payload.clone()),
            _ => None,
        })
    }
    fn dial(&mut self) -> String {
        self.send(1, json!({"action":"dial","target":"2"}));
        let outputs = self.drain();
        assert!(!outputs.iter().any(|o| matches!(o, Output::Voice { .. })));
        let call = Self::event(&outputs, 2, "call").unwrap();
        assert_eq!(call["state"], "ringing");
        assert_eq!(call["incoming"], true);
        call["id"].as_str().unwrap().into()
    }
    fn complete(&mut self, name: &str, ok: bool) {
        let g = self.host.generation("phone-calls").unwrap();
        self.host.host_event("phone-calls",g,"voice_result",json!({"operation":"channel","name":name,"ok":ok,"error":if ok{Value::Null}else{json!("radio channel capacity reached")}})).unwrap();
    }
}
impl Drop for Calls {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn calls_require_acceptance_and_reject_guessed_ids_claimed_senders_and_races() {
    let mut c = Calls::new();
    let id = c.dial();
    for (actor, payload) in [
        (3, json!({"action":"accept","id":id,"sender":"2"})),
        (2, json!({"action":"accept","id":"1-guessed"})),
        (1, json!({"action":"accept","id":id})),
    ] {
        c.send(actor, payload);
        assert!(!c.drain().iter().any(|o| matches!(o, Output::Voice { .. })));
    }
    c.send(2, json!({"action":"dial","target":"1"}));
    assert_eq!(
        Calls::event(&c.drain(), 2, "error").unwrap()["code"],
        "busy"
    );
    c.send(2, json!({"action":"accept","id":id}));
    let out = c.drain();
    assert!(out.iter().any(|o|matches!(o,Output::Voice{operation,..} if operation["kind"]=="channel"&&operation["members"]==json!(["1","2"]))));
    assert_eq!(
        Calls::event(&out, 1, "call").unwrap()["state"],
        "connecting"
    );
    c.send(2, json!({"action":"accept","id":id}));
    assert!(!c.drain().iter().any(|o| matches!(o, Output::Voice { .. })));
    c.complete(&id, true);
    let out = c.drain();
    assert_eq!(Calls::event(&out, 1, "call").unwrap()["state"], "active");
    assert!(Calls::event(&out, 3, "call").is_none());
    c.send(3, json!({"action":"hangup","id":id}));
    assert!(!c.drain().iter().any(|o| matches!(o, Output::Voice { .. })));
    c.send(1, json!({"action":"hangup","id":id}));
    let out = c.drain();
    assert!(
        out.iter()
            .any(|o| matches!(o,Output::Voice{operation,..}if operation["kind"]=="remove_channel"))
    );
    assert_eq!(Calls::event(&out, 2, "call").unwrap()["state"], "ended");
}
#[test]
fn decline_cancel_expiry_cooldowns_and_voice_failures_are_terminal() {
    for action in ["decline", "cancel"] {
        let mut c = Calls::new();
        let id = c.dial();
        c.send(
            if action == "decline" { 2 } else { 1 },
            json!({"action":action,"id":id}),
        );
        assert_eq!(
            Calls::event(&c.drain(), 1, "call").unwrap()["state"],
            "ended"
        );
        c.send(1, json!({"action":"dial","target":"2"}));
        assert_eq!(
            Calls::event(&c.drain(), 1, "error").unwrap()["code"],
            "cooldown"
        );
    }
    let mut c = Calls::new();
    let id = c.dial();
    for _ in 0..32 {
        c.tick(1.);
        for actor in 1..=3 {
            c.send(actor, json!({"action":"presence","enabled":true}));
        }
    }
    let expiry = c.drain();
    assert_eq!(
        Calls::event(&expiry, 1, "call").unwrap_or_else(|| panic!("expiry outputs {expiry:?}"))["state"],
        "ended"
    );
    c.send(2, json!({"action":"accept","id":id}));
    assert!(!c.drain().iter().any(|o| matches!(o, Output::Voice { .. })));
    let mut c = Calls::new();
    let id = c.dial();
    c.send(2, json!({"action":"accept","id":id}));
    c.drain();
    c.complete(&id, false);
    assert_eq!(
        Calls::event(&c.drain(), 1, "call").unwrap()["reason"],
        "Voice channel unavailable. Try again later."
    );
}
#[test]
fn instance_disconnect_opt_out_and_retirement_revoke_calls_and_stale_completions() {
    for mode in ["instance", "disconnect", "optout", "cancel_pending"] {
        let mut c = Calls::new();
        let id = c.dial();
        c.send(2, json!({"action":"accept","id":id}));
        c.drain();
        match mode {
            "instance" => {
                c.players[1]["instance"] = json!("9");
                c.tick(0.01)
            }
            "disconnect" => {
                c.players.as_array_mut().unwrap().remove(1);
                c.tick(0.01)
            }
            "optout" => c.send(2, json!({"action":"presence","enabled":false})),
            _ => c.send(1, json!({"action":"hangup","id":id})),
        };
        let out = c.drain();
        assert!(out.iter().any(
            |o| matches!(o,Output::Voice{operation,..}if operation["kind"]=="remove_channel")
        ));
        c.complete(&id, true);
        assert!(Calls::event(&c.drain(), 1, "call").is_none());
    }
    let mut c = Calls::new();
    let id = c.dial();
    c.host.restart("phone-calls").unwrap();
    assert!(
        c.host
            .receive(
                2,
                "phone-calls",
                1,
                "request",
                json!({"action":"accept","id":id})
            )
            .is_err()
    );
    assert!(c.host.drain_retired().contains(&"phone-calls".into()));
}
#[test]
fn directory_hides_other_instances_and_disabled_peers_and_targets_every_reply() {
    let mut c = Calls::new();
    c.players[2]["instance"] = json!("9");
    c.tick(0.01);
    c.send(2, json!({"action":"presence","enabled":false}));
    c.drain();
    c.send(1, json!({"action":"contacts"}));
    let out = c.drain();
    let contacts = Calls::event(&out, 1, "contacts").unwrap();
    assert_eq!(contacts.as_array().unwrap().len(), 1);
    assert_eq!(contacts[0]["id"], "2");
    assert_eq!(contacts[0]["available"], false);
    assert!(out.iter().all(|o| !matches!(
        o,
        Output::Event {
            recipient: None,
            ..
        } | Output::State { .. }
    )));
    for target in ["2", "3", "1", "999"] {
        c.send(1, json!({"action":"dial","target":target}));
        assert!(!c.drain().iter().any(|o| matches!(o, Output::Voice { .. })));
    }
}
#[test]
fn retiring_channels_retry_without_reactivation_and_capacity_is_bounded() {
    let mut c = Calls::new();
    let id = c.dial();
    c.send(2, json!({"action":"accept","id":id}));
    c.drain();
    c.complete(&id, true);
    c.drain();
    c.send(1, json!({"action":"hangup","id":id}));
    c.drain();
    c.host.host_event("phone-calls",1,"voice_result",json!({"operation":"remove_channel","name":id,"ok":false,"error":"Voice operation queue exhausted"})).unwrap();
    c.tick(2.);
    c.tick(0.01);
    let out = c.drain();
    assert!(out.iter().any(|o|matches!(o,Output::Voice{operation,..}if operation["kind"]=="remove_channel"&&operation["name"]==id)));
    assert!(Calls::event(&out, 1, "call").is_none());
    c.host
        .host_event(
            "phone-calls",
            1,
            "voice_result",
            json!({"operation":"remove_channel","name":id,"ok":true}),
        )
        .unwrap();
    c.tick(2.);
    c.tick(0.01);
    assert!(!c.drain().iter().any(|o| matches!(o, Output::Voice { .. })));
    let mut c = Calls::new();
    c.players = json!(
        (1..=34)
            .map(|id| json!({"id":id.to_string(),"instance":"0"}))
            .collect::<Vec<_>>()
    );
    c.tick(0.01);
    for actor in 1..=34 {
        c.send(actor, json!({"action":"presence","enabled":true}));
    }
    c.drain();
    for actor in (1..=31).step_by(2) {
        c.send(
            actor,
            json!({"action":"dial","target":(actor+1).to_string()}),
        );
        assert_eq!(
            Calls::event(&c.drain(), actor, "call").unwrap()["state"],
            "ringing"
        );
    }
    c.send(33, json!({"action":"dial","target":"34"}));
    assert_eq!(
        Calls::event(&c.drain(), 33, "error").unwrap()["code"],
        "capacity"
    );
}
#[test]
fn client_exports_use_real_server_state_and_cannot_bypass_voice_opt_in() {
    let temp = std::env::temp_dir().join(format!(
        "skate-call-client-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/phone-calls");
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(package.join("resource.json")).unwrap()).unwrap();
    let manifest = manifest.client_projection();
    let grants = manifest.capabilities.iter().cloned().collect();
    let script = temp.join("client");
    std::fs::create_dir(&script).unwrap();
    std::fs::write(script.join("client.lua"),"resource.on('invoke',function(value) resource.send('result',resource.call('phone-calls',value.method,value.payload)) end)").unwrap();
    let observer:Manifest=serde_json::from_value(json!({"format":1,"api":1,"id":"call-observer","version":"1.0.0","language":"lua","client_scripts":["client.lua"],"dependencies":{"phone-calls":"1.0.0"},"capabilities":["resource.events","resource.exports","resource.network"]})).unwrap();
    let observer_grants = observer.capabilities.iter().cloned().collect();
    let mut host = Host::new(Side::Client, temp.join("state"), "client-test").unwrap();
    host.install(vec![
        InstalledResource {
            manifest,
            root: package,
            generation: 1,
            grants,
        },
        InstalledResource {
            manifest: observer,
            root: script,
            generation: 1,
            grants: observer_grants,
        },
    ])
    .unwrap();
    host.start_all().unwrap();
    host.tick(0.01, json!({"voice":{"enabled":false}}));
    host.dispatch("on_ui_update", json!({"dt":0.01}));
    host.drain_outputs();
    host.host_event(
        "call-observer",
        1,
        "invoke",
        json!({"method":"request","payload":{"action":"dial","target":"2"}}),
    )
    .unwrap();
    let out = host.drain_outputs();
    assert!(out.iter().any(|o|matches!(o,Output::Event{resource,payload,..}if resource=="call-observer"&&payload["ok"]==false)));
    host.tick(
        0.01,
        json!({"voice":{"enabled":true,"device":{"state":"ready"}}}),
    );
    host.dispatch("on_ui_update", json!({"dt":0.01}));
    host.drain_outputs();
    host.host_event(
        "call-observer",
        1,
        "invoke",
        json!({"method":"snapshot","payload":{}}),
    )
    .unwrap();
    host.drain_outputs();
    host.host_event(
        "call-observer",
        1,
        "invoke",
        json!({"method":"request","payload":{"action":"dial","target":"2","sender":"forged"}}),
    )
    .unwrap();
    let out = host.drain_outputs();
    assert!(out.iter().any(|o|matches!(o,Output::Event{resource,payload,..}if resource=="phone-calls"&&payload==&json!({"action":"dial","target":"2"}))));
    assert!(
        host.receive(
            99,
            "phone-calls",
            1,
            "call",
            json!({"id":"g1-1","state":"active","channel":"phone-calls/g1-1"})
        )
        .is_err()
    );
    host.receive(
        0,
        "phone-calls",
        1,
        "call",
        json!({"id":"g1-1","state":"active","channel":"phone-calls/g1-1","peer":"2"}),
    )
    .unwrap();
    assert!(host.drain_commands().iter().any(|(_,c)|matches!(c,skate_mods::Command::Voice{operation}if operation["kind"]=="transmit"&&operation["channel"]=="phone-calls/g1-1")));
    host.host_event(
        "call-observer",
        1,
        "invoke",
        json!({"method":"snapshot","payload":{}}),
    )
    .unwrap();
    assert!(host.drain_outputs().iter().any(|o|matches!(o,Output::Event{resource,payload,..}if resource=="call-observer"&&payload["call"]["state"]=="active")));
    host.receive(
        0,
        "phone-calls",
        1,
        "call",
        json!({"id":"g1-old","state":"ended"}),
    )
    .unwrap();
    assert!(
        !host
            .drain_commands()
            .iter()
            .any(|(_, c)| matches!(c, skate_mods::Command::Voice { .. }))
    );
    // Maximum-length device identifiers and a full directory cannot break the export budget.
    let device = "\"\\".repeat(128);
    host.host_event("phone-calls",1,"voice_result",json!({"kind":"devices","value":{"inputs":vec![device.clone();8],"outputs":vec![device.clone();8]}})).unwrap();
    let contacts=(1..=63).map(|i|json!({"id":format!("18446744073709550{i:03}"),"label":format!("Player 18446744073709550{i:03}"),"available":false,"reason":"Phone or voice is unavailable."})).collect::<Vec<_>>();
    host.receive(0, "phone-calls", 1, "contacts", json!(contacts))
        .unwrap();
    host.host_event("call-observer",1,"invoke",json!({"method":"request","payload":{"action":"voice","input_device":device,"output_device":device}})).unwrap();
    host.drain_outputs();
    host.host_event(
        "call-observer",
        1,
        "invoke",
        json!({"method":"snapshot","payload":{}}),
    )
    .unwrap();
    let out = host.drain_outputs();
    let snapshot = out
        .iter()
        .find_map(|o| match o {
            Output::Event {
                resource, payload, ..
            } if resource == "call-observer" => Some(payload),
            _ => None,
        })
        .unwrap();
    assert_eq!(snapshot["devices"]["truncated"], true);
    assert_eq!(snapshot["devices"]["inputs"].as_array().unwrap().len(), 2);
    assert!(serde_json::to_vec(snapshot).unwrap().len() < 16 * 1024);
    assert!(host.running("phone-calls") && host.running("call-observer"));
    host.drain_commands();
    host.drain_outputs();
    // A closed presentation still polls snapshots and keeps its call lease alive.
    for _ in 0..8 {
        host.host_event(
            "call-observer",
            1,
            "invoke",
            json!({"method":"snapshot","payload":{}}),
        )
        .unwrap();
        host.dispatch("on_ui_update", json!({"dt":0.5}));
    }
    assert!(!host.drain_outputs().iter().any(|o|matches!(o,Output::Event{resource,payload,..}if resource=="phone-calls"&&payload["action"]=="presence"&&payload["enabled"]==false)));
    // No polling means the presentation failed/retired, even if calls keeps running.
    host.dispatch("on_ui_update", json!({"dt":3.1}));
    assert!(host.drain_outputs().iter().any(|o|matches!(o,Output::Event{resource,payload,..}if resource=="phone-calls"&&payload==&json!({"action":"presence","enabled":false}))));
    host.host_event(
        "call-observer",
        1,
        "invoke",
        json!({"method":"snapshot","payload":{}}),
    )
    .unwrap();
    host.dispatch("on_ui_update", json!({"dt":0.01}));
    assert!(host.drain_outputs().iter().any(|o|matches!(o,Output::Event{resource,payload,..}if resource=="phone-calls"&&payload==&json!({"action":"presence","enabled":true}))));
    host.stop("phone-calls").unwrap();
    assert!(!host.running("call-observer"));
    let _ = std::fs::remove_dir_all(temp);
}
