//! Process-isolated C# source resources. No resource assembly executes in Rust.
use crate::resources::{RuntimeLimits, bounded_value, json_value};
use mlua::{Lua, LuaSerdeExt, MultiValue, Table};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::{IsolationGuard, WorkerProcess, spawn_worker};
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::{IsolationGuard, WorkerProcess, spawn_worker};
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
struct IsolationGuard;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
struct WorkerProcess {
    stdin: Option<Box<dyn Write + Send>>,
    stdout: Option<Box<dyn Read + Send>>,
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
impl WorkerProcess {
    fn id(&self) -> u32 {
        0
    }
    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    fn wait(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn spawn_worker(_: &std::path::Path, _: &std::path::Path, _: usize) -> Result<(WorkerProcess, IsolationGuard), String> {
    Err("C# isolation is unavailable on this platform".into())
}

const MAX_FRAME: usize = 8 * 1024 * 1024;
fn error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

struct Process {
    child: WorkerProcess,
    _isolation: IsolationGuard,
    incoming: Receiver<Result<Vec<u8>, String>>,
    outgoing: SyncSender<Vec<u8>>,
    retired: bool,
}
impl Process {
    fn stop(&mut self) {
        self.retired = true;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct Managed {
    process: Mutex<Process>,
    lua: mlua::WeakLua,
    registrations: Mutex<BTreeMap<u32, (String, String)>>,
    limits: RuntimeLimits,
    budget: Arc<AtomicUsize>,
    metrics: crate::runtime_metrics::Counters,
}

impl Managed {
    pub(crate) fn resident_bytes(&self)->Option<usize> {
        #[cfg(target_os="linux")]
        { let process=self.process.try_lock().ok()?; if process.retired {None} else {Some(linux::resident_bytes(process.child.id()))} }
        #[cfg(not(target_os="linux"))]
        { None }
    }
    pub(crate) fn new(
        lua: &Lua,
        callbacks: &Table,
        limits: &RuntimeLimits,
        budget: Arc<AtomicUsize>,
        sources: Vec<(String, String)>,
        metrics: crate::runtime_metrics::Counters,
    ) -> mlua::Result<Arc<Self>> {
        let dotnet = std::env::var_os("SKATE_DOTNET_ROOT")
            .map(PathBuf::from)
            .ok_or_else(|| error("C# resources require host-configured SKATE_DOTNET_ROOT"))?;
        let worker = std::env::var_os("SKATE_MANAGED_HOST")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.join("managed-host")))
            })
            .ok_or_else(|| error("C# resources require host-configured SKATE_MANAGED_HOST"))?;
        let dotnet = dotnet.canonicalize().map_err(mlua::Error::external)?;
        let worker = worker.canonicalize().map_err(mlua::Error::external)?;
        if !worker.join("Skate.ResourceHost.dll").is_file() {
            return Err(error("trusted managed resource worker is missing"));
        }
        if sources.len() > 128
            || sources.iter().map(|(_, s)| s.len()).sum::<usize>() > 4 * 1024 * 1024
        {
            return Err(error("C# resource source budget exceeded"));
        }
        let (mut child, isolation) =
            spawn_worker(&dotnet, &worker, limits.managed_memory_bytes).map_err(error)?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| error("managed stdin unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| error("managed stdout unavailable"))?;
        let (tx, incoming) = mpsc::sync_channel(1);
        let reader=std::thread::Builder::new().name("managed-read".into()).spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut frame = Vec::new();
                let read = (&mut output)
                    .take(MAX_FRAME as u64)
                    .read_until(b'\n', &mut frame);
                let result = match read {
                    Ok(0) => Err("managed worker exited".into()),
                    Ok(_) if frame.last() != Some(&b'\n') => {
                        Err("managed IPC frame exceeds limit".into())
                    }
                    Ok(_) => Ok(frame),
                    Err(e) => Err(format!("managed IPC read: {e}")),
                };
                let failed = result.is_err();
                if tx.send(result).is_err() || failed {
                    break;
                }
            }
        });
        if let Err(e)=reader {let _=child.kill();let _=child.wait();return Err(error(format!("managed reader thread: {e}")));}
        let (outgoing, rx) = mpsc::sync_channel::<Vec<u8>>(1);
        let writer=std::thread::Builder::new().name("managed-write".into()).spawn(move || {
            let mut input = input;
            while let Ok(bytes) = rx.recv() {
                if input.write_all(&bytes).and_then(|_| input.flush()).is_err() {
                    break;
                }
            }
        });
        if let Err(e)=writer {let _=child.kill();let _=child.wait();return Err(error(format!("managed writer thread: {e}")));}
        let managed = Arc::new(Self {
            process: Mutex::new(Process {
                child,
                _isolation: isolation,
                incoming,
                outgoing,
                retired: false,
            }),
            lua: lua.weak(),
            registrations: Mutex::new(BTreeMap::new()),
            limits: limits.clone(),
            budget,
            metrics,
        });
        lua.globals().set("_managed_callbacks", callbacks.clone())?;
        let resource: Table = lua.globals().get("resource")?;
        let metadata = json!({"id":resource.get::<String>("id")?,"version":resource.get::<String>("version")?,"side":resource.get::<String>("side")?,"generation":resource.get::<String>("generation")?,
            "grants":lua.from_value::<Value>(mlua::Value::Table(resource.get("grants")?))?});
        managed.execute(json!({"op":"init","sources":sources.into_iter().map(|(name,code)|json!({"name":name,"code":code})).collect::<Vec<_>>(),
            "metadata":metadata,"maxCallbacks":limits.max_callbacks,"maxValueBytes":limits.max_storage_value_bytes}),
            Duration::from_millis(limits.managed_startup_timeout_ms as u64))?;
        Ok(managed)
    }

    fn execute(self: &Arc<Self>, mut request: Value, timeout: Duration) -> mlua::Result<Value> {
        let mut process = self
            .process
            .try_lock()
            .map_err(|_| error("C# resource cannot synchronously re-enter its own worker"))?;
        if process.retired {
            return Err(error("C# resource worker retired"));
        }
        let phase=format!("ipc:{}",request.get("op").and_then(Value::as_str).unwrap_or("unknown"));
        let mut timer=crate::runtime_metrics::Timer::start_for(&self.metrics,&phase,None);
        request["profile"]=Value::Bool(timer.profiling());
        timer.ipc_receive_wait_us=Some(0);
        let result = self.exchange(&mut process, request, timeout,&mut timer);
        timer.finish_profile(result.is_err());
        if result.is_err() {
            process.stop();
        }
        result
    }
    fn send(process: &Process, value: &Value) -> mlua::Result<()> {
        let mut bytes = serde_json::to_vec(value).map_err(mlua::Error::external)?;
        if bytes.len() >= MAX_FRAME {
            return Err(error("managed IPC frame exceeds limit"));
        }
        bytes.push(b'\n');
        process
            .outgoing
            .try_send(bytes)
            .map_err(|_| error("managed IPC output queue full or disconnected"))
    }
    fn exchange(
        self: &Arc<Self>,
        process: &mut Process,
        request: Value,
        timeout: Duration,
        timer: &mut crate::runtime_metrics::Timer,
    ) -> mlua::Result<Value> {
        Self::send(process, &request)?;
        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() >= deadline {
                return Err(error("C# resource execution deadline exceeded"));
            }
            #[cfg(target_os = "linux")]
            if linux::resident_bytes(process.child.id()) > self.limits.managed_memory_bytes {
                return Err(error("C# resource process memory limit exceeded"));
            }
            let waiting=Instant::now();
            let received=process.incoming.recv_timeout(
                (deadline - Instant::now().min(deadline)).min(Duration::from_millis(10)),
            );
            timer.ipc_receive_wait_us=Some(timer.ipc_receive_wait_us.unwrap_or(0).saturating_add(waiting.elapsed().as_micros().min(u64::MAX as u128) as u64));
            let frame = match received {
                Ok(value) => value.map_err(error)?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err(error("managed IPC disconnected")),
            };
            if self
                .budget
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                .is_err()
            {
                return Err(error("C# host operation budget exhausted"));
            }
            let message: Value = serde_json::from_slice(&frame).map_err(mlua::Error::external)?;
            match message.get("op").and_then(Value::as_str) {
                Some("done") => {
                    timer.worker_cpu_us=message.get("workerCpuUs").and_then(Value::as_u64).filter(|v|*v<=timeout.as_micros().min(u64::MAX as u128) as u64*256);
                    if message.get("ok").and_then(Value::as_bool) != Some(true) {
                        return Err(error(format!(
                            "C#: {}",
                            message
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("resource failed")
                        )));
                    }
                    let value = message.get("value").cloned().unwrap_or(Value::Null);
                    if serde_json::to_vec(&value)
                        .map_err(mlua::Error::external)?
                        .len()
                        > self.limits.max_storage_value_bytes
                    {
                        return Err(error("C# result exceeds byte limit"));
                    }
                    return Ok(value);
                }
                Some(op @ ("call" | "register")) => {
                    let id = message
                        .get("request")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| error("invalid managed IPC request id"))?;
                    let result = if op == "call" {
                        self.host_call(&message)
                    } else {
                        self.register(&message).map(|_| Value::Null)
                    };
                    let reply = match result {
                        Ok(value) => json!({"op":"return","request":id,"ok":true,"value":value}),
                        Err(e) => {
                            json!({"op":"return","request":id,"ok":false,"error":e.to_string().chars().take(2048).collect::<String>()})
                        }
                    };
                    Self::send(process, &reply)?;
                }
                _ => return Err(error("invalid managed IPC operation")),
            }
        }
    }

    fn host_call(&self, message: &Value) -> mlua::Result<Value> {
        let lua = self
            .lua
            .try_upgrade()
            .ok_or_else(|| error("resource host retired"))?;
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| error("invalid managed host method"))?;
        if !matches!(
            method,
            "resource.emit"
                | "resource.send"
                | "resource.off"
                | "resource.state.get"
                | "resource.state.set"
                | "resource.settings.get"
                                        | "resource.settings.all"
                                        | "resource.storage.get"
                | "resource.storage.set"
                | "resource.call"
                | "resource.players"
                | "resource.teleport"
                | "resource.entities.command"
                | "resource.entities.all"
                | "resource.voice.submit"
                | "resource.world.command"
                | "resource.competition.submit"
                | "resource.transfer.start"
                | "resource.transfer.cancel"
                | "resource.services.submit"
                | "resource.services.cancel"
                | "sdk.submit"
                | "sdk.log"
                | "sdk.read_text"
                | "sdk.ui.text"
                | "sdk.ui.remove"
                | "sdk.ui.menu"
                | "sdk.ui.canvas"
        ) {
            return Err(error("unsupported C# host method"));
        }
        let args = message
            .get("args")
            .and_then(Value::as_array)
            .filter(|a| a.len() <= 8)
            .ok_or_else(|| error("invalid managed host arguments"))?;
        if serde_json::to_vec(args)
            .map_err(mlua::Error::external)?
            .len()
            > self.limits.max_storage_value_bytes * 2 + 1024
        {
            return Err(error("managed host arguments exceed limit"));
        }
        let parts: Vec<_> = method.split('.').collect();
        let mut table = lua.globals();
        for part in &parts[..parts.len() - 1] {
            table = table.get::<Table>(*part)?;
        }
        let function: mlua::Function = table.get(parts[parts.len() - 1])?;
        let values = args
            .iter()
            .map(|v| json_value(&lua, v))
            .collect::<mlua::Result<Vec<_>>>()?;
        let result = function.call::<mlua::Value>(MultiValue::from_vec(values))?;
        if method == "resource.off" {
            if let Some(name) = args.first().and_then(Value::as_str) {
                self.registrations
                    .lock()
                    .unwrap()
                    .retain(|_, (kind, event)| {
                        !(matches!(kind.as_str(), "on" | "on_net") && event == name)
                    });
            }
        }
        bounded_value(&lua, result, self.limits.max_storage_value_bytes)
    }
    fn register(self: &Arc<Self>, message: &Value) -> mlua::Result<()> {
        let lua = self
            .lua
            .try_upgrade()
            .ok_or_else(|| error("resource host retired"))?;
        let field = |name: &str| {
            message
                .get(name)
                .and_then(Value::as_str)
                .filter(|s| s.len() <= 96)
                .map(str::to_owned)
                .ok_or_else(|| error("invalid managed registration"))
        };
        let kind = field("kind")?;
        let name = field("name")?;
        let permission = field("permission")?;
        let id = message
            .get("callback")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| error("invalid managed callback ID"))?;
        let replacing = kind == "export"
            && self
                .registrations
                .lock()
                .unwrap()
                .values()
                .any(|v| v == &(kind.clone(), name.clone()));
        if self.registrations.lock().unwrap().contains_key(&id)
            || (self.registrations.lock().unwrap().len() >= self.limits.max_callbacks + 8
                && !replacing)
        {
            return Err(error("C# callback limit reached"));
        }
        let weak = Arc::downgrade(self);
        let function = lua.create_function(move |lua, args: MultiValue| {
            let managed = weak.upgrade().ok_or_else(|| error("C# resource retired"))?;
            let args = args
                .into_iter()
                .map(|v| {
                    if v == mlua::Value::NULL {
                        Ok(Value::Null)
                    } else {
                        bounded_value(lua, v, managed.limits.max_storage_value_bytes)
                    }
                })
                .collect::<mlua::Result<Vec<_>>>()?;
            let result = managed.execute(
                json!({"op":"invoke","callback":id,"args":args}),
                Duration::from_millis(managed.limits.managed_callback_timeout_ms as u64),
            )?;
            json_value(lua, &result)
        })?;
        if kind == "lifecycle" {
            if ![
                "on_load",
                "on_unload",
                "on_update",
                "on_fixed_update",
                "on_ui_update",
                "on_event",
                "on_settings",
            ]
            .contains(&name.as_str())
            {
                return Err(error("unknown C# lifecycle callback"));
            }
            let callbacks: Table = lua.globals().get("_managed_callbacks")?;
            if let Some(previous) = callbacks.get::<Option<mlua::Function>>(name.clone())? {
                callbacks.set(
                    name.clone(),
                    lua.create_function(move |_, args: MultiValue| {
                        previous.call::<()>(args.clone())?;
                        function.call::<()>(args)
                    })?,
                )?;
            } else {
                callbacks.set(name.clone(), function)?;
            }
        } else {
            let api: Table = lua.globals().get("resource")?;
            match kind.as_str() {
                "on" | "on_net" | "export" => api
                    .get::<mlua::Function>(kind.clone())?
                    .call::<()>((name.clone(), function))?,
                "command" => api.get::<mlua::Function>("command")?.call::<()>((
                    name.clone(),
                    permission,
                    function,
                ))?,
                _ => return Err(error("unknown C# callback kind")),
            }
        }
        let mut live = self.registrations.lock().unwrap();
        if replacing {
            live.retain(|_, v| v != &(kind.clone(), name.clone()));
        }
        live.insert(id, (kind, name));
        Ok(())
    }
}
