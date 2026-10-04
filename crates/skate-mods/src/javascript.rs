//! QuickJS resources reuse the capability-checked resource host, including its
//! export registry. No QuickJS OS library or module loader is installed.
use crate::resources::{RuntimeLimits, bounded_value, json_value};
use mlua::{Lua, LuaSerdeExt, MultiValue, Table};
use rquickjs::{Context, Ctx, Exception, Function, Runtime, function::Func};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicUsize, Ordering},
};

pub(crate) struct JavaScript {
    // Context must be dropped before its owning runtime.
    context: Context,
    runtime: Runtime,
    budget: Arc<AtomicUsize>,
    max_value: usize,
    unhandled_rejections: Arc<AtomicUsize>,
}

fn lua_error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}
fn js_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if matches!(error, rquickjs::Error::Exception) {
        let value = ctx.catch();
        if let Some(exception) = value.as_exception() {
            format!(
                "JavaScript: {}\n{}",
                exception.message().unwrap_or_default(),
                exception.stack().unwrap_or_default()
            )
        } else {
            format!("JavaScript exception: {value:?}")
        }
    } else {
        format!("JavaScript: {error}")
    }
}

impl JavaScript {
    pub(crate) fn used_memory(&self)->usize { self.runtime.memory_usage().memory_used_size.max(0) as usize }
    pub(crate) fn new(
        lua: &Lua,
        callbacks: &Table,
        limits: &RuntimeLimits,
        budget: Arc<AtomicUsize>,
    ) -> mlua::Result<Arc<Self>> {
        let runtime = Runtime::new().map_err(|e| lua_error(e.to_string()))?;
        runtime.set_memory_limit(limits.lua_memory_bytes);
        runtime.set_max_stack_size(256 * 1024);
        let counter = budget.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            counter
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                .is_err()
        })));
        let unhandled_rejections = Arc::new(AtomicUsize::new(0));
        let rejections = unhandled_rejections.clone();
        runtime.set_host_promise_rejection_tracker(Some(Box::new(move |_, _, _, handled| {
            if handled {
                let _ = rejections.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    Some(n.saturating_sub(1))
                });
            } else {
                rejections.fetch_add(1, Ordering::Relaxed);
            }
        })));
        let context = Context::full(&runtime).map_err(|e| lua_error(e.to_string()))?;
        let js = Arc::new(Self {
            context,
            runtime,
            budget,
            max_value: limits.max_storage_value_bytes,
            unhandled_rejections,
        });
        // JS native functions only hold weak Lua references; Lua callbacks only
        // hold weak JS references. Stopped resources cannot form a VM cycle.
        let weak_lua = lua.weak();
        let weak_js = Arc::downgrade(&js);
        let max_value = limits.max_storage_value_bytes;
        let registrations = Arc::new(Mutex::new(BTreeMap::<u32, (String, String)>::new()));
        let max_registrations = limits.max_callbacks + 8;
        lua.globals().set("_js_callbacks", callbacks.clone())?;
        let metadata = {
            let resource: Table = lua.globals().get("resource")?;
            json!({"id":resource.get::<String>("id")?,"version":resource.get::<String>("version")?,
                "side":resource.get::<String>("side")?,"generation":resource.get::<String>("generation")?,
                "grants":lua.from_value::<Value>(mlua::Value::Table(resource.get("grants")?))?,
                "maxCallbacks":limits.max_callbacks,"maxThreads":limits.max_threads,"maxValueBytes":max_value})
        };
        js.context
            .with(|ctx| {
                let host_lua = weak_lua.clone();
                let live_callbacks = registrations.clone();
                ctx.globals().set(
                    "__resource_host",
                    Func::from(
                        move |ctx: Ctx<'_>,
                              operation: String,
                              args: String|
                              -> rquickjs::Result<String> {
                            let result = (|| -> mlua::Result<String> {
                                let lua = host_lua
                                    .try_upgrade()
                                    .ok_or_else(|| lua_error("resource host retired"))?;
                                if args.len() > max_value.saturating_mul(2) + 1024 {
                                    return Err(lua_error("host arguments exceed byte limit"));
                                }
                                let args: Vec<Value> =
                                    serde_json::from_str(&args).map_err(mlua::Error::external)?;
                                if args.len() > 8 {
                                    return Err(lua_error("too many host arguments"));
                                }
                                let parts: Vec<&str> = operation.split('.').collect();
                                if !matches!(
                                    operation.as_str(),
                                    "resource.emit"
                                        | "resource.send"
                                        | "resource.off"
                                        | "resource.state.get"
                                        | "resource.state.set"
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
                                    return Err(lua_error("unsupported JavaScript host method"));
                                }
                                let mut table = lua.globals();
                                for part in &parts[..parts.len() - 1] {
                                    table = table.get::<Table>(*part)?;
                                }
                                let function: mlua::Function = table.get(parts[parts.len() - 1])?;
                                let remove_name = if operation == "resource.off" {
                                    args.first().and_then(Value::as_str).map(str::to_owned)
                                } else {
                                    None
                                };
                                let args = args
                                    .iter()
                                    .map(|v| json_value(&lua, v))
                                    .collect::<mlua::Result<Vec<_>>>()?;
                                let value =
                                    function.call::<mlua::Value>(MultiValue::from_vec(args))?;
                                if let Some(name) = remove_name {
                                    live_callbacks.lock().unwrap().retain(|_, (kind, event)| {
                                        !(matches!(kind.as_str(), "on" | "on_net")
                                            && *event == name)
                                    });
                                }
                                serde_json::to_string(&bounded_value(&lua, value, max_value)?)
                                    .map_err(mlua::Error::external)
                            })();
                            result
                                .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
                        },
                    ),
                )?;
                ctx.globals().set(
                    "__resource_register",
                    Func::from(
                        move |ctx: Ctx<'_>,
                              kind: String,
                              name: String,
                              callback: u32,
                              permission: String|
                              -> rquickjs::Result<()> {
                            let result = (|| -> mlua::Result<()> {
                                let lua = weak_lua
                                    .try_upgrade()
                                    .ok_or_else(|| lua_error("resource host retired"))?;
                                let replacing_export = kind == "export"
                                    && registrations
                                        .lock()
                                        .unwrap()
                                        .values()
                                        .any(|(k, n)| k == "export" && *n == name);
                                if registrations.lock().unwrap().len() >= max_registrations
                                    && !replacing_export
                                {
                                    return Err(lua_error("JavaScript callback limit reached"));
                                }
                                let registration = (kind.clone(), name.clone());
                                let weak_js: Weak<JavaScript> = weak_js.clone();
                                let function =
                                    lua.create_function(move |lua, args: MultiValue| {
                                        let js = weak_js.upgrade().ok_or_else(|| {
                                            lua_error("JavaScript resource retired")
                                        })?;
                                        let args = args
                                            .into_iter()
                                            .map(|v| {
                                                if v == mlua::Value::NULL {
                                                    Ok(Value::Null)
                                                } else {
                                                    bounded_value(lua, v, js.max_value)
                                                }
                                            })
                                            .collect::<mlua::Result<Vec<_>>>()?;
                                        let result = js.invoke(callback, args)?;
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
                                        return Err(lua_error(
                                            "unknown JavaScript lifecycle callback",
                                        ));
                                    }
                                    let callbacks: Table = lua.globals().get("_js_callbacks")?;
                                    if let Some(previous) =
                                        callbacks.get::<Option<mlua::Function>>(name.clone())?
                                    {
                                        callbacks.set(
                                            name,
                                            lua.create_function(move |_, args: MultiValue| {
                                                previous.call::<()>(args.clone())?;
                                                function.call::<()>(args)
                                            })?,
                                        )?;
                                    } else {
                                        callbacks.set(name, function)?;
                                    }
                                } else {
                                    let api: Table = lua.globals().get("resource")?;
                                    match kind.as_str() {
                                        "on" | "on_net" | "export" => api
                                            .get::<mlua::Function>(kind)?
                                            .call::<()>((name, function))?,
                                        "command" => api
                                            .get::<mlua::Function>("command")?
                                            .call::<()>((name, permission, function))?,
                                        _ => {
                                            return Err(lua_error(
                                                "unknown JavaScript callback kind",
                                            ));
                                        }
                                    }
                                }
                                let mut live = registrations.lock().unwrap();
                                if replacing_export {
                                    live.retain(|_, entry| entry != &registration);
                                }
                                live.insert(callback, registration);
                                Ok(())
                            })();
                            result
                                .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
                        },
                    ),
                )?;
                ctx.globals().set(
                    "__resource_metadata",
                    serde_json::to_string(&metadata).unwrap(),
                )?;
                ctx.eval::<(), _>(include_str!("resource_api.js"))
                    .map_err(|error| Exception::throw_message(&ctx, &js_error(&ctx, error)))
            })
            .map_err(|error| js.context.with(|ctx| lua_error(js_error(&ctx, error))))?;
        Ok(js)
    }
    pub(crate) fn evaluate(&self, source: &str) -> mlua::Result<()> {
        self.context.with(|ctx| {
            ctx.eval::<(), _>(source)
                .map_err(|error| self.execution_error(&ctx, error))
        })?;
        self.jobs()
    }
    fn invoke(&self, callback: u32, args: Vec<Value>) -> mlua::Result<Value> {
        let result = self.context.with(|ctx| {
            let result = (|| -> rquickjs::Result<String> {
                let dispatch: Function = ctx.globals().get("__resource_dispatch")?;
                dispatch.call((callback, serde_json::to_string(&args).unwrap()))
            })();
            result.map_err(|error| self.execution_error(&ctx, error))
        })?;
        if result.len() > self.max_value {
            return Err(lua_error("JavaScript result exceeds byte limit"));
        }
        let result = serde_json::from_str(&result).map_err(mlua::Error::external)?;
        self.jobs()?;
        Ok(result)
    }
    fn jobs(&self) -> mlua::Result<()> {
        // Promise chains with tiny individual jobs must not evade interrupt
        // accounting by continually resetting the engine's instruction counter.
        for _ in 0..256 {
            if !self.runtime.is_job_pending() {
                return self.check_completion();
            }
            if self
                .budget
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
                .is_err()
            {
                return Err(lua_error("JavaScript instruction budget exhausted"));
            }
            self.runtime.execute_pending_job().map_err(|error| {
                error
                    .0
                    .with(|ctx| lua_error(js_error(&ctx, rquickjs::Error::Exception)))
            })?;
        }
        if self.runtime.is_job_pending() {
            Err(lua_error("JavaScript pending job budget exhausted"))
        } else {
            self.check_completion()
        }
    }
    fn execution_error(&self, ctx: &Ctx<'_>, error: rquickjs::Error) -> mlua::Error {
        let detail = js_error(ctx, error);
        if self.budget.load(Ordering::Relaxed) == 0 {
            lua_error(format!("JavaScript instruction budget exhausted: {detail}"))
        } else {
            lua_error(detail)
        }
    }
    fn check_completion(&self) -> mlua::Result<()> {
        self.check_budget()?;
        if self.unhandled_rejections.load(Ordering::Relaxed) > 0 {
            Err(lua_error("unhandled JavaScript Promise rejection"))
        } else {
            Ok(())
        }
    }
    fn check_budget(&self) -> mlua::Result<()> {
        if self.budget.load(Ordering::Relaxed) == 0 {
            Err(lua_error("JavaScript instruction budget exhausted"))
        } else {
            Ok(())
        }
    }
}
