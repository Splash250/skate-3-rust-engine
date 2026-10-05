use serde_json::json;
use skate_mods::resources::{Host, InstalledResource, Output, Side};
use skate_resources::Manifest;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "skate-authorization-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(temp: &Temp, language: &str, capability: bool) -> InstalledResource {
    let root = temp.0.join(language);
    std::fs::create_dir_all(&root).unwrap();
    let (entry, code) = if language == "javascript" {
        (
            "main.js",
            r#"resource.onNet('check',(payload,sender)=>resource.send('result',{allowed:resource.authorized(sender,payload.permission),spoof:resource.authorized('099',payload.permission)}));"#,
        )
    } else {
        (
            "main.lua",
            r#"resource.on_net('check',function(payload,sender) resource.send('result',{allowed=resource.authorized(sender,payload.permission),spoof=resource.authorized('099',payload.permission)}) end)"#,
        )
    };
    std::fs::write(root.join(entry), code).unwrap();
    let mut caps = vec!["resource.network"];
    if capability {
        caps.push("resource.authorization");
    }
    let manifest:Manifest=serde_json::from_value(json!({"format":1,"api":1,"id":language,"version":"1.0.0","language":language,"shared_scripts":[entry],"capabilities":caps,"requires_features":["resource.authorization.v1"]})).unwrap();
    InstalledResource {
        grants: manifest.capabilities.iter().cloned().collect(),
        manifest,
        root,
        generation: 1,
    }
}
fn result(host: &mut Host, language: &str, sender: u64) -> serde_json::Value {
    host.receive(
        sender,
        language,
        1,
        "check",
        json!({"permission":"diagnostics.test","sender":"99","roles":["administrator"]}),
    )
    .unwrap();
    host.drain_outputs()
        .into_iter()
        .find_map(|output| match output {
            Output::Event { name, payload, .. } if name == "result" => Some(payload),
            _ => None,
        })
        .unwrap()
}
#[test]
fn lua_and_javascript_authorization_rechecks_live_trusted_adapter_and_sender() {
    for language in ["lua", "javascript"] {
        let temp = Temp::new();
        let mut host = Host::new(Side::Server, temp.0.join("store"), language).unwrap();
        host.install(vec![fixture(&temp, language, true)]).unwrap();
        host.start_all().unwrap();
        assert_eq!(
            result(&mut host, language, 99)["allowed"],
            false,
            "default deny without verified host adapter"
        );
        let permitted = Arc::new(AtomicBool::new(true));
        let live = permitted.clone();
        host.set_authorizer(Some(Arc::new(move |actor, permission| {
            actor == 99 && permission == "diagnostics.test" && live.load(Ordering::Acquire)
        })))
        .unwrap();
        assert_eq!(
            result(&mut host, language, 7)["allowed"],
            false,
            "payload administrator cannot replace real sender"
        );
        let allowed = result(&mut host, language, 99);
        assert_eq!(allowed["allowed"], true);
        assert_eq!(allowed["spoof"], false);
        permitted.store(false, Ordering::Release);
        assert_eq!(result(&mut host, language, 99)["allowed"], false);
        host.set_authorizer(None).unwrap();
        assert_eq!(result(&mut host, language, 99)["allowed"], false);
    }
}
#[test]
fn capability_missing_and_client_queries_fail_closed() {
    for side in [Side::Server, Side::Client] {
        let temp = Temp::new();
        let mut host = Host::new(side, temp.0.join("store"), "closed").unwrap();
        host.install(vec![fixture(&temp, "lua", side == Side::Client)])
            .unwrap();
        host.start_all().unwrap();
        if side == Side::Server {
            host.set_authorizer(Some(Arc::new(|_, _| true))).unwrap();
        } else {
            assert!(host.set_authorizer(Some(Arc::new(|_, _| true))).is_err());
        }
        assert_eq!(
            result(&mut host, "lua", if side == Side::Server { 99 } else { 0 })["allowed"],
            false
        );
    }
}

#[test]
fn authorization_validates_actor_and_existing_permission_grammar() {
    let temp = Temp::new();
    let installed = fixture(&temp, "lua", true);
    std::fs::write(
        installed.root.join("main.lua"),
        r#"
        resource.on_net('check',function(payload,sender)
            resource.send('result',{
                valid=resource.authorized(sender,'Calls:Test-*'),
                empty=resource.authorized(sender,''),
                long=resource.authorized(sender,string.rep('a',65)),
                space=resource.authorized(sender,'calls test'),
                unicode=resource.authorized(sender,'calls.tést'),
                zero=resource.authorized('0','calls.test'),
                leading_zero=resource.authorized('099','calls.test'),
                negative=resource.authorized('-1','calls.test'),
                numeric=resource.authorized(99,'calls.test'),
                overflow=resource.authorized('18446744073709551616','calls.test')
            })
        end)
    "#,
    )
    .unwrap();
    let mut host = Host::new(Side::Server, temp.0.join("store"), "validation").unwrap();
    host.install(vec![installed]).unwrap();
    host.start_all().unwrap();
    host.set_authorizer(Some(Arc::new(|actor, _| actor == 99)))
        .unwrap();
    let value = result(&mut host, "lua", 99);
    assert_eq!(value["valid"], true);
    for key in [
        "empty",
        "long",
        "space",
        "unicode",
        "zero",
        "leading_zero",
        "negative",
        "numeric",
        "overflow",
    ] {
        assert_eq!(value[key], false, "{key} should fail closed");
    }
}
