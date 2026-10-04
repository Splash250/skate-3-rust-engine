//! Transactional world replacement: prepare without mutating the live session,
//! then commit at a schedule boundary with gameplay suspended.
use bevy::prelude::*;
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicU8, Ordering}}, thread::JoinHandle, time::Instant};
use crate::{config::Config, map_library::Entry, map_render::PreparedScene,
    physics::{GamePhysics, PlayerControls, SkaterRuntime}};

/// Persistent character customisation can reapply after this set, while the
/// transition still holds the loading overlay and gameplay remains suspended.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MapTransitionSet;
#[derive(Message)]
pub(crate) struct WorldChanged { pub preserve_connection: bool }


#[cfg(test)]
#[path = "tests/map_transition.rs"]
mod tests;

#[derive(Resource, Debug)]
pub(crate) struct CurrentMap {
    pub path: Option<PathBuf>,
    pub name: String,
    pub spawn: [f32; 3],
    pub heading: f32,
    pub generation: u64,
    /// The map's own audio definition: its `AUDO` extension (schema 1, UTF-8 JSON;
    /// `game_audio::map_audio`), if it has one.
    pub audio_tag: Option<Arc<[u8]>>,
}
impl CurrentMap {
    fn from_package(path: Option<PathBuf>, map: Option<&skate_data::skate_map::SkateMap>) -> Self {
        Self { path, name: map.map_or_else(|| "Test world".into(), |m| m.name.clone()),
            spawn: map.map_or([0., crate::physics::ground::HEIGHT, 0.], |m| m.spawn),
            heading: map.map_or(0., |m| m.heading), generation: 0,
            audio_tag: map.and_then(|m| m.extensions.iter().find(|e| e.tag == *b"AUDO" && e.schema == 1)).map(|e| Arc::from(&e.payload[..])) }
    }
}

struct PreparedWorld {
    preserve_connection: bool,
    streaming:Option<crate::map_render::streaming::Options>,
    map_fingerprint: u64,
    scene: PreparedScene,
    physics: GamePhysics,
    skater: SkaterRuntime,
    controls: PlayerControls,
    camera: crate::camera::CameraRuntime,
    metadata: CurrentMap,
    retail: bool,
    triggers: crate::trigger_volumes::TriggerVolumes,
    difficulty: crate::difficulty::Difficulty,
}

enum Phase {
    Idle,
    Requested(Entry),
    Loading { entry: Entry, progress: Arc<AtomicU8>, job: JoinHandle<Result<PreparedWorld, String>> },
    // Let extraction see the committed scene before releasing the pause menu.
    Publishing { frames: u8, notice: String },
}
#[derive(Resource)]
pub(crate) struct MapTransition {
    phase: Phase,
    desired: Option<ResourceTarget>,
    active: Option<String>,
    base: Option<Entry>,
    operation: Option<(Option<String>, Option<usize>)>,
    resource_error: Option<(String,String)>,
    // Resource reloads may supersede each other while a worker is in flight.
    // The whole chain owns one temporary loading pause, not the user's menu.
    resource_pause: Option<(bool, bool)>,
}
#[derive(Clone)]
struct ResourceTarget { key:String, entry:Entry, decoded_budget:usize,lods:Vec<(PathBuf,u32)>,streaming:crate::map_render::streaming::Options }
impl Default for MapTransition {
    fn default() -> Self { Self { phase: Phase::Idle,desired:None,active:None,base:None,operation:None,resource_error:None,resource_pause:None } }
}
impl MapTransition {
    pub fn busy(&self) -> bool { !matches!(self.phase, Phase::Idle) }
    pub fn request(&mut self, entry: Entry) {
        if !self.busy() && self.desired.is_none() { self.phase = Phase::Requested(entry); }
    }
    pub fn label(&self) -> String {
        match &self.phase {
            Phase::Idle => String::new(),
            Phase::Requested(entry) => format!("Loading {} — preparing…", entry.label),
            Phase::Loading { entry, progress, .. } => format!("Loading {} — {}…", entry.label,
                match progress.load(Ordering::Relaxed) {
                    0 => "reading and validating map", 1 => "building collision and rendering",
                    2 => "initializing skater, camera and rendering", _ => "finishing meshes, textures and sky",
                }),
            Phase::Publishing { .. } => "Loading — publishing the new world…".into(),
        }
    }
}

/// A resource remains unready until collision, native rails and rendering have
/// been atomically published. This uses the normal loader without leaving the
/// admitted dedicated session or changing its original wire-map identity.
pub(crate) fn mount_resource(world:&mut World,key:String,entry:Entry,decoded_budget:usize,lods:Vec<(PathBuf,u32)>,streaming:crate::map_render::streaming::Options)->Result<bool,String> {
    let base={let current=world.resource::<CurrentMap>();Entry {label:current.name.clone(),path:current.path.clone()}};
    let mut transition=world.resource_mut::<MapTransition>();
    if let Some((failed,error))=&transition.resource_error {if failed==&key {return Err(error.clone());}}
    if transition.active.as_ref()==Some(&key) && !transition.busy() {return Ok(true);}
    if transition.base.is_none() {transition.base=Some(base);}
    transition.desired=Some(ResourceTarget {key,entry,decoded_budget,lods,streaming});
    Ok(false)
}
pub(crate) fn resource_world_idle(world:&World)->Result<bool,String> {
    let transition=world.resource::<MapTransition>();
    if let Some((key,error))=&transition.resource_error {if key.is_empty() {return Err(error.clone());}}
    Ok(transition.base.is_none() && !transition.busy())
}
pub(crate) fn unmount_resource(world:&mut World) {
    if let Some(mut transition)=world.get_resource_mut::<MapTransition>() {
        transition.desired=None;transition.resource_error=None;
        if transition.active.is_none() && !transition.busy() {transition.base=None;}
    }
}

pub(crate) struct MapTransitionPlugin;
impl Plugin for MapTransitionPlugin {
    fn build(&self, app: &mut App) {
        let config = app.world().resource::<Config>();
        let current = CurrentMap::from_package(config.map_path.clone(), config.map.as_ref());
        app.insert_resource(current)
            .init_resource::<MapTransition>()
            .add_message::<WorldChanged>()
            .add_systems(PreUpdate, (crate::map_render::streaming::update,poll).chain().in_set(MapTransitionSet).after(crate::graphics_menu::MenuInput)
                .before(crate::input::poll_controllers))
            .configure_sets(FixedUpdate, (
                crate::app::SimulationSet::Input, crate::app::SimulationSet::Controls,
                crate::app::SimulationSet::Physics,
            ).run_if(crate::graphics_menu::gameplay_active));
    }
}

fn start(world: &World, entry: Entry) -> Result<Phase, String> {
    let config = world.resource::<Config>();
    let root = config.asset_root.clone();
    let difficulty = config.difficulty;
    let graphs = world.resource::<crate::graph_runtime::StockGraphs>().clone();
    let source = world.resource::<SkaterRuntime>().animation.source.clone();
    let preferences = world.resource::<PlayerControls>().preferences;
    let operation=world.get_resource::<MapTransition>().and_then(|t|t.operation.clone());
    let resource=world.get_resource::<MapTransition>().and_then(|t|t.desired.clone())
        .filter(|target|operation.as_ref().is_some_and(|(key,_)|key.as_ref()==Some(&target.key)));
    let streaming=resource.as_ref().map(|r|r.streaming);
    let preserve_connection=operation.is_some();
    let decoded_budget=operation.and_then(|(_,budget)|budget);
    let mut scene = PreparedScene::new(world);
    let selected = entry.clone();
    let progress = Arc::new(AtomicU8::new(0));
    let stage = progress.clone();
    let job = std::thread::Builder::new().name("map-loader".into()).spawn(move || {
        let _span = info_span!("load_map_transition").entered();
        let started = Instant::now();
        let mut remaining=decoded_budget.unwrap_or(usize::MAX);
        let map = info_span!("read_map").in_scope(|| selected.path.as_deref().map(|path| {
            if decoded_budget.is_some() {
                use std::io::Read;
                let file=std::fs::File::open(path).map_err(|e|e.to_string())?;
                let mut bytes=Vec::new();file.take(512*1024*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
                if bytes.len()>512*1024*1024 {return Err("Required world file exceeds 512 MiB".into());}
                let map=skate_data::skate_map::SkateMap::parse_budgeted(&bytes,&mut remaining,false)?;
                skate_data::resource_world::validate(&map)?;Ok(map)
            } else {skate_data::skate_map::SkateMap::load(path)}
        }).transpose())?;
        let mut lods=Vec::new();
        if let Some(resource)=&resource {
            for (path,distance) in &resource.lods {
                use std::io::Read;
                let mut bytes=Vec::new();std::fs::File::open(path).map_err(|e|e.to_string())?.take(512*1024*1024+1)
                    .read_to_end(&mut bytes).map_err(|e|e.to_string())?;
                if bytes.len()>512*1024*1024 {return Err("Required world LOD exceeds512MiB".into());}
                let lod=skate_data::skate_map::SkateMap::parse_budgeted(&bytes,&mut remaining,true)?;
                skate_data::resource_world::validate_render(&lod)?;lods.push((lod,*distance));
            }
        }
        let map_fingerprint = crate::config::map_fingerprint(selected.path.as_deref())?;
        let read_time = started.elapsed();
        let validation_started = Instant::now();
        if let Some(map) = &map { crate::skate_world::validate_runtime(map)?; }
        let validation_time = validation_started.elapsed();
        let triggers = crate::trigger_volumes::TriggerVolumes::load_or_warn(selected.path.as_deref(), map.as_ref());
        let metadata = CurrentMap::from_package(selected.path, map.as_ref());
        let retail = map.as_ref().is_some_and(|m| crate::retail_render::RetailScene::for_map(m));
        // Both builders only read the decoded package. Reserve render handles
        // on a second worker while the first constructs fresh simulation state;
        // neither publishes to the live world until both have succeeded.
        let (physics, skater, controls, camera, simulation_time, render_time) =
            std::thread::scope(|scope| -> Result<_, String> {
                let rendering = std::thread::Builder::new().name("map-render-loader".into())
                    .spawn_scoped(scope, || {
                        let render_started = Instant::now();
                        if let Some(map)=map.as_ref().filter(|_|resource.is_some()) {scene.prepare_resource(map,&lods,&root);}
                        else {scene.prepare(map.as_ref(), &root);}
                        render_started.elapsed()
                    }).map_err(|e| format!("Could not start render loader: {e}"))?;
                let simulation_started = Instant::now();
                let simulation = (|| -> Result<_, String> {
                    stage.store(1, Ordering::Relaxed);
                    let physics = info_span!("load_physics").in_scope(|| GamePhysics::load_with_difficulty(&root, map.as_ref(), difficulty))?;
                    stage.store(2, Ordering::Relaxed);
                    let skater = SkaterRuntime::load_for_world(&root, &graphs, &physics, difficulty.profile_key(), Some(source))?;
                    let mut controls = PlayerControls::load(&root)?;
                    controls.preferences = preferences;
                    let camera = crate::camera::CameraRuntime::load(&root)?;
                    Ok((physics, skater, controls, camera))
                })();
                let simulation_time = simulation_started.elapsed();
                stage.store(3, Ordering::Relaxed);
                // Explicitly join even on a simulation error: no background
                // render work or reserved scene can outlive a failed request.
                let render_time = rendering.join()
                    .map_err(|_| "Map render preparation failed unexpectedly".to_string())?;
                let (physics, skater, controls, camera) = simulation?;
                Ok((physics, skater, controls, camera, simulation_time, render_time))
            })?;
        eprintln!("MAP_LOAD_TIMING name={:?} read_ms={} validation_ms={} simulation_ms={} render_ms={} prepare_ms={} parallel=true",
            metadata.name, read_time.as_millis(), validation_time.as_millis(),
            simulation_time.as_millis(), render_time.as_millis(), started.elapsed().as_millis());
        // Drop the decoded package on this worker. Physics and rendering now
        // own their data; retaining it would double large-city CPU memory.
        Ok(PreparedWorld { preserve_connection, streaming, map_fingerprint, scene, physics, skater, controls, camera, metadata, retail, triggers, difficulty })
    }).map_err(|e| format!("Could not start map loader: {e}"))?;
    Ok(Phase::Loading { entry, progress, job })
}

fn poll(world: &mut World) {
    let mut pause=false;
    {
        let mut transition=world.resource_mut::<MapTransition>();
        if matches!(transition.phase,Phase::Idle) {
            if let Some(target)=transition.desired.clone() {
                if transition.active.as_ref()!=Some(&target.key) && !transition.resource_error.as_ref().is_some_and(|(key,_)|key==&target.key) {
                    transition.operation=Some((Some(target.key),Some(target.decoded_budget)));
                    transition.phase=Phase::Requested(target.entry);pause=true;
                }
            } else if let Some(base)=transition.base.clone() {
                if !transition.resource_error.as_ref().is_some_and(|(key,_)|key.is_empty()) {
                    transition.operation=Some((None,None));transition.phase=Phase::Requested(base);pause=true;
                }
            }
        }
    }
    if pause {
        let prior = (world.resource::<crate::graphics_menu::Menu>().open,
            world.resource::<Time<Virtual>>().is_paused());
        world.resource_mut::<MapTransition>().resource_pause.get_or_insert(prior);
        world.resource_mut::<Time<Virtual>>().pause();
    }
    let ready = match &world.resource::<MapTransition>().phase {
        Phase::Idle => false,
        Phase::Loading { job, .. } => job.is_finished(),
        _ => true,
    };
    if !ready { return; }
    let phase = std::mem::replace(&mut world.resource_mut::<MapTransition>().phase, Phase::Idle);
    let next = match phase {
        Phase::Requested(entry) => match start(world, entry) {
            Ok(phase) => phase,
            Err(error) => { transition_failed(world, error); Phase::Idle }
        },
        Phase::Loading { job, .. } => {
            let operation=world.resource::<MapTransition>().operation.clone();
            let desired=world.resource::<MapTransition>().desired.as_ref().map(|t|t.key.clone());
            let result=job.join().unwrap_or_else(|_| Err("Map loader failed unexpectedly".into()));
            if operation.as_ref().is_some_and(|(key,_)|*key!=desired) {
                // A new offer invalidated this worker, including its errors.
                // If it retained the active world, release our loading pause
                // through the same boundary as a successful publication.
                drop(result);
                if world.resource::<MapTransition>().active==desired {
                    if desired.is_none() {world.resource_mut::<MapTransition>().base=None;}
                    Phase::Publishing {frames:0,notice:"Current world retained.".into()}
                } else {Phase::Idle}
            } else {
                match result {
                    Ok(prepared) => {
                    let mut notice = commit(world, prepared);
                    if let Some((key,_))=operation {
                        let mut transition=world.resource_mut::<MapTransition>();
                        transition.active=key.clone();transition.resource_error=None;
                        if key.is_none() {transition.base=None;}
                    } else {
                        let config = world.resource::<Config>();
                        match crate::map_library::save_default(&config.asset_root, config.map_path.as_deref()) {
                            Ok(()) => notice.push_str(" Default map saved."),
                            Err(error) => notice.push_str(&format!(" Could not save default: {error}")),
                        }
                    }
                    Phase::Publishing { frames: 3, notice }
                    }
                    Err(error) => { transition_failed(world, error); Phase::Idle }
                }
            }
        },
        Phase::Publishing { frames, notice } if frames > 0 => Phase::Publishing { frames: frames - 1, notice },
        Phase::Publishing {notice,..} if !crate::map_render::streaming::ready(world) => Phase::Publishing {frames:1,notice},
        Phase::Publishing { notice, .. } => {
            let (menu_open,paused)=world.resource_mut::<MapTransition>().resource_pause.take().unwrap_or((false,false));
            world.resource_mut::<crate::graphics_menu::Menu>().transition_finished(notice, !menu_open);
            if paused {world.resource_mut::<Time<Virtual>>().pause();}
            else {world.resource_mut::<Time<Virtual>>().unpause();}
            world.resource_mut::<MapTransition>().operation=None;
            Phase::Idle
        }
        Phase::Idle => Phase::Idle,
    };
    world.resource_mut::<MapTransition>().phase = next;
}

fn transition_failed(world:&mut World,error:String) {
    if let Some((key,_))=world.resource::<MapTransition>().operation.clone() {
        world.resource_mut::<MapTransition>().resource_error=Some((key.unwrap_or_default(),error.clone()));
    }
    failed(world,error);
}
fn failed(world: &mut World, error: String) {
    warn!("MAP_TRANSITION_FAILED {error}");
    world.resource_mut::<crate::graphics_menu::Menu>()
        .transition_finished(format!("Could not load map: {error}\nPrevious world retained. Choose another map or Resume."), false);
}

fn commit(world: &mut World, mut prepared: PreparedWorld) -> String {
    let publication_started = Instant::now();
    prepared.metadata.generation = world.resource::<CurrentMap>().generation + 1;
    // All fallible decoding/validation/construction finished before this point.
    // No simulation system can observe a mixture of the two worlds.
    crate::map_render::MapAssets::retire(world);
    prepared.scene.publish(world);
    if let Some(options)=prepared.streaming {
        // Options and source meshes were validated during preparation. Keep
        // admission paused until bounded uploads make every nearby cell ready.
        crate::map_render::streaming::install(world,options,prepared.metadata.spawn)
            .expect("Prepared resource mesh residency must install");
    }
    crate::camera::set_world_environment(world, prepared.retail);
    world.insert_resource(crate::retail_render::RetailScene(prepared.retail));
    info!("SKATE_TRIGGERS map={:?} volumes={} source={}", prepared.metadata.name, prepared.triggers.map.len(), prepared.triggers.origin);
    // Mod-added volumes and switches are world-scoped, like sdk.volumes.
    world.insert_resource(prepared.triggers);
    world.insert_resource(crate::grind_world::GrindGeometry::for_world(prepared.metadata.path.is_none()));
    world.insert_resource(Time::<Fixed>::from_duration(prepared.physics.period()));
    let root_transform = Transform::from_matrix(crate::animation::native_matrix(
        prepared.skater.animated_skeleton.roots.animation_to_world));
    for mut transform in world.query_filtered::<&mut Transform, With<crate::world::PlayerRoot>>().iter_mut(world) {
        *transform = root_transform;
    }
    world.insert_resource(prepared.physics);
    world.insert_resource(prepared.skater);
    world.insert_resource(prepared.controls);
    world.insert_resource(prepared.camera);
    world.insert_resource(crate::presentation::Presentation::default());
    world.insert_resource(crate::replay::Replay::default());
    world.insert_resource(crate::input::ControllerInput::default());
    world.insert_resource(crate::input::PublishedTickInput::default());
    world.resource_mut::<ButtonInput<KeyCode>>().reset_all();
    world.resource_mut::<ButtonInput<MouseButton>>().reset_all();
    let mut config = world.resource_mut::<Config>();
    config.map_path = prepared.metadata.path.clone();
    config.map_fingerprint = prepared.map_fingerprint;
    config.difficulty = prepared.difficulty;
    let notice = format!("Loaded {}.", prepared.metadata.name);
    info!("MAP_TRANSITION_COMMITTED generation={} name={:?} spawn={:?} heading={} pid={}",
        prepared.metadata.generation, prepared.metadata.name, prepared.metadata.spawn,
        prepared.metadata.heading, std::process::id());
    if let Some(mut messages) = world.get_resource_mut::<Messages<WorldChanged>>() {
        messages.write(WorldChanged {preserve_connection:prepared.preserve_connection});
    }
    world.insert_resource(prepared.metadata);
    eprintln!("MAP_PUBLISH_TIMING cpu_ms={}", publication_started.elapsed().as_millis());
    notice
}
