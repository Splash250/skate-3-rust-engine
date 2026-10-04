use serde_json::json;
use skate_mods::resources::{Host, InstalledResource, Output, Side};
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
            "skate-runtime-{}-{}",
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
    temp: &Temp,
    id: &str,
    code: &str,
    deps: &[&str],
    capabilities: &[&str],
) -> InstalledResource {
    let root = temp.0.join(id);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.lua"), code).unwrap();
    InstalledResource {
        manifest: Manifest {
            format: 1,
            api: 1,
            id: id.into(),
            version: "1.0.0".into(),
            language: "lua".into(),
            client_scripts: vec![],
            server_scripts: vec![],
            shared_scripts: vec!["main.lua".into()],
            files: vec![],
            dependencies: deps
                .iter()
                .map(|id| (id.to_string(), "1.0.0".into()))
                .collect(),
            exports: vec!["value".into()],
            capabilities: capabilities.iter().map(|x| x.to_string()).collect(),
        },
        root,
        generation: 1,
        grants: capabilities.iter().map(|s| s.to_string()).collect(),
    }
}
fn host(temp: &Temp, side: Side, scope: &str) -> Host {
    Host::new(side, temp.0.join("store"), scope).unwrap()
}
#[test]
fn resources_dependency_exports_events_and_lifecycle() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Server, "server-a");
    let dep = installed(
        &temp,
        "dep",
        r#"resource.export('value', function(n) return n+7 end)"#,
        &[],
        &["resource.exports"],
    );
    let app = installed(
        &temp,
        "app",
        r#"local result=resource.call('dep','value',5)
 resource.on('increment', function(n,sender) assert(sender=='0');resource.state.set('result',result+n) end)
 resource.emit('increment',3)
 sdk.time.after('later',0.01,function() resource.state.set('timer',true) end)"#,
        &["dep"],
        &["resource.events", "resource.exports", "resource.state"],
    );
    h.install(vec![app, dep]).unwrap();
    h.start_all().unwrap();
    assert!(h.running("app"));
    assert_eq!(h.state("app", "result"), Some(json!(15)));
    h.tick(0.02, json!({}));
    assert_eq!(h.state("app", "timer"), Some(json!(true)));
    h.stop("dep").unwrap();
    assert!(!h.running("dep"));
    assert!(!h.running("app"));
    assert_eq!(h.state("app", "result"), None);
    assert!(h.drain_retired().contains(&"app".to_string()));
    h.start_all().unwrap();
    let generation = h.generation("dep").unwrap();
    h.ensure("dep").unwrap();
    assert!(h.generation("dep").unwrap() > generation);
    assert!(h.running("app"));
}
#[test]
fn resources_network_sender_generation_and_side_authority() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Server, "server-a");
    h.install(vec![installed(&temp,"app",r#"resource.on_net('score',function(data,sender) resource.state.set('sender',sender);resource.state.set('score',data.score) end)"#,&[],&["resource.network","resource.state"])]).unwrap();
    h.start_all().unwrap();
    h.receive(42, "app", 1, "score", json!({"score":8,"sender":100}))
        .unwrap();
    assert_eq!(h.state("app", "sender"), Some(json!("42")));
    h.restart("app").unwrap();
    assert!(
        h.receive(42, "app", 1, "score", json!({"score":999}))
            .is_err()
    );
    assert!(h.receive(42, "app", 2, "unknown", json!({})).is_err());
    let mut client = host(&temp, Side::Client, "server-a");
    client
        .install(vec![installed(
            &temp,
            "client",
            "resource.state.set('x',1)",
            &[],
            &["resource.state"],
        )])
        .unwrap();
    assert!(client.start_all().unwrap_err().contains("server"));
}
#[test]
fn resources_grants_budgets_and_failure_cleanup() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Client, "server-a");
    let mut app = installed(
        &temp,
        "app",
        "sdk.ui.text('hello','hello')",
        &[],
        &["engine.ui"],
    );
    app.grants.clear();
    h.install(vec![app]).unwrap();
    assert!(h.start_all().is_err());
    assert!(!h.running("app"));
    assert!(h.drain_commands().is_empty());
    let app = installed(
        &temp,
        "app",
        "return {on_update=function() while true do end end}",
        &[],
        &[],
    );
    h.install(vec![app]).unwrap();
    h.start_all().unwrap();
    h.tick(0.1, json!({}));
    assert!(!h.running("app"));
    assert!(h.diagnostics.iter().any(|e| e.contains("budget")));
    let app = installed(
        &temp,
        "app",
        "sdk.ui.text('hello','hello'); error('startup')",
        &[],
        &["engine.ui"],
    );
    h.install(vec![app]).unwrap();
    assert!(h.start_all().is_err());
    assert!(h.drain_commands().is_empty());
}
#[test]
fn resources_persistence_permissions_and_scopes() {
    let temp = Temp::new();
    let resource = installed(
        &temp,
        "app",
        r#"resource.command('add','challenge.admin',function(args,actor) resource.storage.set('count',(resource.storage.get('count') or 0)+1);resource.state.set('actor',actor) end)
 resource.state.set('count',resource.storage.get('count') or 0)"#,
        &[],
        &["resource.commands", "resource.storage", "resource.state"],
    );
    let mut h = host(&temp, Side::Server, "a");
    h.install(vec![resource.clone()]).unwrap();
    h.start_all().unwrap();
    assert!(h.command(42, "add", vec![], &BTreeSet::new()).is_err());
    h.command(
        42,
        "add",
        vec![],
        &BTreeSet::from(["challenge.admin".into()]),
    )
    .unwrap();
    assert_eq!(h.state("app", "actor"), Some(json!("42")));
    h.disconnect();
    let mut h = host(&temp, Side::Server, "a");
    h.install(vec![resource.clone()]).unwrap();
    h.start_all().unwrap();
    assert_eq!(h.state("app", "count"), Some(json!(1)));
    let mut h = host(&temp, Side::Server, "b");
    h.install(vec![resource]).unwrap();
    h.start_all().unwrap();
    assert_eq!(h.state("app", "count"), Some(json!(0)));
}
#[test]
fn resources_server_has_no_client_engine_and_client_commands_reach_manager() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Server, "a");
    h.install(vec![installed(
        &temp,
        "server",
        "assert(sdk.camera==nil and sdk.ui==nil and sdk.physics==nil and sdk.session==nil)",
        &[],
        &[],
    )])
    .unwrap();
    h.start_all().unwrap();
    let mut h = host(&temp, Side::Client, "a");
    h.install(vec![installed(&temp,"client",r#"sdk.ui.text('hello','hello'); resource.send('ready',{ok=true});return {on_event=function(e) if e.name=='menu_action' then resource.send('menu',{item=e.item}) end end}"#,&[],&["engine.ui","resource.network"])]).unwrap();
    h.start_all().unwrap();
    let mut manager = skate_mods::Manager::new(temp.0.join("mods"), temp.0.join("prefs"));
    manager.attach_resources(h).unwrap();
    assert!(manager.packages["client"].running());
    assert_eq!(manager.commands.len(), 1);
    manager.call(
        "client",
        "on_event",
        json!({"name":"menu_action","item":"start"}),
    );
    assert!(
        matches!(manager.resources.as_mut().unwrap().drain_outputs().last(),Some(Output::Event{name,..}) if name=="menu")
    );
    manager.detach_resources();
    assert!(!manager.packages.contains_key("client"));
    assert!(manager.retired.contains(&"client".to_string()));
}

#[test]
fn resources_export_failure_retires_export_owner_and_dependents() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Client, "a");
    h.install(vec![
        installed(&temp,"dep",r#"resource.export('value',function() sdk.ui.text('owned','bad');error('export failed') end)"#,&[],&["resource.exports","engine.ui"]),
        installed(&temp,"app",r#"return {on_update=function() resource.call('dep','value',{}) end}"#,&["dep"],&["resource.exports"]),
    ]).unwrap();
    h.start_all().unwrap();
    h.tick(0.1, json!({}));
    assert!(!h.running("app"));
    assert!(!h.running("dep"));
    assert!(h.drain_commands().is_empty());
}

#[test]
fn resources_local_event_loop_handler_timer_and_memory_bounds() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Server, "a");
    h.install(vec![installed(
        &temp,
        "loop",
        r#"resource.on('loop',function() resource.emit('loop',{}) end);resource.emit('loop',{})"#,
        &[],
        &["resource.events"],
    )])
    .unwrap();
    assert!(h.start_all().unwrap_err().contains("budget"));
    assert!(!h.running("loop"));
    for (id, code, grants, expected) in [
        (
            "handlers",
            "for n=1,65 do resource.on('event'..n,function() end) end",
            vec!["resource.events"],
            "64 event handlers",
        ),
        (
            "timers",
            "for n=1,65 do sdk.time.after('timer'..n,1,function() end) end",
            vec![],
            "64 timers",
        ),
        (
            "memory",
            "local values={} for n=1,10000 do values[n]=string.rep('x',16000) end",
            vec![],
            "memory",
        ),
    ] {
        let mut h = host(&temp, Side::Server, id);
        h.install(vec![installed(&temp, id, code, &[], &grants)])
            .unwrap();
        let error = h.start_all().unwrap_err();
        assert!(error.to_lowercase().contains(expected), "{error}");
    }
}

#[test]
fn resources_reject_unknown_capabilities_and_nested_command_grant_bypass() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Client, "a");
    assert!(
        h.install(vec![installed(
            &temp,
            "unknown",
            "",
            &[],
            &["engine.native_dll"]
        )])
        .unwrap_err()
        .contains("unsupported")
    );
    h.install(vec![installed(
        &temp,
        "app",
        r#"sdk.commands.request('bypass',{kind='overlay',key='hello',text='x'})"#,
        &[],
        &[],
    )])
    .unwrap();
    assert!(h.start_all().unwrap_err().contains("engine.ui"));
    assert!(h.drain_commands().is_empty());
}

#[test]
fn resources_bundled_challenge_runs_and_persists_with_client_ui() {
    let temp = Temp::new();
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources");
    let installed: Vec<_> = ["skate-rules", "landing-challenge"]
        .iter()
        .map(|id| {
            let root = examples.join(id);
            let manifest = Manifest::read(&root).unwrap();
            let grants = manifest.capabilities.iter().cloned().collect();
            InstalledResource {
                manifest,
                root,
                generation: 1,
                grants,
            }
        })
        .collect();
    let mut server = host(&temp, Side::Server, "example");
    server.install(installed.clone()).unwrap();
    server.start_all().unwrap();
    let mut client = host(&temp, Side::Client, "example");
    client.install(installed.clone()).unwrap();
    client.start_all().unwrap();
    assert!(
        client
            .drain_commands()
            .iter()
            .any(|(_, c)| matches!(c, skate_mods::Command::UiCanvas { .. }))
    );
    let snapshot = |landed_seq| json!({"players":[{"id":"7","position":[0,0,0],"gameplay":{"landed_seq":landed_seq,"mode":"Skating","landed_trick":"Ollie"}}]});
    server.tick(0.1, snapshot(0));
    server
        .receive(
            7,
            "landing-challenge",
            1,
            "enroll",
            json!({"action":"join"}),
        )
        .unwrap();
    for seq in 1..=3 {
        server.tick(1.1, snapshot(seq));
    }
    assert_eq!(
        server.state("landing-challenge", "round").unwrap()["phase"],
        "finished"
    );
    assert_eq!(
        server.state("landing-challenge", "round").unwrap()["leader_points"],
        3
    );
    for output in server.drain_outputs() {
        match output {
            Output::State {
                resource,
                generation,
                key,
                value,
            } => client
                .apply_state(&resource, generation, &key, value)
                .unwrap(),
            Output::Event {
                resource,
                generation,
                recipient,
                name,
                payload,
            } if recipient == Some(7) || recipient.is_none() => client
                .receive(0, &resource, generation, &name, payload)
                .unwrap(),
            _ => {}
        }
    }
    client.dispatch("on_ui_update", json!({"dt":0.1}));
    assert!(!client.drain_commands().is_empty());
    server.disconnect();
    client.disconnect();
    assert!(client.drain_retired().contains(&"landing-challenge".into()));
    let mut reloaded = host(&temp, Side::Server, "example");
    reloaded.install(installed).unwrap();
    reloaded.start_all().unwrap();
    assert_eq!(
        reloaded.state("landing-challenge", "round").unwrap()["completed_rounds"],
        1
    );
}

#[test]
fn resources_manager_discards_pending_commands_when_owner_retires() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Client, "a");
    h.install(vec![installed(
        &temp,
        "app",
        r#"sdk.ui.text('hello','pending')"#,
        &[],
        &["engine.ui"],
    )])
    .unwrap();
    let mut manager = skate_mods::Manager::new(temp.0.join("mods"), temp.0.join("prefs"));
    manager.attach_resources(h).unwrap();
    assert_eq!(manager.commands.len(), 1);
    manager.resources.as_mut().unwrap().stop("app").unwrap();
    manager.sync_resources();
    assert!(manager.commands.is_empty());
    manager.resources.as_mut().unwrap().ensure("app").unwrap();
    manager.sync_resources();
    assert_eq!(manager.commands.len(), 1);
    manager.detach_resources();
    assert!(manager.commands.is_empty());
}

#[derive(Default)]
struct QueryProbe {
    springs: Vec<String>,
}
impl skate_mods::DynamicsHost for QueryProbe {
    fn raycast(
        &mut self,
        _: [f32; 3],
        _: [f32; 3],
        _: &skate_mods::RaycastOptions,
    ) -> Option<serde_json::Value> {
        None
    }
    fn velocity_at(&self, _: &str, _: [f32; 3]) -> Option<[f32; 3]> {
        None
    }
    fn effective_inv_mass(&self, _: &str, _: [f32; 3], _: [f32; 3]) -> Option<f32> {
        None
    }
    fn spring_ray(
        &mut self,
        key: &str,
        _: skate_dynamics::SpringRayDesc,
    ) -> Option<skate_dynamics::SpringRayHit> {
        self.springs.push(key.into());
        None
    }
    fn local_ang_accel_impulse(&self, _: &str, _: [f32; 3], _: f32) -> Option<[f32; 3]> {
        None
    }
}
#[test]
fn resources_native_queries_require_grants_and_exports_cannot_borrow_callers_body_scope() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Client, "a");
    let mut physics = QueryProbe::default();
    h.install(vec![installed(
        &temp,
        "denied",
        "sdk.physics.spring_ray('other',{})",
        &[],
        &[],
    )])
    .unwrap();
    let result = skate_mods::with_host(&mut physics, || h.start_all());
    assert!(
        result.is_err(),
        "ungranted immediate physics mutation was accepted"
    );
    assert!(physics.springs.is_empty());
    let mut h = host(&temp, Side::Client, "a");
    h.install(vec![
        installed(&temp,"dep",r#"resource.export('value',function() return sdk.physics.spring_ray('foreign',{}) end)"#,&[],&["resource.exports","engine.physics"]),
        installed(&temp,"app",r#"return {on_fixed_update=function() assert(resource.call('dep','value',{})==nil);sdk.physics.spring_ray('own',{}) end}"#,&["dep"],&["resource.exports","engine.physics"]),
    ]).unwrap();
    h.start_all().unwrap();
    skate_mods::with_host(&mut physics, || {
        h.call("app", "on_fixed_update", json!({"dt":0.1}))
    });
    assert!(h.running("app"), "{:?}", h.diagnostics);
    assert_eq!(
        physics.springs,
        vec!["own"],
        "export must never inherit caller-owned native body lookup"
    );
}

#[test]
fn resources_payload_nesting_and_native_query_work_are_bounded() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Client, "a");
    h.install(vec![installed(
        &temp,
        "nested",
        r#"local t=1 for n=1,40 do t={x=t} end resource.send('deep',t)"#,
        &[],
        &["resource.network"],
    )])
    .unwrap();
    assert!(h.start_all().unwrap_err().contains("nesting"));
    let mut h = host(&temp, Side::Client, "a");
    let mut physics = QueryProbe::default();
    h.install(vec![installed(
        &temp,
        "query",
        r#"for n=1,129 do sdk.physics.spring_ray('own',{}) end"#,
        &[],
        &["engine.physics"],
    )])
    .unwrap();
    let error = skate_mods::with_host(&mut physics, || h.start_all()).unwrap_err();
    assert!(error.contains("128 resource operations"));
    assert_eq!(physics.springs.len(), 128);
}

#[test]
fn resources_cfx_client_broadcast_alias_and_console_permission() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Server, "a");
    h.install(vec![installed(&temp,"app",r#"TriggerClientEvent('hello',-1,{greeting='hi'});RegisterCommand('admin',function(args,actor) resource.state.set('actor',actor) end,'app.admin')"#,&[],&["resource.network","resource.commands","resource.state"])]).unwrap();
    h.start_all().unwrap();
    assert!(matches!(&h.drain_outputs()[0],Output::Event{recipient:None,name,..} if name=="hello"));
    assert!(
        h.command(42, "admin", vec![], &BTreeSet::from(["*".into()]))
            .is_err()
    );
    h.command(0, "admin", vec![], &BTreeSet::from(["*".into()]))
        .unwrap();
    assert_eq!(h.state("app", "actor"), Some(json!("0")));
}

#[test]
fn resources_full_u64_identities_round_trip_as_canonical_decimal_strings() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Server, "identities");
    let mut app = installed(
        &temp,
        "app",
        r#"assert(type(resource.generation)=='string');resource.on_net('who',function(_,sender) assert(type(sender)=='string');resource.state.set('sender',sender);resource.send('echo',{sender=sender},sender) end);resource.command('who','who.read',function(_,actor) resource.state.set('actor',actor) end)"#,
        &[],
        &["resource.network", "resource.state", "resource.commands"],
    );
    app.generation = u64::MAX;
    h.install(vec![app]).unwrap();
    h.start_all().unwrap();
    for sender in [9_007_199_254_740_993, u64::MAX - 1, u64::MAX] {
        h.receive(sender, "app", u64::MAX, "who", json!({}))
            .unwrap();
        assert_eq!(h.state("app", "sender"), Some(json!(sender.to_string())));
        assert!(h.drain_outputs().iter().any(|o|matches!(o,Output::Event{recipient:Some(id),payload,..} if *id==sender && payload["sender"]==sender.to_string())));
    }
    h.command(
        u64::MAX,
        "who",
        vec![],
        &BTreeSet::from(["who.read".into()]),
    )
    .unwrap();
    assert_eq!(h.state("app", "actor"), Some(json!(u64::MAX.to_string())));
}

#[test]
fn resources_state_tombstones_do_not_consume_live_key_quota() {
    let temp = Temp::new();
    let mut server = host(&temp, Side::Server, "a");
    server
        .install(vec![installed(
            &temp,
            "app",
            r#"for n=1,64 do resource.state.set('key'..n,n) end resource.state.set('missing',nil)"#,
            &[],
            &["resource.state"],
        )])
        .unwrap();
    server.start_all().unwrap();
    assert!(server.running("app"));
    assert_eq!(server.states()["app"].len(), 64);
    let mut client = host(&temp, Side::Client, "a");
    client
        .install(vec![installed(&temp, "app", "", &[], &["resource.state"])])
        .unwrap();
    client.start_all().unwrap();
    for n in 0..64 {
        client
            .apply_state("app", 1, &format!("key{n}"), json!(n))
            .unwrap();
    }
    client
        .apply_state("app", 1, "missing", serde_json::Value::Null)
        .unwrap();
    assert_eq!(client.states()["app"].len(), 64);
}

#[test]
fn resources_deferred_events_and_unload_cannot_inherit_other_resource_native_scope() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Client, "a");
    let mut physics = QueryProbe::default();
    h.install(vec![
        installed(&temp,"dep",r#"resource.on('touch',function() sdk.physics.spring_ray('event_foreign',{}) end);resource.export('value',function() resource.emit('touch',{}) end);return {on_unload=function() sdk.physics.spring_ray('unload_foreign',{}) end}"#,&[],&["resource.exports","resource.events","engine.physics"]),
        installed(&temp,"app",r#"return {on_fixed_update=function() resource.call('dep','value',{});sdk.physics.spring_ray('own',{}) end}"#,&["dep"],&["resource.exports","engine.physics"]),
    ]).unwrap();
    h.start_all().unwrap();
    skate_mods::with_host(&mut physics, || {
        h.call("app", "on_fixed_update", json!({"dt":0.1}))
    });
    skate_mods::with_host(&mut physics, || h.stop("dep").unwrap());
    assert_eq!(physics.springs, vec!["own"]);
}

#[test]
fn resources_stopped_dependents_can_be_refreshed_during_version_upgrade() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Server, "a");
    let mut dep = installed(&temp, "dep", "", &[], &[]);
    let mut first = installed(&temp, "first", "", &["dep"], &[]);
    let mut second = installed(&temp, "second", "", &["dep"], &[]);
    h.install(vec![dep.clone(), first.clone(), second.clone()])
        .unwrap();
    h.start_all().unwrap();
    h.stop("dep").unwrap();
    dep.manifest.version = "2.0.0".into();
    h.register(vec![dep.clone()]).unwrap();
    h.start("dep").unwrap();
    assert!(h.start("first").unwrap_err().contains("1.0.0"));
    assert!(!h.running("first"));
    assert!(h.running("dep"));
    first
        .manifest
        .dependencies
        .insert("dep".into(), "2.0.0".into());
    h.register(vec![first]).unwrap();
    h.start("first").unwrap();
    assert!(h.start("second").is_err());
    second
        .manifest
        .dependencies
        .insert("dep".into(), "2.0.0".into());
    h.register(vec![second]).unwrap();
    h.start("second").unwrap();
    dep.manifest.version = "3.0.0".into();
    assert!(h.register(vec![dep]).is_err());
    assert!(h.running("first") && h.running("second"));
}

#[test]
#[ignore = "subprocess helper for bounded resource finalizer regression"]
fn resource_finalizer_subprocess() {
    let temp = Temp(PathBuf::from(
        std::env::var("SKATE_FINALIZER_TEST_ROOT").unwrap(),
    ));
    for code in [
        "resource.export('value',function() return 1 end);resource._held=setmetatable({},{__gc=function() resource.state.set('late',true) end})",
        "local mt={};local held=setmetatable({},mt);mt.__gc=function() resource.state.set('late',true) end;resource._held=held;resource.export('value',function()return 1 end);setmetatable(held,mt)",
    ] {
        let mut h = host(&temp, Side::Server, "finalizers");
        h.install(vec![installed(
            &temp,
            "app",
            code,
            &[],
            &["resource.exports", "resource.state"],
        )])
        .unwrap();
        assert!(h.start_all().unwrap_err().contains("finalizers"));
        assert!(h.drain_outputs().is_empty());
        drop(h);
    }
}

#[test]
fn resources_user_finalizers_cannot_deadlock_stop_or_reenter_runtime() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let temp = Temp::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "resource_finalizer_subprocess", "--ignored"])
        .env("SKATE_FINALIZER_TEST_ROOT", &temp.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "resource finalizer subprocess failed");
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("resource stop deadlocked during Lua finalization");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn resources_asset_object_listing_never_imports_external_images_or_buffers() {
    let temp = Temp::new();
    let mut app = installed(
        &temp,
        "app",
        r#"local objects=sdk.assets.objects('unsafe.glb');assert(objects.nodes[1]=='harmless-node' and objects.meshes[1]=='harmless-mesh')"#,
        &[],
        &["engine.graphics"],
    );
    app.manifest.files.push("unsafe.glb".into());
    let mut bytes=serde_json::to_vec(&json!({"asset":{"version":"2.0"},"nodes":[{"name":"harmless-node","mesh":0}],"meshes":[{"name":"harmless-mesh","primitives":[{"attributes":{"POSITION":999}}]}],"buffers":[{"byteLength":1,"uri":"file:///must-never-read"}],"images":[{"uri":"file:///must-never-decode"}]})).unwrap();
    while bytes.len() % 4 != 0 {
        bytes.push(b' ');
    }
    let mut glb = b"glTF".to_vec();
    glb.extend(2u32.to_le_bytes());
    glb.extend((20u32 + bytes.len() as u32).to_le_bytes());
    glb.extend((bytes.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(bytes);
    std::fs::write(app.root.join("unsafe.glb"), glb).unwrap();
    let mut h = host(&temp, Side::Client, "assets");
    h.install(vec![app]).unwrap();
    h.start_all().unwrap();
    assert!(h.running("app"));
}
