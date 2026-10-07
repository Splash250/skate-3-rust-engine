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

#[test]
fn boardwalk_phone_apps_register_and_retire_cleanly() {
    use skate_mods::interactions::Operation;
    let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources");
    let store=std::env::temp_dir().join(format!("skate-boardwalk-phone-ui-{}",std::process::id()));
    let _=std::fs::remove_dir_all(&store);
    let mut host=Host::new(Side::Client,store.clone(),"boardwalk-ui-test").unwrap();
    let names=["platform-profiles","rp-economy","voice-room","boardwalk-borough","phone-calls",
        "interaction-policy","inventory-ui","admin-dashboard","phone","rp-properties","rp-pizza"];
    let packages=names.iter().map(|id|{
        let package=root.join(id);
        let manifest=Manifest::read(&package).unwrap().client_projection();
        let grants=manifest.capabilities.iter().cloned().collect();
        InstalledResource{manifest,root:package,generation:1,grants}
    }).collect();
    host.install(packages).unwrap();
    host.start_all().unwrap();
    for id in names { assert!(host.running(id),"{id}: {:?}",host.diagnostics); }
    let registrations=host.drain_commands();
    for (owner,label) in [("rp-pizza","Pizza Shift"),("rp-properties","Properties")] {
        assert!(registrations.iter().any(|(resource,command)|*resource==owner&&matches!(command,
            Command::UiInterfaces{operation:Operation::Register{key,descriptor}}
                if key=="open"&&descriptor.label==label&&descriptor.phone&&descriptor.quick
        )),"{owner} phone interface was not registered: {registrations:?}");
    }
    for (owner,key) in [("rp-pizza","pizza"),("rp-properties","properties")] {
        host.call(owner,"on_event",json!({"type":"interface","key":"open","generation":"1"}));
        assert!(host.drain_commands().iter().any(|(_,command)|matches!(command,
            Command::UiBrowserOpen{key:browser,..} if browser==key
        )),"{owner} did not open its browser surface");
        host.call(owner,"on_event",json!({"type":"browser","key":key,"event":{"kind":"ready"}}));
        host.drain_commands();
        if owner=="rp-pizza" {
            host.call(owner,"on_event",json!({"type":"browser","key":key,"event":{"kind":"message","value":{"action":"go_to_counter"}}}));
            assert!(host.drain_outputs().iter().any(|output|matches!(output,
                skate_mods::resources::Output::Event{resource,name,payload,..}
                    if resource=="rp-pizza"&&name=="request"&&payload["action"]=="go_to_counter"
            )),"pizza phone did not forward the counter shortcut to the server");
        }
        host.stop(owner).unwrap();
        let retired=host.drain_commands();
        assert!(retired.is_empty(),"retired resources emitted stale commands: {retired:?}");
        assert!(host.drain_retired().contains(&owner.to_owned()),
            "{owner} did not publish its generation retirement for browser/input/interface cleanup");
    }
    let _=std::fs::remove_dir_all(store);
}

#[test]
fn properties_client_handles_the_first_server_response_after_browser_ready() {
    let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources");
    let store=std::env::temp_dir().join(format!("skate-properties-response-{}",std::process::id()));
    let _=std::fs::remove_dir_all(&store);
    let mut host=Host::new(Side::Client,store.clone(),"properties-response-test").unwrap();
    let packages=["platform-profiles","rp-economy","voice-room","boardwalk-borough","phone-calls",
        "interaction-policy","inventory-ui","admin-dashboard","phone","rp-properties","rp-pizza"].map(|id|{
        let package=root.join(id);
        let manifest=Manifest::read(&package).unwrap().client_projection();
        let grants=manifest.capabilities.iter().cloned().collect();
        InstalledResource{manifest,root:package,generation:1,grants}
    });
    host.install(packages.into()).unwrap();
    host.start_all().unwrap();
    host.drain_commands();
    host.call("rp-properties","on_event",json!({"type":"interface","key":"open","generation":"1"}));
    host.drain_commands();
    host.call("rp-properties","on_event",json!({"type":"browser","key":"properties","event":{"kind":"ready"}}));
    host.drain_commands();
    assert!(host.receive(0,"rp-properties",1,"response",json!({"ready":true,"balance":100})).is_ok());
    assert!(host.running("rp-properties"),"server response must not crash the properties client: {:?}",host.diagnostics);
    assert!(host.drain_commands().iter().any(|(_,command)|matches!(command,
        Command::UiBrowserMessage{key,value,..} if key=="properties"&&value["kind"]=="snapshot"
    )));
    let _=std::fs::remove_dir_all(store);
}

#[test]
fn properties_invite_picker_uses_the_server_online_player_list() {
    let html=include_str!("../../../resources/rp-properties/index.html");
    let js=include_str!("../../../resources/rp-properties/properties.js");
    assert!(html.contains("id=\"target-toggle\"")&&html.contains("id=\"target-menu\""),
        "player choices should use a custom button-based dropdown that accepts host-forwarded mouse clicks");
    assert!(!html.contains("<select id=\"target\"" )&&!html.contains("<input id=\"target\""),
        "the picker should not depend on an unsupported native popup or manual session ID entry");
    assert!(js.contains("v.online_players")&&js.contains("choice.onclick")&&js.contains("post('invite',{target:selectedPlayer})"),
        "the picker should render server candidates, select on a button click and invite the chosen player");
    assert!(js.contains("playerSignature")&&js.contains("if(signature!==playerSignature)"),
        "unchanged snapshots must not replace dropdown options while the player is choosing");
}
