//! Lua resource location publication using the existing server-owned state channel.
use super::*;
use skate_resources::locations::{
    LocationSnapshot as MapSnapshot, MAX_SNAPSHOT_BYTES as MAX_BYTES,
};
const STATE_KEY: &str = "__locations_v1";

impl Bootstrap {
    pub(super) fn install_locations(&self, lua: &Lua, api: &Table) -> mlua::Result<()> {
        let locations = lua.create_table()?;
        locations.set("version", 1)?;
        let ctx = self.clone();
        locations.set(
            "set",
            lua.create_function(move |lua, value: mlua::Value| ctx.publish_locations(lua, value))?,
        )?;
        let ctx = self.clone();
        locations.set(
            "clear",
            lua.create_function(move |lua, (): ()| ctx.publish_locations(lua, mlua::Value::Nil))?,
        )?;
        let ctx = self.clone();
        locations.set(
            "get",
            lua.create_function(move |lua, (): ()| {
                ctx.require("resource.locations")?;
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
        api.set("locations", locations)
    }
    fn publish_locations(&self, lua: &Lua, value: mlua::Value) -> mlua::Result<()> {
        if self.side != Side::Server {
            return Err(lua_error(
                "location publication is server-only; use sdk.locations for local layers",
            ));
        }
        self.require("resource.locations")?;
        self.action()?;
        let raw = bounded_value(lua, value, MAX_BYTES.min(self.limits.max_payload_bytes))?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            let state = MapSnapshot::parse(raw).map_err(lua_error)?;
            if state.generation != self.installed.generation.to_string() {
                return Err(lua_error("location generation does not match owner"));
            }
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
        let recent = shared.location_publications.entry(id.clone()).or_default();
        recent.retain(|t| now.duration_since(*t) < std::time::Duration::from_secs(1));
        if recent.len() >= 5 {
            return Err(lua_error("5 location publications per second maximum"));
        }
        shared
            .set_state(id, "resource", STATE_KEY, value, self.limits.max_state_keys)
            .map_err(lua_error)?;
        shared.push_output(output, &self.limits)?;
        shared
            .location_publications
            .entry(id.clone())
            .or_default()
            .push_back(now);
        Ok(())
    }
}
