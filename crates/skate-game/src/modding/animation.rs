//! Resource-owned cosmetic layers, compatible skins and bone attachments.
//! Native physics/animation remain authoritative producers of the base pose.
use super::Mods;
use bevy::{
    ecs::system::SystemState,
    mesh::skinning::SkinnedMesh,
    prelude::*,
    scene::{SceneInstance, SceneSpawner},
};
use skate_mods::animation::{Bank, Delta, Limits, Operation};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Instant,
};
type Slot = (String, String);
type Bindings = Vec<(Entity, usize, Option<usize>)>;
struct Target {
    root: Entity,
    bindings: Bindings,
}
struct Layer {
    bank: String,
    clip: String,
    target: String,
    time: f32,
    speed: f32,
    looped: bool,
    weight: f32,
    fade_in: f32,
    fade_out: f32,
    age: f32,
    stopping: Option<f32>,
    seen: bool,
}
struct Candidate {
    root: Entity,
    started: Instant,
    bindings: Option<Bindings>,
}
struct Appearance {
    target: String,
    current: Option<Candidate>,
    pending: Option<Candidate>,
    seen: bool,
}
struct Attachment {
    target: String,
    bone: usize,
    entity: Entity,
    seen: bool,
    started: Instant,
}
#[derive(Component)]
struct Owned;
#[derive(Resource)]
pub(crate) struct State {
    limits: Limits,
    banks: BTreeMap<Slot, Arc<Bank>>,
    layers: BTreeMap<Slot, Layer>,
    looks: BTreeMap<Slot, Appearance>,
    attachments: BTreeMap<Slot, Attachment>,
    applied: BTreeMap<Entity, (Transform, Transform)>,
    hidden: BTreeMap<Entity, Visibility>,
    events: VecDeque<(String, serde_json::Value)>,
}
/// Bounded read-only counts for native diagnostics and lifecycle verification.
/// Appearance/attachment counts include owned slots still waiting for assets or
/// targets. Pending counts candidates and layers/attachments awaiting a target.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Diagnostics {
    pub banks: usize,
    pub layers: usize,
    pub appearances: usize,
    pub attachments: usize,
    pub pending: usize,
}
impl State {
    pub(crate) fn diagnostics(&self) -> Diagnostics {
        Diagnostics {
            banks: self.banks.len(),
            layers: self.layers.len(),
            appearances: self.looks.len(),
            attachments: self.attachments.len(),
            pending: self.looks.values().filter(|look| look.pending.is_some()).count()
                + self.layers.values().filter(|layer| !layer.seen).count()
                + self.attachments.values().filter(|attachment| !attachment.seen).count(),
        }
    }
}
impl Default for State {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            banks: BTreeMap::new(),
            layers: BTreeMap::new(),
            looks: BTreeMap::new(),
            attachments: BTreeMap::new(),
            applied: BTreeMap::new(),
            hidden: BTreeMap::new(),
            events: VecDeque::new(),
        }
    }
}
pub(super) fn install(app: &mut App) {
    let mut state = State::default();
    if let Ok(value) = std::env::var("SKATE_RESOURCE_ANIMATION_LIMITS") {
        match serde_json::from_str::<Limits>(&value)
            .map_err(|e| e.to_string())
            .and_then(|limits| {
                limits.validate()?;
                Ok(limits)
            }) {
            Ok(limits) => state.limits = limits,
            Err(error) => {
                warn!(
                    "Invalid SKATE_RESOURCE_ANIMATION_LIMITS: {error}; resource animation disabled"
                );
                state.limits.max_banks = 0;
            }
        }
    }
    app.insert_resource(state)
        .add_systems(
            Update,
            restore
                .before(crate::app::FrameSet::Animation)
                .before(crate::multiplayer::RemoteRenderSet),
        )
        .add_systems(
            Update,
            present
                .after(crate::app::FrameSet::Animation)
                .after(crate::multiplayer::RemoteRenderSet),
        );
}
fn event(state: &mut State, owner: &str, key: &str, event: serde_json::Value) {
    if state.events.len() < state.limits.max_events_per_frame {
        state.events.push_back((
            owner.into(),
            serde_json::json!({"type":"animation","version":1,"key":key,"event":event}),
        ));
    }
}
fn targets(world: &mut World) -> BTreeMap<String, Target> {
    let mut out = BTreeMap::new();
    if let Some(root) = world
        .query_filtered::<Entity, With<crate::world::PlayerRoot>>()
        .iter(world)
        .next()
    {
        let bindings = world
            .get_resource::<crate::animation::AnimationStatus>()
            .map_or_else(Vec::new, |s| s.online_bindings());
        let target = Target {
            root,
            bindings: bindings.clone(),
        };
        out.insert("local".into(), target);
        if let Some(net) = world.get_resource::<crate::multiplayer::Multiplayer>() {
            let (active, id, _) = net.mod_identity();
            if active && id > 0 {
                out.insert(id.to_string(), Target { root, bindings });
            }
        }
    }
    for (id, root, bindings) in crate::multiplayer::resource_render_targets(world) {
        out.insert(id.to_string(), Target { root, bindings });
    }
    out
}
fn rig(world: &World) -> Result<(Vec<String>, Vec<i32>), String> {
    let frames = &world
        .get_resource::<crate::physics::SkaterRuntime>()
        .ok_or("native animation rig is unavailable")?
        .animation
        .evaluator
        .frames;
    Ok((frames.bone_names.clone(), frames.parents.clone()))
}
fn remove_scene(world: &mut World, scene: Option<Candidate>) {
    if let Some(scene) = scene {
        world.despawn(scene.root);
    }
}
fn remove_slot(world: &mut World, state: &mut State, slot: &Slot) {
    state.layers.remove(slot);
    if let Some(look) = state.looks.remove(slot) {
        remove_scene(world, look.current);
        remove_scene(world, look.pending);
    }
    if let Some(attachment) = state.attachments.remove(slot) {
        world.despawn(attachment.entity);
    }
}
fn appearance_available(
    state: &State,
    slot: &Slot,
    target: &str,
    targets: &BTreeMap<String, Target>,
) -> bool {
    !state.looks.iter().any(|(other, look)| {
        other != slot
            && (look.target == target
                || targets
                    .get(&look.target)
                    .zip(targets.get(target))
                    .is_some_and(|(a, b)| a.root == b.root))
    })
}
pub(super) fn clear(world: &mut World, owner: Option<&str>) {
    if !world.contains_resource::<State>() {
        return;
    }
    restore(world);
    world.resource_scope(|world, mut state: Mut<State>| {
        let keys: BTreeSet<_> = state
            .layers
            .keys()
            .chain(state.looks.keys())
            .chain(state.attachments.keys())
            .filter(|(id, _)| owner.is_none_or(|o| o == id))
            .cloned()
            .collect();
        for key in keys {
            remove_slot(world, &mut state, &key);
        }
        state
            .banks
            .retain(|(id, _), _| owner.is_some_and(|o| o != id));
        state
            .events
            .retain(|(id, _)| owner.is_some_and(|o| o != id));
    });
}
pub(super) fn poll(_world: &mut World, mods: &mut Mods) {
    let events = std::mem::take(&mut _world.resource_mut::<State>().events);
    for (owner, payload) in events {
        if mods
            .manager
            .packages
            .get(&owner)
            .is_some_and(|p| p.running())
        {
            mods.manager.call(&owner, "on_event", payload);
        }
    }
}
pub(super) fn apply(
    world: &mut World,
    mods: &Mods,
    owner: &str,
    version: u32,
    operation: Operation,
) -> Result<(), String> {
    if version != 1 || !operation.validate() {
        return Err("invalid animation operation or version".into());
    }
    if !world.contains_resource::<State>() {
        return Err("resource animation is not installed".into());
    }
    world.resource_scope(|world, mut state: Mut<State>| {
        if state.limits.max_banks == 0 {
            return Err("resource animation disabled by invalid host limits".into());
        }
        match operation {
            Operation::Load { key, path } => {
                let slot = (owner.into(), key.clone());
                if state
                    .layers
                    .iter()
                    .any(|((id, _), l)| id == owner && l.bank == key)
                {
                    return Err("stop active layers before replacing their bank".into());
                }
                let root = &mods
                    .manager
                    .packages
                    .get(owner)
                    .ok_or("animation owner missing")?
                    .root;
                let bytes = skate_mods::read_bounded(root, &path, state.limits.max_bytes as u64)?;
                let (names, parents) = rig(world)?;
                let bank = Bank::parse(&bytes, &names, &parents, &state.limits)?;
                let resident: usize = state
                    .banks
                    .iter()
                    .filter(|(key, _)| *key != &slot)
                    .map(|(_, bank)| bank.bytes)
                    .sum();
                let keys: usize = state
                    .banks
                    .iter()
                    .filter(|(key, _)| *key != &slot)
                    .map(|(_, bank)| bank.keyframes)
                    .sum();
                if (!state.banks.contains_key(&slot) && state.banks.len() >= state.limits.max_banks)
                    || resident + bank.bytes > state.limits.max_resident_bytes
                    || keys + bank.keyframes > state.limits.max_keyframes
                {
                    return Err("resident animation bank budget exceeded".into());
                }
                state.banks.insert(slot, Arc::new(bank));
                event(
                    &mut state,
                    owner,
                    &key,
                    serde_json::json!({"kind":"loaded"}),
                );
            }
            Operation::Unload { key } => {
                state
                    .layers
                    .retain(|(id, _), layer| id != owner || layer.bank != key);
                state.banks.remove(&(owner.into(), key));
            }
            Operation::Play {
                key,
                bank,
                clip,
                target,
                speed,
                looped,
                fade_in,
                fade_out,
                weight,
                offset,
            } => {
                let slot = (owner.into(), key);
                if state.looks.contains_key(&slot) || state.attachments.contains_key(&slot) {
                    return Err("animation key is owned by a visual object".into());
                }
                let clip_ref = state
                    .banks
                    .get(&(owner.into(), bank.clone()))
                    .and_then(|b| b.clips.get(&clip))
                    .ok_or("animation bank/clip is not loaded")?;
                if offset > clip_ref.duration {
                    return Err("animation offset exceeds clip duration".into());
                }
                if !state.layers.contains_key(&slot)
                    && state.layers.len() >= state.limits.max_layers
                {
                    return Err("animation layer budget exceeded".into());
                }
                state.layers.insert(
                    slot,
                    Layer {
                        bank,
                        clip,
                        target,
                        time: offset,
                        speed,
                        looped,
                        weight,
                        fade_in,
                        fade_out,
                        age: 0.,
                        stopping: None,
                        seen: false,
                    },
                );
            }
            Operation::Stop { key, fade_out } => {
                if let Some(layer) = state.layers.get_mut(&(owner.into(), key)) {
                    layer.stopping = Some(0.);
                    layer.fade_out = fade_out;
                }
            }
            Operation::Remove { key } => remove_slot(world, &mut state, &(owner.into(), key)),
            Operation::Appearance { key, target, path } => {
                let slot = (owner.into(), key.clone());
                if state.layers.contains_key(&slot)
                    || state.attachments.contains_key(&slot)
                    || !appearance_available(&state, &slot, &target, &targets(world))
                {
                    return Err("presentation key/target is already owned".into());
                }
                if !state.looks.contains_key(&slot)
                    && state.looks.len() + state.attachments.len() >= state.limits.max_attachments
                {
                    return Err("presentation asset budget exceeded".into());
                }
                if state
                    .looks
                    .get(&slot)
                    .is_some_and(|look| look.target != target)
                {
                    return Err("remove appearance before changing its target".into());
                }
                let path = super::graphics::asset_path(mods, owner, &path)?;
                let handle = world
                    .resource::<AssetServer>()
                    .load(GltfAssetLabel::Scene(0).from_asset(path));
                let root = world
                    .spawn((
                        Owned,
                        Transform::default(),
                        Visibility::Hidden,
                        SceneRoot(handle),
                    ))
                    .id();
                let look = state.looks.entry(slot).or_insert(Appearance {
                    target,
                    current: None,
                    pending: None,
                    seen: false,
                });
                remove_scene(
                    world,
                    look.pending.replace(Candidate {
                        root,
                        started: Instant::now(),
                        bindings: None,
                    }),
                );
            }
            Operation::Attach {
                key,
                target,
                bone,
                path,
                translation,
                rotation,
                scale,
            } => {
                let slot = (owner.into(), key);
                if state.looks.contains_key(&slot) || state.layers.contains_key(&slot) {
                    return Err("presentation key is already owned".into());
                }
                if !state.attachments.contains_key(&slot)
                    && state.looks.len() + state.attachments.len() >= state.limits.max_attachments
                {
                    return Err("presentation asset budget exceeded".into());
                }
                let (names, _) = rig(world)?;
                let bone = names
                    .iter()
                    .position(|name| name.eq_ignore_ascii_case(&bone))
                    .ok_or("attachment bone is not in native rig")?;
                let path = super::graphics::asset_path(mods, owner, &path)?;
                let handle = world
                    .resource::<AssetServer>()
                    .load(GltfAssetLabel::Scene(0).from_asset(path));
                let entity = world
                    .spawn((
                        Owned,
                        Transform {
                            translation: Vec3::from_array(translation),
                            rotation: Quat::from_array(rotation),
                            scale: Vec3::from_array(scale),
                        },
                        Visibility::Hidden,
                        SceneRoot(handle),
                    ))
                    .id();
                if let Some(old) = state.attachments.insert(
                    slot,
                    Attachment {
                        target,
                        bone,
                        entity,
                        seen: false,
                        started: Instant::now(),
                    },
                ) {
                    world.despawn(old.entity);
                }
            }
        }
        Ok(())
    })
}
fn restore(world: &mut World) {
    if !world.contains_resource::<State>() {
        return;
    }
    world.resource_scope(|world, mut state: Mut<State>| {
        for (entity, (before, after)) in std::mem::take(&mut state.applied) {
            if let Some(mut value) = world.get_mut::<Transform>(entity) {
                if value.to_matrix().abs_diff_eq(after.to_matrix(), 0.00001) {
                    *value = before;
                }
            }
        }
        for (entity, visibility) in std::mem::take(&mut state.hidden) {
            if let Some(mut value) = world.get_mut::<Visibility>(entity) {
                *value = visibility;
            }
        }
    });
}
fn prepared(
    world: &mut World,
    root: Entity,
    names: &[String],
    parents: &[i32],
) -> Result<Option<Bindings>, String> {
    let ready = world
        .get::<SceneInstance>(root)
        .zip(world.get_resource::<SceneSpawner>())
        .is_some_and(|(instance, spawner)| spawner.instance_is_ready(**instance));
    if !ready {
        return Ok(None);
    }
    let mut queries: SystemState<(
        Query<(Entity, &SkinnedMesh)>,
        Query<(&Name, &Transform)>,
        Query<&ChildOf>,
    )> = SystemState::new(world);
    let (skins, nodes, hierarchy) = queries.get(world);
    let bindings =
        crate::animation::AnimationStatus::for_scene(root, names, &skins, &nodes, &hierarchy)?
            .online_bindings();
    for (_, bone, parent) in &bindings {
        let expected = parents
            .get(*bone)
            .copied()
            .filter(|p| *p >= 0)
            .map(|p| p as usize);
        if expected != *parent {
            return Err("resource appearance parent hierarchy differs from native rig".into());
        }
    }
    // A skinned mesh's file bounds describe its bind vertices, not the live
    // native pose. Resource appearances also live outside the stock remote
    // candidate root whose loader installs this component.
    prepare_skin_visibility(world,root);
    Ok(Some(bindings))
}
fn prepare_skin_visibility(world:&mut World,root:Entity) {
    let mut queries:SystemState<(Query<Entity,With<SkinnedMesh>>,Query<&ChildOf>)>=SystemState::new(world);
    let (meshes,parents)=queries.get(world);
    let owned:Vec<_>=meshes.iter().filter(|&entity|parents.iter_ancestors(entity).any(|parent|parent==root)).collect();
    for entity in owned {world.entity_mut(entity).insert(bevy::camera::visibility::NoFrustumCulling);}
}
fn present(world: &mut World) {
    let targets = targets(world);
    let Ok((names, parents)) = rig(world) else {
        return;
    };
    let dt = world
        .get_resource::<Time>()
        .map_or(0., |t| t.delta_secs().clamp(0., 0.25));
    present_targets(world, targets, &names, &parents, dt);
}
fn present_targets(
    world: &mut World,
    targets: BTreeMap<String, Target>,
    names: &[String],
    parents: &[i32],
    dt: f32,
) {
    world.resource_scope(|world,mut state:Mut<State>| {
        let mut layers=std::mem::take(&mut state.layers);
        for (slot,layer) in layers.iter_mut() {
            let Some(target)=targets.get(&layer.target).filter(|t|!t.bindings.is_empty()) else {layer.age+=dt;continue;};
            layer.seen=true;
            let Some(bank)=state.banks.get(&(slot.0.clone(),layer.bank.clone())).cloned() else{continue;};
            let Some(clip)=bank.clips.get(&layer.clip) else{continue;};
            let previous=if layer.age==0. && dt>0. {layer.time-f32::EPSILON}else{layer.time};layer.time+=dt*layer.speed;layer.age=(layer.age+dt).min(6000.);
            if let Some(elapsed)=&mut layer.stopping {*elapsed+=dt;}
            if !layer.looped&&layer.time>=clip.duration&&layer.stopping.is_none() {layer.stopping=Some(0.);}
            let fade_in=if layer.fade_in==0. {1.}else{(layer.age/layer.fade_in).min(1.)};
            let fade_out=layer.stopping.map_or(1.,|age|if layer.fade_out==0. {0.}else{(1.-age/layer.fade_out).max(0.)});
            let weight=layer.weight*fade_in*fade_out;
            let time=if layer.looped {layer.time.rem_euclid(clip.duration)}else{layer.time.min(clip.duration)};
            for (bone,delta) in clip.sample(time) {
                let delta=Delta::default().blend(delta,weight);
                for &(entity,index,_) in &target.bindings {
                    if index!=bone {continue;}
                    if let Some(mut node)=world.get_mut::<Transform>(entity) {
                        let before=*node;node.translation+=Vec3::from_array(delta.translation);node.rotation=(node.rotation*Quat::from_array(delta.rotation)).normalize();node.scale*=Vec3::from_array(delta.scale);
                        state.applied.entry(entity).and_modify(|v|v.1=*node).or_insert((before,*node));
                    }
                }
            }
            for marker in clip.markers_between(previous,layer.time,layer.looped,state.limits.max_events_per_frame.saturating_sub(state.events.len())) {
                event(&mut state,&slot.0,&slot.1,serde_json::json!({"kind":"marker","name":marker.name,"payload":marker.payload,"target":layer.target}));
            }
            if layer.looped {layer.time=layer.time.rem_euclid(clip.duration);}
        }
        layers.retain(|_,layer|targets.get(&layer.target).is_some_and(|t|!t.bindings.is_empty())||(!layer.seen&&layer.age<10.));
        layers.retain(|_,layer|layer.stopping.is_none_or(|age|age<layer.fade_out));state.layers=layers;
        let mut looks=std::mem::take(&mut state.looks);
        let mut claimed=BTreeSet::new();
        looks.retain(|slot,look| {
            let Some(target)=targets.get(&look.target).filter(|t|!t.bindings.is_empty()) else {
                if look.seen||look.pending.as_ref().is_some_and(|p|p.started.elapsed().as_secs()>10) {remove_scene(world,look.current.take());remove_scene(world,look.pending.take());return false;}return true;
            };
            if !claimed.insert(target.root) {
                remove_scene(world,look.current.take());remove_scene(world,look.pending.take());
                event(&mut state,&slot.0,&slot.1,serde_json::json!({"kind":"error","message":"appearance target is already owned through another actor alias"}));return false;
            }
            look.seen=true;
            if look.current.as_ref().is_some_and(|scene|world.get_entity(scene.root).is_err()) {
                look.current=None;
                event(&mut state,&slot.0,&slot.1,serde_json::json!({"kind":"error","message":"appearance target scene was retired"}));
            }
            if let Some(pending)=look.pending.as_mut() {
                let exists=if let Ok(mut entity)=world.get_entity_mut(pending.root) {entity.insert(ChildOf(target.root));true}else{false};
                let result=if !exists {Err("resource appearance target was retired".into())}else if pending.started.elapsed().as_secs()>10 {Err("resource appearance load timed out".into())}else{prepared(world,pending.root,names,parents)};
                match result {
                    Ok(Some(bindings))=>{pending.bindings=Some(bindings);remove_scene(world,look.current.take());look.current=look.pending.take();event(&mut state,&slot.0,&slot.1,serde_json::json!({"kind":"ready"}));},
                    Err(error)=>{remove_scene(world,look.pending.take());event(&mut state,&slot.0,&slot.1,serde_json::json!({"kind":"error","message":error}));},_=>{}
                }
            }
            if let Some(current)=&look.current {
                let source:BTreeMap<_,_>=target.bindings.iter().filter_map(|(entity,bone,_)|world.get::<Transform>(*entity).copied().map(|t|(*bone,t))).collect();
                for &(entity,bone,_) in current.bindings.as_deref().unwrap_or(&[]) {if let Some(value)=source.get(&bone) {if let Some(mut node)=world.get_mut::<Transform>(entity) {*node=*value;}}}
                if let Some(mut visibility)=world.get_mut::<Visibility>(current.root) {*visibility=Visibility::Inherited;}
                let children:Vec<_>=world.get::<Children>(target.root).map_or_else(Vec::new,|c|c.iter().collect());
                for child in children {if world.get::<Owned>(child).is_some() {continue;}if let Some(mut visibility)=world.get_mut::<Visibility>(child) {state.hidden.entry(child).or_insert(*visibility);*visibility=Visibility::Hidden;}}
            }
            look.current.is_some()||look.pending.is_some()
        });state.looks=looks;
        state.attachments.retain(|_,attachment| {
            if world.get_entity(attachment.entity).is_err() {return false;}
            let binding=targets.get(&attachment.target).and_then(|t|t.bindings.iter().find(|(_,bone,_)|*bone==attachment.bone)).map(|v|v.0);
            if let Some(binding)=binding {attachment.seen=true;if let Ok(mut entity)=world.get_entity_mut(attachment.entity) {entity.insert((ChildOf(binding),Visibility::Visible));}true}
            else if attachment.seen||attachment.started.elapsed().as_secs()>10 {world.despawn(attachment.entity);false}else{true}
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (World, Entity, Entity, Entity, Entity) {
        let mut world = World::new();
        world.insert_resource(State::default());
        let local = world
            .spawn((Transform::default(), Visibility::Inherited))
            .id();
        let head = world
            .spawn((Transform::from_xyz(0., 1., 0.), ChildOf(local)))
            .id();
        let remote = world
            .spawn((Transform::default(), Visibility::Inherited))
            .id();
        let remote_head = world
            .spawn((Transform::from_xyz(0., 1., 0.), ChildOf(remote)))
            .id();
        let bytes=serde_json::to_vec(&serde_json::json!({"version":1,"bones":[{"name":"head","parent":null}],"clips":[{"name":"nod","duration":1.0,"tracks":[{"bone":"head","keys":[{"time":0.0,"translation":[0,0.5,0]}]}],"markers":[{"time":0.1,"name":"beat"}]}]})).unwrap();
        let bank = Bank::parse(&bytes, &["head".into()], &[-1], &Limits::default()).unwrap();
        world
            .resource_mut::<State>()
            .banks
            .insert(("owner".into(), "bank".into()), Arc::new(bank));
        for target in ["local", "42"] {
            world.resource_mut::<State>().layers.insert(
                ("owner".into(), target.into()),
                Layer {
                    bank: "bank".into(),
                    clip: "nod".into(),
                    target: target.into(),
                    time: 0.,
                    speed: 1.,
                    looped: true,
                    weight: 1.,
                    fade_in: 0.,
                    fade_out: 0.,
                    age: 0.,
                    stopping: None,
                    seen: false,
                },
            );
        }
        (world, local, head, remote, remote_head)
    }
    fn target_map(
        local: Entity,
        head: Entity,
        remote: Entity,
        remote_head: Entity,
    ) -> BTreeMap<String, Target> {
        BTreeMap::from([
            (
                "local".into(),
                Target {
                    root: local,
                    bindings: vec![(head, 0, None)],
                },
            ),
            (
                "42".into(),
                Target {
                    root: remote,
                    bindings: vec![(remote_head, 0, None)],
                },
            ),
        ])
    }
    #[test]
    fn resource_animation_local_remote_layers_restore_without_drift_and_retire() {
        let (mut world, local, head, remote, remote_head) = fixture();
        for _ in 0..10 {
            restore(&mut world);
            present_targets(
                &mut world,
                target_map(local, head, remote, remote_head),
                &["head".into()],
                &[-1],
                0.02,
            );
            assert!((world.get::<Transform>(head).unwrap().translation.y - 1.5).abs() < 0.0001);
            assert!(
                (world.get::<Transform>(remote_head).unwrap().translation.y - 1.5).abs() < 0.0001
            );
        }
        assert_eq!(world.resource::<State>().events.len(), 2);
        assert_eq!(world.resource::<State>().diagnostics(), Diagnostics {
            banks: 1, layers: 2, appearances: 0, attachments: 0, pending: 0,
        });
        clear(&mut world, Some("owner"));
        assert_eq!(world.get::<Transform>(head).unwrap().translation.y, 1.);
        assert_eq!(
            world.get::<Transform>(remote_head).unwrap().translation.y,
            1.
        );
        assert!(world.resource::<State>().banks.is_empty());
        assert!(world.resource::<State>().layers.is_empty());
        assert!(world.resource::<State>().events.is_empty());
        assert_eq!(world.resource::<State>().diagnostics(), Diagnostics::default());
    }
    #[test]
    fn resource_animation_visibility_loss_removes_layers_and_attached_entities() {
        let (mut world, local, head, remote, remote_head) = fixture();
        let attachment = world.spawn((Owned, Transform::default(),Visibility::Hidden)).id();
        world.resource_mut::<State>().attachments.insert(
            ("owner".into(), "hat".into()),
            Attachment {
                target: "42".into(),
                bone: 0,
                entity: attachment,
                seen: false,
                started: Instant::now(),
            },
        );
        present_targets(
            &mut world,
            target_map(local, head, remote, remote_head),
            &["head".into()],
            &[-1],
            0.02,
        );
        assert_eq!(
            world.get::<ChildOf>(attachment).unwrap().parent(),
            remote_head
        );
        // A replacement appearance hides the stock scene containing this bone.
        // Resource attachments must remain visible until the target leaves view.
        assert_eq!(*world.get::<Visibility>(attachment).unwrap(),Visibility::Visible);
        restore(&mut world);
        present_targets(&mut world, BTreeMap::new(), &["head".into()], &[-1], 0.02);
        assert!(world.resource::<State>().layers.is_empty());
        assert!(world.resource::<State>().attachments.is_empty());
        assert!(world.get_entity(attachment).is_err());
    }
    #[test]
    fn resource_appearance_timeout_keeps_previous_skin_and_owner_stop_restores_stock() {
        let (mut world, local, head, remote, remote_head) = fixture();
        let stock = world.spawn((Visibility::Inherited, ChildOf(local))).id();
        let current = world
            .spawn((Owned, Visibility::Inherited, ChildOf(local)))
            .id();
        let failed = world.spawn((Owned, Visibility::Hidden)).id();
        world.resource_mut::<State>().looks.insert(
            ("owner".into(), "skin".into()),
            Appearance {
                target: "local".into(),
                current: Some(Candidate {
                    root: current,
                    started: Instant::now(),
                    bindings: Some(vec![]),
                }),
                pending: Some(Candidate {
                    root: failed,
                    started: Instant::now() - std::time::Duration::from_secs(11),
                    bindings: None,
                }),
                seen: true,
            },
        );
        present_targets(
            &mut world,
            target_map(local, head, remote, remote_head),
            &["head".into()],
            &[-1],
            0.02,
        );
        assert!(world.get_entity(failed).is_err());
        assert!(world.get_entity(current).is_ok());
        assert_eq!(*world.get::<Visibility>(stock).unwrap(), Visibility::Hidden);
        assert!(
            world
                .resource::<State>()
                .events
                .iter()
                .any(|(_, value)| value["event"]["kind"] == "error")
        );
        clear(&mut world, Some("owner"));
        assert!(world.get_entity(current).is_err());
        assert_eq!(
            *world.get::<Visibility>(stock).unwrap(),
            Visibility::Inherited
        );
    }

    #[test]
    fn resource_animation_root_replacement_retires_missing_scenes_and_attachments() {
        let (mut world, local, head, remote, remote_head) = fixture();
        let stock = world.spawn((Visibility::Inherited, ChildOf(local))).id();
        let vanished = world.spawn_empty().id();
        world.despawn(vanished);
        world.resource_mut::<State>().looks.insert(
            ("owner".into(), "skin".into()),
            Appearance {
                target: "local".into(),
                current: Some(Candidate {
                    root: vanished,
                    started: Instant::now(),
                    bindings: Some(vec![]),
                }),
                pending: None,
                seen: true,
            },
        );
        world.resource_mut::<State>().attachments.insert(
            ("owner".into(), "hat".into()),
            Attachment {
                target: "local".into(),
                bone: 0,
                entity: vanished,
                seen: true,
                started: Instant::now(),
            },
        );
        present_targets(
            &mut world,
            target_map(local, head, remote, remote_head),
            &["head".into()],
            &[-1],
            0.02,
        );
        assert_eq!(
            *world.get::<Visibility>(stock).unwrap(),
            Visibility::Inherited,
            "missing custom skin hid the replacement stock skin"
        );
        assert!(world.resource::<State>().looks.is_empty());
        assert!(world.resource::<State>().attachments.is_empty());
    }

    #[test]
    fn resource_animation_appearance_ownership_resolves_actor_aliases() {
        let (mut world, local, head, remote, remote_head) = fixture();
        let mut targets = target_map(local, head, remote, remote_head);
        targets.insert(
            "1".into(),
            Target {
                root: local,
                bindings: vec![(head, 0, None)],
            },
        );
        world.resource_mut::<State>().looks.insert(
            ("one".into(), "skin".into()),
            Appearance {
                target: "local".into(),
                current: None,
                pending: None,
                seen: false,
            },
        );
        assert!(
            !appearance_available(
                world.resource::<State>(),
                &("two".into(), "skin".into()),
                "1",
                &targets
            ),
            "two resources claimed the same character using different identity spellings"
        );
        assert!(appearance_available(
            world.resource::<State>(),
            &("two".into(), "skin".into()),
            "42",
            &targets
        ));
    }

    #[test]
    fn resource_animation_original_glbs_pass_downloaded_asset_validation_and_bind() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/presentation-demo");
        for name in ["mascot.glb", "hat.glb"] {
            super::super::graphics::validate_resource_glb(&std::fs::read(root.join(name)).unwrap())
                .unwrap();
        }
        let bytes = std::fs::read(root.join("mascot.glb")).unwrap();
        let glb = gltf::Gltf::from_slice(&bytes).unwrap();
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let nodes: Vec<_> = glb
            .nodes()
            .map(|node| {
                world
                    .spawn((
                        Name::new(node.name().unwrap().to_owned()),
                        Transform::default(),
                        ChildOf(root),
                    ))
                    .id()
            })
            .collect();
        for node in glb.nodes() {
            for child in node.children() {
                world
                    .entity_mut(nodes[child.index()])
                    .insert(ChildOf(nodes[node.index()]));
            }
        }
        let skin = glb.skins().next().unwrap();
        let joints: Vec<_> = skin.joints().map(|joint| nodes[joint.index()]).collect();
        world.spawn((
            ChildOf(root),
            SkinnedMesh {
                inverse_bindposes: default(),
                joints,
            },
        ));
        let names = vec![
            "TRAJECTORY", "HIPS", "SPINE", "SPINE1", "SPINE2", "SPINE3", "NECK", "NECK1", "HEAD",
            "RIGHTSHOULDER","RIGHTARM","RIGHTFOREARM","RIGHTHAND",
            "LEFTSHOULDER","LEFTARM","LEFTFOREARM","LEFTHAND",
            "RIGHTUPLEG","RIGHTLEG","RIGHTFOOT","RIGHTTOEBASE",
            "LEFTUPLEG","LEFTLEG","LEFTFOOT","LEFTTOEBASE",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let mut queries: SystemState<(
            Query<(Entity, &SkinnedMesh)>,
            Query<(&Name, &Transform)>,
            Query<&ChildOf>,
        )> = SystemState::new(&mut world);
        let (skins, nodes, parents) = queries.get(&world);
        let bound =
            crate::animation::AnimationStatus::for_scene(root, &names, &skins, &nodes, &parents)
                .unwrap();
        let bindings=bound.online_bindings();
        let expected=[-1,0,1,2,3,4,5,6,7,5,9,10,11,5,13,14,15,1,17,18,19,1,21,22,23];
        assert_eq!(bindings.len(), expected.len());
        for (_,bone,parent) in bindings {assert_eq!(parent,(expected[bone]>=0).then_some(expected[bone] as usize));}
    }

    #[test]
    fn resource_appearance_skin_culling_uses_live_pose_for_local_and_remote_targets() {
        let mut world=World::new();
        let root=world.spawn_empty().id();
        let armature=world.spawn(ChildOf(root)).id();
        let owned=world.spawn((ChildOf(armature),SkinnedMesh{inverse_bindposes:default(),joints:vec![]})).id();
        let foreign=world.spawn(SkinnedMesh{inverse_bindposes:default(),joints:vec![]}).id();
        prepare_skin_visibility(&mut world,root);
        assert!(world.get::<bevy::camera::visibility::NoFrustumCulling>(owned).is_some());
        assert!(world.get::<bevy::camera::visibility::NoFrustumCulling>(foreign).is_none());
    }
}
