//! Native, client-side overview of the active world.
mod context;
pub(crate) use context::{set_context,spawn_interior,reset_context};
mod geometry;
mod hud;
mod input;
pub(crate) use input::MapViewState;
mod layers;
mod markers;
mod material;
mod navigation;
mod overlay;
pub(crate) use layers::{clear as clear_layers, remove_owner, set_local, set_server, status};
mod view;

use bevy::prelude::*;
const LAYER: usize = 30;
const DETAIL_ZOOM_LEVEL: u8 = 2;

pub(crate) struct MapViewPlugin;
impl Plugin for MapViewPlugin {
    fn build(&self, app: &mut App) {
        material::register(app);
        app.init_resource::<input::MapViewState>()
            .init_resource::<layers::MapLayerRegistry>()
            .init_resource::<navigation::MapNavigation>()
            .init_resource::<navigation::NavigationEvents>()
            .add_systems(
                PreUpdate,
                (navigation::collect, input::update)
                    .chain()
                    .after(bevy::input::InputSystems)
                    .after(crate::graphics_menu::MenuInput)
                    .after(crate::map_transition::MapTransitionSet),
            )
            .add_systems(
                PostUpdate,
                hud::draw
                    .before(bevy::camera::CameraUpdateSystems)
                    .before(bevy::ui::UiSystems::Prepare),
            );
    }
}

pub(crate) fn prepare(
    map: Option<&skate_data::skate_map::SkateMap>,
    commands: &mut crate::map_render::SceneCommands,
    meshes: &mut impl crate::map_render::AssetSink<Mesh>,
    materials: &mut impl crate::map_render::AssetSink<StandardMaterial>,
    images: &mut impl crate::map_render::AssetSink<Image>,
) {
    let levels = match map {
        Some(map) => geometry::build_lods(map).map(|lods| lods.levels),
        None => geometry::from_test_world().map(|g| vec![(0, g)]),
    };
    let Some(levels) = levels else {
        return;
    };
    let mut material_handles = std::collections::HashMap::new();
    let mut texture_handles = std::collections::HashMap::new();
    for (level, geometry) in &levels {
        for (material_id, (positions, uvs)) in buckets(geometry) {
            let handle = if let Some(map) = map {
                let Some(handle) = map_material(
                    map,
                    material_id,
                    &mut material_handles,
                    &mut texture_handles,
                    materials,
                    images,
                ) else {
                    continue;
                };
                handle
            } else {
                materials.add(StandardMaterial {
                    base_color: Color::srgb(0.66, 0.68, 0.64),
                    unlit: false,
                    perceptual_roughness: 0.95,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })
            };
            spawn_overview_mesh(
                geometry.bounds,
                positions,
                uvs,
                handle,
                *level,
                commands,
                meshes,
            );
        }
    }
}

fn map_alpha_mode(mode: u32, cutoff: f32) -> AlphaMode {
    // Map translucency belongs to the finished image. Terrain must write depth
    // so city facades and roofs occlude one another normally.
    match mode {
        0 => AlphaMode::Opaque,
        1 => AlphaMode::Mask(cutoff),
        _ => AlphaMode::Mask(0.1),
    }
}

fn buckets(
    geometry: &geometry::OverviewGeometry,
) -> std::collections::BTreeMap<u32, (Vec<[f32; 3]>, Vec<[f32; 2]>)> {
    let mut buckets = std::collections::BTreeMap::new();
    for (face, positions) in geometry.positions.chunks_exact(3).enumerate() {
        let material_id = geometry
            .material_indices
            .get(face * 3)
            .copied()
            .unwrap_or(0);
        let entry: &mut (Vec<[f32; 3]>, Vec<[f32; 2]>) = buckets.entry(material_id).or_default();
        entry.0.extend_from_slice(positions);
        if let Some(uvs) = geometry.uvs.get(face * 3..face * 3 + 3) {
            entry.1.extend_from_slice(uvs);
        } else {
            entry.1.extend([[0., 0.]; 3]);
        }
    }
    buckets
}

fn map_material(
    map: &skate_data::skate_map::SkateMap,
    material_id: u32,
    material_handles: &mut std::collections::HashMap<u32, Handle<StandardMaterial>>,
    texture_handles: &mut std::collections::HashMap<u32, Handle<Image>>,
    materials: &mut impl crate::map_render::AssetSink<StandardMaterial>,
    images: &mut impl crate::map_render::AssetSink<Image>,
) -> Option<Handle<StandardMaterial>> {
    if let Some(handle) = material_handles.get(&material_id) {
        return Some(handle.clone());
    }
    let source = map.materials.get(material_id as usize)?;
    let base_color_texture = source.textures[0].checked_sub(1).and_then(|id| {
        let texture = map.textures.get(id as usize)?;
        if texture.rgba.len() != texture.width as usize * texture.height as usize * 4 {
            return None;
        }
        Some(
            texture_handles
                .entry(id)
                .or_insert_with(|| {
                    images.add(Image::new(
                        bevy::render::render_resource::Extent3d {
                            width: texture.width,
                            height: texture.height,
                            depth_or_array_layers: 1,
                        },
                        bevy::render::render_resource::TextureDimension::D2,
                        texture.rgba.clone(),
                        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                        bevy::asset::RenderAssetUsages::default(),
                    ))
                })
                .clone(),
        )
    });
    let handle = materials.add(StandardMaterial {
        base_color: Color::srgba(source.color[0], source.color[1], source.color[2], 1.),
        base_color_texture,
        alpha_mode: map_alpha_mode(source.alpha_mode, source.alpha_cutoff),
        unlit: false,
        perceptual_roughness: 0.95,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    material_handles.insert(material_id, handle.clone());
    Some(handle)
}

fn spawn_overview_mesh(
    bounds: geometry::OverviewBounds,
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    material: Handle<StandardMaterial>,
    level: u8,
    commands: &mut crate::map_render::SceneCommands,
    meshes: &mut impl crate::map_render::AssetSink<Mesh>,
) {
    if positions.is_empty() {
        return;
    }
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.compute_flat_normals();
    commands.spawn((
        Name::new("Whole-map overview"),
        OverviewScene(bounds),
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(material),
        MapOverviewLod { level },
        if level != 0 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        },
        Transform::default(),
        bevy::camera::visibility::RenderLayers::layer(LAYER),
    ));
}

#[derive(Component)]
pub(super) struct MapOverviewLod {
    pub level: u8,
}

#[derive(bevy::prelude::Component)]
struct OverviewScene(geometry::OverviewBounds);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_render::{MapAssets, PreparedScene};
    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<crate::retail_render::WorldMaterial>>();
        world.init_resource::<Assets<crate::retail_sky::SkyMaterial>>();
        world.init_resource::<Assets<bevy::render::storage::ShaderStorageBuffer>>();
        world
    }
    #[test]
    fn miniature_terrain_has_lit_faces() {
        let mut world = world();
        let mut scene = PreparedScene::new(&world);
        scene.prepare(None, std::path::Path::new("."));
        scene.publish(&mut world);
        let (mesh, material) = world
            .query_filtered::<(&Mesh3d, &MeshMaterial3d<StandardMaterial>), With<OverviewScene>>()
            .single(&world)
            .unwrap();
        assert!(
            world
                .resource::<Assets<Mesh>>()
                .get(&mesh.0)
                .unwrap()
                .contains_attribute(Mesh::ATTRIBUTE_NORMAL)
        );
        assert!(
            !world
                .resource::<Assets<StandardMaterial>>()
                .get(&material.0)
                .unwrap()
                .unlit
        );
    }
    #[test]
    fn overview_survives_streaming_and_retires_with_map() {
        let mut world = world();
        for _ in 0..3 {
            let mut scene = PreparedScene::new(&world);
            scene.prepare(None, std::path::Path::new("."));
            scene.publish(&mut world);
            let (entity, mesh) = world
                .query_filtered::<(Entity, &Mesh3d), With<OverviewScene>>()
                .single(&world)
                .map(|(e, m)| (e, m.0.clone()))
                .expect("overview at startup");
            let gameplay: Vec<_> = world
                .query_filtered::<Entity, (With<Mesh3d>, Without<OverviewScene>)>()
                .iter(&world)
                .collect();
            for entity in gameplay {
                world
                    .entity_mut(entity)
                    .insert(crate::map_render::streaming::Cell {
                        min: [10000.; 3],
                        max: [10001.; 3],
                        near: 0.,
                        far: 0.,
                    });
            }
            crate::map_render::streaming::install(&mut world, Default::default(), [0.; 3]).unwrap();
            assert!(world.get::<Mesh3d>(entity).is_some());
            assert!(world.resource::<Assets<Mesh>>().contains(mesh.id()));
            MapAssets::retire(&mut world);
            assert!(world.get_entity(entity).is_err());
            assert!(world.resource::<Assets<Mesh>>().is_empty());
            assert!(world.resource::<Assets<StandardMaterial>>().is_empty());
        }
        // Publishing no overview after retirement must not resurrect the old one.
        let mut empty = PreparedScene::new(&world);
        empty.publish(&mut world);
        assert_eq!(world.query::<&OverviewScene>().iter(&world).count(), 0);
    }
}

#[cfg(test)]
mod material_tests {
    use super::*;
    #[test]
    fn solid_map_faces_write_depth_and_cutout_keeps_holes() {
        assert_eq!(map_alpha_mode(0, 0.5), AlphaMode::Opaque);
        assert_eq!(map_alpha_mode(1, 0.4), AlphaMode::Mask(0.4));
    }
}
