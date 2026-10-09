//! Native layer ownership and map-generation lifetime.
use bevy::prelude::*;
use skate_mods::map::{MapSettings, MapSnapshot, validate_snapshot};
use std::collections::BTreeMap;

#[derive(Clone)]
pub(super) struct Entry {
    pub resource_generation: u64,
    pub map_generation: u64,
    pub snapshot: MapSnapshot,
}
#[derive(Resource, Default)]
pub(super) struct MapLayerRegistry {
    pub entries: BTreeMap<(bool, String), Entry>,
}
impl MapLayerRegistry {
    pub fn set(
        &mut self,
        owner: &str,
        server: bool,
        resource_generation: u64,
        map_generation: u64,
        snapshot: MapSnapshot,
    ) -> Result<(), String> {
        validate_snapshot(&snapshot)?;
        let key = (server, owner.to_owned());
        if self
            .entries
            .get(&key)
            .is_some_and(|e| e.resource_generation > resource_generation)
        {
            return Err("stale map resource generation".into());
        }
        self.entries.insert(
            key,
            Entry {
                resource_generation,
                map_generation,
                snapshot,
            },
        );
        Ok(())
    }
    pub fn remove(&mut self, owner: &str, server: Option<bool>) {
        self.entries
            .retain(|(s, id), _| id != owner || server.is_some_and(|v| *s != v));
    }
    pub fn retain_generation(&mut self, generation: u64) {
        self.entries.retain(|_, e| e.map_generation == generation);
    }
    pub fn settings(&self) -> MapSettings {
        let mut settings = MapSettings::default();
        // Local defaults first; server defaults win. Conflicting server defaults
        // are ordered by canonical resource id, while all layers retain ownership.
        for e in self.entries.values() {
            let s = &e.snapshot.settings;
            if s.enabled.is_some() {
                settings.enabled = s.enabled;
            }
            if s.opacity.is_some() {
                settings.opacity = s.opacity;
            }
            if s.initial_zoom.is_some() {
                settings.initial_zoom = s.initial_zoom;
            }
            if s.title.is_some() {
                settings.title = s.title.clone();
            }
        }
        settings
    }
}
fn generation(world: &World) -> u64 {
    world
        .get_resource::<crate::map_transition::CurrentMap>()
        .map_or(0, |m| m.generation)
}
pub(crate) fn set_local(
    world: &mut World,
    owner: &str,
    snapshot: MapSnapshot,
) -> Result<(), String> {
    let generation = generation(world);
    world.init_resource::<MapLayerRegistry>();
    world
        .resource_mut::<MapLayerRegistry>()
        .set(owner, false, 0, generation, snapshot)
}
pub(crate) fn set_server(
    world: &mut World,
    owner: &str,
    resource_generation: u64,
    value: serde_json::Value,
) -> Result<(), String> {
    if value.is_null() {
        remove_owner(world, owner, Some(true));
        return Ok(());
    }
    let snapshot = MapSnapshot::parse(value)?;
    let generation = generation(world);
    world.init_resource::<MapLayerRegistry>();
    world.resource_mut::<MapLayerRegistry>().set(
        owner,
        true,
        resource_generation,
        generation,
        snapshot,
    )
}
pub(crate) fn remove_owner(world: &mut World, owner: &str, source: Option<bool>) {
    if let Some(mut registry) = world.get_resource_mut::<MapLayerRegistry>() {
        registry.remove(owner, source);
    }
}
pub(crate) fn clear(world: &mut World) {
    if let Some(mut registry) = world.get_resource_mut::<MapLayerRegistry>() {
        registry.entries.clear();
    }
}
pub(crate) fn status(world: &World) -> serde_json::Value {
    let map = world.get_resource::<crate::map_transition::CurrentMap>();
    let state = world.get_resource::<super::input::MapViewState>();
    let bounds = state
        .and_then(|s| s.bounds)
        .map(|(min, max)| serde_json::json!({"min":min.to_array(),"max":max.to_array()}));
    serde_json::json!({"name":map.map(|m|m.name.as_str()),"generation":map.map(|m|m.generation.to_string()),"bounds":bounds,
        "zoom_level":state.map_or(0,|s|s.zoom_level),"expanded":state.is_some_and(|s|s.expanded)})
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> skate_mods::map::MapSnapshot {
        skate_mods::map::MapSnapshot::parse(
            serde_json::json!({"layers":[{"key":"spots","items":[]}]}),
        )
        .unwrap()
    }
    #[test]
    fn server_layers_ownership_generations_and_invalid_replacement() {
        let mut registry = MapLayerRegistry::default();
        registry.set("same", false, 0, 1, state()).unwrap();
        registry.set("same", true, 4, 1, state()).unwrap();
        assert_eq!(registry.entries.len(), 2);
        let mut invalid = state();
        invalid.settings.opacity = Some(f32::NAN);
        assert!(registry.set("same", true, 4, 1, invalid).is_err());
        assert_eq!(registry.entries.len(), 2);
        assert!(registry.set("same", true, 3, 1, state()).is_err());
        registry.remove("same", Some(true));
        assert_eq!(registry.entries.len(), 1);
        registry.retain_generation(2);
        assert!(registry.entries.is_empty());
    }
}
