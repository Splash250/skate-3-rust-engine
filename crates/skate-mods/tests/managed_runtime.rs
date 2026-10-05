//! Explicit opt-in integration checks require the published trusted worker,
//! .NET 10 and the platform isolation prerequisites documented in SDK/RESOURCES.
use serde_json::json;
use skate_mods::resources::{Host, InstalledResource, Output, RuntimeLimits, Side};
use skate_resources::Manifest;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "skate-managed-{}-{}",
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
fn installed(
    t: &Temp,
    id: &str,
    language: &str,
    source: &str,
    deps: &[&str],
    caps: &[&str],
) -> InstalledResource {
    let root = t.0.join(id);
    std::fs::create_dir_all(&root).unwrap();
    let file = format!(
        "main.{}",
        match language {
            "csharp" => "cs",
            "javascript" => "js",
            _ => "lua",
        }
    );
    std::fs::write(root.join(&file), source).unwrap();
    InstalledResource {
        manifest: Manifest {
            settings: Default::default(),
            requires_features: vec![],
            world: None,
            format: 1,
            api: 1,
            id: id.into(),
            version: "1.0.0".into(),
            language: language.into(),
            shared_scripts: vec![file],
            client_scripts: vec![],
            server_scripts: vec![],
            files: vec![],
            dependencies: deps
                .iter()
                .map(|id| (id.to_string(), "1.0.0".into()))
                .collect(),
            exports: vec!["value".into()],
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
        },
        root,
        generation: 1,
        grants: caps.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(),
    }
}
fn host(t: &Temp, side: Side) -> Host {
    Host::new(side, t.0.join("store"), "managed-test").unwrap()
}
fn storage_path(t:&Temp,side:Side,id:&str)->PathBuf {
    let scope=blake3::hash(format!("resource-storage-v1:{side:?}:managed-test").as_bytes()).to_hex().to_string();
    t.0.join("store").join(scope).join(format!("{id}.json"))
}
fn prerequisites() {
    assert!(
        std::env::var_os("SKATE_DOTNET_ROOT").is_some(),
        "set SKATE_DOTNET_ROOT"
    );
    assert!(
        std::env::var_os("SKATE_MANAGED_HOST").is_some(),
        "set SKATE_MANAGED_HOST"
    );
}

#[test]
#[ignore = "requires rebuilt isolated .NET worker with Resource.Authorized"]
fn csharp_live_authorization_uses_actual_sender_and_revocation() {
    prerequisites();
    let temp=Temp::new();let mut host=host(&temp,Side::Server);
    let app=installed(&temp,"auth_managed","csharp",r#"
      using Skate.Managed;using System.Text.Json.Nodes;
      public class Script:IResourceScript { public void Start(Resource resource) {
        resource.OnNet("check",(value,sender)=>resource.Send("result",JsonValue.Create(resource.Authorized(sender,"calls.test"))));
      }}
    "#,&[],&["resource.network","resource.authorization"]);
    let allowed=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));let live=allowed.clone();
    host.set_authorizer(Some(std::sync::Arc::new(move |actor,permission|actor==42&&permission=="calls.test"&&live.load(Ordering::Acquire)))).unwrap();
    host.install(vec![app]).unwrap();host.start_all().unwrap();
    let query=|host:&mut Host,actor| {
        host.receive(actor,"auth_managed",1,"check",json!({"sender":"42","role":"administrator"})).unwrap();
        host.drain_outputs().into_iter().find_map(|output|if let Output::Event{name,payload,..}=output {(name=="result").then_some(payload)}else{None}).unwrap()
    };
    assert_eq!(query(&mut host,7),false);
    assert_eq!(query(&mut host,42),true);
    allowed.store(false,Ordering::Release);assert_eq!(query(&mut host,42),false);
}

#[test]
fn csharp_empty_side_keeps_resource_active_without_managed_prerequisites() {
    let t = Temp::new();
    if std::env::var_os("SKATE_TEST_MANAGED_EMPTY_SIDE").is_none() {
        // An isolated child avoids mutating process-wide environment while
        // other runtime tests may be using a real owned .NET installation.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "csharp_empty_side_keeps_resource_active_without_managed_prerequisites", "--nocapture"])
            .env("SKATE_TEST_MANAGED_EMPTY_SIDE", "1")
            .env("SKATE_DOTNET_ROOT", t.0.join("absent-dotnet"))
            .env("SKATE_MANAGED_HOST", t.0.join("absent-worker"))
            .output().unwrap();
        assert!(output.status.success(), "empty-side child failed: {}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        return;
    }
    for side in [Side::Client, Side::Server] {
        let mut package = installed(&t, "opposite_side", "csharp", "deliberately invalid C#; this side must not read or compile it", &[], &[]);
        let scripts = std::mem::take(&mut package.manifest.shared_scripts);
        if side == Side::Client { package.manifest.server_scripts = scripts; }
        else { package.manifest.client_scripts = scripts; }
        package.generation = 13;
        let mut h = host(&t, side);
        h.install(vec![package.clone()]).unwrap();
        h.start_all().unwrap();
        assert!(h.running("opposite_side"));
        assert_eq!(h.generation("opposite_side"), Some(13));
        h.tick(1. / 60., json!({}));
        assert!(h.drain_outputs().is_empty());
        assert!(h.drain_commands().is_empty());
        h.disconnect();
        assert!(!h.running("opposite_side"));
        let mut selected = host(&t, if side == Side::Client { Side::Server } else { Side::Client });
        selected.install(vec![package]).unwrap();
        assert!(selected.start_all().is_err(), "the side with a script must still require its managed runtime");
    }
}

#[test]
#[ignore = "requires isolated .NET worker; run with SKATE_DOTNET_ROOT and SKATE_MANAGED_HOST"]
fn csharp_lua_javascript_exports_lifecycle_and_persistence() {
    prerequisites();
    let t = Temp::new();
    let mut h = host(&t, Side::Server);
    let managed = installed(
        &t,
        "managed",
        "csharp",
        r#"using Skate.Managed; using System.Text.Json.Nodes;
public sealed class Script:IResourceScript {public void Start(Resource r) {
r.Export("value",x=>JsonValue.Create(x!.GetValue<int>()+1));
r.Lifecycle("on_load",x=>{r.StorageSet("starts",JsonValue.Create((r.StorageGet("starts")?.GetValue<int>()??0)+1));});
r.Lifecycle("on_unload",x=>{r.StorageSet("stops",JsonValue.Create((r.StorageGet("stops")?.GetValue<int>()??0)+1));});
r.On("raise",(x,sender)=>{r.StateSet("sender",JsonValue.Create(sender));r.StateSet("raised",x);});
}}"#,
        &[],
        &[
            "resource.exports",
            "resource.events",
            "resource.state",
            "resource.storage",
        ],
    );
    let lua = installed(
        &t,
        "lua",
        "lua",
        "resource.export('value',function(x)return resource.call('managed','value',x)+1 end)",
        &["managed"],
        &["resource.exports"],
    );
    let js = installed(
        &t,
        "js",
        "javascript",
        "resource.export('value',x=>resource.call('lua','value',x)+1)",
        &["lua"],
        &["resource.exports"],
    );
    let app = installed(
        &t,
        "app",
        "csharp",
        r#"using Skate.Managed;using System.Text.Json.Nodes;
public sealed class Script:IResourceScript { public void Start(Resource r) {
r.StateSet("result",r.CallExport("js","value",JsonValue.Create(5)));
r.On("set",(value,sender)=>r.StateSet("event",value));r.Emit("set",JsonValue.Create(17));
r.Lifecycle("on_update",x=>r.StateSet("tick",JsonValue.Create(true)));
}}"#,
        &["js"],
        &["resource.exports", "resource.events", "resource.state"],
    );
    h.install(vec![app, js, lua, managed]).unwrap();
    h.start_all().unwrap();
    assert_eq!(h.state("app", "result"), Some(json!(8)));
    let metrics=h.runtime_metrics();
    assert!(metrics["js"].javascript_heap_bytes.is_some_and(|bytes|bytes>0));
    #[cfg(target_os="linux")]
    assert!(metrics["managed"].managed_resident_bytes.is_some_and(|bytes|bytes>0));
    assert_eq!(h.state("app", "event"), Some(json!(17)));
    h.tick(0.02, json!({}));
    assert_eq!(h.state("app", "tick"), Some(json!(true)));
    h.ensure("managed").unwrap();
    assert!(h.running("app"));
    let stored: serde_json::Value = serde_json::from_slice(
        &std::fs::read(storage_path(&t,Side::Server,"managed")).unwrap(),
    )
    .unwrap();
    assert_eq!(stored["starts"], json!(2));
    assert_eq!(stored["stops"], json!(1));
    h.disconnect();
    assert!(!h.running("managed"));
}

#[test]
#[ignore = "requires isolated .NET worker; run with SKATE_DOTNET_ROOT and SKATE_MANAGED_HOST"]
fn csharp_client_server_network_service_and_capability_boundary() {
    prerequisites();
    let t = Temp::new();
    let caps = [
        "resource.events",
        "resource.network",
        "resource.state",
        "resource.database",
        "resource.storage",
        "resource.entities",
        "resource.voice",
        "resource.world",
        "resource.competition",
        "engine.voice",
        "engine.animation",
        "engine.ui",
    ];
    let package = installed(
        &t,
        "managed",
        "csharp",
        r#"using Skate.Managed;using System.Text.Json.Nodes;
public sealed class Script:IResourceScript { public void Start(Resource r) {
if(r.Side=="server") {
var scope=new JsonObject{["kind"]="player",["id"]="9"};
r.StateSet("private",JsonValue.Create(99),scope);
if(r.StateGet("private",scope)!.GetValue<int>()!=99||r.StateGet("private")!=null)throw new System.Exception("scope mixed");
r.OnNet("ask",(value,sender)=>{r.StateSet("sender",JsonValue.Create(sender));r.Send("answer",value,sender,scope);});
r.On("service_result",(value,sender)=>r.StateSet("service",value));
r.ServiceSubmit("read",new JsonObject{["kind"]="query",["statement"]="SELECT 1"},1000);
r.Entity(new JsonObject{["op"]="remove",["key"]="crate"});
r.Voice(new JsonObject{["kind"]="proximity",["meters"]=20});
r.World(new JsonObject{["op"]="rail_remove",["key"]="rail",["instance"]="0"});
r.Competition(new JsonObject{["kind"]="start",["name"]="race",["player"]="9"});
r.TransferStart("bulk","payload",JsonValue.Create("example"),new JsonObject{["recipient"]="9"});r.TransferCancel("bulk");
}else{r.Voice(new JsonObject{["kind"]="devices"});r.Animation(new JsonObject{["op"]="stop",["key"]="example"});r.TransferStart("bulk","payload",JsonValue.Create("example"));r.TransferCancel("bulk");r.Submit(new JsonObject{["kind"]="voice",["operation"]=new JsonObject{["kind"]="transmit",["pressed"]=false}});r.OnNet("answer",(value,sender)=>r.StorageSet("answer",value));r.Send("ask",JsonValue.Create(42));r.UiText("hello","Managed client");}
}}"#,
        &[],
        &caps,
    );
    let mut server = host(&t, Side::Server);
    let mut client = host(&t, Side::Client);
    server.install(vec![package.clone()]).unwrap();
    client.install(vec![package.clone()]).unwrap();
    server.start_all().unwrap();
    client.start_all().unwrap();
    let outbound = client.drain_outputs();
    let (_, generation, name, payload) = outbound
        .into_iter()
        .find_map(|o| {
            if let Output::Event {
                resource,
                generation,
                name,
                payload,
                ..
            } = o
            {
                Some((resource, generation, name, payload))
            } else {
                None
            }
        })
        .unwrap();
    server
        .receive(9, "managed", generation, &name, payload)
        .unwrap();
    assert_eq!(server.state("managed", "sender"), Some(json!("9")));
    let output = server.drain_outputs();
    assert!(
        output
            .iter()
            .any(|o| matches!(o,Output::Service{key,..} if key=="read"))
    );
    assert!(output.iter().any(|o| matches!(o, Output::Entity { .. })));
    assert!(output.iter().any(|o| matches!(o, Output::Voice { .. })));
    assert!(output.iter().any(|o| matches!(o, Output::World { .. })));
    assert!(output.iter().any(|o| matches!(o, Output::Competition { .. })));
    assert!(output.iter().any(|o| matches!(o, Output::Transfer { .. })));
    assert!(output.iter().any(|o| matches!(o, Output::CancelTransfer { .. })));
    let (generation, name, payload) = output
        .into_iter()
        .find_map(|o| {
            if let Output::Event {
                generation,
                name,
                payload,
                recipient: Some(9),
                ..
            } = o
            {
                Some((generation, name, payload))
            } else {
                None
            }
        })
        .unwrap();
    client
        .receive(0, "managed", generation, &name, payload)
        .unwrap();
    let stored:serde_json::Value=serde_json::from_slice(&std::fs::read(storage_path(&t,Side::Client,"managed")).unwrap()).unwrap();
    assert_eq!(stored["answer"],json!(42));
    server
        .service_result("managed", generation, "read", json!({"ok":true}))
        .unwrap();
    assert_eq!(
        server.state("managed", "service"),
        Some(json!({"key":"read","result":{"ok":true}}))
    );
    assert!(!client.drain_commands().is_empty());
    client.stop("managed").unwrap();
    let mut revoked = package;
    revoked.grants.clear();
    client.install(vec![revoked]).unwrap();
    assert!(client.start_all().is_err());
    assert!(!client.running("managed"));
    assert_eq!(client.state("managed", "answer"), None);
    let bad = installed(
        &t,
        "denied",
        "csharp",
        "using Skate.Managed; public sealed class S:IResourceScript{public void Start(Resource r){r.StateSet(\"x\",null);}}",
        &[],
        &[],
    );
    let mut denied = host(&t, Side::Server);
    denied.install(vec![bad]).unwrap();
    assert!(denied.start_all().is_err());
}

#[test]
#[ignore = "requires isolated .NET worker; run with SKATE_DOTNET_ROOT and SKATE_MANAGED_HOST"]
fn csharp_native_reflection_process_and_io_are_rejected() {
    prerequisites();
    let t = Temp::new();
    for (id, body) in [
        ("file", "System.IO.File.ReadAllText(\"/etc/passwd\");"),
        ("reflection", "r.GetType();"),
        ("assembly", "System.Reflection.Assembly.Load(new byte[0]);"),
        ("process", "System.Diagnostics.Process.Start(\"/bin/sh\");"),
        ("console", "System.Console.WriteLine(\"forged IPC\");"),
        ("thread", "new System.Threading.Thread(()=>{}).Start();"),
    ] {
        let code = format!(
            "using Skate.Managed; public sealed class S:IResourceScript{{public void Start(Resource r){{{body}}}}}"
        );
        let mut h = host(&t, Side::Server);
        h.install(vec![installed(&t, id, "csharp", &code, &[], &[])])
            .unwrap();
        assert!(h.start_all().is_err(), "allowed {id}");
        assert!(!h.running(id));
    }
    let mut h = host(&t, Side::Server);
    let native = "using Skate.Managed;using System.Runtime.InteropServices;public sealed class S:IResourceScript{[DllImport(\"libc\")]static extern int system(string s);public void Start(Resource r){system(\"id\");}}";
    h.install(vec![installed(&t, "native", "csharp", native, &[], &[])])
        .unwrap();
    assert!(h.start_all().is_err());
}

#[test]
#[ignore = "requires isolated .NET worker; run with SKATE_DOTNET_ROOT and SKATE_MANAGED_HOST"]
fn csharp_runaway_and_memory_are_killed_and_registration_slots_released() {
    prerequisites();
    let t = Temp::new();
    for (id, body) in [
        ("loop", "while(true){try{while(true){}}catch{}}"),
        (
            "memory",
            "var values=new System.Collections.Generic.List<byte[]>();while(true){values.Add(new byte[1024*1024]);}",
        ),
    ] {
        let code = format!(
            "using Skate.Managed;public sealed class S:IResourceScript{{public void Start(Resource r){{r.Lifecycle(\"on_update\",x=>{{{body}}});}}}}"
        );
        let limits = RuntimeLimits {
            managed_callback_timeout_ms: if id == "memory" {1000} else {50},
            ..RuntimeLimits::default()
        };
        let mut h = Host::new_with_limits(Side::Server, t.0.join("store"), id, limits).unwrap();
        h.install(vec![installed(&t, id, "csharp", &code, &[], &[])])
            .unwrap();
        h.start_all().unwrap();
        let before = std::time::Instant::now();
        h.tick(0.01, json!({}));
        assert!(before.elapsed() < std::time::Duration::from_secs(2));
        assert!(!h.running(id));
        if id == "memory" {assert!(!h.diagnostics.iter().any(|e|e.contains("deadline")), "{:?}", h.diagnostics);}
    }
    let code = r#"using Skate.Managed;using System.Text.Json.Nodes;public sealed class S:IResourceScript{public void Start(Resource r){int count=0;r.Lifecycle("on_update",x=>{r.On("temporary",(v,s)=>{});r.Off("temporary");r.Export("value",v=>v);r.StateSet("count",JsonValue.Create(++count));});}}"#;
    let mut h = host(&t, Side::Server);
    h.install(vec![installed(
        &t,
        "slots",
        "csharp",
        code,
        &[],
        &["resource.events", "resource.exports", "resource.state"],
    )])
    .unwrap();
    h.start_all().unwrap();
    for _ in 0..200 {
        h.tick(0.01, json!({}));
    }
    assert!(h.running("slots"), "{:?}", h.diagnostics);
    assert_eq!(h.state("slots", "count"), Some(json!(200)));
}

#[test]
#[ignore = "requires isolated .NET worker; run with SKATE_DOTNET_ROOT and SKATE_MANAGED_HOST"]
fn csharp_typed_settings_and_profile_ipc_worker_cpu() {
    prerequisites();let t=Temp::new();let mut h=host(&t,Side::Server);
    let mut app=installed(&t,"profile_settings","csharp",r#"
      using Skate.Managed;using System.Text.Json.Nodes;
      public class Script:IResourceScript { public void Start(Resource r) {
        r.StateSet("initial",r.SettingsGet("round"));
        r.Lifecycle("on_settings",c=>r.StateSet("changed",r.SettingsAll()));
        r.Lifecycle("on_update",_=>{long sum=0;for(int i=0;i<100000;i++)sum+=i;r.StateSet("work",JsonValue.Create(sum));});
      }}
    "#,&[],&["resource.settings","resource.state"]);
    app.manifest.settings=serde_json::from_value(json!({"round":{"type":"integer","default":30,"min":1,"max":120,"visibility":"replicated"}})).unwrap();
    h.install(vec![app]).unwrap();h.configure_settings("profile_settings",[("round".into(),json!(45))].into()).unwrap();h.start_all().unwrap();
    assert_eq!(h.state("profile_settings","initial"),Some(json!(45)));
    h.set_setting("profile_settings","round",json!(60)).unwrap();assert_eq!(h.state("profile_settings","changed").unwrap()["round"],60);
    for _ in 0..10 {h.tick(0.01,json!({}));}
    let profile=h.profile_snapshot();let ipc:Vec<_>=profile.spans.iter().filter(|s|s.phase=="ipc:invoke").collect();
    assert!(!ipc.is_empty());assert!(ipc.iter().all(|s|s.ipc_receive_wait_us.is_some()));assert!(ipc.iter().all(|s|s.worker_cpu_time_us.is_some()),"trusted worker must measure actual worker CPU: {ipc:?}");
    assert!(profile.spans.iter().any(|s|s.language=="csharp"&&s.resource=="profile_settings"));
}

#[test]
#[ignore = "actual C# profiler overhead measurement; requires trusted isolated worker"]
fn csharp_profile_instrumentation_overhead_measurement() {
    prerequisites();let t=Temp::new();let mut h=host(&t,Side::Server);
    let app=installed(&t,"profile_benchmark","csharp",r#"
      using Skate.Managed;using System.Text.Json.Nodes;
      public class Script:IResourceScript { public void Start(Resource r) {
        r.Lifecycle("on_update",_=>{long sum=0;for(int i=0;i<10000;i++)sum+=i;});
      }}
    "#,&[],&[]);
    h.install(vec![app]).unwrap();h.start_all().unwrap();for _ in 0..100 {h.tick(0.01,json!({}));}
    let mut off=Vec::new();let mut on=Vec::new();
    for round in 0..6 {let enabled=round%2!=0;h.configure_profiling(enabled,4096,60_000).unwrap();let start=std::time::Instant::now();for _ in 0..1000{h.tick(0.01,json!({}));}let ns=start.elapsed().as_nanos() as f64/1000.;if enabled{on.push(ns)}else{off.push(ns)}}
    off.sort_by(f64::total_cmp);on.sort_by(f64::total_cmp);
    eprintln!("PROFILE_CSHARP_OVERHEAD callbacks=6000 off_median_ns_per_dispatch={:.0} on_median_ns_per_dispatch={:.0} added_ns={:.0} ratio={:.3} retained={}",off[1],on[1],on[1]-off[1],on[1]/off[1],h.profile_snapshot().spans.len());
    assert!(h.running("profile_benchmark"));assert!(h.profile_snapshot().spans.iter().any(|s|s.worker_cpu_time_us.is_some()));
}
