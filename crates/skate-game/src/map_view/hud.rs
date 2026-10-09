//! UI and offscreen-target ownership. World geometry is owned by MapAssets.
use super::{MapOverviewLod, OverviewScene, input::MapViewState, markers::PlayerMarker, view};
use bevy::{
    camera::{RenderTarget, visibility::RenderLayers},
    image::ImageSampler,
    prelude::*,
    render::render_resource::TextureFormat,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Resource)]
struct Hud {
    root: Entity,
    image: Entity,
    title: Entity,
    hint: Entity,
    status: Entity,
    camera: Entity,
    light: Entity,
    controls: Vec<(super::navigation::Control, Entity, Entity)>,
    target: Handle<Image>,
    key: (Option<u64>, u8),
    labels: BTreeMap<Option<u64>, (Entity, Entity)>,
}

fn target(world: &mut World, mode: u8) -> Handle<Image> {
    let size = match mode {
        0 => (640, 480),
        1 => (1280, 960),
        _ => (1920, 1440),
    };
    let mut image = Image::new_target_texture(size.0, size.1, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::linear();
    world.resource_mut::<Assets<Image>>().add(image)
}

fn create(world: &mut World, output: Entity) -> Hud {
    let target = target(world, 0);
    let camera = world
        .spawn((
            Name::new("Live map camera"),
            Camera3d::default(),
            AmbientLight {
                brightness: 350.,
                ..default()
            },
            Camera {
                order: -2,
                is_active: false,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            RenderLayers::layer(super::LAYER),
            Msaa::Off,
        ))
        .id();
    let light = world
        .spawn((
            Name::new("Live map sunlight"),
            DirectionalLight {
                illuminance: 11000.,
                shadows_enabled: true,
                ..default()
            },
            Transform::from_xyz(-3., 5., 2.).looking_at(Vec3::ZERO, Vec3::Y),
            RenderLayers::layer(super::LAYER),
            bevy::light::CascadeShadowConfigBuilder {
                num_cascades: 1,
                maximum_distance: 30000.,
                ..default()
            }
            .build(),
        ))
        .id();
    // Explicit UI target: no UI is ever assigned to the map camera.
    let root = world
        .spawn((
            Name::new("Live map"),
            UiTargetCamera(output),
            GlobalZIndex(6),
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                overflow: Overflow::clip(),
                border: UiRect::all(px(1.)),
                ..default()
            },
            BorderColor::all(Color::srgba(0.6, 0.65, 0.68, 0.4)),
            BackgroundColor(Color::srgba(0.02, 0.025, 0.03, 0.7)),
        ))
        .id();
    let title = world
        .spawn((
            Text::new("LIVE MAP"),
            TextFont {
                font_size: 12.,
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.92, 0.94)),
            Node {
                position_type: PositionType::Absolute,
                left: px(10.),
                top: px(6.),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    let image = world
        .spawn((
            ImageNode {
                color: Color::WHITE.with_alpha(0.92),
                ..ImageNode::new(target.clone())
            },
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: px(0.),
                top: px(28.),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .id();
    let mut controls = Vec::new();
    for control in super::navigation::Control::ALL {
        let entity = world
            .spawn((
                Name::new(format!("Map {control:?}")),
                Pickable::IGNORE,
                Node {
                    position_type: PositionType::Absolute,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.025, 0.035, 0.045, 0.92)),
                GlobalZIndex(8),
            ))
            .id();
        let child = if control == super::navigation::Control::Tilt {
            world
                .spawn((
                    Pickable::IGNORE,
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(18.),
                        height: px(8.),
                        ..default()
                    },
                    BackgroundColor(Color::WHITE),
                ))
                .id()
        } else {
            world
                .spawn((
                    Text::new(control.label(false)),
                    TextFont {
                        font_size: 11.,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    Pickable::IGNORE,
                ))
                .id()
        };
        world.entity_mut(entity).add_child(child);
        world.entity_mut(image).add_child(entity);
        controls.push((control, entity, child));
    }
    let status = world
        .spawn((
            Text::new("Map unavailable"),
            TextFont {
                font_size: 14.,
                ..default()
            },
            TextColor(Color::WHITE),
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: px(12.),
                top: px(55.),
                ..default()
            },
        ))
        .id();
    let hint = world
        .spawn((
            Text::new("M / D-PAD LEFT  ·  MAP"),
            TextFont {
                font_size: 11.,
                ..default()
            },
            TextColor(Color::srgb(0.5, 0.8, 0.8)),
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: px(10.),
                bottom: px(6.),
                ..default()
            },
        ))
        .id();
    world
        .entity_mut(root)
        .add_children(&[image, title, status, hint]);
    Hud {
        root,
        image,
        title,
        hint,
        status,
        camera,
        light,
        controls,
        target,
        key: (None, 0),
        labels: BTreeMap::new(),
    }
}

fn replace_target(world: &mut World, hud: &mut Hud, key: (Option<u64>, u8)) {
    if hud.key == key {
        return;
    }
    let target = target(world, key.1);
    world
        .entity_mut(hud.camera)
        .insert(RenderTarget::Image(target.clone().into()));
    world.get_mut::<ImageNode>(hud.image).unwrap().image = target.clone();
    world
        .resource_mut::<Assets<Image>>()
        .remove(hud.target.id());
    hud.target = target;
    // No label may cross a generation boundary, even if the next map is empty.
    for (_, (entity, _)) in std::mem::take(&mut hud.labels) {
        world.despawn(entity);
    }
    hud.key = key;
}

fn labels(
    world: &mut World,
    hud: &mut Hud,
    mut desired: Vec<(Option<u64>, Vec2, String)>,
    size: Vec2,
) {
    // Stable priority: YOU first, then peer identity. Move text, never the dots.
    desired.sort_by_key(|(id, _, _)| *id);
    let mut occupied: Vec<Rect> = Vec::new();
    let ids: BTreeSet<_> = desired.iter().map(|(id, _, _)| *id).collect();
    hud.labels.retain(|id, (entity, _)| {
        if ids.contains(id) {
            true
        } else {
            world.despawn(*entity);
            false
        }
    });
    for (id, uv, name) in desired {
        let color = if id.is_none() {
            Color::srgb(0.65, 1., 0.65)
        } else {
            Color::srgb(0.3, 0.9, 1.)
        };
        let (entity, text) = *hud.labels.entry(id).or_insert_with(|| {
            let entity = world
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    ZIndex(if id.is_none() { 10 } else { 1 }),
                    Pickable::IGNORE,
                ))
                .id();
            let dot = world
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(-3.),
                        top: px(-3.),
                        width: px(6.),
                        height: px(6.),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(color),
                    Pickable::IGNORE,
                ))
                .id();
            let text = world
                .spawn((
                    Text::new(""),
                    TextFont {
                        font_size: if id.is_none() { 14. } else { 12. },
                        ..default()
                    },
                    TextColor(color),
                    BackgroundColor(Color::srgba(0., 0.015, 0.02, 0.85)),
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(130.),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .id();
            world.entity_mut(entity).add_children(&[dot, text]);
            world.entity_mut(hud.image).add_child(entity);
            (entity, text)
        });
        let p = uv * size;
        let mut node = world.get_mut::<Node>(entity).unwrap();
        node.left = px(p.x);
        node.top = px(p.y);
        let font_size = if id.is_none() { 14. } else { 12. };
        let label_width = (name.chars().count() as f32 * font_size * 0.65 + 6.).min(size.x);
        let preferred = if p.x + label_width + 8. > size.x {
            p.x - label_width - 8.
        } else {
            p.x + 8.
        };
        let left = preferred.clamp(0., (size.x - label_width).max(0.));
        let height = font_size + 4.;
        let mut rect = Rect::default();
        for row in 0..occupied.len() * 2 + 3 {
            let offset = if row % 2 == 0 {
                -height - (row / 2) as f32 * (height + 3.)
            } else {
                6. + (row / 2) as f32 * (height + 3.)
            };
            let top = (p.y + offset).clamp(0., (size.y - height).max(0.));
            rect = Rect::from_corners(
                Vec2::new(left, top),
                Vec2::new(left + label_width, top + height),
            );
            if !occupied.iter().any(|other| {
                rect.min.x < other.max.x
                    && rect.max.x > other.min.x
                    && rect.min.y < other.max.y
                    && rect.max.y > other.min.y
            }) {
                break;
            }
        }
        occupied.push(rect);
        let mut node = world.get_mut::<Node>(text).unwrap();
        node.width = px(label_width);
        node.left = px(rect.min.x - p.x);
        node.top = px(rect.min.y - p.y);
        world.get_mut::<Text>(text).unwrap().0 = name;
    }
}

// Read current local transforms before UI layout (which itself precedes Bevy's
// transform propagation). Walking ancestors also supports attached player roots.
pub(super) fn position(world: &World, entity: Entity) -> Option<Vec3> {
    Some(player_matrix(world, entity)?.transform_point3(Vec3::ZERO))
}

fn player_matrix(world: &World, mut entity: Entity) -> Option<Mat4> {
    let mut matrix = world.get::<Transform>(entity)?.to_matrix();
    while let Some(parent) = world.get::<ChildOf>(entity) {
        entity = parent.parent();
        matrix = world.get::<Transform>(entity)?.to_matrix() * matrix;
    }
    Some(matrix)
}

fn facing_yaw(world: &World, entity: Entity) -> Option<f32> {
    let forward = player_matrix(world, entity)?.transform_vector3(Vec3::Z);
    (forward.is_finite() && forward.xz().length_squared() > 1e-6)
        .then(|| (-forward.x).atan2(-forward.z))
}

pub(super) fn draw(world: &mut World) {
    super::material::reconcile(world);
    let Some((output, screen)) = world
        .query_filtered::<(Entity, &Camera), With<IsDefaultUiCamera>>()
        .iter(world)
        .find_map(|(e, c)| c.logical_viewport_size().map(|s| (e, s)))
    else {
        return;
    };
    let ui_scale = world
        .get_resource::<UiScale>()
        .map_or(1., |s| s.0)
        .max(0.01);
    let screen = screen / ui_scale;
    let current_generation = world
        .get_resource::<crate::map_transition::CurrentMap>()
        .map_or(0, |m| m.generation);
    world
        .resource_mut::<super::layers::MapLayerRegistry>()
        .retain_generation(current_generation);
    let settings = world
        .resource::<super::layers::MapLayerRegistry>()
        .settings();
    let state = world.resource::<MapViewState>();
    let (visible, expanded, generation, zoom_level, yaw, pitch) = (
        state.visible && settings.enabled.unwrap_or(true),
        state.expanded,
        state.generation,
        state.zoom_level,
        state.yaw,
        state.pitch,
    );
    let mut hud = world
        .remove_resource::<Hud>()
        .unwrap_or_else(|| create(world, output));
    replace_target(
        world,
        &mut hud,
        (
            generation,
            if !expanded {
                0
            } else if zoom_level >= super::DETAIL_ZOOM_LEVEL {
                2
            } else {
                1
            },
        ),
    );
    world.get_mut::<Node>(hud.root).unwrap().display = if visible {
        Display::Flex
    } else {
        Display::None
    };
    world.get_mut::<Camera>(hud.camera).unwrap().is_active = false;
    if !visible {
        world
            .resource_mut::<super::navigation::MapNavigation>()
            .frame = None;
        world.insert_resource(hud);
        return;
    }
    let max_width = (screen.x * 0.9).min((screen.y * 0.9 - 56.).max(1.) * 4. / 3.);
    let width = if expanded {
        max_width
    } else {
        240_f32.min(max_width)
    }
    .max(1.);
    let size = Vec2::new(width, width * 0.75);
    let mut node = world.get_mut::<Node>(hud.root).unwrap();
    node.width = px(size.x + 2.);
    node.height = px(size.y + 56.);
    node.left = px(if expanded {
        (screen.x - size.x) * 0.5
    } else {
        (screen.x - size.x - 18.).max(0.)
    });
    node.top = px(if expanded {
        (screen.y - size.y - 56.) * 0.5
    } else {
        18.
    });
    let mut node = world.get_mut::<Node>(hud.image).unwrap();
    node.width = px(size.x);
    node.height = px(size.y);
    let name = world
        .get_resource::<crate::map_transition::CurrentMap>()
        .map_or("Live map", |m| m.name.as_str())
        .to_owned();
    let name=world.get_resource::<super::context::Context>().and_then(|c|c.selected.as_deref()).map(|key|crate::locations::interior_label(world,key)).unwrap_or(name);
    world.get_mut::<Text>(hud.title).unwrap().0 = settings.title.unwrap_or(name);
    world.get_mut::<ImageNode>(hud.image).unwrap().color =
        Color::WHITE.with_alpha(settings.opacity.unwrap_or(0.92));
    world.get_mut::<Text>(hud.hint).unwrap().0 = if expanded {
        "DRAG / ARROWS · PAN   COMPASS · ROTATE   2D/3D · VIEW   YOU · RECENTER".into()
    } else {
        "M / D-PAD LEFT  ·  MAP".into()
    };
    let selected_lod = if !expanded {
        2
    } else if zoom_level == 0 {
        0
    } else if zoom_level < super::DETAIL_ZOOM_LEVEL {
        1
    } else {
        2
    };
    let selected_context=world.get_resource::<super::context::Context>().and_then(|c|c.selected.clone());
    let available: BTreeSet<_> = world
        .query::<(&MapOverviewLod,Option<&super::context::InteriorOverview>)>()
        .iter(world)
        .filter(|(_,key)|key.map(|k|&k.0)==selected_context.as_ref())
        .map(|(lod,_)| lod.level)
        .collect();
    let selected_lod = available
        .iter()
        .copied()
        .filter(|level| *level <= selected_lod)
        .max()
        .or_else(|| available.iter().next().copied())
        .unwrap_or(0);
    let lod_entities: Vec<_> = world
        .query::<(Entity, &MapOverviewLod,Option<&super::context::InteriorOverview>)>()
        .iter(world)
        .map(|(entity, lod,key)| (entity, lod.level,key.map(|k|&k.0)==selected_context.as_ref()))
        .collect();
    for (entity, lod_is_detailed, context_matches) in lod_entities {
        if let Some(mut visibility) = world.get_mut::<Visibility>(entity) {
            *visibility = if context_matches && lod_is_detailed == selected_lod {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
    let bounds = world
        .query::<(&OverviewScene,Option<&super::context::InteriorOverview>)>()
        .iter(world)
        .find(|(_,key)|key.map(|k|&k.0)==selected_context.as_ref())
        .map(|(s,_)| s.0);
    world.resource_mut::<MapViewState>().bounds = bounds.map(|b| (b.min, b.max));
    world.get_mut::<Node>(hud.status).unwrap().display = if bounds.is_some() {
        Display::None
    } else {
        Display::Flex
    };
    world.get_mut::<Node>(hud.image).unwrap().display = if bounds.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    let Some(bounds) = bounds else {
        world
            .resource_mut::<super::navigation::MapNavigation>()
            .frame = None;
        labels(world, &mut hud, vec![], size);
        world.insert_resource(hud);
        return;
    };
    let local_entity = world
        .query_filtered::<Entity, With<crate::world::PlayerRoot>>()
        .iter(world)
        .next();
    let local = local_entity.and_then(|e| position(world, e));
    let viewport = Rect::from_corners(
        Vec2::new(
            if expanded {
                (screen.x - size.x) * 0.5 + 1.
            } else {
                (screen.x - size.x - 18.).max(0.) + 1.
            },
            if expanded {
                (screen.y - size.y - 56.) * 0.5 + 29.
            } else {
                47.
            },
        ),
        Vec2::new(
            if expanded {
                (screen.x - size.x) * 0.5 + 1.
            } else {
                (screen.x - size.x - 18.).max(0.) + 1.
            },
            if expanded {
                (screen.y - size.y - 56.) * 0.5 + 29.
            } else {
                47.
            },
        ) + size,
    );
    let (center, top_down, zoom_fraction) = {
        let mut state = world.resource_mut::<MapViewState>();
        if expanded && state.center.is_none() {
            state.center = Some(if zoom_level == 0 {
                bounds.min * 0.5 + bounds.max * 0.5
            } else {
                local.unwrap_or(bounds.min * 0.5 + bounds.max * 0.5)
            });
        }
        (state.center, state.top_down, state.zoom_fraction)
    };
    for &(control, entity, child) in &hud.controls {
        let rect = super::navigation::control_rect(Rect::from_corners(Vec2::ZERO, size), control);
        let mut node = world.get_mut::<Node>(entity).unwrap();
        node.display = if expanded {
            Display::Flex
        } else {
            Display::None
        };
        node.left = px(rect.min.x);
        node.top = px(rect.min.y);
        node.width = px(rect.width());
        node.height = px(rect.height());
        if control == super::navigation::Control::Tilt {
            world.get_mut::<Node>(child).unwrap().top = px((1.
                - (if top_down { 90. } else { pitch.to_degrees() } - 25.) / 65.)
                * (rect.height() - 8.));
        } else {
            world.get_mut::<Text>(child).unwrap().0 =
                if control == super::navigation::Control::Compass {
                    format!("N {:.0}°", yaw.to_degrees())
                } else {
                    control.label(top_down).into()
                };
        }
    }
    let frame = if expanded {
        let frame_fn = if selected_context.is_some() {view::frame_interior} else {view::frame_free};
        frame_fn(
            bounds,
            center.unwrap(),
            4. / 3.,
            1.4_f32.powf(f32::from(zoom_level) + zoom_fraction),
            yaw,
            if top_down {
                std::f32::consts::FRAC_PI_2
            } else {
                pitch
            },
        )
    } else {
        view::frame_orbit(
            bounds,
            local.unwrap_or(bounds.min * 0.5 + bounds.max * 0.5),
            false,
            4. / 3.,
            1.,
            local_entity.and_then(|e| facing_yaw(world, e)).unwrap_or(0.),
            65_f32.to_radians(),
        )
    };
    super::material::update(world, &frame, local.filter(|_| !expanded));
    world.get_mut::<DirectionalLight>(hud.light).unwrap().shadows_enabled = expanded;
    {
        let mut nav = world.resource_mut::<super::navigation::MapNavigation>();
        nav.viewport = viewport;
        nav.frame = Some(frame.clone());
        nav.generation = generation;
    }
    let radius = (bounds.max - bounds.min).length().max(120.);
    world.entity_mut(hud.light).insert(
        bevy::light::CascadeShadowConfigBuilder {
            num_cascades: 1,
            minimum_distance: (frame.projection.far - radius * 3. - 100.).max(0.1),
            maximum_distance: frame.projection.far,
            ..default()
        }
        .build(),
    );
    world.entity_mut(hud.camera).insert((
        frame.transform,
        GlobalTransform::from(frame.transform),
        Projection::Perspective(frame.projection.clone()),
    ));
    world.get_mut::<Camera>(hud.camera).unwrap().is_active = true;
    super::overlay::draw(world, hud.image, &frame, size);
    let mut players = Vec::new();
    if let Some(position) = local {
        players.push(PlayerMarker {
            id: None,
            position,
            label: "YOU".into(),
        });
    }
    let actor_entities: Vec<_> = world
        .query::<(Entity, &crate::multiplayer::NetworkActor)>()
        .iter(world)
        .map(|(e, id)| (e, id.0))
        .collect();
    let actors: Vec<_> = actor_entities
        .into_iter()
        .filter_map(|(e, id)| {let p=position(world,e)?;super::context::contains(world,p).then_some((id,p))})
        .collect();
    if let Some(net) = world
        .get_resource::<crate::multiplayer::Multiplayer>()
        .filter(|n| n.active())
    {
        let local_id = net.mod_identity().1.to_string();
        for (id, position) in actors {
            if world
                .get_resource::<crate::modding::Mods>()
                .is_some_and(|m| crate::modding::peer_suspended(m, id))
            {
                continue;
            }
            players.push(PlayerMarker {
                id: Some(id),
                position,
                label: net.skater_name(&id.to_string(), &local_id),
            });
        }
    }
    labels(
        world,
        &mut hud,
        super::markers::project_markers(&frame, &players),
        size,
    );
    world.insert_resource(hud);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_heading_uses_current_player_and_parent_transforms() {
        let mut world = World::new();
        let parent = world.spawn(Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))).id();
        let player = world.spawn((Transform::default(), ChildOf(parent))).id();
        let yaw = facing_yaw(&world, player).unwrap();
        let bounds = super::super::geometry::OverviewBounds { min: Vec3::splat(-100.), max: Vec3::splat(100.) };
        let frame = view::frame_orbit(bounds, Vec3::ZERO, false, 4./3., 1., yaw, 65_f32.to_radians());
        let uv = view::project(&frame, Vec3::X * 10.).unwrap();
        assert!((uv.x - 0.5).abs() < 0.001 && uv.y < 0.5);
        world.get_mut::<Transform>(player).unwrap().rotation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        assert!(facing_yaw(&world, player).is_none());
    }
    #[test]
    fn generations_and_modes_release_images_and_labels() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let output = world.spawn_empty().id();
        let mut hud = create(&mut world, output);
        for generation in 0..3 {
            for expanded in [false, true, false] {
                let old = hud.target.id();
                labels(
                    &mut world,
                    &mut hud,
                    vec![
                        (None, Vec2::splat(0.5), "YOU".into()),
                        (Some(8), Vec2::splat(0.4), "Alex".into()),
                    ],
                    Vec2::new(240., 180.),
                );
                let old_entities: Vec<_> = hud.labels.values().map(|(e, _)| *e).collect();
                replace_target(&mut world, &mut hud, (Some(generation), u8::from(expanded)));
                assert!(!world.resource::<Assets<Image>>().contains(old));
                assert_eq!(world.resource::<Assets<Image>>().len(), 1);
                assert!(old_entities.iter().all(|e| world.get_entity(*e).is_err()));
                assert!(hud.labels.is_empty());
                let image = world.resource::<Assets<Image>>().get(&hud.target).unwrap();
                assert_eq!(image.width(), if expanded { 1280 } else { 640 });
            }
        }
    }
    #[test]
    fn labels_reconcile_without_accumulating_disconnected_players() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let output = world.spawn_empty().id();
        let mut hud = create(&mut world, output);
        let baseline = world.query::<Entity>().iter(&world).count();
        labels(
            &mut world,
            &mut hud,
            vec![(None, Vec2::splat(0.5), "YOU".into())],
            Vec2::new(240., 180.),
        );
        let (_, text) = hud.labels[&None];
        let node = world.get::<Node>(text).unwrap();
        let (Val::Px(left), Val::Px(width)) = (node.left, node.width) else {
            panic!("pixel layout");
        };
        assert!(
            120. + left >= 0. && 120. + left + width <= 240.,
            "YOU must fit inside compact map"
        );
        labels(&mut world, &mut hud, vec![], Vec2::new(240., 180.));
        for _ in 0..3 {
            labels(
                &mut world,
                &mut hud,
                vec![(Some(9), Vec2::splat(0.5), "Alex".into())],
                Vec2::new(240., 180.),
            );
            let (entity, text) = hud.labels[&Some(9)];
            labels(
                &mut world,
                &mut hud,
                vec![(Some(9), Vec2::new(0.25, 0.75), "Sam".into())],
                Vec2::new(240., 180.),
            );
            assert_eq!(world.get::<Text>(text).unwrap().0, "Sam");
            assert_eq!(world.get::<Node>(entity).unwrap().left, px(60.));
            labels(&mut world, &mut hud, vec![], Vec2::new(240., 180.));
            assert_eq!(world.query::<Entity>().iter(&world).count(), baseline);
        }
    }
}

#[cfg(test)]
mod overlap_tests {
    use super::*;
    #[test]
    fn nearby_players_keep_readable_names_and_exact_dots() {
        let mut world = World::new();
        world.init_resource::<Assets<Image>>();
        let output = world.spawn_empty().id();
        let mut hud = create(&mut world, output);
        labels(
            &mut world,
            &mut hud,
            vec![
                (None, Vec2::splat(0.5), "YOU".into()),
                (Some(1), Vec2::splat(0.5), "Player".into()),
            ],
            Vec2::new(240., 180.),
        );
        let rect = |id| {
            let (dot, text) = hud.labels[&id];
            let dot = world.get::<Node>(dot).unwrap();
            assert_eq!((dot.left, dot.top), (px(120.), px(90.)));
            let node = world.get::<Node>(text).unwrap();
            let (Val::Px(left), Val::Px(top), Val::Px(width)) = (node.left, node.top, node.width)
            else {
                panic!("pixel layout");
            };
            Rect::from_corners(Vec2::new(left, top), Vec2::new(left + width, top + 16.))
        };
        let a = rect(None);
        let b = rect(Some(1));
        assert!(
            a.max.y <= b.min.y || b.max.y <= a.min.y || a.max.x <= b.min.x || b.max.x <= a.min.x,
            "nearby names overlap: {a:?}, {b:?}"
        );
    }
}
