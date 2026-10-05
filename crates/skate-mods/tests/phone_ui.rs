//! Shipped phone lifecycle/bridge tests in real resource Lua VMs.
use serde_json::{Value, json};
use skate_mods::{
    Command,
    resources::{Host, InstalledResource, Output, Side},
};
use skate_resources::Manifest;
use std::path::PathBuf;
#[test]
fn shipped_phone_opens_composited_surface_keeps_camera_focus_and_cleans_up() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources");
    let store = std::env::temp_dir().join(format!("skate-phone-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&store);
    let mut host = Host::new(Side::Client, store.clone(), "ui-test").unwrap();
    let packages = [
        "phone-calls",
        "interaction-policy",
        "inventory-ui",
        "admin-dashboard",
        "phone",
    ]
    .map(|id| {
        let package = root.join(id);
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(package.join("resource.json")).unwrap()).unwrap();
        let manifest = manifest.client_projection();
        let grants = manifest.capabilities.iter().cloned().collect();
        InstalledResource {
            manifest,
            root: package,
            generation: 1,
            grants,
        }
    });
    host.install(packages.into()).unwrap();
    host.start_all().unwrap();
    assert!(host.running("phone"), "{:?}", host.diagnostics);
    host.drain_commands();
    host.call(
        "phone",
        "on_event",
        json!({"type":"interface","key":"open","generation":"1"}),
    );
    let commands = host.drain_commands();
    assert!(commands.iter().any(|(_,c)|matches!(c,Command::UiBrowserOpen{options,..}if options.surface.is_some()&&options.width==390&&options.height==760)));
    host.call(
        "phone",
        "on_event",
        json!({"type":"browser","key":"phone","event":{"kind":"ready"}}),
    );
    assert!(
        host.drain_commands()
            .iter()
            .any(|(_, c)| matches!(c,Command::UiBrowserMessage{value,..}if value["kind"]=="boot"))
    );
    // UI refresh and the call presentation lease share real elapsed time, including low FPS.
    host.drain_outputs();
    for _ in 0..4 {
        host.tick(
            1.0,
            json!({"voice":{"enabled":true,"device":{"state":"ready"}}}),
        );
        host.dispatch("on_ui_update", json!({"dt":1.0}));
    }
    assert!(!host.drain_outputs().iter().any(|output|matches!(output,Output::Event{resource,payload,..}if resource=="phone-calls"&&payload["action"]=="presence"&&payload["enabled"]==false)),"a healthy low-FPS presentation must keep its call lease alive");
    host.drain_commands();
    let page = |value: Value| json!({"type":"browser","key":"phone","event":{"kind":"message","value":value}});
    host.call(
        "phone",
        "on_event",
        page(json!({"action":"route","route":"camera"})),
    );
    let commands = host.drain_commands();
    assert!(
        commands
            .iter()
            .any(|(_, c)| matches!(c, Command::Photos { .. }))
    );
    assert!(
        !commands
            .iter()
            .any(|(_, c)| matches!(c, Command::UiBrowserFocus { focused: false, .. })),
        "camera retains its owning focused surface"
    );
    host.host_event(
        "phone",
        1,
        "photo",
        json!({"kind":"mode","enabled":true,"width":1920,"height":1080}),
    )
    .unwrap();
    host.drain_commands();
    host.call("phone", "on_event", page(json!({"action":"close"})));
    assert!(
        host.drain_commands()
            .iter()
            .any(|(_, c)| matches!(c, Command::UiBrowserClose { .. }))
    );
    host.call(
        "phone",
        "on_event",
        json!({"type":"interface","key":"open","generation":"1"}),
    );
    host.call(
        "phone",
        "on_event",
        json!({"type":"browser","key":"phone","event":{"kind":"ready"}}),
    );
    host.drain_commands();
    host.call(
        "phone",
        "on_event",
        page(json!({"action":"preferences","theme":"paper","scale":0.85})),
    );
    assert!(host.running("phone"), "{:?}", host.diagnostics);
    let commands = host.drain_commands();
    assert!(commands.iter().any(|(_,c)|matches!(c,Command::UiBrowserOpen{options,..}if options.surface.as_ref().is_some_and(|s|(s.scale-0.85).abs()<0.001))));
    host.stop("phone").unwrap();
    assert!(host.drain_retired().contains(&"phone".into()));
    let _ = std::fs::remove_dir_all(store);
}
