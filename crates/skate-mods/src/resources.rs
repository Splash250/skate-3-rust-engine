//! Transport-independent, capability-granted Lua resources. The host supplies
//! connection identities; scripts can neither select their owner nor generation.
use crate::{Command, SnapshotFields, vm::Vm};
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

pub const MAX_PAYLOAD: usize = 384;
const MAX_ACTIONS: usize = 128;
const MAX_CALLBACKS: usize = 64;
const MAX_STATE_KEYS: usize = 64;
const MAX_STORAGE: usize = 64 * 1024;

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
    },
    State {
        resource: String,
        generation: u64,
        key: String,
        value: Value,
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
    events: VecDeque<(String, u64, String, Value)>,
    states: BTreeMap<String, BTreeMap<String, Value>>,
    exports: BTreeMap<(String, String), Export>,
    commands: BTreeMap<String, RegisteredCommand>,
    export_depth: usize,
    faults: Vec<(String, String)>,
}
#[derive(Clone)]
pub(crate) struct Bootstrap {
    installed: InstalledResource,
    side: Side,
    storage: PathBuf,
    shared: Arc<Mutex<Shared>>,
    handlers: Arc<Mutex<BTreeMap<String, Vec<Handler>>>>,
    actions: Arc<AtomicUsize>,
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
        } else if kind.starts_with("ui_") || matches!(kind, "overlay" | "multiplayer_debug") {
            "engine.ui"
        } else if kind.starts_with("player_") || matches!(kind, "rig_part" | "native_impulse") {
            "engine.player"
        } else if kind.starts_with("volume_") {
            "engine.world"
        } else if kind == "input_override" {
            "engine.input"
        } else if kind == "graph_gate" {
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
        if self.actions.fetch_add(1, Ordering::Relaxed) >= MAX_ACTIONS {
            Err(lua_error("128 resource operations per callback maximum"))
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
                    if handlers.values().map(Vec::len).sum::<usize>() >= MAX_CALLBACKS {
                        return Err(lua_error("64 event handlers maximum"));
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
                let payload = bounded_value(lua, payload, MAX_PAYLOAD)?;
                let mut shared = ctx.shared.lock().unwrap();
                if shared.events.len() >= MAX_ACTIONS {
                    return Err(lua_error("local event queue full"));
                }
                shared.events.push_back((
                    ctx.installed.manifest.id.clone(),
                    ctx.installed.generation,
                    name,
                    payload,
                ));
                Ok(())
            })?,
        )?;
        let ctx = self.clone();
        api.set(
            "send",
            lua.create_function(
                move |lua, (name, payload, recipient): (String, mlua::Value, mlua::Value)| {
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
                    let payload = bounded_value(lua, payload, MAX_PAYLOAD)?;
                    let mut shared = ctx.shared.lock().unwrap();
                    if shared.outputs.len() >= 4096 {
                        return Err(lua_error("resource output queue full"));
                    }
                    shared.outputs.push(Output::Event {
                        resource: ctx.installed.manifest.id.clone(),
                        generation: ctx.installed.generation,
                        recipient,
                        name,
                        payload,
                    });
                    Ok(())
                },
            )?,
        )?;
        let state = lua.create_table()?;
        let ctx = self.clone();
        state.set(
            "get",
            lua.create_function(move |lua, key: String| {
                ctx.require("resource.state")?;
                valid_name(&key).map_err(lua_error)?;
                let value = ctx
                    .shared
                    .lock()
                    .unwrap()
                    .states
                    .get(&ctx.installed.manifest.id)
                    .and_then(|s| s.get(&key))
                    .cloned()
                    .unwrap_or(Value::Null);
                json_value(lua, &value)
            })?,
        )?;
        let ctx = self.clone();
        state.set(
            "set",
            lua.create_function(move |lua, (key, value): (String, mlua::Value)| {
                if ctx.side != Side::Server {
                    return Err(lua_error("replicated state is owned by the server"));
                }
                ctx.require("resource.state")?;
                ctx.action()?;
                valid_name(&key).map_err(lua_error)?;
                let value = bounded_value(lua, value, MAX_PAYLOAD)?;
                let mut shared = ctx.shared.lock().unwrap();
                if shared.outputs.len() >= 4096 {
                    return Err(lua_error("resource output queue full"));
                }
                let values = shared
                    .states
                    .entry(ctx.installed.manifest.id.clone())
                    .or_default();
                if !value.is_null() && !values.contains_key(&key) && values.len() >= MAX_STATE_KEYS
                {
                    return Err(lua_error("64 state keys maximum"));
                }
                if value.is_null() {
                    values.remove(&key);
                } else {
                    values.insert(key.clone(), value.clone());
                }
                shared.outputs.push(Output::State {
                    resource: ctx.installed.manifest.id.clone(),
                    generation: ctx.installed.generation,
                    key,
                    value,
                });
                Ok(())
            })?,
        )?;
        api.set("state", state)?;
        self.install_storage(lua, &api)?;
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
                    let value = bounded_value(lua, value, 16 * 1024)?;
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
                        .store(crate::vm::LUA_BUDGET_UNITS, Ordering::Relaxed);
                    // The engine bridge is scoped to the *calling* resource.
                    // A callee may enqueue its own owned commands, but must not
                    // look up or mutate the caller's bodies through that bridge.
                    let result = crate::query::without_host(|| {
                        let argument = json_value(&export.lua, &value)?;
                        let result = export.function.call::<mlua::Value>(argument)?;
                        bounded_value(&export.lua, result, 16 * 1024)
                    });
                    {
                        let mut shared = ctx.shared.lock().unwrap();
                        shared.export_depth -= 1;
                        if let Err(error) = &result {
                            shared.faults.push((target, error.to_string()));
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
            if metadata.len() > MAX_STORAGE as u64 {
                return Err(lua_error("resource persistence exceeds 64 KiB"));
            }
            let bytes = std::fs::read(&self.storage).map_err(mlua::Error::external)?;
            serde_json::from_slice::<BTreeMap<String, Value>>(&bytes)
                .map_err(mlua::Error::external)?
        } else {
            BTreeMap::new()
        };
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
                let value = bounded_value(lua, value, 16 * 1024)?;
                let mut original = values.lock().unwrap();
                let mut next = original.clone();
                if value.is_null() {
                    next.remove(&key);
                } else {
                    next.insert(key, value);
                }
                if next.len() > 128 {
                    return Err(lua_error("128 persistence keys maximum"));
                }
                let bytes = serde_json::to_vec(&next).map_err(mlua::Error::external)?;
                if bytes.len() > MAX_STORAGE {
                    return Err(lua_error("resource persistence exceeds 64 KiB"));
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
fn bounded_value(lua: &Lua, value: mlua::Value, limit: usize) -> mlua::Result<Value> {
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
fn json_value(lua: &Lua, value: &Value) -> mlua::Result<mlua::Value> {
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
    storage: PathBuf,
    installed: BTreeMap<String, InstalledResource>,
    order: Vec<String>,
    instances: BTreeMap<String, Instance>,
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
        if scope.is_empty() || scope.len() > 2048 {
            return Err("persistence scope must contain 1..2048 bytes".into());
        }
        let scope = blake3::hash(format!("resource-storage-v1:{side:?}:{scope}").as_bytes())
            .to_hex()
            .to_string();
        Ok(Self {
            side,
            storage: storage_root.join(scope),
            installed: BTreeMap::new(),
            order: vec![],
            instances: BTreeMap::new(),
            started: BTreeSet::new(),
            shared: Arc::new(Mutex::new(Shared::default())),
            commands: vec![],
            retired: vec![],
            snapshot: Arc::new(Value::Null),
            fields: SnapshotFields::default(),
            diagnostics: vec![],
        })
    }
    pub fn side(&self) -> Side {
        self.side
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
    pub fn state(&self, id: &str, key: &str) -> Option<Value> {
        self.shared
            .lock()
            .unwrap()
            .states
            .get(id)
            .and_then(|s| s.get(key))
            .cloned()
    }
    pub fn states(&self) -> BTreeMap<String, BTreeMap<String, Value>> {
        self.shared.lock().unwrap().states.clone()
    }
    pub fn install(&mut self, resources: Vec<InstalledResource>) -> Result<(), String> {
        if !self.instances.is_empty() {
            return Err("stop resources before replacing the installed set".into());
        }
        if resources.len() > 128 {
            return Err("128 resources maximum".into());
        }
        let manifests: Vec<_> = resources.iter().map(|r| r.manifest.clone()).collect();
        let order = skate_resources::ordered_manifests(&manifests).map_err(|e| e.to_string())?;
        for r in &resources {
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
        Ok(())
    }
    /// Add stopped/new packages without disturbing live VMs. Running definitions
    /// must be identical; refresh their manifest only after an explicit stop.
    pub fn register(&mut self, resources: Vec<InstalledResource>) -> Result<(), String> {
        let mut installed = self.installed.clone();
        for resource in resources {
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
        if installed.len() > 128 {
            return Err("128 resources maximum".into());
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
        let installed = self.installed.get_mut(id).unwrap();
        if self.started.contains(id) {
            installed.generation = installed
                .generation
                .checked_add(1)
                .ok_or("resource generation exhausted")?;
        }
        self.started.insert(id.to_string());
        let bootstrap = Bootstrap {
            installed: installed.clone(),
            side: self.side,
            storage: self.storage.join(format!("{id}.json")),
            shared: self.shared.clone(),
            handlers: Arc::new(Mutex::new(BTreeMap::new())),
            actions: Arc::new(AtomicUsize::new(0)),
        };
        let vm = match Vm::new_resource(
            &bootstrap.installed.root,
            &bootstrap.installed.manifest.id,
            &self.snapshot,
            &bootstrap,
        ) {
            Ok(vm) => vm,
            Err(error) => {
                self.cleanup(id);
                self.diagnostics.push(format!("{id} startup: {error}"));
                return Err(format!("{id} startup: {error}"));
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
        shared.states.remove(id);
        shared.events.retain(|(owner, _, _, _)| owner != id);
        shared.outputs.retain(|output| match output {
            Output::Event { resource, .. }
            | Output::State { resource, .. }
            | Output::Log { resource, .. } => resource != id,
        });
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
    pub fn fail(&mut self, id: &str, error: String) {
        self.diagnostics.push(format!("{id}: {error}"));
        if self.diagnostics.len() > 128 {
            self.diagnostics.remove(0);
        }
        let _ = self.stop(id);
        self.retire_export_failures();
    }
    fn retire_export_failures(&mut self) {
        let faults = std::mem::take(&mut self.shared.lock().unwrap().faults);
        for (id, error) in faults {
            self.diagnostics.push(format!("{id} export: {error}"));
            let _ = self.stop(&id);
        }
        if self.diagnostics.len() > 128 {
            self.diagnostics.drain(..self.diagnostics.len() - 128);
        }
    }
    pub fn disconnect(&mut self) {
        for id in self.order.clone().into_iter().rev() {
            self.stop_one(&id);
        }
        self.shared.lock().unwrap().outputs.clear();
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
        for id in self.running_ids() {
            self.call(&id, callback, payload.clone());
        }
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
            self.diagnostics.push(error);
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
                .call_shared(callback, payload, &self.snapshot, physics, &self.fields)?;
        self.accept_commands(id, commands);
        Ok(())
    }
    fn accept_commands(&mut self, id: &str, commands: Vec<Command>) {
        for command in commands {
            if let Command::Log { text } = &command {
                let mut shared = self.shared.lock().unwrap();
                if shared.outputs.len() < 4096 {
                    shared.outputs.push(Output::Log {
                        resource: id.into(),
                        text: text.clone(),
                    });
                }
            }
            if self.side == Side::Client {
                self.commands.push((id.into(), command));
            }
        }
    }
    fn pump_events(&mut self) -> Result<(), String> {
        self.retire_export_failures();
        for _ in 0..MAX_ACTIONS {
            let event = self.shared.lock().unwrap().events.pop_front();
            let Some((id, generation, name, payload)) = event else {
                return Ok(());
            };
            if self.generation(&id) == Some(generation) && self.running(&id) {
                if let Err(error) = self.event(&id, &name, payload, 0, false) {
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
        let instance = self
            .instances
            .get_mut(id)
            .ok_or_else(|| format!("resource {id} is not running"))?;
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
            handlers.iter().map(|h| h.function.clone()).collect(),
            payload,
            sender,
            &self.snapshot,
            &self.fields,
        )?;
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
        validate_value(&payload, MAX_PAYLOAD)?;
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
    /// The transport must authenticate this as server data before invoking it.
    pub fn apply_state(
        &mut self,
        resource: &str,
        generation: u64,
        key: &str,
        value: Value,
    ) -> Result<(), String> {
        if self.side != Side::Client {
            return Err("only clients consume replicated server state".into());
        }
        valid_name(key)?;
        validate_value(&value, MAX_PAYLOAD)?;
        if self.generation(resource) != Some(generation) || !self.running(resource) {
            return Err("stale or stopped resource generation".into());
        }
        let mut shared = self.shared.lock().unwrap();
        let state = shared.states.entry(resource.into()).or_default();
        if !value.is_null() && !state.contains_key(key) && state.len() >= MAX_STATE_KEYS {
            return Err("64 state keys maximum".into());
        }
        if value.is_null() {
            state.remove(key);
        } else {
            state.insert(key.into(), value);
        }
        Ok(())
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
            vec![command.function],
            payload,
            actor,
            &self.snapshot,
            &self.fields,
        ) {
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
        std::mem::take(&mut self.shared.lock().unwrap().outputs)
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
            | "resource.storage"
            | "resource.commands"
            | "resource.exports"
            | "engine.ui"
            | "engine.audio"
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
