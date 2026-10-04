//! Transport-independent, capability-granted Lua resources. The host supplies
//! connection identities; scripts can neither select their owner nor generation.
use crate::{Command, SnapshotFields, vm::Vm};
pub use crate::runtime_metrics::RuntimeMetrics;
pub use crate::runtime_profile::{ProfileSnapshot,ProfileSpan,ProfileSummary,ProfileScope};
#[path="resource_settings.rs"]
mod settings;
pub use settings::{SettingAudience, SettingStatus, SettingsChange};
use crate::runtime_metrics::{Counters, Timer, bounded_error};
use mlua::{Function, Lua, LuaSerdeExt, Table};
use serde_json::Value;
use skate_resources::Manifest;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

pub const MAX_PAYLOAD: usize = 16 * 1024;
fn default_transfer_timeout()->u64 {10_000}

/// Host-selected limits. Peers never choose the limits of a local VM.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeLimits {
    pub max_resources: usize,
    pub max_payload_bytes: usize,
    pub max_state_keys: usize,
    pub max_storage_bytes: usize,
    pub max_storage_value_bytes: usize,
    pub max_storage_keys: usize,
    pub max_actions: usize,
    pub max_callbacks: usize,
    pub max_queued_events: usize,
    pub max_queued_outputs: usize,
    pub max_queued_output_bytes: usize,
    pub max_threads: usize,
    pub lua_memory_bytes: usize,
    pub instruction_budget_units: usize,
    pub managed_memory_bytes: usize,
    pub managed_callback_timeout_ms: usize,
    pub managed_startup_timeout_ms: usize,
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self { max_resources: 128, max_payload_bytes: MAX_PAYLOAD, max_state_keys: 1024,
            max_storage_bytes: 4 * 1024 * 1024, max_storage_value_bytes: 256 * 1024,
            max_storage_keys: 1024, max_actions: 128, max_callbacks: 64,
            max_queued_events: 256, max_queued_outputs: 4096, max_queued_output_bytes: 16 * 1024 * 1024, max_threads: 64,
            lua_memory_bytes: 16 * 1024 * 1024, instruction_budget_units: crate::vm::LUA_BUDGET_UNITS, managed_memory_bytes: 256 * 1024 * 1024,
            managed_callback_timeout_ms: 100, managed_startup_timeout_ms: 10_000 }
    }
}
impl RuntimeLimits {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value, maximum) in [
            ("resources", self.max_resources, 1024),
            ("payload bytes", self.max_payload_bytes, 256 * 1024),
            ("state keys", self.max_state_keys, 4096),
            ("storage bytes", self.max_storage_bytes, 64 * 1024 * 1024),
            ("storage value bytes", self.max_storage_value_bytes, 4 * 1024 * 1024),
            ("storage keys", self.max_storage_keys, 16384),
            ("actions", self.max_actions, 4096),
            ("callbacks", self.max_callbacks, 1024),
            ("queued events", self.max_queued_events, 8192),
            ("queued outputs", self.max_queued_outputs, 8192),
            ("queued output bytes", self.max_queued_output_bytes, 64 * 1024 * 1024),
            ("threads", self.max_threads, 1024),
            ("Lua memory bytes", self.lua_memory_bytes, 128 * 1024 * 1024),
            ("instruction budget units", self.instruction_budget_units, 10000),
            ("managed memory bytes", self.managed_memory_bytes, 1024 * 1024 * 1024),
            ("managed callback timeout milliseconds", self.managed_callback_timeout_ms, 1000),
            ("managed startup timeout milliseconds", self.managed_startup_timeout_ms, 30_000),
        ] {
            if value == 0 || value > maximum { return Err(format!("{name} must be in 1..={maximum}")); }
        }
        if self.managed_memory_bytes < 64 * 1024 * 1024 { return Err("managed memory limit must be at least 64 MiB".into()); }
        if self.lua_memory_bytes < 1024 * 1024 || self.instruction_budget_units < 64 {
            return Err("Lua memory must be at least 1 MiB and instruction budget at least 64 units".into());
        }
        if self.max_storage_value_bytes > self.max_storage_bytes {
            return Err("storage value limit exceeds storage limit".into());
        }
        for count in [self.max_state_keys, self.max_queued_events, self.max_queued_outputs] {
            if count.saturating_mul(self.max_payload_bytes) > 64 * 1024 * 1024 {
                return Err("state/event/output allocation ceiling exceeds 64 MiB; reduce count or payload size".into());
            }
        }
        Ok(())
    }
}

fn resource_scope()->Value {serde_json::json!({"kind":"resource"})}
fn normalize_scope(value:Value)->Result<(String,Value),String> {
    #[derive(serde::Deserialize)]
    #[serde(tag="kind",rename_all="lowercase",deny_unknown_fields)]
    enum Scope {Resource,Instance{id:String},Player{id:String},Entity{id:String}}
    let scope:Scope=serde_json::from_value(if value.is_null(){resource_scope()}else{value}).map_err(|_|"invalid resource visibility scope".to_string())?;
    let (kind,id,zero)=match scope {Scope::Resource=>return Ok(("resource".into(),resource_scope())),
        Scope::Instance{id}=>("instance",id,true),Scope::Player{id}=>("player",id,false),Scope::Entity{id}=>("entity",id,false)};
    let number=id.parse::<u64>().map_err(|_|"scope ID must be a canonical decimal string")?;
    if id!=number.to_string()||(!zero&&number==0) {return Err("scope ID must be a canonical decimal string".into());}
    Ok((format!("{kind}:{id}"),serde_json::json!({"kind":kind,"id":id})))
}
fn scope_from_key(key:&str)->Value {
    if let Some((kind,id))=key.split_once(':') {serde_json::json!({"kind":kind,"id":id})} else {resource_scope()}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Client,
    Server,
}
#[derive(Clone, Debug)]
pub struct InstalledResource {
    pub manifest: Manifest,
    pub root: PathBuf,
    pub generation: u64,
    /// Explicit host grants. A requested capability alone grants nothing.
    pub grants: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    Event {
        resource: String,
        generation: u64,
        recipient: Option<u64>,
        name: String,
        payload: Value,
        scope: Value,
    },
    State {
        resource: String,
        generation: u64,
        key: String,
        value: Value,
        scope: Value,
    },
    Service {
        resource: String, generation: u64, key: String, operation: Value, timeout_ms: u64,
    },
    CancelService { resource: String, generation: u64, key: String },
    Entity { resource: String, generation: u64, command: Value },
    Voice { resource: String, generation: u64, operation: Value },
    World { resource: String, generation: u64, operation: Value },
    Competition { resource: String, generation: u64, operation: Value },
    Transfer { resource: String, generation: u64, key: String, name: String, payload: Value, recipient: Option<u64>, timeout_ms: u64 },
    CancelTransfer { resource: String, generation: u64, key: String },
    Teleport {
        resource: String,
        generation: u64,
        player: u64,
        position: [f32; 3],
        heading: Option<f32>,
        velocity: Option<[f32; 3]>,
        instance: Option<u32>,
        restore_on_stop: bool,
        restore_previous: bool,
    },
    Log {
        resource: String,
        text: String,
    },
}
#[derive(Clone)]
struct Handler {
    function: Function,
    network: bool,
}
#[derive(Clone)]
struct Export {
    lua: Lua,
    function: Function,
    budget: Arc<AtomicUsize>,
    budget_units: usize,
    metrics: Counters,
}
#[derive(Clone)]
struct RegisteredCommand {
    owner: String,
    permission: String,
    function: Function,
    lua: Lua,
}
#[derive(Default)]
struct Shared {
    active: BTreeSet<String>,
    outputs: Vec<Output>,
    output_bytes: usize,
    events: VecDeque<(String, u64, String, Value, std::time::Instant)>,
    states: BTreeMap<(String, String), BTreeMap<String, Value>>,
    settings: BTreeMap<String, BTreeMap<String, Value>>,
    exports: BTreeMap<(String, String), Export>,
    commands: BTreeMap<String, RegisteredCommand>,
    export_depth: usize,
    faults: Vec<(String, String)>,
}
impl Shared {
    fn output_size(output: &Output) -> usize {
        256 + match output {
            Output::Event {payload,scope,..} => serde_json::to_vec(payload).map_or(usize::MAX-512,|b|b.len()) + serde_json::to_vec(scope).map_or(256,|b|b.len()),
            Output::State {value,scope,..} => serde_json::to_vec(value).map_or(usize::MAX-512,|b|b.len()) + serde_json::to_vec(scope).map_or(256,|b|b.len()),
            Output::Entity {command:operation,..} | Output::Service {operation,..} | Output::Voice {operation,..} | Output::World {operation,..} | Output::Competition {operation,..} | Output::Transfer {payload:operation,..} => serde_json::to_vec(operation).map_or(usize::MAX-256,|b|b.len()),
            Output::Log {text,..} => text.len(),
            Output::Teleport {..} | Output::CancelService {..} | Output::CancelTransfer {..} => 0,
        }
    }
    fn set_state(&mut self, resource:&str, scope:&str, key:&str, value:Value, limit:usize)->Result<(),String> {
        let address=(resource.to_owned(),scope.to_owned());
        if value.is_null() {
            if let Some(state)=self.states.get_mut(&address) {state.remove(key);if state.is_empty(){self.states.remove(&address);}}
            return Ok(());
        }
        let exists=self.states.get(&address).is_some_and(|state|state.contains_key(key));
        let count=self.states.iter().filter(|((owner,_),_)|owner==resource).map(|(_,state)|state.len()).sum::<usize>();
        if !exists&&count>=limit {return Err("state key limit reached".into());}
        self.states.entry(address).or_default().insert(key.to_owned(),value);Ok(())
    }
    fn push_output(&mut self, output: Output, limits: &RuntimeLimits) -> mlua::Result<()> {
        let bytes = Self::output_size(&output);
        if self.outputs.len() >= limits.max_queued_outputs || self.output_bytes.saturating_add(bytes) > limits.max_queued_output_bytes {
            return Err(lua_error("resource output queue full"));
        }
        self.output_bytes += bytes;
        self.outputs.push(output);
        Ok(())
    }
}
#[derive(Clone)]
pub(crate) struct Bootstrap {
    installed: InstalledResource,
    side: Side,
    storage: PathBuf,
    shared: Arc<Mutex<Shared>>,
    handlers: Arc<Mutex<BTreeMap<String, Vec<Handler>>>>,
    actions: Arc<AtomicUsize>,
    pub(crate) limits: RuntimeLimits,
    pub(crate) metrics: Counters,
}
impl Bootstrap {
    pub(crate) fn permits(&self, command: &Command) -> Result<(), String> {
        if let Command::Request { command, .. } = command {
            return self.permits(command);
        }
        if matches!(command, Command::Log { .. }) {
            return Ok(());
        }
        if self.side == Side::Server {
            return Err("client engine commands are unavailable on the server".into());
        }
        let kind = crate::vm::command_kind(command);
        let capability = if kind.starts_with("session_") || kind == "network_state" {
            return Err(
                "resource sessions use permanent server authority and resource networking".into(),
            );
        } else if kind.starts_with("physics_") {
            "engine.physics"
        } else if kind.starts_with("graphics_") {
            "engine.graphics"
        } else if kind.starts_with("camera_") {
            "engine.camera"
        } else if kind.starts_with("audio_") {
            "engine.audio"
        } else if kind == "voice" {
            "engine.voice"
        } else if kind.starts_with("ui_") || matches!(kind, "overlay" | "multiplayer_debug") {
            "engine.ui"
        } else if kind.starts_with("player_") || matches!(kind, "rig_part" | "native_impulse") {
            "engine.player"
        } else if kind.starts_with("volume_") {
            "engine.world"
        } else if kind == "input_override" {
            "engine.input"
        } else if matches!(kind,"graph_gate"|"animation") {
            "engine.animation"
        } else if kind == "engine_inspect" {
            "engine.inspect"
        } else {
            return Err(format!("unclassified resource engine command {kind}"));
        };
        self.require(capability).map_err(|e| e.to_string())
    }
    fn require(&self, cap: &str) -> mlua::Result<()> {
        if self.installed.grants.contains(cap)
            && self
                .installed
                .manifest
                .capabilities
                .iter()
                .any(|c| c == cap)
        {
            Ok(())
        } else {
            Err(lua_error(format!(
                "resource {} lacks capability grant {cap}",
                self.installed.manifest.id
            )))
        }
    }
    fn action(&self) -> mlua::Result<()> {
        if self.actions.fetch_add(1, Ordering::Relaxed) >= self.limits.max_actions {
            Err(lua_error(format!("{} resource operations per callback maximum", self.limits.max_actions)))
        } else {
            Ok(())
        }
    }
    pub(crate) fn native_query(&self, capability: &str) -> mlua::Result<()> {
        if self.side != Side::Client {
            return Err(lua_error(
                "client engine queries are unavailable on the server",
            ));
        }
        self.require(capability)?;
        self.action()
    }
    pub(crate) fn reset_budget(&self) {
        self.actions.store(0, Ordering::Relaxed);
    }
    pub(crate) fn language(&self) -> &str { &self.installed.manifest.language }
    pub(crate) fn scripts(&self) -> Vec<String> {
        self.installed
            .manifest
            .shared_scripts
            .iter()
            .chain(if self.side == Side::Server {
                &self.installed.manifest.server_scripts
            } else {
                &self.installed.manifest.client_scripts
            })
            .cloned()
            .collect()
    }
    pub(crate) fn install(
        &self,
        lua: &Lua,
        sdk: &Table,
        budget: Arc<AtomicUsize>,
    ) -> mlua::Result<()> {
        let api = lua.create_table()?;
        api.set("api_version", 1)?;
        api.set("id", self.installed.manifest.id.clone())?;
        api.set("version", self.installed.manifest.version.clone())?;
        api.set(
            "side",
            if self.side == Side::Server {
                "server"
            } else {
                "client"
            },
        )?;
        api.set("generation", self.installed.generation.to_string())?;
        let grants = lua.create_table()?;
        for cap in &self.installed.grants {
            if self.installed.manifest.capabilities.contains(cap) {
                grants.set(cap.clone(), true)?;
            }
        }
        api.set("grants", grants.clone())?;
        sdk.set("grants", grants)?;
        for (name, network) in [("on", false), ("on_net", true)] {
            let ctx = self.clone();
            api.set(
                name,
                lua.create_function(move |_, (name, function): (String, Function)| {
                    ctx.require(if network {
                        "resource.network"
                    } else {
                        "resource.events"
                    })?;
                    valid_name(&name).map_err(lua_error)?;
                    ctx.action()?;
                    let mut handlers = ctx.handlers.lock().unwrap();
                    if handlers.values().map(Vec::len).sum::<usize>() >= ctx.limits.max_callbacks {
                        return Err(lua_error(format!("{} event handlers maximum",ctx.limits.max_callbacks)));
                    }
                    handlers
                        .entry(name)
                        .or_default()
                        .push(Handler { function, network });
                    Ok(())
                })?,
            )?;
        }
        let ctx = self.clone();
        api.set(
            "off",
            lua.create_function(move |_, name: String| {
                ctx.action()?;
                ctx.handlers.lock().unwrap().remove(&name);
                Ok(())
            })?,
        )?;
        let ctx = self.clone();
        api.set(
            "emit",
            lua.create_function(move |lua, (name, payload): (String, mlua::Value)| {
                ctx.require("resource.events")?;
                ctx.action()?;
                valid_name(&name).map_err(lua_error)?;
                let payload = bounded_value(lua, payload, ctx.limits.max_payload_bytes)?;
                let mut shared = ctx.shared.lock().unwrap();
                if shared.events.len() >= ctx.limits.max_queued_events {
                    return Err(lua_error("local event queue full"));
                }
                shared.events.push_back((
                    ctx.installed.manifest.id.clone(),
                    ctx.installed.generation,
                    name,
                    payload,
                    std::time::Instant::now(),
                ));
                Ok(())
            })?,
        )?;
        let ctx = self.clone();
        api.set(
            "send",
            lua.create_function(
                move |lua, (name, payload, recipient, scope): (String, mlua::Value, mlua::Value, mlua::Value)| {
                    ctx.require("resource.network")?;
                    ctx.action()?;
                    valid_name(&name).map_err(lua_error)?;
                    let recipient = match recipient {
                        mlua::Value::Nil=>None,
                        mlua::Value::String(text)=>{
                            let text=text.to_str()?;
                            let id=text.parse::<u64>().map_err(|_|lua_error("recipient must be a canonical decimal player ID string"))?;
                            if id==0 || id.to_string()!=text.as_ref() {return Err(lua_error("recipient must be a canonical nonzero decimal player ID string"));}
                            Some(id)
                        },
                        _=>return Err(lua_error("recipient must be a canonical decimal player ID string")),
                    };
                    if ctx.side == Side::Client && recipient.is_some() {
                        return Err(lua_error("clients can send only to the server"));
                    }
                    let (_,scope)=normalize_scope(bounded_value(lua,scope,128)?).map_err(lua_error)?;
                    if ctx.side==Side::Client&&scope!=resource_scope() {return Err(lua_error("clients cannot select a private event scope"));}
                    let payload = bounded_value(lua, payload, ctx.limits.max_payload_bytes)?;
                    let mut shared = ctx.shared.lock().unwrap();
                    if shared.outputs.len() >= ctx.limits.max_queued_outputs {
                        return Err(lua_error("resource output queue full"));
                    }
                    shared.push_output(Output::Event {
                        resource: ctx.installed.manifest.id.clone(),
                        generation: ctx.installed.generation,
                        recipient,
                        name,
                        payload,
                        scope,
                    }, &ctx.limits)?;
                    Ok(())
                },
            )?,
        )?;
        let state = lua.create_table()?;
        let ctx = self.clone();
        state.set("get",lua.create_function(move |lua,(key,scope):(String,mlua::Value)| {
            ctx.require("resource.state")?;valid_name(&key).map_err(lua_error)?;
            let (scope,_)=normalize_scope(bounded_value(lua,scope,128)?).map_err(lua_error)?;
            let value=ctx.shared.lock().unwrap().states.get(&(ctx.installed.manifest.id.clone(),scope))
                .and_then(|state|state.get(&key)).cloned().unwrap_or(Value::Null);
            json_value(lua,&value)
        })?)?;
        let ctx=self.clone();
        state.set("set",lua.create_function(move |lua,(key,value,scope):(String,mlua::Value,mlua::Value)| {
            if ctx.side!=Side::Server {return Err(lua_error("replicated state is owned by the server"));}
            ctx.require("resource.state")?;ctx.action()?;valid_name(&key).map_err(lua_error)?;
            if key=="__settings" {return Err(lua_error("reserved host settings state key"));}
            let (scope_key,scope)=normalize_scope(bounded_value(lua,scope,128)?).map_err(lua_error)?;
            let value=bounded_value(lua,value,ctx.limits.max_payload_bytes)?;
            let output=Output::State{resource:ctx.installed.manifest.id.clone(),generation:ctx.installed.generation,key:key.clone(),value:value.clone(),scope};
            let mut shared=ctx.shared.lock().unwrap();
            if shared.outputs.len()>=ctx.limits.max_queued_outputs||shared.output_bytes.saturating_add(Shared::output_size(&output))>ctx.limits.max_queued_output_bytes {
                return Err(lua_error("resource output queue full"));
            }
            shared.set_state(&ctx.installed.manifest.id,&scope_key,&key,value,ctx.limits.max_state_keys).map_err(lua_error)?;
            shared.push_output(output,&ctx.limits)
        })?)?;
        api.set("state", state)?;
        self.install_storage(lua, &api)?;
        let settings=lua.create_table()?;
        let ctx=self.clone();
        settings.set("get",lua.create_function(move |lua,key:String| {
            ctx.require("resource.settings")?;
            if !ctx.installed.manifest.settings.contains_key(&key) {return Err(lua_error("unknown resource setting"));}
            let value=ctx.shared.lock().unwrap().settings.get(&ctx.installed.manifest.id).and_then(|v|v.get(&key)).cloned().unwrap_or(Value::Null);
            json_value(lua,&value)
        })?)?;
        let ctx=self.clone();
        settings.set("all",lua.create_function(move |lua,()| {
            ctx.require("resource.settings")?;
            let values=ctx.shared.lock().unwrap().settings.get(&ctx.installed.manifest.id).cloned().unwrap_or_default();
            json_value(lua,&serde_json::to_value(values).map_err(mlua::Error::external)?)
        })?)?;
        api.set("settings",settings)?;
        let ctx = self.clone();
        api.set(
            "export",
            lua.create_function(move |lua, (name, function): (String, Function)| {
                ctx.require("resource.exports")?;
                ctx.action()?;
                if !ctx.installed.manifest.exports.contains(&name) {
                    return Err(lua_error(format!("undeclared export {name}")));
                }
                ctx.shared.lock().unwrap().exports.insert(
                    (ctx.installed.manifest.id.clone(), name),
                    Export {
                        lua: lua.clone(),
                        function,
                        budget: budget.clone(),
                        budget_units: ctx.limits.instruction_budget_units,
                        metrics: ctx.metrics.clone(),
                    },
                );
                Ok(())
            })?,
        )?;
        let ctx = self.clone();
        api.set(
            "call",
            lua.create_function(
                move |lua, (target, name, value): (String, String, mlua::Value)| {
                    ctx.require("resource.exports")?;
                    ctx.action()?;
                    if !ctx.installed.manifest.dependencies.contains_key(&target) {
                        return Err(lua_error(format!(
                            "export target {target} is not a declared dependency"
                        )));
                    }
                    let value = bounded_value(lua, value, ctx.limits.max_storage_value_bytes)?;
                    let export = {
                        let mut shared = ctx.shared.lock().unwrap();
                        if shared.export_depth >= 16 {
                            return Err(lua_error("export recursion depth exceeded"));
                        }
                        if !shared.active.contains(&target) {
                            return Err(lua_error(format!(
                                "export resource {target} is not running"
                            )));
                        }
                        let export = shared
                            .exports
                            .get(&(target.clone(), name.clone()))
                            .cloned()
                            .ok_or_else(|| lua_error(format!("missing export {target}:{name}")))?;
                        shared.export_depth += 1;
                        export
                    };
                    // Every cross-VM call has its own bounded budget, and nesting is capped.
                    export
                        .budget
                        .store(export.budget_units, Ordering::Relaxed);
                    // The engine bridge is scoped to the *calling* resource.
                    // A callee may enqueue its own owned commands, but must not
                    // look up or mutate the caller's bodies through that bridge.
                    let phase=format!("export:{name}");
                    let timer = Timer::start_for(&export.metrics,&phase,crate::runtime_metrics::function_source(&export.function));
                    let result = crate::query::without_host(|| {
                        let argument = json_value(&export.lua, &value)?;
                        let result = export.function.call::<mlua::Value>(argument)?;
                        crate::vm::check_instruction_budget(&export.budget)?;
                        bounded_value(&export.lua, result, 16 * 1024)
                    }).map_err(|error|lua_error(bounded_error(error)));
                    timer.record(&export.metrics, "export", export.budget_units.saturating_sub(export.budget.load(Ordering::Relaxed)), result.as_ref().err().map(ToString::to_string).as_deref());
                    {
                        let mut shared = ctx.shared.lock().unwrap();
                        shared.export_depth -= 1;
                        if let Err(error) = &result {
                            // One pending failure is sufficient to retire an
                            // owner; repeated caught errors must not accumulate.
                            if !shared.faults.iter().any(|(owner,_)|owner==&target) {
                                shared.faults.push((target, bounded_error(error)));
                            }
                        }
                    }
                    json_value(lua, &result?)
                },
            )?,
        )?;
        let ctx = self.clone();
        api.set(
            "command",
            lua.create_function(
                move |lua, (name, permission, function): (String, String, Function)| {
                    if ctx.side != Side::Server {
                        return Err(lua_error("commands are server-only"));
                    }
                    ctx.require("resource.commands")?;
                    ctx.action()?;
                    valid_name(&name).map_err(lua_error)?;
                    valid_name(&permission).map_err(lua_error)?;
                    let mut shared = ctx.shared.lock().unwrap();
                    if shared.commands.len() >= 128 {
                        return Err(lua_error("128 commands maximum"));
                    }
                    if shared.commands.contains_key(&name) {
                        return Err(lua_error(format!("duplicate command {name}")));
                    }
                    shared.commands.insert(
                        name,
                        RegisteredCommand {
                            owner: ctx.installed.manifest.id.clone(),
                            permission,
                            function,
                            lua: lua.clone(),
                        },
                    );
                    Ok(())
                },
            )?,
        )?;
        api.set(
            "players",
            lua.create_function(|lua, ()| {
                let snapshot: Table = lua.globals().get::<Table>("sdk")?.get("snapshot")?;
                match snapshot.get::<mlua::Value>("players")? {
                    mlua::Value::Nil => Ok(mlua::Value::Table(lua.create_table()?)),
                    value => Ok(value),
                }
            })?,
        )?;
        let ctx = self.clone();
        api.set("teleport", lua.create_function(move |lua, (player, value): (String, mlua::Value)| {
            if ctx.side != Side::Server { return Err(lua_error("teleport approval is server-only")); }
            ctx.require("resource.teleport")?;
            ctx.action()?;
            let id = player.parse::<u64>().map_err(|_|lua_error("player must be a canonical nonzero decimal ID string"))?;
            if id == 0 || id.to_string() != player { return Err(lua_error("player must be a canonical nonzero decimal ID string")); }
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Destination {
                position: Option<[f32; 3]>, heading: Option<f32>, velocity: Option<[f32; 3]>, instance: Option<u32>,
                #[serde(default)] restore_on_stop: bool,
                #[serde(default)] restore_previous: bool,
            }
            let dest: Destination = lua.from_value(value)?;
            if dest.restore_previous && (dest.position.is_some() || dest.heading.is_some() || dest.velocity.is_some() || dest.instance.is_some() || dest.restore_on_stop) {
                return Err(lua_error("restore_previous requires only its true flag"));
            }
            if !dest.restore_previous && dest.position.is_none() {return Err(lua_error("teleport requires position"));}
            let position=dest.position.unwrap_or([0.;3]);
            let validation = crate::vm::TeleportOptions { position, heading: dest.heading, velocity: dest.velocity };
            if !validation.validate() { return Err(lua_error("invalid teleport destination")); }
            let mut shared = ctx.shared.lock().unwrap();
            if shared.outputs.len() >= ctx.limits.max_queued_outputs { return Err(lua_error("resource output queue full")); }
            shared.push_output(Output::Teleport { resource: ctx.installed.manifest.id.clone(), generation: ctx.installed.generation,
                player: id, position, heading: dest.heading, velocity: dest.velocity, instance: dest.instance, restore_on_stop: dest.restore_on_stop, restore_previous: dest.restore_previous }, &ctx.limits)?;
            Ok(())
        })?)?;
        let ctx = self.clone();
        let entity = lua.create_function(move |lua, value: mlua::Value| {
            if ctx.side != Side::Server { return Err(lua_error("shared entities are server-only")); }
            ctx.require("resource.entities")?;
            ctx.action()?;
            let command = bounded_value(lua, value, 4096)?;
            if !command.is_object() || !matches!(command.get("op").and_then(Value::as_str), Some("spawn"|"remove"|"impulse"|"velocity"|"pose"|"transfer")) {
                return Err(lua_error("invalid shared entity operation"));
            }
            let mut shared = ctx.shared.lock().unwrap();
            shared.push_output(Output::Entity { resource: ctx.installed.manifest.id.clone(), generation: ctx.installed.generation, command }, &ctx.limits)
        })?;
        let entities = lua.create_table()?;
        entities.set("command", entity.clone())?;
        let ctx=self.clone();
        entities.set("all",lua.create_function(move |lua,()| {
            ctx.require("resource.entities")?;
            let snapshot:Table=lua.globals().get::<Table>("sdk")?.get("snapshot")?;
            match snapshot.get::<mlua::Value>("entities")? {mlua::Value::Nil=>lua.to_value(&Vec::<Value>::new()),value=>Ok(value)}
        })?)?;
        api.set("entities", entities)?;
        api.set("entity", entity)?;
        let voice=lua.create_table()?;
        let ctx=self.clone();
        let submit:Function=sdk.get("submit")?;
        voice.set("submit",lua.create_function(move |lua,value:mlua::Value| {
            ctx.require(if ctx.side==Side::Server {"resource.voice"} else {"engine.voice"})?;
            ctx.action()?;
            let operation=bounded_value(lua,value,4096)?;
            if ctx.side==Side::Server {
                let command:skate_voice::ServerCommand=serde_json::from_value(operation.clone()).map_err(mlua::Error::external)?;
                command.validate().map_err(lua_error)?;
                ctx.shared.lock().unwrap().push_output(Output::Voice{resource:ctx.installed.manifest.id.clone(),generation:ctx.installed.generation,operation},&ctx.limits)
            } else {
                submit.call::<()>(lua.to_value(&serde_json::json!({"kind":"voice","operation":operation}))?)
            }
        })?)?;
        api.set("voice",voice)?;
        for (name,cap,limit,field,operations) in [
            ("world","resource.world",64*1024,"op",&["rail_upsert","rail_remove","select"][..]),
            ("competition","resource.competition",16*1024,"kind",&["define","start","cancel","remove","native_start","native_cancel"][..]),
        ] {
            let table=lua.create_table()?;let ctx=self.clone();
            table.set(if name=="world" {"command"} else {"submit"},lua.create_function(move |lua,value:mlua::Value| {
                if ctx.side!=Side::Server {return Err(lua_error("world/competition authority is server-only"));}
                ctx.require(cap)?;ctx.action()?;
                let operation=bounded_value(lua,value,limit)?;
                if !operation.get(field).and_then(Value::as_str).is_some_and(|op|operations.contains(&op)) {return Err(lua_error("invalid authoritative operation"));}
                let resource=ctx.installed.manifest.id.clone();let generation=ctx.installed.generation;
                let output=if name=="world" {Output::World{resource,generation,operation}}else{Output::Competition{resource,generation,operation}};
                ctx.shared.lock().unwrap().push_output(output,&ctx.limits)
            })?)?;api.set(name,table)?;
        }
        let transfer=lua.create_table()?;let ctx=self.clone();
        transfer.set("start",lua.create_function(move |lua,(key,name,payload,options):(String,String,mlua::Value,Option<Table>)| {
            ctx.require("resource.events")?;ctx.action()?;valid_name(&key).map_err(lua_error)?;valid_name(&name).map_err(lua_error)?;
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Options {#[serde(default)]recipient:Option<String>,#[serde(default="default_transfer_timeout")]timeout_ms:u64}
            let options:Options=match options {Some(table)=>lua.from_value(mlua::Value::Table(table))?,None=>Options{recipient:None,timeout_ms:10_000}};
            let recipient=options.recipient.map(|text|text.parse::<u64>().ok().filter(|id|*id>0&&id.to_string()==text).ok_or_else(||lua_error("recipient must be a canonical nonzero player ID string"))).transpose()?;
            if (ctx.side==Side::Server)!=recipient.is_some() {return Err(lua_error("server transfers require an explicit recipient; client transfers go only to the server"));}
            if !(1..=120_000).contains(&options.timeout_ms) {return Err(lua_error("transfer timeout must be 1..120000 milliseconds"));}
            let payload=bounded_value(lua,payload,ctx.limits.max_payload_bytes)?;
            ctx.shared.lock().unwrap().push_output(Output::Transfer{resource:ctx.installed.manifest.id.clone(),generation:ctx.installed.generation,key,name,payload,recipient,timeout_ms:options.timeout_ms},&ctx.limits)
        })?)?;
        let ctx=self.clone();transfer.set("cancel",lua.create_function(move |_,key:String| {
            ctx.require("resource.events")?;ctx.action()?;valid_name(&key).map_err(lua_error)?;
            ctx.shared.lock().unwrap().push_output(Output::CancelTransfer{resource:ctx.installed.manifest.id.clone(),generation:ctx.installed.generation,key},&ctx.limits)
        })?)?;api.set("transfer",transfer)?;
        let services = lua.create_table()?;
        let ctx = self.clone();
        services.set("submit", lua.create_function(move |lua, (key, operation, timeout_ms): (String, mlua::Value, u64)| {
            if ctx.side != Side::Server { return Err(lua_error("backend services are server-only")); }
            ctx.action()?;
            valid_name(&key).map_err(lua_error)?;
            if !(1..=120_000).contains(&timeout_ms) { return Err(lua_error("service timeout must be 1..120000 milliseconds")); }
            let operation = bounded_value(lua, operation, ctx.limits.max_storage_value_bytes)?;
            match operation.get("kind").and_then(Value::as_str) {
                Some("http") => ctx.require("resource.http")?,
                Some("query" | "transaction" | "migrate") => ctx.require("resource.database")?,
                _ => return Err(lua_error("unknown backend service operation kind")),
            }
            let mut shared = ctx.shared.lock().unwrap();
            if shared.outputs.len() >= ctx.limits.max_queued_outputs { return Err(lua_error("resource output queue full")); }
            shared.push_output(Output::Service { resource: ctx.installed.manifest.id.clone(), generation: ctx.installed.generation,
                key, operation, timeout_ms }, &ctx.limits)?;
            Ok(())
        })?)?;
        let ctx = self.clone();
        services.set("cancel", lua.create_function(move |_, key: String| {
            if ctx.side != Side::Server { return Err(lua_error("backend services are server-only")); }
            if ctx.require("resource.http").is_err() { ctx.require("resource.database")?; }
            ctx.action()?;
            valid_name(&key).map_err(lua_error)?;
            let mut shared = ctx.shared.lock().unwrap();
            if shared.outputs.len() >= ctx.limits.max_queued_outputs { return Err(lua_error("resource output queue full")); }
            shared.push_output(Output::CancelService { resource: ctx.installed.manifest.id.clone(), generation: ctx.installed.generation, key }, &ctx.limits)?;
            Ok(())
        })?)?;
        api.set("services",services)?;
        // Restrict resource reads to declared content; no server configuration or
        // arbitrary neighboring files become accessible through the SDK.
        let ctx = self.clone();
        sdk.set(
            "read_text",
            lua.create_function(move |_, path: String| {
                ctx.action()?;
                let m = &ctx.installed.manifest;
                if !m
                    .files
                    .iter()
                    .chain(&m.shared_scripts)
                    .chain(&m.client_scripts)
                    .chain(if ctx.side == Side::Server {
                        &m.server_scripts
                    } else {
                        &m.client_scripts
                    })
                    .any(|s| s == &path)
                {
                    return Err(lua_error("read_text requires a declared resource file"));
                }
                String::from_utf8(
                    crate::read_bounded(&ctx.installed.root, &path, 256 * 1024)
                        .map_err(lua_error)?,
                )
                .map_err(mlua::Error::external)
            })?,
        )?;
        sdk.set("resource", api.clone())?;
        lua.globals().set("resource", api)?;
        // Retain the API-2 engine adapter for clients, but never peer-host session
        // authority. Server VMs expose no rendering, input, camera or physics API.
        sdk.set("session", mlua::Value::Nil)?;
        if self.side == Side::Server {
            for key in [
                "ui", "physics", "graphics", "camera", "audio", "input", "assets", "player", "rig",
                "bodies", "graphs", "engine", "commands", "volumes", "net",
            ] {
                sdk.set(key, mlua::Value::Nil)?;
            }
            sdk.set("capabilities", lua.create_table()?)?;
        } else {
            sdk.get::<Table>("net")?.set("publish", mlua::Value::Nil)?;
        }
        lua.load(include_str!("resource_api.lua"))
            .set_name("@skate-resource-api-1")
            .exec()
    }
    fn install_storage(&self, lua: &Lua, api: &Table) -> mlua::Result<()> {
        let values = if self.storage.exists() {
            let metadata = std::fs::metadata(&self.storage).map_err(mlua::Error::external)?;
            if metadata.len() > self.limits.max_storage_bytes as u64 {
                return Err(lua_error("resource persistence exceeds configured byte limit"));
            }
            let mut bytes = Vec::new();
            use std::io::Read;
            std::fs::File::open(&self.storage).map_err(mlua::Error::external)?
                .take(self.limits.max_storage_bytes as u64 + 1).read_to_end(&mut bytes).map_err(mlua::Error::external)?;
            if bytes.len() > self.limits.max_storage_bytes { return Err(lua_error("resource persistence exceeds configured byte limit")); }
            serde_json::from_slice::<BTreeMap<String, Value>>(&bytes)
                .map_err(mlua::Error::external)?
        } else {
            BTreeMap::new()
        };
        if values.len() > self.limits.max_storage_keys { return Err(lua_error("persistence key limit reached")); }
        for (key, value) in &values {
            valid_name(key).map_err(lua_error)?;
            validate_value(value, self.limits.max_storage_value_bytes).map_err(lua_error)?;
        }
        let values = Arc::new(Mutex::new(values));
        let storage = lua.create_table()?;
        let ctx = self.clone();
        let read = values.clone();
        storage.set(
            "get",
            lua.create_function(move |lua, key: String| {
                ctx.require("resource.storage")?;
                valid_name(&key).map_err(lua_error)?;
                json_value(lua, read.lock().unwrap().get(&key).unwrap_or(&Value::Null))
            })?,
        )?;
        let ctx = self.clone();
        storage.set(
            "set",
            lua.create_function(move |lua, (key, value): (String, mlua::Value)| {
                ctx.require("resource.storage")?;
                ctx.action()?;
                valid_name(&key).map_err(lua_error)?;
                let value = bounded_value(lua, value, ctx.limits.max_storage_value_bytes)?;
                let mut original = values.lock().unwrap();
                let mut next = original.clone();
                if value.is_null() {
                    next.remove(&key);
                } else {
                    next.insert(key, value);
                }
                if next.len() > ctx.limits.max_storage_keys {
                    return Err(lua_error("persistence key limit reached"));
                }
                let bytes = serde_json::to_vec(&next).map_err(mlua::Error::external)?;
                if bytes.len() > ctx.limits.max_storage_bytes {
                    return Err(lua_error("resource persistence exceeds configured byte limit"));
                }
                std::fs::create_dir_all(ctx.storage.parent().unwrap())
                    .map_err(mlua::Error::external)?;
                let temporary = ctx.storage.with_extension("tmp");
                std::fs::write(&temporary, bytes).map_err(mlua::Error::external)?;
                std::fs::rename(&temporary, &ctx.storage).map_err(mlua::Error::external)?;
                *original = next;
                Ok(())
            })?,
        )?;
        api.set("storage", storage)
    }
}
fn lua_error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}
fn valid_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
    {
        Err("name must contain 1..64 ASCII identifier characters".into())
    } else {
        Ok(())
    }
}
pub(crate) fn bounded_value(lua: &Lua, value: mlua::Value, limit: usize) -> mlua::Result<Value> {
    fn visit(
        value: &mlua::Value,
        depth: usize,
        remaining: &mut usize,
        bytes: &mut usize,
        active: &mut BTreeSet<usize>,
    ) -> mlua::Result<()> {
        if depth > 32 {
            return Err(lua_error("payload nesting exceeds 32 levels"));
        }
        if *remaining == 0 {
            return Err(lua_error("payload contains too many values"));
        }
        *remaining -= 1;
        match value {
            mlua::Value::Table(table) => {
                let identity = table.to_pointer() as usize;
                if !active.insert(identity) {
                    return Err(lua_error("cyclic payload table"));
                }
                for pair in table.clone().pairs::<mlua::Value, mlua::Value>() {
                    let (key, value) = pair?;
                    visit(&key, depth + 1, remaining, bytes, active)?;
                    visit(&value, depth + 1, remaining, bytes, active)?;
                }
                active.remove(&identity);
            }
            mlua::Value::String(text) => {
                *bytes = bytes
                    .checked_sub(text.as_bytes().len())
                    .ok_or_else(|| lua_error("payload exceeds byte limit"))?;
            }
            mlua::Value::Nil | mlua::Value::Boolean(_) | mlua::Value::Integer(_) => {}
            mlua::Value::Number(number) if number.is_finite() => {}
            _ => return Err(lua_error("payload must contain only finite JSON values")),
        }
        Ok(())
    }
    let mut remaining_nodes = limit.saturating_mul(2) + 128;
    let mut remaining_bytes = limit;
    visit(
        &value,
        0,
        &mut remaining_nodes,
        &mut remaining_bytes,
        &mut BTreeSet::new(),
    )?;
    let value: Value = lua.from_value(value)?;
    validate_value(&value, limit).map_err(lua_error)?;
    Ok(value)
}
fn validate_value(value: &Value, limit: usize) -> Result<(), String> {
    fn depth(value: &Value, level: usize) -> Result<(), String> {
        if level > 32 {
            return Err("payload nesting exceeds 32 levels".into());
        }
        match value {
            Value::Array(values) => {
                for value in values {
                    depth(value, level + 1)?;
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    depth(value, level + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    depth(value, 0)?;
    if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > limit {
        Err(format!("payload exceeds {limit} bytes"))
    } else {
        Ok(())
    }
}
pub(crate) fn json_value(lua: &Lua, value: &Value) -> mlua::Result<mlua::Value> {
    lua.to_value_with(
        value,
        mlua::serde::SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false),
    )
}

struct Instance {
    vm: Vm,
    bootstrap: Bootstrap,
}
/// A host is intentionally independent of any UDP/HTTP/Steam implementation.
pub struct Host {
    side: Side,
    limits: RuntimeLimits,
    storage: PathBuf,
    installed: BTreeMap<String, InstalledResource>,
    order: Vec<String>,
    instances: BTreeMap<String, Instance>,
    metrics: BTreeMap<String, Counters>,
    profile: crate::runtime_profile::Recorder,
    settings_revision: u64,
    settings_defaults: BTreeMap<String,BTreeMap<String,Value>>,
    settings_desired: BTreeMap<String,BTreeMap<String,Value>>,
    started: BTreeSet<String>,
    shared: Arc<Mutex<Shared>>,
    commands: Vec<(String, Command)>,
    retired: Vec<String>,
    snapshot: Arc<Value>,
    fields: SnapshotFields,
    pub diagnostics: Vec<String>,
}
impl Host {
    pub fn new(side: Side, storage_root: PathBuf, scope: &str) -> Result<Self, String> {
        Self::new_with_limits(side, storage_root, scope, RuntimeLimits::default())
    }
    pub fn new_with_limits(side: Side, storage_root: PathBuf, scope: &str, limits: RuntimeLimits) -> Result<Self, String> {
        limits.validate()?;
        if scope.is_empty() || scope.len() > 2048 {
            return Err("persistence scope must contain 1..2048 bytes".into());
        }
        let scope = blake3::hash(format!("resource-storage-v1:{side:?}:{scope}").as_bytes())
            .to_hex()
            .to_string();
        Ok(Self {
            side,
            limits,
            storage: storage_root.join(scope),
            installed: BTreeMap::new(),
            order: vec![],
            instances: BTreeMap::new(),
            metrics: BTreeMap::new(),
            profile: crate::runtime_profile::History::new(),
            settings_revision: 0,
            settings_defaults: BTreeMap::new(),
            settings_desired: BTreeMap::new(),
            started: BTreeSet::new(),
            shared: Arc::new(Mutex::new(Shared::default())),
            commands: vec![],
            retired: vec![],
            snapshot: Arc::new(Value::Null),
            fields: SnapshotFields::default(),
            diagnostics: vec![],
        })
    }
    pub fn configure_profiling(&mut self,enabled:bool,capacity:usize,retention_ms:u64)->Result<(),String> {self.profile.lock().unwrap().configure(enabled,capacity,retention_ms)}
    pub fn profile_snapshot(&self)->ProfileSnapshot {self.profile.lock().unwrap().snapshot()}
    pub fn profile_scope(&self,phase:&str)->ProfileScope {ProfileScope::new(Timer::start_context(crate::runtime_profile::Context{recorder:self.profile.clone(),resource:"@host".into(),generation:0,language:"host".into(),source:None},phase))}
    pub fn side(&self) -> Side {
        self.side
    }
    /// Current measurements and the last stopped/failed generation. Inclusive
    /// durations cannot be summed across resources to infer process CPU load.
    pub fn runtime_metrics(&self) -> BTreeMap<String, RuntimeMetrics> {
        let mut metrics:BTreeMap<_,_>=self.metrics.iter().map(|(id,counter)| {
            let mut value=counter.lock().unwrap().clone();
            value.running=self.running(id);
            if let Some(instance)=self.instances.get(id) { instance.vm.measure_memory(&mut value); }
            (id.clone(),value)
        }).collect();
        for output in &self.shared.lock().unwrap().outputs {
            let resource=match output {
                Output::Event{resource,..}|Output::State{resource,..}|Output::Entity{resource,..}|Output::Service{resource,..}|Output::CancelService{resource,..}|Output::Voice{resource,..}|Output::World{resource,..}|Output::Competition{resource,..}|Output::Transfer{resource,..}|Output::CancelTransfer{resource,..}|Output::Teleport{resource,..}|Output::Log{resource,..}=>resource,
            };
            if let Some(value)=metrics.get_mut(resource) {value.queued_outputs+=1;value.queued_output_accounted_bytes=value.queued_output_accounted_bytes.saturating_add(Shared::output_size(output));}
        }
        metrics
    }
    pub fn installed(&self) -> &BTreeMap<String, InstalledResource> {
        &self.installed
    }
    pub fn running(&self, id: &str) -> bool {
        self.instances.contains_key(id)
    }
    pub fn running_ids(&self) -> Vec<String> {
        self.order
            .iter()
            .filter(|id| self.running(id))
            .cloned()
            .collect()
    }
    pub fn generation(&self, id: &str) -> Option<u64> {
        self.installed.get(id).map(|i| i.generation)
    }
    pub fn state(&self,id:&str,key:&str)->Option<Value> {
        self.shared.lock().unwrap().states.get(&(id.into(),"resource".into())).and_then(|state|state.get(key)).cloned()
    }
    pub fn scoped_state(&self,id:&str,key:&str,scope:&Value)->Result<Option<Value>,String> {
        let (scope,_)=normalize_scope(scope.clone())?;
        Ok(self.shared.lock().unwrap().states.get(&(id.into(),scope)).and_then(|state|state.get(key)).cloned())
    }
    pub fn states(&self)->BTreeMap<String,BTreeMap<String,Value>> {
        self.shared.lock().unwrap().states.iter().filter(|((_,scope),_)|scope=="resource").map(|((owner,_),state)|(owner.clone(),state.clone())).collect()
    }
    pub fn scoped_states(&self)->Vec<(String,Value,String,Value)> {
        self.shared.lock().unwrap().states.iter().flat_map(|((owner,scope),state)|state.iter().map(move |(key,value)|(owner.clone(),scope_from_key(scope),key.clone(),value.clone()))).collect()
    }
    pub fn install(&mut self, resources: Vec<InstalledResource>) -> Result<(), String> {
        if !self.instances.is_empty() {
            return Err("stop resources before replacing the installed set".into());
        }
        if resources.len() > self.limits.max_resources {
            return Err("resource count limit reached".into());
        }
        let manifests: Vec<_> = resources.iter().map(|r| r.manifest.clone()).collect();
        let order = skate_resources::ordered_manifests(&manifests).map_err(|e| e.to_string())?;
        for r in &resources {
            if self.side==Side::Client && r.manifest.settings.values().any(|d|d.visibility==skate_resources::SettingVisibility::Private) {return Err("client manifest contains private settings".into());}
            if r.generation == 0 {
                return Err("resource generation must be positive".into());
            }
            for cap in &r.manifest.capabilities {
                if !supported_capability(cap) {
                    return Err(format!(
                        "{} requests unsupported capability {cap}",
                        r.manifest.id
                    ));
                }
            }
        }
        self.installed = resources
            .into_iter()
            .map(|r| (r.manifest.id.clone(), r))
            .collect();
        self.order = order;
        self.started.clear();
        self.metrics.clear();
        self.shared.lock().unwrap().settings.clear();
        self.settings_defaults.clear();
        self.settings_desired.clear();
        for id in self.order.clone() {self.prepare_settings(&id)?;}
        Ok(())
    }
    /// Add stopped/new packages without disturbing live VMs. Running definitions
    /// must be identical; refresh their manifest only after an explicit stop.
    pub fn register(&mut self, resources: Vec<InstalledResource>) -> Result<(), String> {
        let mut installed = self.installed.clone();
        for resource in resources {
            if self.side==Side::Client && resource.manifest.settings.values().any(|d|d.visibility==skate_resources::SettingVisibility::Private) {return Err("client manifest contains private settings".into());}
            let id = &resource.manifest.id;
            if resource.generation == 0 {
                return Err("resource generation must be positive".into());
            }
            for cap in &resource.manifest.capabilities {
                if !supported_capability(cap) {
                    return Err(format!("{id} requests unsupported capability {cap}"));
                }
            }
            if let Some(previous) = installed.get(id) {
                if self.running(id)
                    && (previous.manifest != resource.manifest
                        || previous.root != resource.root
                        || previous.grants != resource.grants
                        || previous.generation != resource.generation)
                {
                    return Err(format!(
                        "stop running resource {id} before replacing its definition"
                    ));
                }
                if self.started.contains(id) && resource.generation < previous.generation {
                    return Err(format!("{id}: resource generation cannot decrease"));
                }
            }
            installed.insert(id.clone(), resource);
        }
        if installed.len() > self.limits.max_resources {
            return Err("resource count limit reached".into());
        }
        let mut definitions = installed
            .values()
            .map(|r| r.manifest.clone())
            .collect::<Vec<_>>();
        // Stopped contracts may still describe the previous dependency version
        // while an operator refreshes several packages. Validate their paths and
        // graph now; exact versions are rechecked across the entire startup closure.
        // Running contracts always retain strict compatibility.
        for manifest in &mut definitions {
            if !self.running(&manifest.id) {
                for (dependency, required) in &mut manifest.dependencies {
                    if let Some(current) = installed.get(dependency) {
                        *required = current.manifest.version.clone();
                    }
                }
            }
        }
        let order = skate_resources::ordered_manifests(&definitions).map_err(|e| e.to_string())?;
        self.installed = installed;
        self.order = order;
        Ok(())
    }

    pub fn start_all(&mut self) -> Result<(), String> {
        let before: BTreeSet<_> = self.instances.keys().cloned().collect();
        for id in self.order.clone() {
            if let Err(error) = self.start(&id) {
                for id in self.order.clone().into_iter().rev() {
                    if !before.contains(&id) {
                        self.stop_one(&id);
                    }
                }
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn start(&mut self, id: &str) -> Result<(), String> {
        if self.running(id) {
            return Ok(());
        }
        // Validate the full closure before any startup side effect, including
        // stopped definitions retained during an incremental version upgrade.
        let mut pending = vec![id.to_string()];
        let mut closure = BTreeMap::new();
        while let Some(next) = pending.pop() {
            if closure.contains_key(&next) {
                continue;
            }
            let installed = self
                .installed
                .get(&next)
                .ok_or_else(|| format!("unknown resource {next}"))?;
            pending.extend(installed.manifest.dependencies.keys().cloned());
            closure.insert(next, installed.manifest.clone());
        }
        skate_resources::ordered_manifests(&closure.into_values().collect::<Vec<_>>())
            .map_err(|e| e.to_string())?;
        let dependencies = self
            .installed
            .get(id)
            .ok_or_else(|| format!("unknown resource {id}"))?
            .manifest
            .dependencies
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for dependency in dependencies {
            self.start(&dependency)
                .map_err(|e| format!("{id} dependency {dependency}: {e}"))?;
        }
        self.prepare_settings(id)?;
        let installed = self.installed.get_mut(id).unwrap();
        if self.started.contains(id) {
            installed.generation = installed
                .generation
                .checked_add(1)
                .ok_or("resource generation exhausted")?;
        }
        self.started.insert(id.to_string());
        let metrics=Arc::new(Mutex::new(RuntimeMetrics {generation:installed.generation,language:installed.manifest.language.clone(),profile:Some(crate::runtime_profile::Context{recorder:self.profile.clone(),resource:id.into(),generation:installed.generation,language:installed.manifest.language.clone(),source:Some(installed.manifest.shared_scripts.iter().chain(if self.side==Side::Server{&installed.manifest.server_scripts}else{&installed.manifest.client_scripts}).map(String::as_str).collect::<Vec<_>>().join(",").chars().take(160).collect())}),..RuntimeMetrics::default()}));
        self.metrics.insert(id.to_string(),metrics.clone());
        let bootstrap = Bootstrap {
            installed: installed.clone(),
            side: self.side,
            storage: self.storage.join(format!("{id}.json")),
            shared: self.shared.clone(),
            handlers: Arc::new(Mutex::new(BTreeMap::new())),
            actions: Arc::new(AtomicUsize::new(0)),
            limits: self.limits.clone(),
            metrics: metrics.clone(),
        };
        let timer=Timer::start_for(&metrics,"startup",None);
        let result = Vm::new_resource(
            &bootstrap.installed.root,
            &bootstrap.installed.manifest.id,
            &self.snapshot,
            &bootstrap,
        );
        timer.record(&metrics,"startup",0,result.as_ref().err().map(String::as_str));
        let vm = match result {
            Ok(vm) => vm,
            Err(error) => {
                self.cleanup(id);
                let error=bounded_error(format_args!("{id} startup: {error}"));
                self.diagnostic(&error);
                return Err(error);
            }
        };
        self.instances
            .insert(id.to_string(), Instance { vm, bootstrap });
        self.shared.lock().unwrap().active.insert(id.to_string());
        if let Err(error) = self.invoke(id, "on_load", Value::Null, None) {
            self.fail(id, error.clone());
            return Err(error);
        }
        self.pump_events()?;
        Ok(())
    }
    pub fn stop(&mut self, id: &str) -> Result<(), String> {
        if !self.installed.contains_key(id) {
            return Err(format!("unknown resource {id}"));
        }
        let dependents = self.dependents(id);
        for child in self.order.clone().into_iter().rev() {
            if child == id || dependents.contains(&child) {
                self.stop_one(&child);
            }
        }
        Ok(())
    }
    pub fn restart(&mut self, id: &str) -> Result<(), String> {
        if !self.running(id) {
            return Err(format!("resource {id} is not running; use start or ensure"));
        }
        let mut restart = self.dependents(id);
        restart.retain(|id| self.running(id));
        restart.insert(id.to_string());
        self.stop(id)?;
        for id in self.order.clone() {
            if restart.contains(&id) {
                self.start(&id)?;
            }
        }
        Ok(())
    }
    /// Like Cfx ensure: start a stopped resource, restart an already-running one.
    pub fn ensure(&mut self, id: &str) -> Result<(), String> {
        if self.running(id) {
            self.restart(id)
        } else {
            self.start(id)
        }
    }
    fn dependents(&self, id: &str) -> BTreeSet<String> {
        let mut result = BTreeSet::new();
        loop {
            let previous = result.len();
            for (name, r) in &self.installed {
                if r.manifest
                    .dependencies
                    .keys()
                    .any(|d| d == id || result.contains(d))
                {
                    result.insert(name.clone());
                }
            }
            if result.len() == previous {
                return result;
            }
        }
    }
    fn stop_one(&mut self, id: &str) {
        // Cascading teardown can run under another resource's fixed callback.
        // Include Lua destruction, not only on_unload, in the isolated scope.
        crate::query::without_host(|| {
            if self.running(id) {
                let _ = self.invoke(id, "on_unload", Value::Null, None);
                self.instances.remove(id);
            }
            self.cleanup(id);
        });
    }
    fn cleanup(&mut self, id: &str) {
        let mut shared = self.shared.lock().unwrap();
        shared.active.remove(id);
        // These entries own strong Lua handles. Never destroy a Lua state while
        // holding Shared: closing it may run native finalizers which reenter Rust.
        let export_keys = shared
            .exports
            .keys()
            .filter(|(owner, _)| owner == id)
            .cloned()
            .collect::<Vec<_>>();
        let retired_exports = export_keys
            .into_iter()
            .filter_map(|key| shared.exports.remove(&key))
            .collect::<Vec<_>>();
        let command_keys = shared
            .commands
            .iter()
            .filter(|(_, command)| command.owner == id)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let retired_commands = command_keys
            .into_iter()
            .filter_map(|key| shared.commands.remove(&key))
            .collect::<Vec<_>>();
        shared.states.retain(|(owner,_),_|owner!=id);
        shared.events.retain(|(owner, _, _, _, _)| owner != id);
        shared.outputs.retain(|output| match output {
            Output::Event { resource, .. }
            | Output::State { resource, .. }
            | Output::Log { resource, .. }
            | Output::Teleport { resource, .. }
            | Output::Entity { resource, .. }
            | Output::Voice { resource, .. }
            | Output::World { resource, .. }
            | Output::Competition { resource, .. }
            | Output::Transfer { resource, .. }
            | Output::CancelTransfer { resource, .. }
            | Output::Service { resource, .. }
            | Output::CancelService { resource, .. } => resource != id,
        });
        shared.output_bytes = shared.outputs.iter().map(Shared::output_size).sum();
        drop(shared);
        crate::query::without_host(|| {
            drop(retired_exports);
            drop(retired_commands);
        });
        self.commands.retain(|(owner, _)| owner != id);
        if !self.retired.iter().any(|owner| owner == id) {
            self.retired.push(id.to_string());
        }
    }
    fn diagnostic(&mut self, message: impl std::fmt::Display) {
        self.diagnostics.push(bounded_error(message));
        if self.diagnostics.len() > 128 {
            self.diagnostics.drain(..self.diagnostics.len()-128);
        }
    }
    pub fn fail(&mut self, id: &str, error: String) {
        self.diagnostic(format_args!("{id}: {error}"));
        let _ = self.stop(id);
        self.retire_export_failures();
    }
    fn retire_export_failures(&mut self) {
        let faults = std::mem::take(&mut self.shared.lock().unwrap().faults);
        for (id, error) in faults {
            self.diagnostic(format_args!("{id} export: {error}"));
            let _ = self.stop(&id);
        }
    }
    pub fn disconnect(&mut self) {
        for id in self.order.clone().into_iter().rev() {
            self.stop_one(&id);
        }
        let mut shared=self.shared.lock().unwrap();
        shared.outputs.clear();
        shared.output_bytes=0;
    }
    pub fn set_snapshot(&mut self, snapshot: Arc<Value>, fields: SnapshotFields) {
        self.snapshot = snapshot;
        self.fields = fields;
    }
    pub fn tick(&mut self, dt: f64, snapshot: Value) {
        self.snapshot = Arc::new(snapshot);
        if !dt.is_finite() || !(0.0..=10.0).contains(&dt) {
            return;
        }
        self.dispatch("on_update", serde_json::json!({"dt":dt}));
    }
    pub fn dispatch(&mut self, callback: &str, payload: Value) {
        let timer=Timer::start_context(crate::runtime_profile::Context{recorder:self.profile.clone(),resource:"@host".into(),generation:0,language:"host".into(),source:None},&format!("dispatch:{callback}"));
        for id in self.running_ids() {
            self.call(&id, callback, payload.clone());
        }
        timer.finish_profile(false);
    }
    pub fn call(&mut self, id: &str, callback: &str, payload: Value) {
        self.call_with_physics(id, callback, payload, None);
    }
    pub fn call_with_physics(
        &mut self,
        id: &str,
        callback: &str,
        payload: Value,
        physics: Option<Value>,
    ) {
        if let Err(error) = self.invoke(id, callback, payload, physics) {
            self.fail(id, error);
        }
        if let Err(error) = self.pump_events() {
            self.diagnostic(error);
        }
    }
    fn invoke(
        &mut self,
        id: &str,
        callback: &str,
        payload: Value,
        physics: Option<Value>,
    ) -> Result<(), String> {
        let Some(instance) = self.instances.get_mut(id) else {
            return Ok(());
        };
        instance.bootstrap.reset_budget();
        let commands =
            instance
                .vm
                .call_shared(callback, payload, &self.snapshot, physics, &self.fields).map_err(bounded_error)?;
        self.accept_commands(id, commands);
        Ok(())
    }
    fn accept_commands(&mut self, id: &str, commands: Vec<Command>) {
        for command in commands {
            if let Command::Log { text } = &command {
                let mut shared = self.shared.lock().unwrap();
                if shared.outputs.len() < self.limits.max_queued_outputs {
                    let _ = shared.push_output(Output::Log {
                        resource: id.into(),
                        text: text.clone(),
                    }, &self.limits);
                }
            }
            if self.side == Side::Client {
                self.commands.push((id.into(), command));
            }
        }
    }
    fn pump_events(&mut self) -> Result<(), String> {
        self.retire_export_failures();
        for _ in 0..self.limits.max_actions {
            let event = self.shared.lock().unwrap().events.pop_front();
            let Some((id, generation, name, payload,queued)) = event else {
                return Ok(());
            };
            if self.generation(&id) == Some(generation) && self.running(&id) {
                if let Err(error) = self.event_queued(&id, &name, payload, 0, false,Some(queued.elapsed().as_micros().min(u64::MAX as u128) as u64)) {
                    self.fail(&id, error.clone());
                    return Err(error);
                }
            }
        }
        let id = self
            .shared
            .lock()
            .unwrap()
            .events
            .front()
            .map(|e| e.0.clone());
        self.shared.lock().unwrap().events.clear();
        if let Some(id) = id {
            let error = "local event dispatch budget exhausted".to_string();
            self.fail(&id, error.clone());
            Err(error)
        } else {
            Ok(())
        }
    }
    fn event(
        &mut self,
        id: &str,
        name: &str,
        payload: Value,
        sender: u64,
        network: bool,
    ) -> Result<(), String> {
        self.event_queued(id,name,payload,sender,network,None)
    }
    fn event_queued(&mut self,id:&str,name:&str,payload:Value,sender:u64,network:bool,queue_wait_us:Option<u64>)->Result<(),String> {
        let instance = self.instances.get_mut(id).ok_or_else(||format!("resource {id} is not running"))?;
        let handlers = instance
            .bootstrap
            .handlers
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|h| !network || h.network)
            .collect::<Vec<_>>();
        if network && handlers.is_empty() {
            return Err(format!("unregistered network event {id}:{name}"));
        }
        instance.bootstrap.reset_budget();
        let commands = instance.vm.resource_callbacks(
            &format!("event:{name}"),queue_wait_us,
            handlers.iter().map(|h| h.function.clone()).collect(),
            payload,
            sender,
            &self.snapshot,
            &self.fields,
        ).map_err(bounded_error)?;
        self.accept_commands(id, commands);
        Ok(())
    }
    /// Call only with an identity obtained from an admitted transport connection.
    pub fn receive(
        &mut self,
        sender: u64,
        resource: &str,
        generation: u64,
        name: &str,
        payload: Value,
    ) -> Result<(), String> {
        valid_name(name)?;
        validate_value(&payload, self.limits.max_payload_bytes)?;
        if self.side == Side::Server && sender == 0 {
            return Err("client sender identity must be nonzero".into());
        }
        if self.side == Side::Client && sender != 0 {
            return Err("client events must originate from the server".into());
        }
        if self.generation(resource) != Some(generation) || !self.running(resource) {
            return Err("stale or stopped resource generation".into());
        }
        // Unknown event names are rejected without disabling a healthy resource.
        if !self.instances[resource]
            .bootstrap
            .handlers
            .lock()
            .unwrap()
            .get(name)
            .is_some_and(|h| h.iter().any(|h| h.network))
        {
            return Err(format!("unregistered network event {resource}:{name}"));
        }
        if let Err(error) = self.event(resource, name, payload, sender, true) {
            self.fail(resource, error.clone());
            return Err(error);
        }
        self.pump_events()
    }
    /// Deliver a trusted asynchronous host completion to a live resource.
    pub fn host_event(&mut self,resource:&str,generation:u64,name:&str,payload:Value)->Result<(),String> {
        valid_name(name)?;validate_value(&payload,self.limits.max_payload_bytes)?;
        if self.generation(resource)!=Some(generation)||!self.running(resource) {return Err("stale or stopped resource generation".into());}
        if let Err(error)=self.event(resource,name,payload,0,false) {self.fail(resource,error.clone());return Err(error);}
        self.pump_events()
    }
    /// Complete a server-owned asynchronous operation. A stopped/restarted VM
    /// must never receive an earlier generation's result.
    pub fn service_result(&mut self, resource: &str, generation: u64, key: &str, result: Value) -> Result<(), String> {
        if self.side != Side::Server { return Err("backend service completions are server-only".into()); }
        valid_name(key)?;
        validate_value(&result, self.limits.max_storage_value_bytes)?;
        if self.generation(resource) != Some(generation) || !self.running(resource) {
            return Err("stale or stopped resource generation".into());
        }
        let payload = serde_json::json!({"key":key,"result":result});
        if let Err(error) = self.event(resource, "service_result", payload, 0, false) {
            self.fail(resource,error.clone()); return Err(error);
        }
        self.pump_events()
    }
    /// The transport must authenticate this as server data before invoking it.
    pub fn apply_state(&mut self,resource:&str,generation:u64,key:&str,value:Value)->Result<(),String> {
        self.apply_scoped_state(resource,generation,key,value,resource_scope())
    }
    pub fn apply_scoped_state(&mut self,resource:&str,generation:u64,key:&str,value:Value,scope:Value)->Result<(),String> {
        if self.side!=Side::Client {return Err("only clients consume replicated server state".into());}
        if key=="__settings" {
            if normalize_scope(scope)?.0!="resource" {return Err("settings state requires resource scope".into());}
            let values=serde_json::from_value(value).map_err(|_|"invalid settings snapshot")?;
            return self.apply_settings(resource,generation,values);
        }
        valid_name(key)?;validate_value(&value,self.limits.max_payload_bytes)?;
        let (scope,_)=normalize_scope(scope)?;
        if self.generation(resource)!=Some(generation)||!self.running(resource) {return Err("stale or stopped resource generation".into());}
        self.shared.lock().unwrap().set_state(resource,&scope,key,value,self.limits.max_state_keys)
    }
    /// Drop private state that is no longer visible after an interest/instance update.
    pub fn retain_scoped_state(&mut self,scopes:&[Value]) {
        if self.side!=Side::Client {return;}
        let allowed:BTreeSet<_>=scopes.iter().take(65536).filter_map(|scope|normalize_scope(scope.clone()).ok().map(|(key,_)|key)).collect();
        self.shared.lock().unwrap().states.retain(|(_,scope),_|scope=="resource"||allowed.contains(scope));
    }
    pub fn prune_resource_scope(&mut self,resource:&str,scope:&Value)->Result<(),String> {
        let (scope,_)=normalize_scope(scope.clone())?;
        let mut shared=self.shared.lock().unwrap();
        shared.states.remove(&(resource.to_owned(),scope.clone()));
        shared.outputs.retain(|output|match output {
            Output::Event{resource:owner,scope:value,..}|Output::State{resource:owner,scope:value,..}=>owner!=resource||normalize_scope(value.clone()).is_ok_and(|(key,_)|key!=scope),
            _=>true,
        });
        shared.output_bytes=shared.outputs.iter().map(Shared::output_size).sum();Ok(())
    }
    /// Server target retirement never removes persistent instance-wide or global state.
    pub fn prune_scoped_targets(&mut self,players:&[u64],entities:&[(String,u64,u64)]) {
        if self.side!=Side::Server {return;}
        let players:BTreeSet<_>=players.iter().copied().collect();
        let entities:BTreeSet<_>=entities.iter().cloned().collect();
        let visible=|owner:&str,scope:&str| {
            match scope.split_once(':') {
                Some(("player",id))=>id.parse::<u64>().is_ok_and(|id|players.contains(&id)),
                Some(("entity",id))=>id.parse::<u64>().is_ok_and(|id|self.generation(owner).is_some_and(|generation|entities.contains(&(owner.to_owned(),id,generation)))),
                _=>true,
            }
        };
        let mut shared=self.shared.lock().unwrap();
        shared.states.retain(|(owner,scope),_|visible(owner,scope));
        shared.outputs.retain(|output|match output {
            Output::Event{resource,scope,..}|Output::State{resource,scope,..}=>normalize_scope(scope.clone()).is_ok_and(|(scope,_)|visible(resource,&scope)),
            _=>true,
        });
        shared.output_bytes=shared.outputs.iter().map(Shared::output_size).sum();
    }
    pub fn command(
        &mut self,
        actor: u64,
        name: &str,
        args: Vec<String>,
        permissions: &BTreeSet<String>,
    ) -> Result<(), String> {
        if self.side != Side::Server {
            return Err("commands are server-only".into());
        }
        valid_name(name)?;
        if args.len() > 32 || args.iter().any(|a| a.len() > 256) {
            return Err("command arguments exceed limits".into());
        }
        let command = self
            .shared
            .lock()
            .unwrap()
            .commands
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown resource command {name}"))?;
        if !permissions.contains(&command.permission) && !(actor == 0 && permissions.contains("*"))
        {
            return Err(format!("command {name} requires {}", command.permission));
        }
        let instance = self
            .instances
            .get_mut(&command.owner)
            .ok_or("command resource is stopped")?;
        instance.bootstrap.reset_budget();
        let _ = &command.lua;
        let payload = serde_json::to_value(args).map_err(|e| e.to_string())?;
        match instance.vm.resource_callbacks(
            &format!("command:{name}"),None,
            vec![command.function],
            payload,
            actor,
            &self.snapshot,
            &self.fields,
        ).map_err(bounded_error) {
            Ok(commands) => {
                self.accept_commands(&command.owner, commands);
                self.pump_events()
            }
            Err(error) => {
                self.fail(&command.owner, error.clone());
                Err(error)
            }
        }
    }
    pub fn drain_outputs(&mut self) -> Vec<Output> {
        let mut shared=self.shared.lock().unwrap();
        shared.output_bytes=0;
        std::mem::take(&mut shared.outputs)
    }
    pub fn drain_commands(&mut self) -> Vec<(String, Command)> {
        std::mem::take(&mut self.commands)
    }
    pub fn drain_retired(&mut self) -> Vec<String> {
        std::mem::take(&mut self.retired)
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.disconnect();
    }
}
fn supported_capability(cap: &str) -> bool {
    matches!(
        cap,
        "resource.events"
            | "resource.network"
            | "resource.state"
            | "resource.settings"
            | "resource.storage"
            | "resource.commands"
            | "resource.exports"
            | "resource.teleport"
            | "resource.entities"
            | "resource.voice"
            | "resource.world"
            | "resource.competition"
            | "resource.database"
            | "resource.http"
            | "engine.ui"
            | "engine.audio"
            | "engine.voice"
            | "engine.graphics"
            | "engine.physics"
            | "engine.player"
            | "engine.camera"
            | "engine.input"
            | "engine.world"
            | "engine.animation"
            | "engine.inspect"
    )
}
