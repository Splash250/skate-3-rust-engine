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
            settings: Default::default(),
            requires_features: vec![],
            world: None,
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
fn resources_repeated_large_export_failures_retain_bounded_utf8_diagnostics() {
    let temp=Temp::new();
    let mut h=host(&temp,Side::Server,"bounded-errors");
    let dep=installed(&temp,"large_error",r#"
        local message=string.rep('🙂',262144)
        resource.export('value',function()error(message)end)
        return {}
    "#,&[],&["resource.exports"]);
    let caller=installed(&temp,"caller",r#"
        return {on_update=function()
            for i=1,8 do pcall(resource.call,'large_error','value',nil) end
        end}
    "#,&["large_error"],&["resource.exports"]);
    h.install(vec![dep,caller]).unwrap();h.start_all().unwrap();
    h.tick(0.01,json!({}));
    assert!(!h.running("large_error"));assert!(!h.running("caller"));
    let sizes:Vec<_>=h.diagnostics.iter().map(String::len).collect();
    assert_eq!(sizes.len(),1,"one pending export failure per owner");
    assert!(sizes.iter().all(|&size|size<=2048),"retained diagnostic byte counts: {sizes:?}");
    assert!(h.diagnostics.iter().any(|error|error.contains("large_error")));
    let metrics=h.runtime_metrics();
    assert!(metrics["large_error"].errors>=8);
    assert!(metrics["large_error"].last_error.as_ref().is_some_and(|error|error.len()<=2048));
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
                ..
            } => client
                .apply_state(&resource, generation, &key, value)
                .unwrap(),
            Output::Event {
                resource,
                generation,
                recipient,
                name,
                payload,
                ..
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

#[test]
fn resources_cfx_threads_vectors_and_protected_calls() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Server, "cfx");
    h.install(vec![installed(&temp, "app", r#"
        local ok, text = pcall(function() error('ordinary failure') end)
        assert(not ok and string.find(text, 'ordinary failure'))
        local v = vector3(3, 4, 0)
        assert(#v == 5 and (v + vector3(1, 2, 3)).x == 4)
        assert((2 * v).y == 8 and (v / 2).x == 1.5)
        CreateThread(function()
            resource.state.set('phase', 1)
            Wait(0)
            resource.state.set('phase', 2)
            assert(pcall(function() Citizen.Wait(50) end))
            resource.state.set('phase', 3)
        end)
    "#, &[], &["resource.state"])]).unwrap();
    h.start_all().unwrap();
    h.tick(0.01, json!({}));
    assert_eq!(h.state("app", "phase"), Some(json!(1)));
    h.tick(0.01, json!({}));
    assert_eq!(h.state("app", "phase"), Some(json!(2)));
    h.tick(0.02, json!({}));
    assert_eq!(h.state("app", "phase"), Some(json!(2)));
    h.tick(0.04, json!({}));
    assert_eq!(h.state("app", "phase"), Some(json!(3)));
    h.restart("app").unwrap();
    assert_eq!(h.state("app", "phase"), None);
}

#[test]
fn resources_cfx_runaways_cannot_escape_through_error_handlers_or_coroutines() {
    if std::env::var_os("SKATE_CFX_RUNAWAY_CHILD").is_none() {
        use std::{process::Command, time::{Duration, Instant}};
        let mut child=Command::new(std::env::current_exe().unwrap())
            .args(["--exact","resources_cfx_runaways_cannot_escape_through_error_handlers_or_coroutines"])
            .env("SKATE_CFX_RUNAWAY_CHILD","1").spawn().unwrap();
        let deadline=Instant::now()+Duration::from_secs(5);
        loop {
            if let Some(status)=child.try_wait().unwrap() { assert!(status.success(),"runaway child failed"); return; }
            if Instant::now()>=deadline { child.kill().unwrap();child.wait().unwrap();panic!("resource runaway escaped its budget"); }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let temp = Temp::new();
    for (index, runaway) in [
        "while true do pcall(function() while true do end end) end",
        "while true do xpcall(function() while true do end end, function() return 'caught' end) end",
        "local t=coroutine.create(function() while true do end end); coroutine.resume(t)",
        "local f=coroutine.wrap(function() while true do end end); pcall(f)",
        "xpcall(function()error('start')end,function()while true do pcall(function()while true do end end)end end)",
        "pcall(function() local t=coroutine.create(function() local inner=coroutine.create(function()while true do end end);coroutine.resume(inner)end);coroutine.resume(t) end)",
        "pcall(function()string.find(string.rep('a',20000),'.*.*.*.*b')end)",
        "pcall(function()string.match(string.rep('a',20000),'.*.*.*.*b')end)",
        "pcall(function()string.gsub(string.rep('a',20000),'.*.*.*.*b','x')end)",
        "pcall(function()for part in string.gmatch(string.rep('a',20000),'.*.*.*.*b')do end end)",
    ].iter().enumerate() {
        let mut h = host(&temp, Side::Server, &format!("runaway-{index}"));
        h.install(vec![installed(&temp, "app", &format!("CreateThread(function() {runaway} end)"), &[], &[])]).unwrap();
        h.start_all().unwrap();
        h.tick(0.01, json!({}));
        assert!(!h.running("app"), "runaway {index} survived");
        assert!(h.diagnostics.iter().any(|line| line.contains("budget")), "{:?}", h.diagnostics);
    }
}

#[test]
fn resources_native_patterns_preserve_token_scans_captures_and_substitution() {
    let temp=Temp::new();
    let mut h=host(&temp,Side::Server,"patterns");
    h.install(vec![installed(&temp,"app",r#"
        assert(string.find(string.rep('a',32000)..'end','end',1,true)==32001)
        assert(string.match('example_42','^([%w_]+)$')=='example_42')
        assert(string.match('(one(two))','%b()')=='(one(two))')
        assert(string.match('abcabc','(%a+)%1')=='abc')
        assert(string.gsub('one 2 three 4','%d+',function(s)return tonumber(s)*2 end)=='one 4 three 8')
        assert(string.gsub('abc','(.)','[%1]')=='[a][b][c]')
        assert(string.match(string.rep('a',12000),'%a+')==string.rep('a',12000))
        local count=0
        for word in string.gmatch(string.rep('word ',80),'%a+') do assert(word=='word');count=count+1 end
        assert(count==80)
        resource.state.set('ok',true)
    "#,&[],&["resource.state"])]).unwrap();
    h.start_all().unwrap();
    assert_eq!(h.state("app","ok"),Some(json!(true)));
}

#[test]
fn resources_runtime_metrics_measure_live_vms_and_retain_generation_failure() {
    let temp=Temp::new();
    let mut h=host(&temp,Side::Server,"metrics");
    h.install(vec![installed(&temp,"app",r#"
        resource.state.set('queued',string.rep('x',1024))
        resource.on('fail',function()error('measured failure')end)
        return {on_update=function() local n=0;for i=1,10000 do n=n+i end end}
    "#,&[],&["resource.state","resource.events"])]).unwrap();
    h.start_all().unwrap();
    h.tick(0.01,json!({}));
    let metrics=h.runtime_metrics();
    let live=&metrics["app"];
    assert!(live.running && live.invocations>=3 && live.lua_heap_bytes.unwrap()>0);
    assert!(live.total_wall_time_us>=live.max_wall_time_us && live.max_wall_time_us>0);
    assert!(live.max_budget_units>0 && live.queued_outputs==1 && live.queued_output_accounted_bytes>=1024);
    #[cfg(any(target_os="linux",target_os="windows"))]
    assert!(live.total_host_cpu_time_us.is_some());
    let generation=live.generation;
    assert!(h.host_event("app",generation,"fail",json!(null)).is_err());
    let metrics=h.runtime_metrics();
    let failed=&metrics["app"];
    assert!(!failed.running && failed.errors>0 && failed.lua_heap_bytes.is_none());
    assert!(failed.last_error.as_ref().unwrap().contains("measured failure"));
    assert_eq!(failed.generation,generation);
    h.ensure("app").unwrap();
    let metrics=h.runtime_metrics();
    assert!(metrics["app"].running && metrics["app"].generation>generation);
    assert_eq!(metrics["app"].errors,0);
    assert_eq!(metrics.len(),1);
}

#[test]
fn resources_values_and_persistence_exceed_legacy_caps_and_survive_restart() {
    let temp = Temp::new();
    let mut h = host(&temp, Side::Server, "larger-store");
    h.install(vec![installed(&temp, "app", r#"
        local blob = string.rep('x', 8192)
        for i=1,12 do resource.storage.set('entry'..i, blob) end
        resource.state.set('large', resource.storage.get('entry12'))
    "#, &[], &["resource.storage", "resource.state"])]).unwrap();
    h.start_all().unwrap();
    assert_eq!(h.state("app", "large"), Some(json!("x".repeat(8192))));
    h.restart("app").unwrap();
    assert_eq!(h.state("app", "large"), Some(json!("x".repeat(8192))));
    let scope = std::fs::read_dir(temp.0.join("store")).unwrap().next().unwrap().unwrap().path();
    assert!(std::fs::metadata(scope.join("app.json")).unwrap().len() > 64 * 1024);
}

#[test]
fn resources_configured_limits_reject_overflows_without_persisting_partial_writes() {
    use skate_mods::resources::RuntimeLimits;
    let temp = Temp::new();
    let limits=RuntimeLimits { max_payload_bytes: 1024, max_storage_bytes: 128 * 1024,
        max_storage_value_bytes: 96 * 1024, max_storage_keys: 2, max_threads: 1, ..RuntimeLimits::default() };
    let mut h=Host::new_with_limits(Side::Server,temp.0.join("store"),"custom",limits).unwrap();
    h.install(vec![installed(&temp,"app",r#"
        resource.storage.set('big',string.rep('v',80*1024))
        assert(not pcall(function()resource.storage.set('other',string.rep('x',80*1024))end))
        assert(resource.storage.get('other')==nil)
        assert(not pcall(function()resource.state.set('large',string.rep('x',1025))end))
        CreateThread(function()Wait(100)end)
        assert(not pcall(function()CreateThread(function()end)end))
        resource.state.set('valid',true)
    "#,&[],&["resource.storage","resource.state"])]).unwrap();
    h.start_all().unwrap();
    assert_eq!(h.state("app","valid"),Some(json!(true)));
    assert_eq!(h.state("app","large"),None);
    h.restart("app").unwrap();
    let invalid=RuntimeLimits{max_payload_bytes:256*1024+1,..RuntimeLimits::default()};
    assert!(Host::new_with_limits(Side::Server,temp.0.join("store"),"bad",invalid).is_err());
}

#[test]
fn resources_teleport_is_server_granted_and_generation_scoped() {
    let temp=Temp::new();
    let app=installed(&temp,"app",r#"
        assert(not pcall(function()resource.teleport('42',{restore_previous=true,position={1,2,3}})end))
        assert(not pcall(function()resource.teleport('42',{})end))
        resource.teleport('42',{position={1,2,3},instance=7,restore_on_stop=true})
        resource.teleport('42',{restore_previous=true})
    "#,&[],&["resource.teleport"]);
    let mut h=host(&temp,Side::Server,"teleport");
    h.install(vec![app.clone()]).unwrap();
    h.start_all().unwrap();
    assert!(matches!(h.drain_outputs().as_slice(),[
        Output::Teleport{resource,generation:1,player:42,position:[1.0,2.0,3.0],instance:Some(7),restore_on_stop:true,restore_previous:false,..},
        Output::Teleport{player:42,restore_previous:true,restore_on_stop:false,..}
    ] if resource=="app"));
    let mut h=host(&temp,Side::Client,"teleport");
    h.install(vec![app.clone()]).unwrap();
    assert!(h.start_all().unwrap_err().contains("server-only"));
    let mut denied=app;denied.grants.clear();
    let mut h=host(&temp,Side::Server,"teleport");
    h.install(vec![denied]).unwrap();
    assert!(h.start_all().unwrap_err().contains("lacks capability"));
}

#[test]
fn resources_async_service_requests_are_server_only_and_capability_scoped() {
    let temp=Temp::new();
    let app=installed(&temp,"app",r#"
        resource.services.submit('inventory',{kind='query',statement={sql='SELECT 1',params={}}},1000)
        resource.services.cancel('inventory')
        assert(not pcall(function()resource.services.submit('web',{kind='http',request={url='http://127.0.0.1/'}},1000)end))
    "#,&[],&["resource.database"]);
    let mut h=host(&temp,Side::Server,"services");
    h.install(vec![app.clone()]).unwrap();
    h.start_all().unwrap();
    assert_eq!(h.drain_outputs().len(),2);
    let mut h=host(&temp,Side::Client,"services");
    h.install(vec![app]).unwrap();
    assert!(h.start_all().unwrap_err().contains("server-only"));
}

#[test]
fn resources_service_results_reject_stale_generation_and_deliver_bounded_local_payload() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"results");
    h.install(vec![installed(&temp,"app",r#"resource.on('service_result',function(payload,sender) assert(sender=='0');resource.state.set('result',payload)end)"#,&[],&["resource.events","resource.state"])]).unwrap();
    h.start_all().unwrap();
    h.service_result("app",1,"request",json!({"ok":true})).unwrap();
    assert_eq!(h.state("app","result"),Some(json!({"key":"request","result":{"ok":true}})));
    h.restart("app").unwrap();
    assert!(h.service_result("app",1,"request",json!({"ok":false})).is_err());
    assert_eq!(h.state("app","result"),None);
    assert!(h.service_result("app",2,"huge",json!("x".repeat(256*1024))).is_err());
}

fn installed_javascript(temp:&Temp,id:&str,code:&str,deps:&[&str],caps:&[&str])->InstalledResource {
    let mut app=installed(temp,id,"",deps,caps);
    std::fs::write(app.root.join("main.js"),code).unwrap();
    app.manifest.language="javascript".into();
    app.manifest.shared_scripts=vec!["main.js".into()];
    app
}

#[test]
fn resources_javascript_players_are_an_array_before_join_and_after_disconnect() {
    let temp=Temp::new();let mut runtime=host(&temp,Side::Server,"js-empty-players");
    runtime.install(vec![installed_javascript(&temp,"observer",r#"
resource.lifecycle({on_load(){
    const ids=resource.players().map(p=>p.id);
    if(ids.length!==0)throw Error('startup must have an empty player array');
},on_update(){
    resource.state.set('ids',resource.players().map(p=>p.id).join(','));
}});
"#,&[],&["resource.state"])]).unwrap();
    runtime.start_all().unwrap();
    runtime.tick(0.01,json!({"players":[{"id":"42"}]}));
    assert_eq!(runtime.state("observer","ids"),Some(json!("42")));
    runtime.tick(0.01,json!({"players":[]}));
    assert!(runtime.running("observer"),"disconnect leaves array operations usable");
    assert_eq!(runtime.state("observer","ids"),Some(json!("")));
}

#[test]
fn resources_javascript_cross_language_lifecycle_events_and_persistence() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"javascript");
    let javascript=installed_javascript(&temp,"js-base",r#"
        resource.export('value', x=>({count:x.count+3}));
        resource.on('persist', value=>resource.storage.set('count',value));
        resource.lifecycle({on_load(){resource.emit('persist',7)}});
    "#,&[],&["resource.exports","resource.events","resource.storage"]);
    let lua=installed(&temp,"lua-middle",r#"resource.export('value',function(x) local v=resource.call('js-base','value',x);return {count=v.count+5} end)"#,&["js-base"],&["resource.exports"]);
    let consumer=installed_javascript(&temp,"js-app",r#"
        resource.lifecycle({on_load(){resource.state.set('answer',resource.call('lua-middle','value',{count:4}).count)}});
        resource.onNet('echo',(payload,sender)=>resource.state.set('sender',sender));
        setTimeout(()=>resource.state.set('timer',true),20);
    "#,&["lua-middle"],&["resource.exports","resource.state","resource.network"]);
    h.install(vec![javascript,lua,consumer]).unwrap();h.start_all().unwrap();
    assert_eq!(h.state("js-app","answer"),Some(json!(12)));
    h.receive(u64::MAX,"js-app",1,"echo",json!({})).unwrap();
    assert_eq!(h.state("js-app","sender"),Some(json!(u64::MAX.to_string())));
    h.tick(0.03,json!({}));assert_eq!(h.state("js-app","timer"),Some(json!(true)));
    h.stop("js-base").unwrap();assert!(!h.running("js-app"));
    h.start_all().unwrap();assert_eq!(h.state("js-app","answer"),Some(json!(12)));
}

#[test]
fn resources_javascript_client_grants_engine_commands_and_isolation() {
    let temp=Temp::new();let mut h=host(&temp,Side::Client,"javascript-client");
    h.install(vec![installed_javascript(&temp,"js-client",r#"
        if(typeof require!=='undefined'||typeof process!=='undefined'||typeof fetch!=='undefined')throw Error('ambient I/O exposed');
        sdk.ui.text('hello','JavaScript');
        resource.send('ready',{language:'javascript'});
    "#,&[],&["engine.ui","resource.network"])]).unwrap();h.start_all().unwrap();
    assert_eq!(h.drain_commands().len(),1);
    assert!(matches!(h.drain_outputs().as_slice(),[Output::Event{name,..}] if name=="ready"));
    h.disconnect();assert!(!h.running("js-client"));
    let mut app=installed_javascript(&temp,"denied", "sdk.ui.text('hello','denied')",&[],&["engine.ui"]);
    app.grants.clear();h.install(vec![app]).unwrap();assert!(h.start_all().unwrap_err().contains("capability"));
}

#[test]
fn resources_javascript_runaways_promises_and_memory_are_contained() {
    if std::env::var_os("SKATE_JS_RUNAWAY_CHILD").is_none() {
        use std::{process::Command,time::{Duration,Instant}};
        let mut child=Command::new(std::env::current_exe().unwrap())
            .args(["--exact","resources_javascript_runaways_promises_and_memory_are_contained"])
            .env("SKATE_JS_RUNAWAY_CHILD","1").spawn().unwrap();
        let deadline=Instant::now()+Duration::from_secs(10);
        loop {
            if let Some(status)=child.try_wait().unwrap(){assert!(status.success(),"JavaScript runaway child failed");return;}
            if Instant::now()>=deadline {child.kill().unwrap();child.wait().unwrap();panic!("JavaScript runaway escaped containment");}
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let temp=Temp::new();
    for (index,body) in [
        "while(true){try{while(true){}}catch(e){}}",
        "const recur=()=>Promise.resolve().then(recur);recur();",
        "const values=[];while(true)values.push('x'.repeat(1024*1024));",
        "Promise.reject(Error('unhandled failure'));",
    ].into_iter().enumerate() {
        let mut h=host(&temp,Side::Server,&format!("js-runaway-{index}"));
        h.install(vec![installed_javascript(&temp,"app",&format!("resource.lifecycle({{on_update(){{{body}}}}});"),&[],&[])]).unwrap();
        h.start_all().unwrap();h.tick(0.01,json!({}));
        assert!(!h.running("app"),"JavaScript failure case {index} survived");
        assert!(!h.diagnostics.is_empty());
    }
    let mut h=host(&temp,Side::Server,"handled-promises");
    h.install(vec![installed_javascript(&temp,"healthy",r#"
        resource.lifecycle({on_update(){Promise.reject(Error('handled')).catch(()=>resource.state.set('handled',true));}});
    "#,&[],&["resource.state"])]).unwrap();h.start_all().unwrap();h.tick(0.01,json!({}));
    assert!(h.running("healthy"));assert_eq!(h.state("healthy","handled"),Some(json!(true)));
}

#[test]
fn resources_bundled_javascript_lua_example_runs_both_sides_and_restarts() {
    let temp=Temp::new();
    let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources");
    let resources=["js-rules","lua-rule-adapter","cross-language-demo"].map(|id| {
        let resource_root=root.join(id);let manifest=Manifest::read(&resource_root).unwrap();
        InstalledResource{grants:manifest.capabilities.iter().cloned().collect(),manifest,root:resource_root,generation:1}
    });
    let mut server=host(&temp,Side::Server,"bundled-js");server.install(resources.to_vec()).unwrap();server.start_all().unwrap();
    assert_eq!(server.state("cross-language-demo","preview"),Some(json!({"label":"Checkpoint rule preview","points":75})));
    assert_eq!(server.state("cross-language-demo","starts"),Some(json!(1)));
    let mut client=host(&temp,Side::Client,"bundled-js");client.install(resources.to_vec()).unwrap();client.start_all().unwrap();
    for out in client.drain_outputs() { if let Output::Event{resource,generation,name,payload,..}=out {server.receive(42,&resource,generation,&name,payload).unwrap();} }
    for out in server.drain_outputs() {if let Output::Event{resource,generation,name,payload,recipient:Some(42),..}=out {client.receive(0,&resource,generation,&name,payload).unwrap();} }
    assert_eq!(client.drain_commands().len(),1);
    server.disconnect();
    let mut recovered=host(&temp,Side::Server,"bundled-js");recovered.install(resources.to_vec()).unwrap();recovered.start_all().unwrap();
    assert_eq!(recovered.state("cross-language-demo","starts"),Some(json!(2)));
    client.disconnect();assert!(client.drain_retired().contains(&"cross-language-demo".to_string()));
}

#[test]
fn resources_javascript_export_rejects_promises_and_custom_object_serialization() {
    let temp=Temp::new();
    for (index,result) in ["Promise.resolve(7)","new Date(0)"].into_iter().enumerate() {
        let mut h=host(&temp,Side::Server,&format!("js-async-export-{index}"));
        let js=installed_javascript(&temp,"js-dep",&format!("resource.export('value',()=>{result});"),&[],&["resource.exports"]);
        let lua=installed(&temp,"app","return {on_update=function() resource.call('js-dep','value',{}) end}",&["js-dep"],&["resource.exports"]);
        h.install(vec![js,lua]).unwrap();h.start_all().unwrap();h.tick(0.01,json!({}));
        assert!(!h.running("js-dep"),"non-JSON export survived: {result}");
        assert!(!h.running("app"));
    }
}

#[test]
fn resources_coroutine_close_cannot_swallow_budget_exhaustion() {
    let temp=Temp::new();
    let exhaust=r#"
        local co=coroutine.create(function()
            local closer <close> = setmetatable({}, {__close=function() while true do end end})
            coroutine.yield()
        end)
        coroutine.resume(co)
        coroutine.close(co)
    "#;
    let mut h=host(&temp,Side::Server,"close-callback");
    h.install(vec![installed(&temp,"app",&format!("return {{on_update=function() {exhaust} end}}"),&[],&[])]).unwrap();
    h.start_all().unwrap();h.tick(0.01,json!({}));
    assert!(!h.running("app"),"native close swallowed the exhausted callback budget");
    assert!(h.diagnostics.iter().any(|e|e.contains("budget")));
    let mut h=host(&temp,Side::Server,"close-startup");
    h.install(vec![installed(&temp,"app",exhaust,&[],&[])]).unwrap();
    assert!(h.start_all().unwrap_err().contains("budget"));
}

#[test]
fn resources_javascript_off_and_export_replacement_release_callback_slots() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"js-callback-reuse");
    let app=installed_javascript(&temp,"app",r#"
        let ticks=0;
        resource.lifecycle({on_update(){
            ticks++;
            resource.on('temporary',()=>{});
            resource.off('temporary');
            resource.export('value',()=>ticks);
        }});
    "#,&[],&["resource.events","resource.exports"]);
    let consumer=installed(&temp,"consumer",r#"return {on_update=function()resource.state.set('ticks',resource.call('app','value',{}))end}"#,&["app"],&["resource.exports","resource.state"]);
    h.install(vec![app,consumer]).unwrap();h.start_all().unwrap();
    for _ in 0..200 {h.tick(0.01,json!({}));assert!(h.running("app"),"{:?}",h.diagnostics);}
    assert_eq!(h.state("consumer","ticks"),Some(json!(200)));
}

#[test]
fn resources_javascript_callback_cap_survives_mutated_map_intrinsics() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"js-intrinsics");
    h.install(vec![installed_javascript(&temp,"app",r#"
        Object.defineProperty(Map.prototype,'size',{get(){return 0}});
        for(let i=0;i<2000;i++)resource.lifecycle({on_event(){}});
    "#,&[],&[])]).unwrap();
    assert!(h.start_all().unwrap_err().contains("callback limit"));
}

#[test]
fn resources_install_obeys_configured_resource_count() {
    let temp=Temp::new();
    let limits=skate_mods::resources::RuntimeLimits {max_resources:1,..Default::default()};
    let mut h=Host::new_with_limits(Side::Server,temp.0.join("store"),"count-limit",limits).unwrap();
    let a=installed(&temp,"one","return {}",&[],&[]);
    let b=installed(&temp,"two","return {}",&[],&[]);
    assert!(h.install(vec![a,b]).unwrap_err().contains("resource count"));
}

#[test]
fn resources_scoped_state_keeps_equal_keys_separate() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"scopes");
    let app=installed(&temp,"scoped",r#"
local a={kind='instance',id='1'}; local b={kind='instance',id='2'}
resource.state.set('same','one',a);resource.state.set('same','two',b)
assert(resource.state.get('same',a)=='one','instance state leaked')
assert(resource.state.get('same',b)=='two')
assert(resource.state.get('same')==nil)
resource.state.set('verified',true)
resource.send('private',{hello=true},nil,a)
"#,&[],&["resource.state","resource.network"]);
    h.install(vec![app]).unwrap();h.start_all().unwrap();
    assert_eq!(h.state("scoped","verified"),Some(json!(true)));
}

#[test]
fn resources_client_cannot_select_private_event_scope() {
    let temp=Temp::new();let mut h=host(&temp,Side::Client,"scopes");
    let app=installed(&temp,"scoped",r#"
local ok=pcall(function()resource.send('private',{},nil,{kind='instance',id='2'})end)
assert(not ok,'client selected private scope')
"#,&[],&["resource.network"]);
    h.install(vec![app]).unwrap();h.start_all().unwrap();
}

#[test]
fn resources_voice_uses_side_grants_and_generation_owned_results() {
    let temp=Temp::new();let mut server=host(&temp,Side::Server,"voice");
    server.install(vec![installed(&temp,"radio",r#"
resource.on('voice_result',function(result,sender) assert(sender=='0');resource.state.set('voice',result.ok) end)
resource.voice.submit({kind='channel',name='team',members={'18446744073709551615'}})
assert(not pcall(function()resource.voice.submit({kind='configure',muted=false,deafened=false})end))
assert(not pcall(function()resource.voice.submit({kind='proximity',meters=101})end))
assert(not pcall(function()resource.voice.submit({kind='mute',player='01',muted=true})end))
"#,&[],&["resource.voice","resource.events","resource.state"])]).unwrap();
    server.start_all().unwrap();assert_eq!(server.drain_outputs().len(),1);
    let generation=server.generation("radio").unwrap();
    server.host_event("radio",generation,"voice_result",json!({"ok":true})).unwrap();
    assert_eq!(server.state("radio","voice"),Some(json!(true)));
    server.ensure("radio").unwrap();
    assert!(server.host_event("radio",generation,"voice_result",json!({"ok":false})).is_err());
    let mut client=host(&temp,Side::Client,"voice");
    client.install(vec![installed(&temp,"listener",r#"
resource.voice.submit({kind='devices'})
resource.voice.submit({kind='configure',muted=true,deafened=false})
assert(not pcall(function()resource.voice.submit({kind='channel',name='team',members={}})end))
"#,&[],&["engine.voice"])]).unwrap();client.start_all().unwrap();
    assert_eq!(client.drain_commands().len(),2);assert!(client.drain_outputs().is_empty());
    let mut denied=host(&temp,Side::Client,"denied");
    denied.install(vec![installed(&temp,"denied","resource.voice.submit({kind='devices'})",&[],&[])]).unwrap();
    assert!(denied.start_all().is_err());
}

#[test]
fn resources_javascript_voice_reuses_shared_command_validation() {
    let temp=Temp::new();let mut client=host(&temp,Side::Client,"js-voice");
    client.install(vec![installed_javascript(&temp,"radio",r#"
resource.voice.submit({kind:'transmit',pressed:true,channel:'team/radio'});
sdk.submit({kind:'voice',operation:{kind:'devices'}});
let rejected=false;try {resource.voice.submit({kind:'transmit',pressed:true,channel:'too/many/parts'});} catch(e) {rejected=true;}
if(!rejected) throw Error('invalid voice channel accepted');
"#,&[],&["engine.voice"])]).unwrap();
    client.start_all().unwrap();assert_eq!(client.drain_commands().len(),2);
}

#[test]
fn resources_world_and_bulk_transfer_preserve_owner_and_validate_destinations() {
    let temp=Temp::new();let limits=skate_mods::resources::RuntimeLimits{max_payload_bytes:64*1024,max_queued_outputs:1024,..Default::default()};
    let mut server=Host::new_with_limits(Side::Server,temp.0.join("store"),"world-transfer",limits.clone()).unwrap();
    server.install(vec![installed(&temp,"world",r#"
resource.world.command({op='rail_upsert',key='rail',instance='0',points={{0,0,0},{1,0,0}},closed=false})
resource.transfer.start('large','data',string.rep('x',50000),{recipient='18446744073709551615',timeout_ms=10000})
resource.transfer.cancel('large')
assert(not pcall(function()resource.transfer.start('bad','data',{}, {})end))
assert(not pcall(function()resource.transfer.start('bad','data',{}, {recipient='01'})end))
"#,&[],&["resource.world","resource.events"])]).unwrap();
    server.start_all().unwrap();assert_eq!(server.drain_outputs().len(),3);
    let mut client=Host::new_with_limits(Side::Client,temp.0.join("store"),"world-transfer",limits).unwrap();
    client.install(vec![installed_javascript(&temp,"bulk",r#"
resource.transfer.start('large','data','x'.repeat(50000),{});
resource.transfer.cancel('large');
let denied=false;try{resource.transfer.start('bad','data',{}, {recipient:'9'});}catch(e){denied=true;}if(!denied)throw Error('client selected recipient');
"#,&[],&["resource.events"])]).unwrap();client.start_all().unwrap();assert_eq!(client.drain_outputs().len(),2);
}

#[test]
fn resources_animation_commands_are_versioned_client_cosmetics() {
    let temp=Temp::new();let mut client=host(&temp,Side::Client,"animation");
    client.install(vec![installed_javascript(&temp,"actor",r#"
sdk.animation.submit({op:'load',key:'moves',path:'clips.json'});
sdk.animation.submit({op:'play',key:'dance',bank:'moves',clip:'nod',target:'18446744073709551615'});
let denied=false;try{sdk.submit({kind:'animation',version:2,operation:{op:'stop',key:'dance'}});}catch(e){denied=true;}if(!denied)throw Error('unsupported version');
"#,&[],&["engine.animation"])]).unwrap();client.start_all().unwrap();assert_eq!(client.drain_commands().len(),2);
    let mut server=host(&temp,Side::Server,"animation");server.install(vec![installed(&temp,"actor","sdk.animation.submit({op='stop',key='dance'})",&[],&["engine.animation"])]).unwrap();assert!(server.start_all().is_err());
}

#[test]
fn resources_competition_is_server_only_and_has_no_score_submission() {
    let temp=Temp::new();let mut server=host(&temp,Side::Server,"competition");
    server.install(vec![installed(&temp,"race",r#"
resource.competition.submit({kind='start',name='race',player='9'})
assert(not pcall(function()resource.competition.submit({kind='score',points=1000})end))
"#,&[],&["resource.competition"])]).unwrap();server.start_all().unwrap();assert_eq!(server.drain_outputs().len(),1);
    let mut client=host(&temp,Side::Client,"competition");client.install(vec![installed(&temp,"race","resource.competition.submit({kind='start',name='race',player='9'})",&[],&["resource.competition"])]).unwrap();assert!(client.start_all().is_err());
}

#[test]
fn resources_native_competition_routes_lua_and_javascript_and_preserves_authority_guards() {
    let temp = Temp::new();
    for javascript in [false, true] {
        let mut package = if javascript {
            installed_javascript(&temp, "native", r#"
resource.competition.submit({kind:'native_start',player:'9',ticks:300});
resource.competition.submit({kind:'native_cancel',player:'9'});
let denied=false;try{resource.competition.submit({kind:'score',points:1000});}catch(error){denied=true;}
if(!denied)throw Error('client score operation accepted');
"#, &[], &["resource.competition"])
        } else {
            installed(&temp, "native", r#"
resource.competition.submit({kind='native_start',player='9',ticks=300})
resource.competition.submit({kind='native_cancel',player='9'})
assert(not pcall(function()resource.competition.submit({kind='score',points=1000})end))
"#, &[], &["resource.competition"])
        };
        package.generation = 17;
        let mut server = host(&temp, Side::Server, "native-routing");
        server.install(vec![package.clone()]).unwrap();
        server.start_all().unwrap();
        let outputs = server.drain_outputs();
        assert_eq!(outputs.len(), 2);
        for (output, expected) in outputs.iter().zip([
            json!({"kind":"native_start","player":"9","ticks":300}),
            json!({"kind":"native_cancel","player":"9"}),
        ]) {
            let Output::Competition { resource, generation, operation } = output else { panic!("not a competition operation"); };
            assert_eq!(resource, "native");
            assert_eq!(*generation, 17);
            assert_eq!(*operation, expected);
        }
        let mut client = host(&temp, Side::Client, "native-client-denied");
        client.install(vec![package.clone()]).unwrap();
        assert!(client.start_all().is_err());
        assert!(client.drain_outputs().is_empty());
        package.grants.clear();
        let mut ungranted = host(&temp, Side::Server, "native-ungranted");
        ungranted.install(vec![package]).unwrap();
        assert!(ungranted.start_all().is_err());
        assert!(ungranted.drain_outputs().is_empty());
    }
}

#[test]
fn resources_bundled_presentation_replication_replays_authorized_selection_and_retires() {
    let temp=Temp::new();let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/presentation-demo");
    let manifest:Manifest=serde_json::from_slice(&std::fs::read(root.join("resource.json")).unwrap()).unwrap();
    manifest.validate().unwrap();
    let package=InstalledResource{grants:manifest.capabilities.iter().cloned().collect(),manifest,root,generation:1};
    let mut server=host(&temp,Side::Server,"presentation");server.install(vec![package.clone()]).unwrap();server.start_all().unwrap();
    let players=json!({"players":[{"id":"1","instance":"0"},{"id":"2","instance":"0"}]});
    server.tick(0.01,players.clone());server.drain_outputs();
    let mut client=host(&temp,Side::Client,"presentation");client.install(vec![package]).unwrap();client.start_all().unwrap();client.drain_commands();
    for output in client.drain_outputs() {if let Output::Event{resource,generation,name,payload,..}=output {server.receive(1,&resource,generation,&name,payload).unwrap();}}
    server.receive(2,"presentation-demo",1,"choose",json!({"mode":"nod","skin":true,"hat":true})).unwrap();
    // A payload target cannot change the authenticated sender's ownership.
    server.receive(1,"presentation-demo",1,"choose",json!({"mode":"off","target":"2","skin":false,"hat":false})).unwrap();
    server.tick(0.01,players);
    let scope=json!({"kind":"instance","id":"0"});
    let state=server.scoped_state("presentation-demo","actors",&scope).unwrap().unwrap();
    assert_eq!(state["1"]["mode"],"off");assert_eq!(state["2"]["mode"],"nod");
    client.apply_scoped_state("presentation-demo",1,"actors",state,scope.clone()).unwrap();
    client.receive(0,"presentation-demo",1,"room",json!({"id":"0"})).unwrap();client.tick(0.01,json!({}));
    let commands=client.drain_commands();
    assert!(commands.iter().any(|(_,c)|matches!(c,skate_mods::Command::Animation{operation:skate_mods::animation::Operation::Appearance{target,..},..} if target=="2")));
    assert!(!commands.iter().any(|(_,c)|matches!(c,skate_mods::Command::Animation{operation:skate_mods::animation::Operation::Appearance{target,..},..} if target=="1")));
    server.tick(0.01,json!({"players":[{"id":"1","instance":"0"}]}));
    let state=server.scoped_state("presentation-demo","actors",&scope).unwrap().unwrap();
    client.apply_scoped_state("presentation-demo",1,"actors",state,scope).unwrap();client.tick(0.01,json!({}));
    assert!(client.drain_commands().iter().any(|(_,c)|matches!(c,skate_mods::Command::Animation{operation:skate_mods::animation::Operation::Remove{key},..} if key=="2_skin")));
}

#[test]
fn resources_bundled_park_assigns_64_distinct_trusted_pads_once_and_reuses_departures() {
    let temp=Temp::new();
    let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/community-park");
    let manifest:Manifest=serde_json::from_slice(&std::fs::read(root.join("resource.json")).unwrap()).unwrap();
    manifest.validate().unwrap();
    let package=InstalledResource{grants:manifest.capabilities.iter().cloned().collect(),manifest,root,generation:1};
    let mut server=host(&temp,Side::Server,"park-pads");
    server.install(vec![package]).unwrap();server.start_all().unwrap();
    let mut players:Vec<_>=(1..=64).map(|id|json!({"id":id.to_string(),"instance":"0","position":[1000,1,1000]})).collect();
    server.tick(0.01,json!({"players":players}));
    server.call("community-park","on_fixed_update",json!({"dt":0.01}));
    assert!(server.running("community-park"),"{:?}",server.diagnostics);
    let pads:std::collections::BTreeMap<_,_>=server.drain_outputs().into_iter().filter_map(|output|match output {
        Output::Teleport{resource,generation,player,position,instance,velocity,..}=>{
            assert_eq!(resource,"community-park");assert_eq!(generation,1);
            assert_eq!(instance,Some(0));assert_eq!(velocity,Some([0.;3]));Some((player,position))
        },_=>None,
    }).collect();
    assert_eq!(pads.len(),64);
    let distinct:BTreeSet<_>=pads.values().map(|p|p.map(f32::to_bits)).collect();assert_eq!(distinct.len(),64);
    for _ in 0..3 {server.call("community-park","on_fixed_update",json!({"dt":0.01}));}
    assert!(!server.drain_outputs().iter().any(|out|matches!(out,Output::Teleport{..})));
    players.remove(0);
    server.tick(0.01,json!({"players":players}));server.call("community-park","on_fixed_update",json!({"dt":0.01}));server.drain_outputs();
    players.push(json!({"id":"65","instance":"0","position":[1000,1,1000]}));
    server.tick(0.01,json!({"players":players}));server.call("community-park","on_fixed_update",json!({"dt":0.01}));
    let outputs=server.drain_outputs();
    assert_eq!(outputs.iter().filter(|out|matches!(out,Output::Teleport{..})).count(),1);
    assert!(outputs.iter().any(|out|matches!(out,Output::Teleport{player:65,position,..} if position==&pads[&1])));
    assert!(server.running("community-park"),"{:?}",server.diagnostics);

    // Dedicated travel briefly removes players from admitted observations.
    // Neither returning nor first-observed private actors belong on park pads.
    server.tick(0.01,json!({"players":[]}));
    server.call("community-park","on_fixed_update",json!({"dt":0.01}));
    server.drain_outputs();
    let mut private_teleports=Vec::new();
    players=vec![json!({"id":"1","instance":"7","position":[1000,1,1000]})];
    for new_private in [None,Some("66")] {
        if let Some(id)=new_private {players.push(json!({"id":id,"instance":"7","position":[1000,1,1000]}));}
        server.tick(0.01,json!({"players":players}));
        server.call("community-park","on_fixed_update",json!({"dt":0.01}));
        private_teleports.extend(server.drain_outputs().into_iter().filter_map(|output|match output {
            Output::Teleport{player,..}=>Some(player),_=>None,
        }));
    }
    assert!(server.running("community-park"),"{:?}",server.diagnostics);
    assert!(private_teleports.is_empty(),"private-instance actors were moved into the park: {private_teleports:?}");
    players[1]["instance"]=json!("0");
    server.tick(0.01,json!({"players":players}));
    server.call("community-park","on_fixed_update",json!({"dt":0.01}));
    let public_teleports:Vec<_>=server.drain_outputs().into_iter().filter_map(|output|match output {
        Output::Teleport{player,position,..}=>Some((player,position)),_=>None,
    }).collect();
    assert_eq!(public_teleports,vec![(66,pads[&1])],"public admission should claim the first free pad");
}

#[test]
fn resources_scoped_state_is_bounded_and_pruned_on_visibility_change() {
    let temp=Temp::new();
    let limits=skate_mods::resources::RuntimeLimits {max_state_keys:2,..Default::default()};
    let mut h=Host::new_with_limits(Side::Client,temp.0.join("store"),"scopes",limits).unwrap();
    h.install(vec![installed(&temp,"scope","return {}",&[],&[])]).unwrap();h.start_all().unwrap();
    let generation=h.generation("scope").unwrap();
    let a=json!({"kind":"instance","id":"0"});let b=json!({"kind":"player","id":"9"});
    h.apply_scoped_state("scope",generation,"same",json!(1),a.clone()).unwrap();
    h.apply_scoped_state("scope",generation,"same",json!(2),b.clone()).unwrap();
    assert!(h.apply_state("scope",generation,"third",json!(3)).is_err());
    assert_eq!(h.scoped_state("scope","same",&a).unwrap(),Some(json!(1)));
    assert!(h.apply_scoped_state("scope",generation,"same",json!(0),json!({"kind":"player","id":"0"})).is_err());
    assert!(h.apply_scoped_state("scope",generation,"same",json!(0),json!({"kind":"resource","id":"1"})).is_err());
    h.retain_scoped_state(&[b.clone()]);assert_eq!(h.scoped_state("scope","same",&a).unwrap(),None);
    assert_eq!(h.scoped_state("scope","same",&b).unwrap(),Some(json!(2)));
    h.apply_state("scope",generation,"global",json!(3)).unwrap();h.retain_scoped_state(&[]);
    assert_eq!(h.state("scope","global"),Some(json!(3)));assert_eq!(h.scoped_states().len(),1);
    h.stop("scope").unwrap();assert!(h.scoped_states().is_empty());
}

#[test]
fn resources_javascript_scopes_preserve_metadata_and_entity_observations() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"js-scopes");
    h.set_snapshot(std::sync::Arc::new(json!({"entities":[{"id":"18446744073709551615"}]})),Default::default());
    let app=installed_javascript(&temp,"scoped",r#"
const scope={kind:'entity',id:'18446744073709551615'};
resource.state.set('secret',{value:7},scope);
if(resource.state.get('secret')!==null||resource.state.get('secret',scope).value!==7)throw Error('scope mixed');
if(resource.entities.all()[0].id!==scope.id)throw Error('entity identity lost');
resource.send('private',{hello:true},null,scope);
"#,&[],&["resource.state","resource.network","resource.entities"]);
    h.install(vec![app]).unwrap();h.start_all().unwrap();
    let scope=json!({"kind":"entity","id":"18446744073709551615"});
    assert!(h.drain_outputs().iter().any(|o|matches!(o,Output::Event{scope:s,..} if s==&scope)));
}

#[test]
fn resources_host_completion_and_private_target_retirement_are_generation_safe() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"targets");
    let app=installed(&temp,"scope",r#"
resource.on('voice_result',function(value,sender)assert(sender=='0');resource.state.set('completed',value)end)
resource.state.set('a',1,{kind='player',id='9'})
resource.state.set('b',2,{kind='entity',id='4'})
resource.state.set('c',3,{kind='instance',id='0'})
"#,&[],&["resource.events","resource.state"]);
    h.install(vec![app]).unwrap();h.start_all().unwrap();let generation=h.generation("scope").unwrap();
    assert!(h.host_event("scope",generation+1,"voice_result",json!(true)).is_err());
    h.host_event("scope",generation,"voice_result",json!(true)).unwrap();assert_eq!(h.state("scope","completed"),Some(json!(true)));
    h.prune_scoped_targets(&[9],&[("scope".into(),4,generation)]);assert_eq!(h.scoped_states().len(),4);
    h.prune_scoped_targets(&[],&[("scope".into(),4,generation+1)]);assert_eq!(h.scoped_states().len(),2);
    assert!(!h.drain_outputs().iter().any(|o|matches!(o,Output::State{scope,..} if scope["kind"]=="player"||scope["kind"]=="entity")));
    h.prune_resource_scope("scope",&json!({"kind":"instance","id":"0"})).unwrap();assert_eq!(h.scoped_states().len(),1);
}

fn with_settings(mut resource: InstalledResource) -> InstalledResource {
    resource.manifest.settings=serde_json::from_value(json!({
        "round":{"type":"integer","default":30,"min":10,"max":120,"visibility":"replicated","change":"live"},
        "map":{"type":"enum","default":"park","options":["park","street"],"visibility":"public","change":"restart"},
        "private_note":{"type":"string","default":"private-default","max_bytes":32,"visibility":"private"}
    })).unwrap();
    resource
}
#[test]
fn resource_settings_live_restart_persistence_and_private_projection() {
    use skate_mods::resources::SettingAudience;
    let temp=Temp::new();
    let app=with_settings(installed(&temp,"settings",r#"
        assert(resource.settings.get('round')>=10)
        return {on_load=function()resource.state.set('loaded',resource.settings.all())end,
        on_settings=function(change)resource.state.set('changed',change);resource.state.set('current',resource.settings.all())end}
    "#,&[],&["resource.settings","resource.state"]));
    let mut h=host(&temp,Side::Server,"settings");h.install(vec![app.clone()]).unwrap();
    h.configure_settings("settings",[("round".into(),json!(45))].into()).unwrap();h.start_all().unwrap();
    assert_eq!(h.state("settings","loaded").unwrap()["round"],45);
    assert!(h.set_setting("settings","round",json!(121)).is_err());
    assert!(h.set_setting("settings","round",json!(true)).is_err());
    assert!(h.set_setting("settings","undeclared",json!(1)).is_err());
    assert!(h.set_setting("settings","round",json!(60)).unwrap().applied);
    assert_eq!(h.state("settings","changed").unwrap(),json!({"key":"round","value":60}));
    assert!(h.set_setting("settings","map",json!("street")).unwrap().restart_required);
    assert_eq!(h.settings_values("settings",SettingAudience::Public).unwrap(),[("map".into(),json!("park"))].into());
    assert_eq!(h.settings_snapshot("settings").unwrap()["map"].pending,Some(json!("street")));
    assert!(!h.settings_values("settings",SettingAudience::Client).unwrap().contains_key("private_note"));
    h.restart("settings").unwrap();assert_eq!(h.state("settings","loaded").unwrap()["map"],"street");
    h.disconnect();drop(h);
    let mut h=host(&temp,Side::Server,"settings");h.install(vec![app]).unwrap();
    h.configure_settings("settings",[("round".into(),json!(40))].into()).unwrap();h.start_all().unwrap();
    assert_eq!(h.state("settings","loaded").unwrap()["round"],60,"persisted override wins startup defaults");
    let mut client_app=h.installed()["settings"].clone();client_app.manifest=client_app.manifest.client_projection();
    std::fs::write(client_app.root.join("main.lua"),r#"return {on_load=function()assert(resource.settings.get('round')==75);assert(resource.settings.all().private_note==nil)end}"#).unwrap();
    let mut client=host(&temp,Side::Client,"settings");client.install(vec![client_app]).unwrap();
    let generation=client.generation("settings").unwrap();
    assert!(client.set_setting("settings","round",json!(75)).is_err());
    assert!(client.apply_settings("settings",generation+1,[("round".into(),json!(75)),("map".into(),json!("park"))].into()).is_err());
    assert!(client.apply_settings("settings",generation,[("round".into(),json!(75)),("private_note".into(),json!("leak"))].into()).is_err());
    client.apply_state("settings",generation,"__settings",json!({"round":75,"map":"park"})).unwrap();
    client.start_all().unwrap();
}
#[test]
fn resource_settings_javascript_and_reserved_host_state() {
    let temp=Temp::new();
    let mut app=with_settings(installed(&temp,"settings_js","",&[],&["resource.settings","resource.state"]));
    app.manifest.language="javascript".into();app.manifest.shared_scripts=vec!["main.js".into()];
    std::fs::write(app.root.join("main.js"),r#"
      resource.lifecycle({on_load(){resource.state.set('round',resource.settings.get('round'));},on_settings(c){resource.state.set('changed',c.value);}});
      let denied=false;try{resource.state.set('__settings',{round:120});}catch(e){denied=true;}if(!denied)throw Error('reserved state write accepted');
    "#).unwrap();
    let mut h=host(&temp,Side::Server,"settings-js");h.install(vec![app]).unwrap();h.start_all().unwrap();
    assert_eq!(h.state("settings_js","round"),Some(json!(30)));
    h.set_setting("settings_js","round",json!(99)).unwrap();assert_eq!(h.state("settings_js","changed"),Some(json!(99)));
}

#[test]
fn resource_profile_records_nested_calls_queue_sources_and_generations_with_bounds() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"profile");
    let dep=installed(&temp,"callee",r#"resource.export('value',function(n)local a=0;for i=1,1000 do a=a+i end;return n+a end)"#,&[],&["resource.exports"]);
    let app=installed(&temp,"caller",r#"resource.on('later',function()resource.call('callee','value',1)end);return{on_update=function()resource.emit('later',{token='synthetic-private-event-value'})end}"#,&["callee"],&["resource.events","resource.exports"]);
    h.install(vec![dep,app]).unwrap();h.start_all().unwrap();h.tick(0.01,json!({}));
    let profile=h.profile_snapshot();
    let event=profile.spans.iter().find(|s|s.phase=="event:later").unwrap();
    assert!(event.queue_wait_us.is_some());assert!(event.source.as_ref().unwrap().contains("caller/main.lua"));
    let exported=profile.spans.iter().find(|s|s.phase=="export:value").unwrap();assert_eq!(exported.resource,"callee");assert_eq!(exported.parent,Some(event.id));
    assert!(profile.spans.iter().any(|s|s.phase=="dispatch:on_update"&&s.resource=="@host"));
    assert!(profile.spans.iter().all(|s|s.exclusive_host_cpu_time_us.zip(s.host_cpu_time_us).is_none_or(|(exclusive,inclusive)|exclusive<=inclusive)));
    assert!(profile.summaries.iter().all(|s|s.p50_wall_time_us<=s.p95_wall_time_us&&s.p95_wall_time_us<=s.p99_wall_time_us));
    h.restart("callee").unwrap();h.tick(0.01,json!({}));
    let profile=h.profile_snapshot();assert!(profile.spans.iter().any(|s|s.resource=="callee"&&s.generation==1));assert!(profile.spans.iter().any(|s|s.resource=="callee"&&s.generation==2));
    h.configure_profiling(true,64,60_000).unwrap();for _ in 0..100 {h.tick(0.01,json!({}));}
    let profile=h.profile_snapshot();assert_eq!(profile.spans.len(),64);assert!(profile.evicted>0);
    let trace=profile.chrome_trace();assert_eq!(trace["traceEvents"].as_array().unwrap().len(),64);assert!(!trace.to_string().contains("payload"));assert!(!trace.to_string().contains("synthetic-private-event-value"));
    h.configure_profiling(false,64,60_000).unwrap();let before=h.profile_snapshot().spans.len();h.tick(0.01,json!({}));assert_eq!(h.profile_snapshot().spans.len(),before);
}

#[test]
#[ignore = "measurement workload; run explicitly with --nocapture --test-threads=1"]
fn resource_profile_instrumentation_overhead_measurement() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"profile-overhead");
    let mut resources=Vec::new();
    for i in 0..8 {let mut app=installed(&temp,&format!("benchmark_{i}"),"return{on_update=function()local total=0;for i=1,100 do total=total+i end end}",&[],&[]);
        if i%2==1 {app.manifest.language="javascript".into();app.manifest.shared_scripts=vec!["main.js".into()];std::fs::write(app.root.join("main.js"),"resource.lifecycle({on_update(){let total=0;for(let i=1;i<=100;i++)total+=i;}})").unwrap();}resources.push(app);}
    h.install(resources).unwrap();h.start_all().unwrap();
    for _ in 0..1000 {h.tick(0.01,json!({}));}
    let mut off=Vec::new();let mut on=Vec::new();
    for round in 0..6 {let enabled=round%2!=0;h.configure_profiling(enabled,4096,60_000).unwrap();let start=std::time::Instant::now();for _ in 0..5000 {h.tick(0.01,json!({}));}let ns=start.elapsed().as_nanos() as f64/5000.;if enabled{on.push(ns)}else{off.push(ns)}}
    off.sort_by(f64::total_cmp);on.sort_by(f64::total_cmp);
    eprintln!("PROFILE_OVERHEAD resources=8 (4 Lua/4 JS) callbacks=240000 off_median_ns_per_dispatch={:.0} on_median_ns_per_dispatch={:.0} added_ns_per_resource={:.0} ratio={:.3} retained={} capacity=4096",off[1],on[1],(on[1]-off[1])/8.,on[1]/off[1],h.profile_snapshot().spans.len());
    assert!(h.running_ids().len()==8);assert_eq!(h.profile_snapshot().spans.len(),4096);
}

#[test]
fn resource_settings_failure_preserves_durable_update_and_grants_are_enforced() {
    let temp=Temp::new();let mut h=host(&temp,Side::Server,"settings-failure");
    let mut denied=with_settings(installed(&temp,"denied","resource.settings.all()",&[],&["resource.settings"]));denied.grants.clear();h.install(vec![denied]).unwrap();assert!(h.start_all().unwrap_err().contains("capability"));
    let app=with_settings(installed(&temp,"notify_fail","return {on_settings=function()error('callback rejected')end}",&[],&["resource.settings"]));h.install(vec![app.clone()]).unwrap();h.start_all().unwrap();
    let result=h.set_setting("notify_fail","round",json!(77)).unwrap();assert!(result.applied);assert!(result.notification_error.is_some());assert!(!h.running("notify_fail"));
    let mut restart=host(&temp,Side::Server,"settings-failure");restart.install(vec![app]).unwrap();assert_eq!(restart.settings_snapshot("notify_fail").unwrap()["round"].value,json!(77));
}
