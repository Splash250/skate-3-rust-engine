//! Bounded embedded GLB loader for native interiors, independent of Lua graphics.
use bevy::{
    asset::RenderAssetUsages,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
pub struct Model {
    pub parts: Vec<(Mesh, usize, Transform)>,
    pub materials: Vec<StandardMaterial>,
    pub images: Vec<Image>,
    pub textures: Vec<(Option<usize>, Option<usize>)>,
    pub bounds: (Vec3, Vec3),
}
pub fn prepare(bytes: &[u8], transform: [[f32; 4]; 4]) -> Result<Model, String> {
    crate::modding::validate_interior_model(bytes)?;
    let glb = gltf::Gltf::from_slice(bytes).map_err(|e| e.to_string())?;
    let blob = glb
        .blob
        .as_deref()
        .ok_or("Interior needs embedded geometry")?;
    let mut images = Vec::new();
    for image in glb.images() {
        let gltf::image::Source::View { view, mime_type } = image.source() else {
            return Err("Interior needs embedded images".into());
        };
        images.push(
            Image::from_buffer(
                &blob[view.offset()..view.offset() + view.length()],
                ImageType::MimeType(mime_type),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::linear(),
                RenderAssetUsages::default(),
            )
            .map_err(|e| e.to_string())?,
        );
    }
    let mut materials = Vec::new();
    let mut textures = Vec::new();
    for m in glb.materials() {
        let p = m.pbr_metallic_roughness();
        let c = p.base_color_factor();
        let e = m.emissive_factor();
        if m.normal_texture().is_some()
            || m.occlusion_texture().is_some()
            || p.metallic_roughness_texture().is_some()
        {
            return Err(
                "Prepare baked base/emissive materials before loading this interior".into(),
            );
        }
        if p.base_color_texture().is_some_and(|t| t.tex_coord() != 0)
            || m.emissive_texture().is_some_and(|t| t.tex_coord() != 0)
        {
            return Err("Interior material requires UV channel zero".into());
        }
        materials.push(StandardMaterial {
            base_color: Color::linear_rgba(c[0], c[1], c[2], c[3]),
            emissive: LinearRgba::rgb(e[0], e[1], e[2]),
            metallic: p.metallic_factor(),
            perceptual_roughness: p.roughness_factor(),
            double_sided: m.double_sided(),
            cull_mode: if m.double_sided() {
                None
            } else {
                Some(bevy::render::render_resource::Face::Back)
            },
            alpha_mode: match m.alpha_mode() {
                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => AlphaMode::Mask(m.alpha_cutoff().unwrap_or(0.5)),
                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            },
            ..default()
        });
        textures.push((
            p.base_color_texture().map(|t| t.texture().source().index()),
            m.emissive_texture().map(|t| t.texture().source().index()),
        ));
    }
    let default_material = materials.len();
    materials.push(StandardMaterial::default());
    textures.push((None, None));
    let mut parts = Vec::new();
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    fn walk(
        node: gltf::Node,
        parent: Mat4,
        blob: &[u8],
        parts: &mut Vec<(Mesh, usize, Transform)>,
        lo: &mut Vec3,
        hi: &mut Vec3,
        default_material: usize,
    ) -> Result<(), String> {
        let at = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    return Err("Interior requires triangle primitives".into());
                }
                let r = primitive.reader(|_| Some(blob));
                let positions: Vec<_> = r
                    .read_positions()
                    .ok_or("Missing interior positions")?
                    .map(|p| at.transform_point3(Vec3::from_array(p)).to_array())
                    .collect();
                for p in &positions {
                    let v = Vec3::from_array(*p);
                    if !v.is_finite() {
                        return Err("Invalid transformed interior position".into());
                    }
                    *lo = lo.min(v);
                    *hi = hi.max(v);
                }
                let mut mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                )
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                if let Some(n) = r.read_normals() {
                    mesh.insert_attribute(
                        Mesh::ATTRIBUTE_NORMAL,
                        n.map(|n| {
                            at.inverse()
                                .transpose()
                                .transform_vector3(Vec3::from_array(n))
                                .normalize_or_zero()
                                .to_array()
                        })
                        .collect::<Vec<_>>(),
                    );
                }
                if let Some(uv) = r.read_tex_coords(0) {
                    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv.into_f32().collect::<Vec<_>>());
                }
                if let Some(c) = r.read_colors(0) {
                    mesh.insert_attribute(
                        Mesh::ATTRIBUTE_COLOR,
                        c.into_rgba_f32().collect::<Vec<_>>(),
                    );
                }
                let count = mesh.count_vertices();
                let mut indices = r
                    .read_indices()
                    .map(|i| i.into_u32().collect::<Vec<_>>())
                    .unwrap_or_else(|| (0..count as u32).collect());
                if at.determinant() < 0. {
                    for face in indices.chunks_exact_mut(3) {
                        face.swap(1, 2);
                    }
                }
                mesh.insert_indices(Indices::U32(indices));
                if !mesh.contains_attribute(Mesh::ATTRIBUTE_NORMAL) {
                    mesh.compute_normals();
                }
                parts.push((
                    mesh,
                    primitive.material().index().unwrap_or(default_material),
                    Transform::IDENTITY,
                ));
            }
        }
        for child in node.children() {
            walk(child, at, blob, parts, lo, hi, default_material)?;
        }
        Ok(())
    }
    let scene = glb
        .default_scene()
        .or_else(|| glb.scenes().next())
        .ok_or("Interior has no scene")?;
    for node in scene.nodes() {
        walk(
            node,
            Mat4::from_cols_array_2d(&transform),
            blob,
            &mut parts,
            &mut lo,
            &mut hi,
            default_material,
        )?;
    }
    if parts.is_empty() {
        return Err("Interior scene has no geometry".into());
    }
    Ok(Model {
        parts,
        materials,
        images,
        textures,
        bounds: (lo, hi),
    })
}

/// Derive a roof cutaway from retained complete geometry; world geometry is untouched.
pub fn overview(source: &Mesh, ceiling: f32) -> Mesh {
    let mut mesh = source.clone();
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
        source.attribute(Mesh::ATTRIBUTE_POSITION)
    {
        let indices: Vec<_> = source
            .indices()
            .map(|i| i.iter().map(|i| i as u32).collect())
            .unwrap_or_else(|| (0..p.len() as u32).collect());
        let kept = indices
            .chunks_exact(3)
            .filter(|t| t.iter().any(|i| p[*i as usize][1] < ceiling))
            .flatten()
            .copied()
            .collect();
        mesh.insert_indices(Indices::U32(kept));
    }
    mesh
}
