//! Lua resource map publication using the existing server-owned state channel.
use super::*;
use crate::map::{MAX_BYTES, MapSnapshot, STATE_KEY};

impl Bootstrap {
    pub(super) fn install_map(&self, lua: &Lua, api: &Table) -> mlua::Result<()> {
        let map = lua.create_table()?;
        map.set("version", 1)?;
        let ctx = self.clone();
        map.set(
            "set",
            lua.create_function(move |lua, value: mlua::Value| ctx.publish_map(lua, value))?,
        )?;
        let ctx = self.clone();
        map.set(
            "clear",
            lua.create_function(move |lua, (): ()| ctx.publish_map(lua, mlua::Value::Nil))?,
        )?;
        let ctx = self.clone();
        map.set(
            "get",
            lua.create_function(move |lua, (): ()| {
                ctx.require("resource.map")?;
                let value = ctx
                    .shared
                    .lock()
                    .unwrap()
                    .states
                    .get(&(ctx.installed.manifest.id.clone(), "resource".into()))
                    .and_then(|s| s.get(STATE_KEY))
                    .cloned()
                    .unwrap_or(Value::Null);
                json_value(lua, &value)
            })?,
        )?;
        api.set("map", map)
    }
    fn publish_map(&self, lua: &Lua, value: mlua::Value) -> mlua::Result<()> {
        if self.side != Side::Server {
            return Err(lua_error(
                "map publication is server-only; use sdk.map for local layers",
            ));
        }
        self.require("resource.map")?;
        self.action()?;
        let raw = bounded_value(lua, value, MAX_BYTES.min(self.limits.max_payload_bytes))?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            let state = MapSnapshot::parse(raw).map_err(lua_error)?;
            serde_json::to_value(state).map_err(mlua::Error::external)?
        };
        let id = &self.installed.manifest.id;
        let output = Output::State {
            resource: id.clone(),
            generation: self.installed.generation,
            key: STATE_KEY.into(),
            value: value.clone(),
            scope: resource_scope(),
        };
        let mut shared = self.shared.lock().unwrap();
        if shared.outputs.len() >= self.limits.max_queued_outputs
            || shared
                .output_bytes
                .saturating_add(Shared::output_size(&output))
                > self.limits.max_queued_output_bytes
        {
            return Err(lua_error("resource output queue full"));
        }
        let now = std::time::Instant::now();
        let recent = shared.map_publications.entry(id.clone()).or_default();
        recent.retain(|t| now.duration_since(*t) < std::time::Duration::from_secs(1));
        if recent.len() >= 5 {
            return Err(lua_error("5 map publications per second maximum"));
        }
        shared
            .set_state(id, "resource", STATE_KEY, value, self.limits.max_state_keys)
            .map_err(lua_error)?;
        shared.push_output(output, &self.limits)?;
        shared
            .map_publications
            .entry(id.clone())
            .or_default()
            .push_back(now);
        Ok(())
    }
}
