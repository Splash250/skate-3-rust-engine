//! Context-local camera state keeps distant interior pockets out of city bounds.
use super::*;
#[derive(Resource,Default)]
pub(crate) struct Context {
    pub selected:Option<String>,
    saved:std::collections::BTreeMap<Option<String>,MapViewState>,
}
#[derive(Component)]
pub(crate) struct InteriorOverview(pub String);
pub(crate) fn set_context(world:&mut World,key:Option<String>) {
    world.init_resource::<Context>();
    let mut context=world.remove_resource::<Context>().unwrap();
    if context.selected!=key {
        if let Some(state)=world.get_resource::<MapViewState>().cloned() {
            context.saved.insert(context.selected.clone(),state.clone());
            let mut next=context.saved.get(&key).cloned().unwrap_or_default();
            next.visible=state.visible;next.expanded=state.expanded;next.generation=state.generation;next.previous_buttons=state.previous_buttons;
            world.insert_resource(next);
        }
        context.selected=key;
    }
    world.insert_resource(context);
}
pub(crate) fn spawn_interior(world:&mut World,key:&str,mesh:Handle<Mesh>,material:Handle<StandardMaterial>,at:Transform,bounds:(Vec3,Vec3))->Entity {
    world.spawn((InteriorOverview(key.into()),OverviewScene(geometry::OverviewBounds {min:bounds.0,max:bounds.1}),MapOverviewLod{level:0},Mesh3d(mesh),MeshMaterial3d(material),at,Visibility::Hidden,bevy::camera::visibility::RenderLayers::layer(LAYER))).id()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn map_context_preserves_free_view_and_restores_exterior() {
        let mut world=World::new();let mut state=MapViewState::default();state.center=Some(Vec3::new(5.,0.,6.));state.zoom_level=3;
        world.insert_resource(state);set_context(&mut world,Some("room".into()));
        assert_eq!(world.resource::<MapViewState>().center,None);
        world.resource_mut::<MapViewState>().center=Some(Vec3::splat(4096.));
        set_context(&mut world,None);
        assert_eq!(world.resource::<MapViewState>().center,Some(Vec3::new(5.,0.,6.)));
        assert_eq!(world.resource::<MapViewState>().zoom_level,3);
    }
}

/// Peer dots belong to the selected world pocket, independent of map zoom.
pub(crate) fn contains(world:&mut World,position:Vec3)->bool {
    let selected=world.get_resource::<Context>().and_then(|c|c.selected.clone());
    let mut in_any=false;let mut in_selected=false;
    for (interior,bounds) in world.query::<(&InteriorOverview,&OverviewScene)>().iter(world) {
            let margin=Vec3::new(0.5,2.,0.5);
            if position.cmpge(bounds.0.min-margin).all() && position.cmple(bounds.0.max+margin).all(){in_any=true;in_selected|=selected.as_deref()==Some(interior.0.as_str());}
    }
    if selected.is_some(){in_selected}else{!in_any}
}

pub(crate) fn reset_context(world:&mut World){set_context(world,None);world.insert_resource(Context::default());}
