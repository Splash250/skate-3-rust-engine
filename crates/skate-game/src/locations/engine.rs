//! Engine-owned interior preparation, occupancy and native travel.
use super::{
    catalog,
    input::{FloorMenu, LocationInput},
    model,
    runtime::{Phase, Runtime, Ticket, Visit},
};
use bevy::prelude::*;
use skate_resources::locations::PreparedCatalog;
use std::{sync::Arc, thread::JoinHandle, time::Instant};
#[derive(Resource)]
pub(crate) struct LocationRegistry {
    map_generation: Option<u64>,
    package: Option<Arc<PreparedCatalog>>,
    owners: std::collections::BTreeMap<String, (u64, Arc<PreparedCatalog>)>,
    failed: std::collections::BTreeMap<String, (u64, String, String)>,
    rollback: Option<std::collections::BTreeMap<String, (u64, Arc<PreparedCatalog>)>>,
    base: Option<skate_core::physics::board_world::BoardWorld>,
    reload: bool,
    retiring: Vec<String>,
    queued: std::collections::BTreeMap<String, (u64, PreparedCatalog)>,
    settings: std::collections::BTreeMap<String, skate_resources::locations::LocationSnapshot>,
    publications: std::collections::BTreeMap<String, Vec<Instant>>,
    loading: Option<JoinHandle<Result<catalog::Prepared, String>>>,
    loading_started: Instant,
    entities: Vec<Entity>,
    runtime: Runtime,
    input: LocationInput,
    floors: FloorMenu,
    selected: Option<String>,
    previous_buttons: u16,
    started: Instant,
    request: u64,
    travel: Option<(u64, Ticket, bool)>,
    approval_epoch: Option<u64>,
    prompt: Option<Entity>,
    error: Option<String>,
    ready: bool,
    markers_dirty: bool,
}
impl Default for LocationRegistry {
    fn default() -> Self {
        Self {
            map_generation: None,
            package: None,
            owners: Default::default(),
            failed: Default::default(),
            rollback: None,
            base: None,
            reload: false,
            retiring: vec![],
            queued: Default::default(),
            settings: Default::default(),
            publications: Default::default(),
            loading: None,
            loading_started: Instant::now(),
            entities: vec![],
            runtime: Runtime::default(),
            input: LocationInput::default(),
            floors: FloorMenu::default(),
            selected: None,
            previous_buttons: 0,
            started: Instant::now(),
            request: 0,
            travel: None,
            approval_epoch: None,
            prompt: None,
            error: None,
            ready: false,
            markers_dirty: false,
        }
    }
}
pub(crate) struct LocationPlugin;
impl Plugin for LocationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocationRegistry>().add_systems(
            PreUpdate,
            update
                .after(crate::map_transition::MapTransitionSet)
                .after(crate::input::poll_controllers)
                .after(crate::graphics_menu::MenuInput),
        );
    }
}
pub(crate) fn blocked(registry: Option<&LocationRegistry>) -> bool {
    registry.is_some_and(|r| r.input.blocked() || r.floors.active())
}
pub(crate) fn status(world: &World) -> serde_json::Value {
    world.get_resource::<LocationRegistry>().map_or(serde_json::Value::Null,|r|serde_json::json!({"phase":r.runtime.status(),"ready":r.ready,"error":r.error,"interior":r.runtime.visit.as_ref().map(|v|&v.interior),"catalog_revision":r.package.as_ref().map(|p|&p.revision),"catalogs":r.owners.iter().map(|(owner,(g,p))|serde_json::json!({"owner":owner,"generation":g.to_string(),"revision":p.revision,"ready":r.ready&&!r.reload&&r.loading.is_none()})).collect::<Vec<_>>()}))
}
#[derive(Component)]
struct MarkerVisual {
    key: Option<String>,
    position: [f32; 3],
    base: skate_resources::locations::MarkerStyle,
    ring: bool,
    icon: bool,
}
fn update(world: &mut World) {
    let mut r = world
        .remove_resource::<LocationRegistry>()
        .unwrap_or_default();
    if let Err(error) = advance(world, &mut r) {
        warn!("Interiors: {error}");
        r.error = Some(error.clone());
        r.runtime.fail(error);
    }
    if r.loading.is_some() {
        r.selected = None;
        r.floors.cancel();
    }
    let facing = world
        .query_filtered::<&Transform, With<crate::camera::GameplayCamera>>()
        .iter(world)
        .next()
        .map(|t| t.rotation);
    if let Some(facing) = facing {
        for (v, mut t) in world
            .query::<(&MarkerVisual, &mut Transform)>()
            .iter_mut(world)
        {
            if v.icon {
                t.rotation = facing;
            }
        }
    }
    draw_prompt(world, &mut r);
    world.insert_resource(r);
}
fn advance(world: &mut World, r: &mut LocationRegistry) -> Result<(), String> {
    let generation = world
        .resource::<crate::map_transition::CurrentMap>()
        .generation;
    if r.map_generation != Some(generation) && r.loading.is_some() {
        r.selected = None;
        r.floors.cancel();
        r.input.consume();
        if !r.loading.as_ref().unwrap().is_finished() {
            return Ok(());
        }
        let _ = r.loading.take().unwrap().join();
    }
    if r.map_generation != Some(generation) {
        for e in r.entities.drain(..) {
            world.despawn(e);
        }
        crate::map_view::reset_context(world);
        crate::map_view::remove_owner(world, "engine-locations", None);
        r.owners.clear();
        r.failed.clear();
        r.queued.clear();
        r.rollback = None;
        r.settings.clear();
        r.publications.clear();
        r.retiring.clear();
        r.base = None;
        r.reload = false;
        r.runtime = Runtime::default();
        r.ready = false;
        r.package = None;
        r.loading = None;
        r.travel = None;
        r.error = None;
        r.floors.cancel();
        r.map_generation = Some(generation);
        let config = world.resource::<crate::config::Config>();
        let path = world
            .resource::<crate::map_transition::CurrentMap>()
            .path
            .clone();
        let explicit = config.locations.clone();
        let base = world
            .resource::<crate::physics::GamePhysics>()
            .world()
            .clone();
        r.base = Some(base.clone());
        let material = world
            .resource::<crate::physics::GamePhysics>()
            .settings
            .floor_material;
        if config.multiplayer.connect.is_none() {
            r.loading_started = Instant::now();
            r.loading = Some(std::thread::spawn(move || {
                let package = PreparedCatalog::discover(path.as_deref(), explicit.as_deref())?
                    .ok_or("No interior catalog installed")?;
                catalog::prepare(&base, material, Arc::new(package))
            }));
        }
    }
    let expired = r.loading.is_some() && r.loading_started.elapsed().as_secs_f64() >= 10.;
    if expired {
        r.error = Some("Interior preparation timed out after 10 seconds".into());
    }
    if r.loading.as_ref().is_some_and(|j| j.is_finished()) {
        let result = r
            .loading
            .take()
            .unwrap()
            .join()
            .map_err(|_| "Interior loader failed")?;
        // A late job can release its owned data, but cannot publish after its deadline.
        match if expired {
            Err("Interior preparation timed out after 10 seconds".into())
        } else {
            result
        } {
            Ok(prepared) => {
                let mut physics = world
                    .remove_resource::<crate::physics::GamePhysics>()
                    .unwrap();
                let result = physics.replace_location_collision(
                    &mut world.resource_mut::<crate::physics::SkaterRuntime>(),
                    prepared.collision,
                );
                world.insert_resource(physics);
                result?;
                for e in r.entities.drain(..) {
                    world.despawn(e);
                }
                for (key, model) in prepared.models {
                    publish_model(world, r, &key, model);
                }
                if r.owners.is_empty() && !r.reload {
                    r.owners
                        .insert("engine-locations".into(), (1, prepared.package.clone()));
                }
                r.package = Some(prepared.package);
                r.ready = true;
                r.error = None;
                r.reload = false;
                r.rollback = None;
                r.markers_dirty = true;
                publish_markers(world, r);
                pins(world, r)?;
                info!(
                    "INTERIORS_READY revision={}",
                    r.package.as_ref().unwrap().revision
                );
            }
            Err(e) if e == "No interior catalog installed" => {}
            Err(e) => {
                if let Some(old) = r.rollback.take() {
                    for (owner, (g, p)) in &r.owners {
                        if old
                            .get(owner)
                            .is_none_or(|(old_g, old_p)| old_g != g || old_p.revision != p.revision)
                        {
                            r.failed
                                .insert(owner.clone(), (*g, p.revision.clone(), e.clone()));
                        }
                    }
                    r.owners = old;
                }
                r.reload = false;
                return Err(e);
            }
        }
    }
    if r.loading.is_none() && !r.retiring.is_empty() {
        if r.runtime
            .visit
            .as_ref()
            .is_some_and(|v| r.retiring.contains(&v.ticket.owner))
        {
            if world
                .resource::<crate::multiplayer::Multiplayer>()
                .is_dedicated()
                && matches!(
                    r.runtime.phase,
                    Phase::AwaitingApproval | Phase::Interior | Phase::Returning
                )
            {
                // The host evacuates occupants before publishing a retired catalog.
                // Keep support until the admitted movement reset has physically landed.
                let skater = world.resource::<crate::physics::SkaterRuntime>();
                let p = skater.skeleton.part_transforms()[0][3];
                let visit = r.runtime.visit.as_ref().unwrap().clone();
                if skater.player_input.pending_teleport().is_none()
                    && r.travel
                        .as_ref()
                        .is_none_or(|(prior, _, _)| skater.travel_generation > *prior)
                    && (Vec3::new(p[0], p[1], p[2]) - Vec3::from_array(visit.return_position))
                        .length()
                        < 2.5
                {
                    r.runtime.phase = Phase::Returning;
                    r.runtime.returned(&visit.ticket);
                    r.travel = None;
                    crate::map_view::set_context(world, None);
                }
            } else if r.runtime.phase == Phase::Interior {
                request_return(world, r)?;
            } else if matches!(r.runtime.phase, Phase::Preparing | Phase::AwaitingApproval) {
                r.runtime
                    .fail("Interior owner retired during preparation".into());
                r.travel = None;
            }
        } else {
            for owner in r.retiring.drain(..) {
                r.owners.remove(&owner);
                r.settings.remove(&owner);
            }
            r.reload = true;
        }
    }
    if !r.reload && r.loading.is_none() && !r.queued.is_empty() {
        let (owner, (generation, package)) = r.queued.pop_first().unwrap();
        if r.runtime
            .visit
            .as_ref()
            .is_some_and(|v| v.ticket.owner == owner)
        {
            r.queued.insert(owner, (generation, package));
            return Ok(());
        }
        let mut owners = r.owners.clone();
        owners.insert(owner, (generation, Arc::new(package)));
        aggregate(&owners)?;
        r.rollback = Some(r.owners.clone());
        r.owners = owners;
        r.reload = true;
    }
    if r.reload && r.loading.is_none() {
        let package = aggregate(&r.owners)?;
        let base = r.base.clone().ok_or("Base collision unavailable")?;
        let material = world
            .resource::<crate::physics::GamePhysics>()
            .settings
            .floor_material;
        r.loading_started = Instant::now();
        r.loading = Some(std::thread::spawn(move || {
            catalog::prepare(&base, material, Arc::new(package))
        }));
    }
    if r.markers_dirty {
        refresh_markers(world, r);
        pins(world, r)?;
        r.markers_dirty = false;
    }
    let now = r.started.elapsed().as_secs_f64();
    r.runtime.timeout(now);
    if let Some((prior, ticket, returning)) = r.travel.clone() {
        let skater = world.resource::<crate::physics::SkaterRuntime>();
        if skater.travel_generation > prior
            && skater.player_input.pending_teleport().is_none()
            && (!world
                .resource::<crate::multiplayer::Multiplayer>()
                .is_dedicated()
                || r.approval_epoch.is_some_and(|epoch| {
                    world
                        .resource::<crate::multiplayer::Multiplayer>()
                        .movement_epoch()
                        == Some(epoch)
                }))
        {
            if returning {
                r.runtime.returned(&ticket);
                crate::map_view::set_context(world, None);
            } else if r.runtime.committed(&ticket) {
                crate::map_view::set_context(
                    world,
                    r.runtime.visit.as_ref().map(|v| v.interior.clone()),
                );
            }
            r.travel = None;
            pins(world, r)?;
        }
    }
    let keys = world.resource::<ButtonInput<KeyCode>>();
    let pad = world.resource::<crate::input::ControllerInput>();
    let slot = pad.active_slot();
    let buttons = pad.raw_input().buttons;
    let down =
        keys.pressed(KeyCode::KeyE) || keys.just_pressed(KeyCode::KeyE) || buttons & 0x1000 != 0;
    let active = !world.resource::<crate::graphics_menu::Menu>().open
        && !world
            .resource::<crate::map_transition::MapTransition>()
            .busy()
        && !world.resource::<crate::map_view::MapViewState>().expanded
        && world
            .get_resource::<crate::replay::Replay>()
            .is_none_or(|r| !r.active)
        && world
            .get_resource::<crate::teleport_menu::Travel>()
            .is_none_or(|t| !t.open && !t.closed_this_frame)
        && world
            .get_resource::<crate::customiser::Customiser>()
            .is_none_or(|c| !c.open)
        && !crate::modding::browser_focused(world.get_resource::<crate::modding::Mods>())
        && !crate::modding::interactions::blocked(
            world.get_resource::<crate::modding::interactions::Interfaces>(),
        )
        && world.query::<&Window>().iter(world).all(|w| w.focused);
    let edge = r.input.advance(down, active, slot.map(|s| s as u64));
    if edge {
        info!(
            "LOCATION_INPUT edge selected={:?} phase={:?}",
            r.selected, r.runtime.phase
        );
    }
    let pressed = buttons & !r.previous_buttons;
    r.previous_buttons = buttons;
    if !active || r.loading.is_some() || r.reload || !r.ready {
        r.floors.cancel();
        r.selected = None;
        return Ok(());
    }
    let Some(package) = r.package.clone() else {
        return Ok(());
    };
    let skater = world.resource::<crate::physics::SkaterRuntime>();
    let volumes = crate::physics::skeleton_colliders::enabled_volumes(
        &skater.skeleton,
        &skater.skeleton_collision,
    )?
    .into_iter()
    .map(|v| v.primitive)
    .collect::<Vec<_>>();
    let position = skater.skeleton.part_transforms()[0][3];
    let center = [position[0], position[1], position[2]];
    if r.runtime.phase == Phase::Interior {
        let visit = r.runtime.visit.as_ref().unwrap();
        let interior = package
            .catalog
            .interiors
            .iter()
            .find(|i| i.key == visit.interior)
            .unwrap();
        let intersect = super::markers::intersect_marker(
            &volumes,
            interior.exit.position,
            &interior.exit.style,
        );
        r.selected = intersect.then(|| "exit".into());
        if intersect && edge {
            r.input.consume();
            request_return(world, r)?;
        }
    } else if r.runtime.phase == Phase::Exterior {
        let selected = super::markers::nearest(
            center,
            package
                .catalog
                .locations
                .iter()
                .filter(|l| {
                    r.owners.iter().any(|(owner, (_, p))| {
                        !r.retiring.contains(owner)
                            && p.catalog.locations.iter().any(|entry| entry.key == l.key)
                    })
                })
                .filter(|l| {
                    r.settings
                        .values()
                        .flat_map(|s| &s.locations)
                        .find(|s| s.key == l.key)
                        .is_none_or(|s| s.enabled)
                })
                .filter(|l| {
                    super::markers::intersect_marker(
                        &volumes,
                        l.position,
                        r.settings
                            .values()
                            .flat_map(|s| &s.locations)
                            .find(|s| s.key == l.key)
                            .map_or(&l.style, |s| &s.style),
                    )
                })
                .map(|l| (l.key.as_str(), l.position)),
        )
        .map(str::to_owned);
        if r.selected != selected {
            r.floors.cancel();
        }
        r.selected = selected;
        if let Some(location) = r
            .selected
            .as_ref()
            .and_then(|key| package.catalog.locations.iter().find(|l| &l.key == key))
        {
            let keys = world.resource::<ButtonInput<KeyCode>>();
            if r.floors.active() {
                if keys.just_pressed(KeyCode::Escape) || pressed & 0x2000 != 0 {
                    r.floors.cancel();
                    r.input.consume();
                } else {
                    if keys.just_pressed(KeyCode::ArrowUp) || pressed & 1 != 0 {
                        r.floors.navigate(-1);
                    }
                    if keys.just_pressed(KeyCode::ArrowDown) || pressed & 2 != 0 {
                        r.floors.navigate(1);
                    }
                    if edge || keys.just_pressed(KeyCode::Enter) {
                        r.input.consume();
                        if let Some(i) = r.floors.choose() {
                            request_entry(world, r, &location.key, &location.floors[i].key)?;
                        }
                    }
                }
            } else if edge {
                r.input.consume();
                if let Some(i) = r.floors.open(location.floors.len()) {
                    request_entry(world, r, &location.key, &location.floors[i].key)?;
                }
            }
        }
    } else {
        r.selected = None;
    }
    Ok(())
}
fn matrix(position: [f32; 3], heading: f32) -> [[f32; 4]; 4] {
    let mut m =
        Mat4::from_rotation_translation(Quat::from_rotation_y(heading), Vec3::from_array(position))
            .to_cols_array_2d();
    m[3][3] = 0.;
    m
}
fn request_entry(
    world: &mut World,
    r: &mut LocationRegistry,
    location: &str,
    floor: &str,
) -> Result<(), String> {
    let package = r.package.as_ref().ok_or("Interior catalog is not ready")?;
    let l = package
        .catalog
        .locations
        .iter()
        .find(|l| l.key == location)
        .ok_or("Unknown entrance")?;
    let f = l
        .floors
        .iter()
        .find(|f| f.key == floor)
        .ok_or("Unknown floor")?;
    let i = package
        .catalog
        .interiors
        .iter()
        .find(|i| i.key == f.interior)
        .ok_or("Unknown interior")?;
    r.request = r
        .request
        .checked_add(1)
        .ok_or("Interior request exhausted")?;
    let (owner, generation) = r
        .owners
        .iter()
        .find(|(_, (_, p))| {
            p.catalog
                .locations
                .iter()
                .any(|entry| entry.key == location)
        })
        .map(|(id, (g, _))| (id.clone(), *g))
        .ok_or("Entrance owner unavailable")?;
    let ticket = Ticket {
        owner,
        owner_generation: generation,
        map_generation: r.map_generation.unwrap(),
        request: r.request.to_string(),
    };
    let dedicated = world
        .resource::<crate::multiplayer::Multiplayer>()
        .is_dedicated();
    r.error = None;
    if !dedicated {
        if let Some(name) = r
            .settings
            .get(&ticket.owner)
            .and_then(|s| s.locations.iter().find(|s| s.key == location))
            .and_then(|s| s.interaction.as_ref())
        {
            if let Some(mut mods) = world.get_resource_mut::<crate::modding::Mods>() {
                mods.manager.call(&ticket.owner,"on_event",serde_json::json!({"type":"location","name":name,"location":location,"floor":floor}));
            }
        }
    }
    r.runtime.begin(
        Visit {
            ticket: ticket.clone(),
            location: l.key.clone(),
            floor: f.key.clone(),
            interior: i.key.clone(),
            return_position: l.return_position,
            return_heading: l.return_heading,
        },
        r.started.elapsed().as_secs_f64(),
    )?;
    r.runtime.ready(
        &ticket,
        r.started.elapsed().as_secs_f64(),
        true,
        r.ready,
        true,
        dedicated,
    );
    if dedicated {
        let owner_package = &r.owners[&ticket.owner].1;
        let intent = skate_resources::locations::EntryIntent {
            location: location.into(),
            floor: floor.into(),
            generation: ticket.owner_generation.to_string(),
            request: ticket.request.clone(),
        };
        crate::modding::location_event(
            world,
            &ticket.owner,
            ticket.owner_generation,
            "location_entry",
            serde_json::json!({"intent":intent,"revision":owner_package.revision}),
        )?;
        r.approval_epoch = None;
        r.travel = Some((
            world
                .resource::<crate::physics::SkaterRuntime>()
                .travel_generation,
            ticket,
            false,
        ));
        return Ok(());
    }
    let mut skater = world.resource_mut::<crate::physics::SkaterRuntime>();
    let before = skater.travel_generation;
    skater.travel(matrix(i.spawn, i.heading), Some([0.; 3]))?;
    r.travel = Some((before, ticket, false));
    Ok(())
}
fn request_return(world: &mut World, r: &mut LocationRegistry) -> Result<(), String> {
    let mut v = r.runtime.visit.clone().ok_or("Not inside an interior")?;
    if world
        .resource::<crate::multiplayer::Multiplayer>()
        .is_dedicated()
    {
        r.request = r
            .request
            .checked_add(1)
            .ok_or("Interior request exhausted")?;
        v.ticket.request = r.request.to_string();
        r.runtime.visit = Some(v.clone());
        let package = &r.owners[&v.ticket.owner].1;
        let intent = skate_resources::locations::EntryIntent {
            location: v.location.clone(),
            floor: v.floor.clone(),
            generation: v.ticket.owner_generation.to_string(),
            request: v.ticket.request.clone(),
        };
        crate::modding::location_event(
            world,
            &v.ticket.owner,
            v.ticket.owner_generation,
            "location_exit",
            serde_json::json!({"intent":intent,"revision":package.revision}),
        )?;
        r.runtime.return_requested();
        r.approval_epoch = None;
        r.travel = Some((
            world
                .resource::<crate::physics::SkaterRuntime>()
                .travel_generation,
            v.ticket,
            true,
        ));
        return Ok(());
    }
    catalog::support(
        world.resource::<crate::physics::GamePhysics>().world(),
        v.return_position,
    )?;
    let mut skater = world.resource_mut::<crate::physics::SkaterRuntime>();
    let prior = skater.travel_generation;
    skater.travel(matrix(v.return_position, v.return_heading), Some([0.; 3]))?;
    r.runtime.return_requested();
    r.travel = Some((prior, v.ticket, true));
    Ok(())
}
fn publish_model(world: &mut World, r: &mut LocationRegistry, key: &str, model: model::Model) {
    let images: Vec<_> = model
        .images
        .into_iter()
        .map(|i| world.resource_mut::<Assets<Image>>().add(i))
        .collect();
    let materials: Vec<_> = model
        .materials
        .into_iter()
        .zip(model.textures)
        .map(|(mut m, (base, emissive))| {
            m.base_color_texture = base.map(|i| images[i].clone());
            m.emissive_texture = emissive.map(|i| images[i].clone());
            world.resource_mut::<Assets<StandardMaterial>>().add(m)
        })
        .collect();
    for (mesh, material, at) in model.parts {
        let overview = super::model::overview(
            &mesh,
            model.bounds.0.y + (model.bounds.1.y - model.bounds.0.y) * 0.75,
        );
        let overview = world.resource_mut::<Assets<Mesh>>().add(overview);
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        let material = materials[material].clone();
        r.entities.push(
            world
                .spawn((
                    Name::new(format!("Interior {key}")),
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    at,
                ))
                .id(),
        );
        r.entities.push(crate::map_view::spawn_interior(
            world,
            key,
            overview,
            material,
            at,
            model.bounds,
        ));
    }
}
fn publish_markers(world: &mut World, r: &mut LocationRegistry) {
    let Some(p) = &r.package else {
        return;
    };
    for (key, position, style) in p
        .catalog
        .locations
        .iter()
        .map(|l| (Some(l.key.clone()), l.position, &l.style))
        .chain(
            p.catalog
                .interiors
                .iter()
                .map(|i| (None, i.exit.position, &i.exit.style)),
        )
    {
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::srgba(
                    style.color[0],
                    style.color[1],
                    style.color[2],
                    style.opacity,
                ),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                double_sided: true,
                cull_mode: None,
                ..default()
            });
        let cylinder = world.resource_mut::<Assets<Mesh>>().add(
            Cylinder::new(style.radius, style.height)
                .mesh()
                .resolution(48),
        );
        r.entities.push(
            world
                .spawn((
                    MarkerVisual {
                        key: key.clone(),
                        position,
                        base: style.clone(),
                        ring: false,
                        icon: false,
                    },
                    Name::new("Interior entry cylinder"),
                    Mesh3d(cylinder),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(position[0], position[1] + style.height / 2., position[2]),
                ))
                .id(),
        );
        let icon = world.resource_mut::<Assets<Mesh>>().add(house_mesh());
        let icon_material =
            world
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    base_color: Color::srgb(style.color[0], style.color[1], style.color[2]),
                    unlit: true,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                });
        r.entities.push(
            world
                .spawn((
                    MarkerVisual {
                        key: key.clone(),
                        position,
                        base: style.clone(),
                        ring: false,
                        icon: true,
                    },
                    Name::new("Interior house icon"),
                    Mesh3d(icon),
                    MeshMaterial3d(icon_material),
                    Transform::from_xyz(position[0], position[1] + style.height + 0.3, position[2]),
                ))
                .id(),
        );
        let ring = world
            .resource_mut::<Assets<Mesh>>()
            .add(Torus::new(style.radius - 0.035, style.radius + 0.035));
        r.entities.push(
            world
                .spawn((
                    MarkerVisual {
                        key: key.clone(),
                        position,
                        base: style.clone(),
                        ring: true,
                        icon: false,
                    },
                    Name::new("Interior entry ground ring"),
                    Mesh3d(ring),
                    MeshMaterial3d(material),
                    Transform::from_xyz(position[0], position[1] + 0.04, position[2]),
                ))
                .id(),
        );
    }
}
fn pins(world: &mut World, r: &LocationRegistry) -> Result<(), String> {
    let Some(p) = &r.package else {
        return Ok(());
    };
    let items = if let Some(visit) = r
        .runtime
        .visit
        .as_ref()
        .filter(|_| r.runtime.phase == Phase::Interior)
    {
        let i = p
            .catalog
            .interiors
            .iter()
            .find(|i| i.key == visit.interior)
            .unwrap();
        vec![
            serde_json::json!({"kind":"marker","key":"exit","position":i.exit.position,"label":"Exit"}),
        ]
    } else {
        p.catalog.locations.iter().filter_map(|l|{
        let setting=r.settings.values().flat_map(|s|&s.locations).find(|s|s.key==l.key);
        if setting.is_some_and(|s|!s.enabled){return None;}
        Some(serde_json::json!({"kind":"marker","key":l.key,"position":l.position,"label":format!("{}",setting.map_or(l.label.as_str(),|s|s.label.as_str()))}))
    }).collect()
    };
    crate::map_view::set_local(
        world,
        "engine-locations",
        skate_mods::map::MapSnapshot::parse(
            serde_json::json!({"layers":[{"key":"locations","items":items}]}),
        )?,
    )
}
fn draw_prompt(world: &mut World, r: &mut LocationRegistry) {
    let input = world.resource::<crate::input::ControllerInput>();
    let face = input
        .active_slot()
        .and_then(|s| input.kind(s))
        .map(|k| k.prompt_style.face_labels());
    let confirm = face.map_or("Enter / E", |f| if f[0] == "Cross" { "X" } else { f[0] });
    let cancel = face.map_or("Esc", |f| if f[1] == "Circle" { "O" } else { f[1] });
    let text = if r.floors.active() {
        r.package
            .as_ref()
            .and_then(|p| {
                p.catalog
                    .locations
                    .iter()
                    .find(|l| Some(&l.key) == r.selected.as_ref())
            })
            .map(|l| {
                format!(
                    "Choose a floor\n{}\nUp / Down   {confirm}   {cancel} to cancel",
                    l.floors
                        .iter()
                        .enumerate()
                        .map(|(n, f)| format!(
                            "{} {}",
                            if n == r.floors.selected { ">" } else { " " },
                            f.label
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            })
            .unwrap_or_default()
    } else if r.selected.is_some() {
        let input = world.resource::<crate::input::ControllerInput>();
        let button = input
            .active_slot()
            .and_then(|s| input.kind(s))
            .map(|k| k.prompt_style.face_labels()[0])
            .unwrap_or("E");
        format!(
            "Press {} to {}{}",
            if button == "Cross" { "X" } else { button },
            if r.runtime.phase == Phase::Interior {
                "exit"
            } else {
                "enter"
            },
            r.error
                .as_ref()
                .or(r.runtime.error.as_ref())
                .map_or(String::new(), |e| format!("\n{e}"))
        )
    } else {
        String::new()
    };
    let entity = match r.prompt.filter(|e| world.get_entity(*e).is_ok()) {
        Some(e) => e,
        None => {
            let e = world
                .spawn((
                    Text::default(),
                    TextFont {
                        font_size: 22.,
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(24),
                        top: px(24),
                        padding: UiRect::all(px(10)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0., 0., 0., 0.7)),
                    GlobalZIndex(8),
                ))
                .id();
            r.prompt = Some(e);
            e
        }
    };
    world.get_mut::<Node>(entity).unwrap().display = if text.is_empty() {
        Display::None
    } else {
        Display::Flex
    };
    world.get_mut::<Text>(entity).unwrap().0 = text;
}

fn aggregate(
    owners: &std::collections::BTreeMap<String, (u64, Arc<PreparedCatalog>)>,
) -> Result<PreparedCatalog, String> {
    use skate_resources::locations::{Catalog, catalog_revision};
    let mut catalog = Catalog {
        version: 1,
        map: owners
            .values()
            .next()
            .map_or("test-world".into(), |(_, p)| p.catalog.map.clone()),
        interiors: vec![],
        locations: vec![],
    };
    let mut files = std::collections::BTreeMap::new();
    for (owner, (_, p)) in owners {
        if !p.catalog.map.eq_ignore_ascii_case(&catalog.map) {
            return Err("Interior packages target different maps".into());
        }
        for i in &p.catalog.interiors {
            let mut i = i.clone();
            for path in [&mut i.model, &mut i.collision] {
                let target = format!("{owner}/{path}");
                files.insert(target.clone(), p.files[&*path].clone());
                *path = target;
            }
            catalog.interiors.push(i);
        }
        catalog.locations.extend(p.catalog.locations.clone());
    }
    let revision = catalog_revision(&catalog, &files)?;
    Ok(PreparedCatalog {
        catalog,
        files,
        revision,
    })
}
pub(crate) fn load(
    world: &mut World,
    owner: &str,
    generation: u64,
    package: PreparedCatalog,
) -> Result<bool, String> {
    let map = world
        .resource::<crate::map_transition::CurrentMap>()
        .path
        .as_ref()
        .and_then(|p| p.file_stem())
        .and_then(|p| p.to_str())
        .unwrap_or("test-world")
        .to_owned();
    if !package.catalog.map.eq_ignore_ascii_case(&map) {
        return Err("Interior catalog targets a different map".into());
    }
    let mut r = world.resource_mut::<LocationRegistry>();
    if let Some((g, revision, error)) = r.failed.get(owner) {
        if *g == generation && revision == &package.revision {
            return Err(error.clone());
        }
    }
    if r.retiring.iter().any(|id| id == owner)
        && r.runtime
            .visit
            .as_ref()
            .is_some_and(|v| v.ticket.owner == owner)
    {
        return Err("Interior owner is still retiring".into());
    }
    if let Some((g, p)) = r.owners.get(owner) {
        if *g == generation && p.revision == package.revision {
            r.retiring.retain(|id| id != owner);
            r.markers_dirty = true;
            return Ok(r.ready && !r.reload && r.loading.is_none());
        }
    }
    if r.retiring.iter().any(|id| id == owner) {
        return Err("Interior owner is still retiring".into());
    }
    if r.loading.is_some() || r.reload {
        if r.queued.len() >= 16 && !r.queued.contains_key(owner) {
            return Err("Interior loading queue is full".into());
        }
        r.queued.insert(owner.into(), (generation, package));
        return Ok(false);
    }
    if r.runtime
        .visit
        .as_ref()
        .is_some_and(|v| v.ticket.owner == owner)
    {
        return Err("Return occupants before replacing their interior catalog".into());
    }
    let mut owners = r.owners.clone();
    owners.insert(owner.into(), (generation, Arc::new(package)));
    aggregate(&owners)?;
    r.rollback = Some(r.owners.clone());
    r.owners = owners;
    r.reload = true;
    Ok(false)
}
pub(crate) fn retire(world: &mut World, owner: &str) {
    if let Some(mut r) = world.get_resource_mut::<LocationRegistry>() {
        r.queued.remove(owner);
        if r.owners.contains_key(owner) && !r.retiring.iter().any(|s| s == owner) {
            r.retiring.push(owner.into());
        }
    }
}
pub(crate) fn set(
    world: &mut World,
    owner: &str,
    snapshot: skate_resources::locations::LocationSnapshot,
) -> Result<(), String> {
    let mut r = world.resource_mut::<LocationRegistry>();
    let (generation, package) = r
        .owners
        .get(owner)
        .ok_or("Load the owner's interior catalog before changing its markers")?;
    snapshot.validate_catalog(&package.catalog, *generation)?;
    let now = Instant::now();
    let recent = r.publications.entry(owner.into()).or_default();
    recent.retain(|t| now.duration_since(*t).as_secs_f32() < 1.);
    if recent.len() >= 5 {
        return Err("5 location publications per second maximum".into());
    }
    recent.push(now);
    r.settings.insert(owner.into(), snapshot);
    r.markers_dirty = true;
    Ok(())
}
pub(crate) fn clear(world: &mut World, owner: &str) {
    if let Some(mut r) = world.get_resource_mut::<LocationRegistry>() {
        r.settings.remove(owner);
        r.markers_dirty = true;
    }
}

fn refresh_markers(world: &mut World, r: &LocationRegistry) {
    let updates: Vec<_> = world
        .query::<(Entity, &MarkerVisual, &MeshMaterial3d<StandardMaterial>)>()
        .iter(world)
        .map(|(e, v, m)| {
            let setting = v.key.as_ref().and_then(|k| {
                r.settings
                    .values()
                    .flat_map(|s| &s.locations)
                    .find(|s| &s.key == k)
            });
            let style = setting.map_or(&v.base, |s| &s.style);
            (
                e,
                m.0.clone(),
                setting.is_none_or(|s| s.enabled),
                style.clone(),
                Transform::from_xyz(
                    v.position[0],
                    v.position[1]
                        + if v.icon {
                            style.height + 0.3
                        } else if v.ring {
                            0.04
                        } else {
                            style.height / 2.
                        },
                    v.position[2],
                )
                .with_scale(if v.icon {
                    Vec3::ONE
                } else {
                    Vec3::new(
                        style.radius / v.base.radius,
                        if v.ring {
                            1.
                        } else {
                            style.height / v.base.height
                        },
                        style.radius / v.base.radius,
                    )
                }),
            )
        })
        .collect();
    for (e, m, enabled, style, at) in updates {
        world.entity_mut(e).insert((
            at,
            if enabled {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ));
        if let Some(material) = world.resource_mut::<Assets<StandardMaterial>>().get_mut(&m) {
            material.base_color = Color::srgba(
                style.color[0],
                style.color[1],
                style.color[2],
                style.opacity,
            );
        }
    }
}

pub(crate) fn approval(world: &mut World, owner: &str, generation: u64, value: &serde_json::Value) {
    let Some(mut r) = world.get_resource_mut::<LocationRegistry>() else {
        return;
    };
    let Some(v) = r.runtime.visit.as_ref() else {
        return;
    };
    if v.ticket.owner != owner
        || v.ticket.owner_generation != generation
        || value.get("request").and_then(|v| v.as_str()) != Some(v.ticket.request.as_str())
    {
        return;
    }
    if value.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        r.approval_epoch = value
            .get("epoch")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());
    } else {
        let error = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Interior travel rejected")
            .to_string();
        r.runtime.fail(error.clone());
        if r.runtime.phase == Phase::Returning {
            r.runtime.phase = Phase::Interior;
        }
        r.error = Some(error);
        r.travel = None;
    }
}
pub(crate) fn catalogs(world: &World) -> Vec<Arc<PreparedCatalog>> {
    world
        .get_resource::<LocationRegistry>()
        .map_or_else(Vec::new, |r| {
            r.owners.values().map(|(_, p)| p.clone()).collect()
        })
}

pub(crate) fn load_status(
    world: &World,
    owner: &str,
    generation: u64,
    revision: &str,
) -> Option<bool> {
    let r = world.get_resource::<LocationRegistry>()?;
    if r.retiring.iter().any(|id| id == owner) {
        return None;
    }
    if r.queued
        .get(owner)
        .is_some_and(|(g, p)| *g == generation && p.revision == revision)
    {
        return Some(false);
    }
    r.owners
        .get(owner)
        .filter(|(g, p)| *g == generation && p.revision == revision)
        .map(|_| r.ready && !r.reload && r.loading.is_none())
}

fn house_mesh() -> Mesh {
    use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology};
    let mut points = Vec::new();
    let lines = [
        ([-0.22, 0.], [0., 0.22]),
        ([0., 0.22], [0.22, 0.]),
        ([-0.17, 0.03], [-0.17, -0.2]),
        ([-0.17, -0.2], [0.17, -0.2]),
        ([0.17, -0.2], [0.17, 0.03]),
        ([-0.04, -0.2], [-0.04, -0.06]),
        ([-0.04, -0.06], [0.04, -0.06]),
        ([0.04, -0.06], [0.04, -0.2]),
    ];
    for (a, b) in lines {
        let a = Vec2::from_array(a);
        let b = Vec2::from_array(b);
        let d = (b - a).normalize();
        let n = Vec2::new(-d.y, d.x) * 0.018;
        for p in [a - n, b - n, b + n, a - n, b + n, a + n] {
            points.push([p.x, p.y, 0.]);
        }
    }
    let n = points.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, points)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; n])
}

/// Resource admission must exclude every local-only collision owner before ready.
pub(crate) fn retain_admitted(
    world: &mut World,
    allowed: &std::collections::BTreeSet<&str>,
) -> bool {
    let Some(mut r) = world.get_resource_mut::<LocationRegistry>() else {
        return true;
    };
    r.queued.retain(|owner, _| allowed.contains(owner.as_str()));
    let extras: Vec<_> = r
        .owners
        .keys()
        .filter(|owner| !allowed.contains(owner.as_str()))
        .cloned()
        .collect();
    for owner in &extras {
        if !r.retiring.contains(owner) {
            r.retiring.push(owner.clone());
        }
    }
    extras.is_empty() && r.loading.is_none() && !r.reload
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    #[test]
    fn occupied_readmission_cannot_cancel_pending_return() {
        let package = PreparedCatalog {
            catalog: skate_resources::locations::Catalog {
                version: 1,
                map: "test-world".into(),
                interiors: vec![],
                locations: vec![],
            },
            files: Default::default(),
            revision: "same".into(),
        };
        let mut registry = LocationRegistry::default();
        registry.ready = true;
        registry
            .owners
            .insert("engine-locations".into(), (1, Arc::new(package.clone())));
        registry.retiring.push("engine-locations".into());
        registry.runtime.visit = Some(Visit {
            ticket: Ticket {
                owner: "engine-locations".into(),
                owner_generation: 1,
                map_generation: 0,
                request: "1".into(),
            },
            location: "entry".into(),
            floor: "one".into(),
            interior: "room".into(),
            return_position: [0.; 3],
            return_heading: 0.,
        });
        registry.runtime.phase = Phase::Interior;
        let mut world = World::new();
        world.insert_resource(registry);
        world.insert_resource(crate::map_transition::CurrentMap {
            path: None,
            name: "Test world".into(),
            spawn: [0.; 3],
            heading: 0.,
            generation: 0,
            audio_tag: None,
        });
        assert_eq!(
            load(&mut world, "engine-locations", 1, package.clone()).unwrap_err(),
            "Interior owner is still retiring"
        );
        assert_eq!(
            world.resource::<LocationRegistry>().retiring,
            vec!["engine-locations"]
        );
        world.resource_mut::<LocationRegistry>().runtime = Runtime::default();
        assert!(load(&mut world, "engine-locations", 1, package).unwrap());
        assert!(world.resource::<LocationRegistry>().retiring.is_empty());
    }
    #[test]
    fn readiness_requires_exact_admitted_revision_and_excludes_local_owners() {
        let package = PreparedCatalog {
            catalog: skate_resources::locations::Catalog {
                version: 1,
                map: "test-world".into(),
                interiors: vec![],
                locations: vec![],
            },
            files: Default::default(),
            revision: "local-old".into(),
        };
        let mut registry = LocationRegistry::default();
        registry.ready = true;
        registry
            .owners
            .insert("engine-locations".into(), (1, Arc::new(package.clone())));
        registry
            .owners
            .insert("local-mod".into(), (1, Arc::new(package)));
        let mut world = World::new();
        world.insert_resource(registry);
        assert_eq!(
            load_status(&world, "engine-locations", 1, "server-new"),
            None
        );
        assert_eq!(
            load_status(&world, "engine-locations", 1, "local-old"),
            Some(true)
        );
        assert!(!retain_admitted(
            &mut world,
            &["engine-locations"].into_iter().collect()
        ));
        assert_eq!(
            world.resource::<LocationRegistry>().retiring,
            vec!["local-mod"]
        );
    }
}

pub(crate) fn interior_label(world: &World, key: &str) -> String {
    world
        .get_resource::<LocationRegistry>()
        .and_then(|r| r.package.as_ref())
        .and_then(|p| {
            p.catalog
                .locations
                .iter()
                .flat_map(|l| &l.floors)
                .find(|f| f.interior == key)
        })
        .map_or_else(|| key.to_string(), |floor| floor.label.clone())
}
