//! Shipped resources run in actual Lua/QuickJS VMs, not a UI-only fixture.
use serde_json::{Value, json};
use skate_mods::resources::{Host, InstalledResource, Output, Side};
use skate_resources::Manifest;
use std::path::PathBuf;
fn install(host: &mut Host, name: &str, root: PathBuf) {
    let manifest = Manifest::read(&root).unwrap();
    assert_eq!(manifest.id, name);
    let grants = manifest.capabilities.iter().cloned().collect();
    let mut installed = host.installed().values().cloned().collect::<Vec<_>>();
    installed.push(InstalledResource {
        manifest,
        root,
        generation: 1,
        grants,
    });
    host.install(installed).unwrap();
}
fn base() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources")
}
#[test]
fn policy_filters_live_roles_configured_access_and_expired_private_visibility() {
    let temp = std::env::temp_dir().join(format!("skate-policy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(temp.join("probe")).unwrap();
    let mut h = Host::new(Side::Client, temp.join("data"), "policy-test").unwrap();
    install(
        &mut h,
        "interaction-policy",
        base().join("interaction-policy"),
    );
    std::fs::write(temp.join("probe/resource.json"),r#"{"format":1,"api":1,"id":"probe","version":"1.0.0","language":"lua","client_scripts":["client.lua"],"dependencies":{"interaction-policy":"1.0.0"},"capabilities":["resource.exports","resource.network"]}"#).unwrap();
    std::fs::write(temp.join("probe/client.lua"),r#"return {on_event=function(event) resource.send('result',resource.call('interaction-policy','filter',event)) end}"#).unwrap();
    install(&mut h, "probe", temp.join("probe"));
    h.start_all().unwrap();
    h.drain_commands();
    h.drain_outputs();
    h.call("interaction-policy", "on_ui_update", json!({"dt":1.1}));
    assert!(h.drain_outputs().iter().any(|output|matches!(output,Output::Event{name,payload,..} if name=="__host_admin" && payload["action"].get("check").is_none())),"an empty Lua table must not become a JSON object where the native API expects an array");
    // Exercise the exact native descriptor serialization, including absent
    // bindings and keyboard-only shortcuts. JSON null is not a Lua scalar.
    let mut registry = skate_mods::interactions::Registry::default();
    registry
        .register(
            "admin-dashboard",
            1,
            "open",
            serde_json::from_value(json!({
                "version":1,"label":"Admin","icon":"admin","category":"Administration",
                "destination":"dashboard","permissions":["status.read","custom.manage"]
            }))
            .unwrap(),
        )
        .unwrap();
    registry
        .register(
            "phone",
            1,
            "open",
            serde_json::from_value(json!({
                "version":1,"label":"Phone","icon":"phone","category":"Player",
                "destination":"phone","binding":{"key":"KeyP"}
            }))
            .unwrap(),
        )
        .unwrap();
    let entries = json!({"entries":registry.entries()});
    fn filtered(h: &mut Host, entries: Value) -> Vec<Value> {
        h.call("probe", "on_event", entries);
        h.drain_outputs()
            .into_iter()
            .find_map(|o| {
                if let Output::Event { name, payload, .. } = o {
                    (name == "result").then_some(payload)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("full native descriptor export failed: {:?}", h.diagnostics))
            .as_array()
            .unwrap()
            .clone()
    }
    assert_eq!(filtered(&mut h, entries.clone()).len(), 1);
    h.call("interaction-policy", "on_ui_update", json!({"dt":1.1}));
    assert!(h.drain_outputs().iter().any(|output|matches!(output,Output::Event{name,payload,..} if name=="__host_admin" && payload["action"]["check"].as_array().unwrap().contains(&json!("custom.manage")))));
    h.receive(
        0,
        "interaction-policy",
        1,
        "__host_admin_result",
        json!({"seq":"2","ok":true,"value":{"permissions":["status.read","custom.manage"]}}),
    )
    .unwrap();
    let result = filtered(&mut h, entries.clone());
    assert_eq!(result.len(), 2);
    assert_eq!(result[0]["id"], "phone/open");
    h.call("interaction-policy", "on_ui_update", json!({"dt":2.1}));
    // A delayed old permission response must not revive expired access.
    h.receive(
        0,
        "interaction-policy",
        1,
        "__host_admin_result",
        json!({"seq":"2","ok":true,"value":{"permissions":["status.read","custom.manage"]}}),
    )
    .unwrap();
    assert_eq!(filtered(&mut h, entries).len(), 1);
    std::fs::remove_dir_all(temp).unwrap();
}
#[test]
fn supplied_rp_client_resources_start_and_register_real_interfaces() {
    let temp = std::env::temp_dir().join(format!("skate-rp-start-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let mut h = Host::new(Side::Client, temp.clone(), "rp-start").unwrap();
    let names = [
        "interaction-policy",
        "inventory-ui",
        "admin-dashboard",
        "phone-calls",
        "phone",
        "park-guide",
        "master-menu",
        "call-diagnostics",
    ];
    let mut installed = Vec::new();
    for name in names {
        let root = base().join(name);
        let manifest = Manifest::read(&root).unwrap().client_projection();
        let grants = manifest.capabilities.iter().cloned().collect();
        installed.push(InstalledResource {
            manifest,
            root,
            generation: 1,
            grants,
        });
    }
    h.install(installed).unwrap();
    h.start_all().unwrap();
    for _ in 0..30 {
        h.tick(0.1, json!({"players":[],"voice":{"enabled":false}}));
        h.dispatch("on_ui_update", json!({"dt":0.1,"paused":false}));
    }
    for name in names {
        assert!(h.running(name), "{name}: {:?}", h.diagnostics);
    }
    let commands = h.drain_commands();
    assert!(
        commands
            .iter()
            .filter(|(_, c)| matches!(c, skate_mods::Command::UiInterfaces { .. }))
            .count()
            >= 5
    );
    assert!(
        !commands
            .iter()
            .any(|(_, c)| matches!(c, skate_mods::Command::UiBrowserOpen { .. })),
        "joining must not steal focus"
    );
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
fn dashboard_dropped_responses_expire_without_blocking_retry_or_accepting_late_data() {
    let temp = std::env::temp_dir().join(format!("skate-admin-timeout-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let mut host = Host::new(Side::Client, temp.clone(), "admin-timeout").unwrap();
    install(&mut host, "admin-dashboard", base().join("admin-dashboard"));
    host.start_all().unwrap();
    host.call(
        "admin-dashboard",
        "on_event",
        json!({"type":"browser","key":"admin","event":{"kind":"ready"}}),
    );
    for _ in 0..10 {
        host.call("admin-dashboard","on_event",json!({"type":"browser","key":"admin","event":{"kind":"message","value":{"key":"status","request":{"kind":"status"}}}}));
    }
    let requests = host.drain_outputs();
    let seq = requests
        .iter()
        .find_map(|output| match output {
            Output::Event { name, payload, .. } if name == "__host_admin" => {
                payload["seq"].as_str().map(str::to_owned)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|output| matches!(output,Output::Event{name,..} if name=="__host_admin"))
            .count(),
        8
    );
    host.call("admin-dashboard", "on_ui_update", json!({"dt":5.1}));
    host.drain_commands();
    host.receive(
        0,
        "admin-dashboard",
        1,
        "__host_admin_result",
        json!({"seq":seq,"ok":true,"value":{"private":"late"}}),
    )
    .unwrap();
    assert!(
        host.drain_commands().is_empty(),
        "expired replies must not reach the page"
    );
    host.call("admin-dashboard","on_event",json!({"type":"browser","key":"admin","event":{"kind":"message","value":{"key":"retry","request":{"kind":"status"}}}}));
    assert!(
        host.drain_outputs()
            .iter()
            .any(|output| matches!(output,Output::Event{name,..} if name=="__host_admin")),
        "lost packets must not exhaust the dashboard forever"
    );
    assert!(host.running("admin-dashboard"), "{:?}", host.diagnostics);
    std::fs::remove_dir_all(temp).unwrap();
}
