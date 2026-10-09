//! Camera-local cutaway materials. Never mutate shared gameplay materials.
use bevy::{
    asset::{AssetId, AssetPath, embedded_asset, embedded_path},
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};
use std::collections::{HashMap, HashSet};

type MapMaterial = ExtendedMaterial<StandardMaterial, Cutaway>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
struct Cutaway {
    #[uniform(100)]
    focus: Vec4,
    #[uniform(100)]
    camera: Vec4,
}
impl MaterialExtension for Cutaway {
    fn fragment_shader() -> ShaderRef {
        AssetPath::from(embedded_path!("minimap.wgsl"))
            .with_source("embedded")
            .into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        Self::fragment_shader()
    }
}

#[derive(Component)]
struct SourceMaterial(Handle<StandardMaterial>);

#[derive(Resource, Default)]
struct Materials(HashMap<AssetId<StandardMaterial>, Handle<MapMaterial>>);

pub(super) fn register(app: &mut App) {
    embedded_asset!(app, "minimap.wgsl");
    app.add_plugins(MaterialPlugin::<MapMaterial>::default());
}

pub(super) fn reconcile(world: &mut World) {
    // Also run while the HUD is hidden: map/context retirement must release
    // derived materials without requiring the next visible minimap frame.
    if !world.contains_resource::<Assets<MapMaterial>>() {
        return;
    }
    let mut materials = world.remove_resource::<Materials>().unwrap_or_default();
    let pending: Vec<_> = world
        .query_filtered::<(Entity, &MeshMaterial3d<StandardMaterial>), With<super::OverviewScene>>()
        .iter(world)
        .map(|(e, m)| (e, m.0.clone()))
        .collect();
    for (entity, source) in pending {
        let Some(base) = world
            .resource::<Assets<StandardMaterial>>()
            .get(&source)
            .cloned()
        else {
            continue;
        };
        let handle = materials
            .0
            .entry(source.id())
            .or_insert_with(|| {
                world
                    .resource_mut::<Assets<MapMaterial>>()
                    .add(MapMaterial {
                        base,
                        extension: default(),
                    })
            })
            .clone();
        world
            .entity_mut(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((MeshMaterial3d(handle), SourceMaterial(source)));
    }
    let live: HashSet<_> = world
        .query::<&SourceMaterial>()
        .iter(world)
        .map(|s| s.0.id())
        .collect();
    materials.0.retain(|id, handle| {
        if live.contains(id) {
            true
        } else {
            world
                .resource_mut::<Assets<MapMaterial>>()
                .remove(handle.id());
            false
        }
    });
    world.insert_resource(materials);
}

pub(super) fn update(world: &mut World, frame: &super::view::ViewFrame, focus: Option<Vec3>) {
    let Some(mut assets) = world.get_resource_mut::<Assets<MapMaterial>>() else {
        return;
    };
    for (_, material) in assets.iter_mut() {
        material.extension.focus = focus.map_or(Vec4::ZERO, |p| p.extend(1.));
        material.extension.camera = frame.transform.translation.extend(0.);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cutaway_isolated_from_gameplay_and_released_on_retirement() {
        let mut world = World::new();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<MapMaterial>>();
        let source = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let bounds = super::super::geometry::OverviewBounds {
            min: Vec3::splat(-10.),
            max: Vec3::splat(10.),
        };
        let gameplay = world.spawn(MeshMaterial3d(source.clone())).id();
        let overview = world
            .spawn((
                super::super::OverviewScene(bounds),
                MeshMaterial3d(source.clone()),
            ))
            .id();
        reconcile(&mut world);
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(gameplay)
                .unwrap()
                .0,
            source
        );
        assert!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(overview)
                .is_none()
        );
        let derived = world
            .get::<MeshMaterial3d<MapMaterial>>(overview)
            .unwrap()
            .0
            .clone();
        let frame = super::super::view::frame(bounds, Vec3::ZERO, false, 4. / 3., 1.);
        update(&mut world, &frame, Some(Vec3::Y));
        assert_eq!(
            world
                .resource::<Assets<MapMaterial>>()
                .get(&derived)
                .unwrap()
                .extension
                .focus,
            Vec4::new(0., 1., 0., 1.)
        );
        assert_eq!(
            world
                .resource::<Assets<StandardMaterial>>()
                .get(&source)
                .unwrap()
                .alpha_mode,
            AlphaMode::Opaque
        );
        update(&mut world, &frame, None);
        assert_eq!(
            world
                .resource::<Assets<MapMaterial>>()
                .get(&derived)
                .unwrap()
                .extension
                .focus
                .w,
            0.
        );
        world.despawn(overview);
        reconcile(&mut world);
        assert!(world.resource::<Assets<MapMaterial>>().is_empty());
        assert!(
            world
                .resource::<Assets<StandardMaterial>>()
                .contains(source.id())
        );
    }
}

#[cfg(test)]
mod shader_tests {
    use super::*;
    #[test]
    fn cutaway_shader_compiles_with_bevy_pbr_imports() {
        use bevy::render::render_resource::{DownlevelFlags, WgpuFeatures};
        use bevy::shader::{ShaderCache, ShaderDefVal};
        fn walk(path: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(path).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, files);
                } else if p.extension().is_some_and(|e| e == "wgsl") {
                    files.push(p);
                }
            }
        }
        let cargo = std::env::var_os("CARGO_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cargo")
            });
        let mut files = Vec::new();
        for registry in std::fs::read_dir(cargo.join("registry/src"))
            .unwrap()
            .flatten()
        {
            for entry in std::fs::read_dir(registry.path()).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("bevy_") && name.ends_with("-0.18.1") {
                    walk(&entry.path().join("src"), &mut files);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor");
        for name in ["bevy_pbr", "bevy_core_pipeline"] {
            walk(&root.join(name).join("src"), &mut files);
        }
        let mut assets = Assets::<Shader>::default();
        let mut cache =
            ShaderCache::<(), ()>::new(WgpuFeatures::empty(), DownlevelFlags::all(), |_, _, _| {
                Ok(())
            });
        let mut defs: Vec<_> = [
            ("MAX_DIRECTIONAL_LIGHTS", 10),
            ("MAX_CASCADES_PER_LIGHT", 4),
            ("MAX_VIEW_LIGHT_PROBES", 8),
            ("PER_OBJECT_BUFFER_BATCH_SIZE", 64),
            ("MATERIAL_BIND_GROUP", 3),
            ("AVAILABLE_STORAGE_BUFFER_BINDINGS", 8),
        ]
        .into_iter()
        .map(|(n, v)| ShaderDefVal::UInt(n.into(), v))
        .collect();
        for path in files {
            let source = std::fs::read_to_string(&path).unwrap();
            if !source.contains("#define_import_path") {
                continue;
            }
            let shader = Shader::from_wgsl_with_defs(
                source,
                path.to_string_lossy().to_string(),
                defs.clone(),
            );
            let h = assets.add(shader.clone());
            cache.set_shader(h.id(), shader);
        }
        for name in [
            "VERTEX_UVS",
            "VERTEX_UVS_A",
            "VERTEX_OUTPUT_INSTANCE_INDEX",
            "VERTEX_NORMALS",
            "MESH_PIPELINE",
            "STANDARD_MATERIAL",
            "STANDARD_MATERIAL_BASE_COLOR_TEXTURE",
            "MAY_DISCARD",
        ] {
            defs.push(ShaderDefVal::Bool(name.into(), true));
        }
        let shader = Shader::from_wgsl(include_str!("minimap.wgsl"), "minimap.wgsl");
        let h = assets.add(shader.clone());
        cache.set_shader(h.id(), shader);
        cache.get(&(), 0, h.id(), &defs).unwrap();
        defs.push(ShaderDefVal::Bool("PREPASS_PIPELINE".into(), true));
        defs.push(ShaderDefVal::Bool("DEFERRED_PREPASS".into(), true));
        defs.push(ShaderDefVal::Bool("PREPASS_FRAGMENT".into(), true));
        defs.push(ShaderDefVal::Bool(
            "NORMAL_PREPASS_OR_DEFERRED_PREPASS".into(),
            true,
        ));
        cache.get(&(), 0, h.id(), &defs).unwrap();
    }
}
