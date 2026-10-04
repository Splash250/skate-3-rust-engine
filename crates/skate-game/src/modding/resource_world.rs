//! Resource-owned native grind registrations. Updates retire cached acquisition
//! state through the normal physical lifecycle before replacing the provider.
use bevy::prelude::*;
use std::{collections::BTreeMap,sync::Arc};
use skate_net::rails::Snapshot;
#[derive(Resource,Default,Clone)]
struct Rails {
    map_generation:u64,
    base:Option<Arc<crate::grind_world::StaticProvider>>,
    layers:BTreeMap<(String,u64),Snapshot>,
}
pub(super) fn apply(world:&mut World,resource:String,generation:u64,value:serde_json::Value)->Result<(),String> {
    let current=world.resource::<crate::map_transition::CurrentMap>().generation;
    let mut state=world.get_resource::<Rails>().cloned().unwrap_or_default();
    if state.map_generation!=current || state.base.is_none() {
        state=Rails {map_generation:current,base:Some(world.resource::<crate::physics::GamePhysics>().grind_provider()),layers:Default::default()};
    }
    let key=(resource,generation);
    if value.is_null() {state.layers.remove(&key);} else {
        let snapshot:Snapshot=serde_json::from_value(value).map_err(|e|format!("Invalid native rail state: {e}"))?;
        if !snapshot.valid() {return Err("Invalid or over-budget native rail state".into());}
        if state.layers.get(&key).is_some_and(|old|old.revision.parse::<u64>().unwrap()>=snapshot.revision.parse::<u64>().unwrap()) {return Ok(());}
        state.layers.insert(key,snapshot);
    }
    let points=state.layers.values().flat_map(|s|&s.rails).map(|r|r.points.len()).sum::<usize>();
    if points>skate_net::rails::MAX_TOTAL_POINTS {return Err("Aggregate native rail point budget exhausted".into());}
    let rails=state.layers.iter().flat_map(|((resource,generation),snapshot)|snapshot.rails.iter().map(move |rail|skate_data::skate_map::Rail {
        name:format!("{resource}:{generation}:{}",rail.key),closed:rail.closed,points:rail.points.clone(),native:None,
    })).collect::<Vec<_>>();
    let provider=Arc::new(state.base.as_ref().unwrap().with_authored(&rails)?);
    replace(world,provider)?;world.insert_resource(state);Ok(())
}
pub(super) fn clear(world:&mut World)->Result<(),String> {
    let Some(state)=world.remove_resource::<Rails>() else {return Ok(());};
    if state.map_generation==world.resource::<crate::map_transition::CurrentMap>().generation {
        if let Some(base)=state.base.clone() {if let Err(error)=replace(world,base) {world.insert_resource(state);return Err(error);}}
    }
    Ok(())
}
fn replace(world:&mut World,provider:Arc<crate::grind_world::StaticProvider>)->Result<(),String> {
    world.resource_scope(|world,mut physics:Mut<crate::physics::GamePhysics>| {
        let mut skater=world.resource_mut::<crate::physics::SkaterRuntime>();
        physics.replace_grind_provider(&mut skater,provider)
    })
}
