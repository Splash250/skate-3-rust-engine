//! Package-scoped graphics and immutable-bind-pose node overrides. All physics
//! and gameplay stay outside this module. A scene node is not a vehicle part.
use super::Mods;
use bevy::{prelude::*, scene::{SceneInstance,SceneSpawner}};
use skate_mods::scene::{GraphicsDefinition, NodeState, TransformOptions, TransformState};
use std::{collections::{BTreeMap,BTreeSet},time::Instant};

#[derive(Clone)]
pub(super) struct TimedNode { pub state:NodeState, pub received:Instant }
#[derive(Clone,Copy)]
struct Binding { entity:Entity, authored:Transform }
pub(super) struct Owned {
    pub(super) deformation: super::graphics_deformation::State,
    pub entity:Entity,
    pub mesh:Option<AssetId<Mesh>>,
    pub material:Option<AssetId<StandardMaterial>>,
    pub body:Option<String>,
    pub definition:GraphicsDefinition,
    pub transform:TransformState,
    pub visible:bool,
    pub serial:u64,
    pub nodes:BTreeMap<String,TimedNode>,
    bindings:BTreeMap<String,Binding>,
    ambiguous:BTreeSet<String>,
    warned:BTreeSet<String>,
    pub(super) ready:bool,
}

pub(super) fn transform(state:&TransformState) -> Transform {
    Transform {
        translation:Vec3::from_array(state.position),
        rotation:Quat::from_array(state.rotation).normalize(),
        scale:Vec3::from_array(state.scale),
    }
}
pub(super) fn node_transform(authored:Transform, state:&NodeState, age:f32) -> Transform {
    let mut delta=transform(&state.transform);
    let age=age.clamp(0.,0.10);
    delta.translation += Vec3::from_array(state.linear_velocity)*age;
    delta.rotation=(Quat::from_scaled_axis(Vec3::from_array(state.angular_velocity)*age)*delta.rotation).normalize();
    if state.relative {
        Transform { translation:authored.translation+delta.translation,
            rotation:(delta.rotation*authored.rotation).normalize(),scale:authored.scale*delta.scale }
    } else { delta }
}

/// Canonical containment defeats symlink escapes as well as lexical traversal.
/// Remote peers supply only a relative descriptor in a matching local package.
pub(super) fn asset_path(mods:&Mods, package_id:&str, path:&str) -> Result<String,String> {
    if !skate_mods::scene::valid_asset(path) || path.is_empty() { return Err("invalid GLB asset path".into()); }
    let package=mods.manager.packages.get(package_id).ok_or("missing local package")?;
    let root=package.root.canonicalize().map_err(|e|format!("package path: {e}"))?;
    let full=root.join(path).canonicalize().map_err(|e|format!("GLB {path}: {e}"))?;
    if !full.starts_with(&root) || !full.is_file() { return Err("GLB escapes its package".into()); }
    if mods.server_selected {
        // Downloaded GLBs cannot smuggle AssetServer reads into another resource,
        // the cache's private state/policy files, or an external URL via glTF URIs.
        let bytes=skate_mods::read_bounded(&root,path,16*1024*1024)?;
        validate_resource_glb(&bytes)?;
    }
    let (source, reader) = if mods.server_selected {
        ("resources", mods.resource_asset_root.clone())
    } else { ("mods", super::package_root()) };
    let reader_root=reader.canonicalize().map_err(|e|e.to_string())?;
    let relative=full.strip_prefix(&reader_root).map_err(|_|"GLB outside mods asset reader")?;
    Ok(format!("{source}://{}",relative.to_string_lossy().replace('\\',"/")))
}

/// Read image headers before any pixel allocation. The same limits already
/// applied to procedural PNG textures, but must be checked before decoding.
pub(super) fn bounded_image_dimensions(bytes: &[u8]) -> Result<u64, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format().map_err(|e| format!("image header: {e}"))?;
    let bytes_per_pixel=match reader.format() {
        Some(image::ImageFormat::Png) => {
            // AssetServer subsequently uses an unlimited PNG decoder. Reject
            // compressed profiles/text/APNG and unrecognized ancillary chunks
            // before even probing dimensions: ICC expansion is independent of
            // the image dimensions and PNG ignores some profile-limit errors.
            let mut offset=8usize;
            let mut depth=8;
            while offset < bytes.len() {
                let header=bytes.get(offset..offset+8).ok_or("invalid PNG chunk")?;
                let len=u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
                let end=offset.checked_add(12).and_then(|n|n.checked_add(len)).ok_or("invalid PNG chunk")?;
                if end>bytes.len() {return Err("invalid PNG chunk".into());}
                let valid=match &header[4..8] {
                    b"IHDR" => {if len==13 {depth=bytes[offset+16];} len==13},
                    b"PLTE" => len<=768,
                    b"tRNS" => len<=256,
                    b"IDAT" => true,
                    b"IEND" => len==0 && end==bytes.len(),
                    b"sRGB" => len==1,
                    b"gAMA" => len==4,
                    b"cHRM" => len==32,
                    b"pHYs" => len==9,
                    _ => false,
                };
                if !valid {return Err("downloaded GLB PNG metadata or animation is unsupported".into());}
                offset=end;
            }
            if depth>8 {8} else {4}
        }
        Some(image::ImageFormat::Jpeg) => 4,
        _ => return Err("resource images must use PNG or JPEG".into()),
    };
    let mut limits=image::Limits::default();limits.max_alloc=Some(32*1024*1024);
    reader.limits(limits);
    let (width, height) = reader.into_dimensions().map_err(|e| format!("image header: {e}"))?;
    let pixels = u64::from(width) * u64::from(height);
    let decoded=pixels*bytes_per_pixel;
    if width == 0 || height == 0 || width > 2048 || height > 2048 || decoded > 16 * 1024 * 1024 {
        return Err("image dimensions exceed 2048 pixels or 16 MiB decoded data".into());
    }
    Ok(decoded)
}

pub(super) fn validate_resource_glb(bytes: &[u8]) -> Result<(), String> {
    const GEOMETRY_BYTES: usize = 16 * 1024 * 1024;
    let invalid = || "resource GLB exceeds geometry or scene limits".to_owned();
    let binary = gltf::binary::Glb::from_slice(bytes).map_err(|e| format!("resource GLB: {e}"))?;
    if binary.json.len()>256*1024 {return Err("resource GLB JSON exceeds 256 KiB".into());}
    let json: serde_json::Value = serde_json::from_slice(&binary.json).map_err(|e| e.to_string())?;
    // Check collection sizes before constructing the glTF object graph. Extensions
    // may describe compressed geometry or instancing with a different expansion
    // model, so downloaded scenes currently support standard glTF only.
    fn extensions(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => map.iter().any(|(key, value)|
                matches!(key.as_str(), "extensions" | "extensionsUsed" | "extensionsRequired")
                    && value.as_object().is_none_or(|v| !v.is_empty())
                    && value.as_array().is_none_or(|v| !v.is_empty()) || extensions(value)),
            serde_json::Value::Array(values) => values.iter().any(extensions),
            _ => false,
        }
    }
    if extensions(&json) { return Err("downloaded GLB extensions are unsupported".into()); }
    for (key, max) in [("nodes",256), ("meshes",32), ("skins",16), ("images",16),
        ("textures",32), ("materials",32), ("accessors",512), ("bufferViews",512),
        ("scenes",8), ("animations",16), ("buffers",1)] {
        if json[key].as_array().is_some_and(|v| v.len() > max) { return Err(invalid()); }
    }
    // gltf-json's POSITION validation hook indexes this reference directly;
    // reject it before the library's validation pass can panic on malformed data.
    let accessor_count=json["accessors"].as_array().map_or(0,Vec::len);
    for mesh in json["meshes"].as_array().into_iter().flatten() {
        for primitive in mesh["primitives"].as_array().into_iter().flatten() {
            if primitive["attributes"]["POSITION"].as_u64().is_none_or(|i|i>=accessor_count as u64) {return Err(invalid());}
        }
    }
    let node_count=json["nodes"].as_array().map_or(0,Vec::len);
    for animation in json["animations"].as_array().into_iter().flatten() {
        for channel in animation["channels"].as_array().into_iter().flatten() {
            if channel["target"]["node"].as_u64().is_none_or(|i|i>=node_count as u64)
                || !matches!(channel["target"]["path"].as_str(),Some("translation"|"rotation"|"scale"|"weights")) {return Err(invalid());}
        }
    }
    let glb = gltf::Gltf::from_slice(bytes).map_err(|e| format!("resource GLB: {e}"))?;
    let blob = glb.blob.as_ref().ok_or("downloaded GLB assets must embed their buffers and images")?;
    if glb.document.buffers().any(|b| matches!(b.source(),gltf::buffer::Source::Uri(_)))
        || glb.document.images().any(|i| matches!(i.source(),gltf::image::Source::Uri{..})) {
        return Err("downloaded GLB assets must embed their buffers and images".into());
    }
    if glb.document.buffers().any(|b| b.length() > blob.len()) { return Err(invalid()); }
    for view in glb.document.views() {
        if view.offset().checked_add(view.length()).is_none_or(|end| end > blob.len()) {
            return Err(invalid());
        }
    }
    let mut image_bytes = vec![0; glb.document.images().len()];
    let mut decoded_images = 0;
    for image in glb.document.images() {
        let gltf::image::Source::View { view, mime_type } = image.source() else { return Err(invalid()); };
        if !matches!(mime_type,"image/png" | "image/jpeg") { return Err(invalid()); }
        image_bytes[image.index()] = bounded_image_dimensions(&blob[view.offset()..view.offset()+view.length()])?;
        decoded_images += image_bytes[image.index()];
        if decoded_images > 16 * 1024 * 1024 { return Err("resource GLB images exceed 16 MiB RGBA".into()); }
    }
    // Bevy decodes once per texture, even if several textures share one image.
    let texture_bytes: u64 = glb.document.textures().map(|t| image_bytes[t.source().index()]).sum();
    if texture_bytes > 16 * 1024 * 1024 { return Err("resource GLB textures exceed 16 MiB RGBA".into()); }
    let mut accessor_bytes = 0usize;
    for accessor in glb.document.accessors() {
        // Sparse accessors can allocate output without a correspondingly sized
        // input buffer. Explicit data is required for remotely supplied scenes.
        let view = accessor.view().ok_or_else(invalid)?;
        if accessor.sparse().is_some() || accessor.count() > 262_144 { return Err(invalid()); }
        let stride = view.stride().unwrap_or(accessor.size());
        let used = accessor.count().saturating_sub(1).checked_mul(stride)
            .and_then(|n| n.checked_add(accessor.size()))
            .and_then(|n| n.checked_add(accessor.offset())).ok_or_else(invalid)?;
        if stride < accessor.size() || used > view.length() { return Err(invalid()); }
        if accessor.data_type()==gltf::accessor::DataType::F32 {
            for index in 0..accessor.count() {
                let start=view.offset()+accessor.offset()+index*stride;
                if blob[start..start+accessor.size()].chunks_exact(4)
                    .any(|word|!f32::from_le_bytes(word.try_into().unwrap()).is_finite()) {
                    return Err("resource GLB has nonfinite geometry/animation values".into());
                }
            }
        }
        accessor_bytes = accessor_bytes.checked_add(accessor.count() * accessor.size()).ok_or_else(invalid)?;
        if accessor_bytes > GEOMETRY_BYTES { return Err(invalid()); }
    }
    // Charge every reader use too: repeated references to one large accessor
    // must not multiply the loader's allocation beyond the resource budget.
    let mut reads = 0usize;
    let mut charge = |a: gltf::Accessor<'_>| -> Result<(),String> {
        reads = reads.checked_add(a.count()*a.size()).ok_or_else(invalid)?;
        if reads > GEOMETRY_BYTES { Err(invalid()) } else { Ok(()) }
    };
    let mut mesh_vertices = vec![0usize; glb.document.meshes().len()];
    let mut primitives = 0;
    let mut decoded_geometry = 0usize;
    for mesh in glb.document.meshes() {
        for primitive in mesh.primitives() {
            primitives += 1;
            if primitives > 64 { return Err(invalid()); }
            use gltf::accessor::{DataType,Dimensions};
            let position=primitive.get(&gltf::Semantic::Positions).ok_or_else(invalid)?;
            if position.data_type()!=DataType::F32 || position.dimensions()!=Dimensions::Vec3 {return Err(invalid());}
            let count=position.count();
            let mut decoded_stride=0;
            for (_, a) in primitive.attributes() {
                if a.count()!=count {return Err("resource GLB attribute counts differ".into());}
                decoded_stride += a.dimensions().multiplicity()*4;
                charge(a)?;
            }
            let index_count=if let Some(a)=primitive.indices() {
                if a.dimensions()!=Dimensions::Scalar || !matches!(a.data_type(),DataType::U8|DataType::U16|DataType::U32) {return Err(invalid());}
                charge(a.clone())?;
                let reader=primitive.reader(|_|Some(blob.as_slice()));
                if reader.read_indices().ok_or_else(invalid)?.into_u32().any(|i|i as usize>=count) {
                    return Err("resource GLB index exceeds vertex count".into());
                }
                a.count()
            } else {0};
            let flat=primitive.mode()==gltf::mesh::Mode::Triangles && primitive.get(&gltf::Semantic::Normals).is_none();
            let expanded=if flat && index_count>0 {index_count} else {count};
            // Bevy duplicates every attribute before generating flat normals.
            // Account for generated normals/tangents and promoted integer data.
            decoded_geometry += expanded*(decoded_stride+28)+index_count*4;
            mesh_vertices[mesh.index()] += expanded;
            let targets = primitive.morph_targets().collect::<Vec<_>>();
            if targets.len() > 8 { return Err(invalid()); }
            for target in targets {
                let accessors=[target.positions(),target.normals(),target.tangents()];
                if accessors.iter().all(Option::is_none) {return Err(invalid());}
                for a in accessors.into_iter().flatten() {
                    if a.count()!=count || a.data_type()!=DataType::F32 || a.dimensions()!=Dimensions::Vec3 {return Err(invalid());}
                    charge(a)?;
                }
                decoded_geometry += count*48; // Morph texture channels/padding.
            }
            if decoded_geometry>GEOMETRY_BYTES {return Err("resource GLB expanded geometry exceeds 16 MiB".into());}
        }
    }
    for skin in glb.document.skins() {
        if skin.joints().len() > 256 { return Err(invalid()); }
        if let Some(a) = skin.inverse_bind_matrices() { charge(a)?; }
    }
    let mut channels = 0;
    for animation in glb.document.animations() {
        channels += animation.channels().count();
        if channels > 128 { return Err(invalid()); }
        // Bevy builds a separate curve for each channel, including channels
        // sharing one sampler. Charge the actual reads rather than declarations.
        for channel in animation.channels() {
            let sampler = channel.sampler();charge(sampler.input())?;charge(sampler.output())?;
        }
    }
    let nodes = glb.document.nodes().collect::<Vec<_>>();
    let mut parents = vec![false; nodes.len()];
    for node in &nodes {
        if node.transform().matrix().iter().flatten().any(|v|!v.is_finite()) {return Err(invalid());}
        for child in node.children() {
            if child.index() == node.index() || std::mem::replace(&mut parents[child.index()],true) { return Err(invalid()); }
        }
    }
    fn visit(node: gltf::Node<'_>, depth: usize, seen: &mut [bool], vertices: &mut usize, meshes: &[usize]) -> Result<(),String> {
        if depth > 64 || std::mem::replace(&mut seen[node.index()],true) { return Err("resource GLB has cyclic or repeated scene nodes".into()); }
        if let Some(mesh) = node.mesh() { *vertices += meshes[mesh.index()]; }
        if *vertices > 1_000_000 { return Err("resource GLB instances exceed vertex budget".into()); }
        for child in node.children() { visit(child,depth+1,seen,vertices,meshes)?; }
        Ok(())
    }
    // Traverse every node, even ones not selected by a scene, to reject cycles.
    let mut seen = vec![false; nodes.len()];
    let mut vertices = 0;
    for node in nodes.iter().filter(|node| !parents[node.index()]) {
        visit(node.clone(),0,&mut seen,&mut vertices,&mesh_vertices)?;
    }
    if seen.iter().any(|seen| !seen) { return Err(invalid()); }
    let mut vertices = 0;
    for scene in glb.document.scenes() {
        let mut seen = vec![false; nodes.len()];
        for node in scene.nodes() { visit(node,0,&mut seen,&mut vertices,&mesh_vertices)?; }
    }
    Ok(())
}

pub(super) fn spawn(
    world:&mut World, mods:&mut Mods, owner:&str, package:&str, key:String,
    definition:GraphicsDefinition, state:TransformState, visible:bool, serial:Option<u64>,
) -> Result<(),String> {
    if !definition.validate() || !state.validate() || !skate_mods::scene::valid_key(&key) {
        return Err("invalid graphics descriptor or transform".into());
    }
    let slot=(owner.to_owned(),key);
    if !mods.graphics.contains_key(&slot) && mods.graphics.keys().filter(|(o,_)| o==owner).count() >= 64 {
        return Err("64 graphics instances per owner maximum".into());
    }
    // Validate before removing a live instance. An async GLB load is never a
    // license to fall back to an arbitrary path or another package's scene.
    let path=if definition.path.is_empty() { None } else { Some(asset_path(mods,package,&definition.path)?) };
    super::retire_graphics(world,mods,&slot);
    let t=transform(&state);
    let visibility=if visible { Visibility::Visible } else { Visibility::Hidden };
    let (entity,mesh,material)=if let Some(path)=path {
        let scene=world.resource::<AssetServer>().load(GltfAssetLabel::Scene(0).from_asset(path));
        (world.spawn((SceneRoot(scene),t,visibility)).id(),None,None)
    } else {
        let mesh=world.resource_mut::<Assets<Mesh>>().add(Cuboid::new(1.,1.,1.));
        let c=definition.color;
        let a=definition.opacity;
        let material=world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            base_color:Color::srgba(c[0],c[1],c[2],a),
            alpha_mode: if a < 1. { AlphaMode::Blend } else { AlphaMode::Opaque },
            ..default()
        });
        (world.spawn((Mesh3d(mesh.clone()),MeshMaterial3d(material.clone()),t,visibility)).id(),Some(mesh.id()),Some(material.id()))
    };
    mods.graphics_serial=mods.graphics_serial.wrapping_add(1);
    world.entity_mut(entity).insert((crate::retail_character::ModGraphicsLit,bevy::camera::visibility::RenderLayers::from_layers(&[0,28])));
    mods.graphics.insert(slot,Owned {
        deformation: Default::default(),
        entity,mesh,material,body:definition.body.clone(),definition,transform:state,visible,
        serial:serial.unwrap_or(mods.graphics_serial),nodes:BTreeMap::new(),bindings:BTreeMap::new(),
        ambiguous:BTreeSet::new(),warned:BTreeSet::new(),ready:false,
    });
    Ok(())
}
pub(super) fn set_node(mods:&mut Mods,owner:&str,key:&str,node:String,options:TransformOptions) -> Result<(),String> {
    let owned=mods.graphics.get_mut(&(owner.to_owned(),key.to_owned())).ok_or("unknown graphics key")?;
    if owned.nodes.len() >= 64 && !owned.nodes.contains_key(&node) { return Err("64 node overrides per graphics instance maximum".into()); }
    let frame=owned.nodes.entry(node).or_insert_with(||TimedNode { state:NodeState::default(),received:Instant::now() });
    frame.state.apply(&options); frame.received=Instant::now();
    Ok(())
}
pub(super) fn reset_node(world:&mut World,mods:&mut Mods,owner:&str,key:&str,node:&str) {
    if let Some(owned)=mods.graphics.get_mut(&(owner.to_owned(),key.to_owned())) {
        owned.nodes.remove(node);
        if let Some(b)=owned.bindings.get(node) {
            if let Some(mut t)=world.get_mut::<Transform>(b.entity) { *t=b.authored; }
        }
    }
}

fn bind(world:&mut World,owned:&mut Owned) {
    if owned.ready { return; }
    if owned.definition.path.is_empty() { owned.ready=true; return; }
    let Some(instance)=world.get::<SceneInstance>(owned.entity) else { return };
    let spawner=world.resource::<SceneSpawner>();
    if !spawner.instance_is_ready(**instance) { return; }
    let entities:Vec<_>=spawner.iter_instance_entities(**instance).collect();
    let mut opacity_materials=BTreeMap::new();
    for entity in entities {
        prepare_mesh(world,entity,owned.definition.opacity,&mut opacity_materials);
        let (Some(name),Some(t))=(world.get::<Name>(entity),world.get::<Transform>(entity)) else { continue };
        let name=name.as_str().to_owned();
        if owned.bindings.insert(name.clone(),Binding { entity,authored:*t }).is_some() {
            owned.ambiguous.insert(name);
        }
    }
    owned.ready=true;
}

fn prepare_mesh(world:&mut World,entity:Entity,opacity:f32,cache:&mut BTreeMap<AssetId<StandardMaterial>,Handle<StandardMaterial>>) {
    let Some(handle)=world.get::<MeshMaterial3d<StandardMaterial>>(entity).map(|m|m.0.clone()) else {return};
    // The baked world's dynamic-shadow map includes layer 28. Keep authored
    // materials (and their alpha-aware shadow shaders) for every GLB primitive.
    let layers=world.get::<bevy::camera::visibility::RenderLayers>(entity).cloned().unwrap_or_default().with(28);
    world.entity_mut(entity).insert(layers);
    if opacity>=0.999 {return;}
    let replacement=if let Some(existing)=cache.get(&handle.id()) {existing.clone()} else {
        let mut assets=world.resource_mut::<Assets<StandardMaterial>>();
        let Some(mut material)=assets.get(&handle).cloned() else {return};
        material.base_color.set_alpha(material.base_color.alpha()*opacity);
        material.alpha_mode=AlphaMode::Blend;
        let replacement=assets.add(material);
        cache.insert(handle.id(),replacement.clone());
        replacement
    };
    world.entity_mut(entity).insert(MeshMaterial3d(replacement));
}

pub(super) fn sync(world:&mut World,mods:&mut Mods) {
    for ((owner,key),owned) in &mut mods.graphics {
        let mut t=transform(&owned.transform);
        let mut visible=owned.visible;
        if let Some(body)=&owned.body {
            if let Some(snap)=mods.bodies.get(&(owner.clone(),body.clone())).and_then(|id|mods.world.read(*id)) {
                let q=Quat::from_array(snap.rotation).normalize();
                t.translation=Vec3::from_array(snap.position)+q*t.translation;
                t.rotation=(q*t.rotation).normalize();
            } else if let Some((position,rotation))=mods.replication.pending_root(owner,body) {
                t.translation=position+rotation*t.translation;
                t.rotation=(rotation*t.rotation).normalize();
            } else {
                // Out-of-order network spawn: never show a body-bound scene
                // at the origin while no validated pose has arrived.
                visible=false;
            }
        }
        if let Some(mut current)=world.get_mut::<Transform>(owned.entity) { if *current!=t {*current=t;} }
        if let Some(mut current)=world.get_mut::<Visibility>(owned.entity) { let next=if visible { Visibility::Visible } else { Visibility::Hidden }; if *current!=next {*current=next;} }
        bind(world,owned);
        if let Some(id)=owned.body.as_ref().and_then(|body|mods.bodies.get(&(owner.clone(),body.clone()))).copied() {
            if let Some(field)=mods.world.deformation(id) {
                super::graphics_deformation::sync(world,owned,id,field,mods.world.collider_offset(id).unwrap_or([0.;3]));
            }
        }
        for (name,node) in &owned.nodes {
            if owned.ambiguous.contains(name) || !owned.bindings.contains_key(name) {
                if owned.ready && owned.warned.insert(name.clone()) {
                    warn!("graphics {owner}/{key}: node '{name}' missing or ambiguous in local GLB");
                }
                continue;
            }
            let binding=owned.bindings[name];
            let age=if owner.starts_with('@') { node.received.elapsed().as_secs_f32() } else { 0. };
            if let Some(mut current)=world.get_mut::<Transform>(binding.entity) {
                let next=node_transform(binding.authored,&node.state,age);
                if *current!=next {*current=next;}
            }
        }
    }
}

/// Actual current solid collider edges, not the mesh or an approximate box.
/// Green=local; amber=remote replica; cyan=physical center of mass.
pub(crate) fn debug(mods:Res<Mods>,mut gizmos:Gizmos) {
    if mods.debug_owners.is_empty() { return; }
    let mut budget=60_000;
    for body in mods.world.solid_bodies() {
        let Some(((owner,_),_))=mods.bodies.iter().find(|(_,id)| **id==body.id) else { continue };
        let package=owner.split_once(':').map_or(owner.as_str(),|(_,id)|id);
        if !mods.debug_owners.contains(owner) && !mods.debug_owners.contains(package) { continue; }
        let color=if owner.starts_with('@') { Color::srgb(1.,0.65,0.1) } else { Color::srgb(0.1,1.,0.25) };
        // Shared triangle edges need only one line. This keeps detailed
        // compounds visible without spending the line budget twice per edge.
        let mut edges=BTreeSet::new();
        for collider in &body.colliders {
            for triangle in skate_dynamics::solid::collider_triangles(collider) {
                for (a,b) in [(0,1),(1,2),(2,0)] {
                    let mut first=triangle[a].map(f32::to_bits);
                    let mut second=triangle[b].map(f32::to_bits);
                    if first>second { std::mem::swap(&mut first,&mut second); }
                    if !edges.insert((first,second)) { continue; }
                    if budget==0 { return; } budget-=1;
                    gizmos.line(Vec3::from_array(triangle[a]),Vec3::from_array(triangle[b]),color);
                }
            }
        }
        let com=Vec3::from_array(body.center_of_mass.to_array());
        for axis in [Vec3::X,Vec3::Y,Vec3::Z] {
            gizmos.line(com-axis*0.12,com+axis*0.12,Color::srgb(0.,1.,1.));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resource_glb(document:serde_json::Value)->Vec<u8> {
        resource_glb_with_bin(document,vec![0u8;4])
    }
    fn resource_glb_with_bin(document:serde_json::Value,mut bin:Vec<u8>)->Vec<u8> {
        while bin.len()%4!=0 {bin.push(0);}
        let mut json=serde_json::to_vec(&document).unwrap();
        while json.len()%4!=0 {json.push(b' ');}
        let mut bytes=b"glTF".to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend((12u32+8+json.len() as u32+8+bin.len() as u32).to_le_bytes());
        bytes.extend((json.len() as u32).to_le_bytes());bytes.extend(b"JSON");bytes.extend(json);
        bytes.extend((bin.len() as u32).to_le_bytes());bytes.extend(b"BIN\0");bytes.extend(bin);bytes
    }
    #[test]
    fn dedicated_resource_glb_rejects_external_buffers_and_images() {
        let embedded=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4}]});
        validate_resource_glb(&resource_glb(embedded.clone())).unwrap();
        let mut external=embedded.clone();external["buffers"][0]["uri"]=serde_json::json!("../../state/private.json");
        assert!(validate_resource_glb(&resource_glb(external)).unwrap_err().contains("embed"));
        let mut image=embedded;image["images"]=serde_json::json!([{"uri":"../other/texture.png"}]);
        assert!(validate_resource_glb(&resource_glb(image)).unwrap_err().contains("embed"));
    }
    pub(super) fn oversized_png() -> Vec<u8> {
        let mut bytes=include_bytes!("../tests/fixtures/texture-codec.png").to_vec();
        bytes[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        let mut crc=!0u32;
        for byte in &bytes[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {crc=(crc>>1)^if crc&1!=0 {0xedb88320} else {0};}
        }
        bytes[29..33].copy_from_slice(&(!crc).to_be_bytes());
        bytes
    }
    #[test]
    fn dedicated_resource_image_headers_are_bounded_before_decode() {
        assert!(bounded_image_dimensions(&oversized_png()).unwrap_err().contains("dimensions"));
        bounded_image_dimensions(include_bytes!("../tests/fixtures/texture-codec.png")).unwrap();
    }
    #[test]
    fn dedicated_resource_glb_bounds_embedded_images_and_accessor_expansion() {
        let png=oversized_png();
        let document=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":png.len()}],
            "bufferViews":[{"buffer":0,"byteLength":png.len()}],"images":[{"bufferView":0,"mimeType":"image/png"}]});
        assert!(validate_resource_glb(&resource_glb_with_bin(document,png)).unwrap_err().contains("dimensions"));
        let document=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],
            "bufferViews":[{"buffer":0,"byteLength":4}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":262144,"type":"VEC3"}]});
        assert!(validate_resource_glb(&resource_glb(document)).is_err());
    }
    #[test]
    fn dedicated_resource_glb_rejects_cyclic_and_expanded_scene_instances() {
        let cycle=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],
            "nodes":[{"children":[1]},{"children":[0]}]});
        assert!(validate_resource_glb(&resource_glb(cycle)).is_err());
        let shared=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],
            "nodes":[{"children":[2]},{"children":[2]},{}]});
        assert!(validate_resource_glb(&resource_glb(shared)).is_err());
        let size=250001*12;
        let document=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":size}],
            "bufferViews":[{"buffer":0,"byteLength":size}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":250001,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],
            "nodes":[{"mesh":0},{"mesh":0},{"mesh":0},{"mesh":0}],"scenes":[{"nodes":[0,1,2,3]}]});
        assert!(validate_resource_glb(&resource_glb_with_bin(document,vec![0;size])).unwrap_err().contains("instances"));
    }
    #[test]
    fn dedicated_resource_glb_rejects_decoder_metadata_and_repeated_textures() {
        let mut png=include_bytes!("../tests/fixtures/texture-codec.png").to_vec();
        // Any compressed ICC profile is rejected before ImageReader can inflate
        // it; the chunk deliberately needs no valid zlib stream or checksum.
        let mut chunk=1u32.to_be_bytes().to_vec();chunk.extend(b"iCCP");chunk.extend([0u8;5]);
        png.splice(33..33,chunk);
        assert!(bounded_image_dimensions(&png).unwrap_err().contains("metadata"));
        let mut encoded=std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1024,1024).write_to(&mut encoded,image::ImageFormat::Png).unwrap();
        let png=encoded.into_inner();
        let document=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":png.len()}],
            "bufferViews":[{"buffer":0,"byteLength":png.len()}],"images":[{"bufferView":0,"mimeType":"image/png"}],
            "textures":[{"source":0},{"source":0},{"source":0},{"source":0},{"source":0}]});
        assert!(validate_resource_glb(&resource_glb_with_bin(document,png)).unwrap_err().contains("textures"));
    }
    #[test]
    fn dedicated_resource_glb_rejects_invalid_indices_and_flat_normal_expansion() {
        let doc=|count:usize|serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":36+count}],
            "bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":count}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[0,0,0]},
                {"bufferView":1,"componentType":5121,"count":count,"type":"SCALAR"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}]});
        let mut bin=vec![0;39];bin[38]=3;
        assert!(validate_resource_glb(&resource_glb_with_bin(doc(3),bin)).unwrap_err().contains("index"));
        let mut document=doc(262143);document["meshes"][0]["primitives"]=serde_json::json!(vec![serde_json::json!({"attributes":{"POSITION":0},"indices":1});63]);
        assert!(validate_resource_glb(&resource_glb_with_bin(document,vec![0;36+262143])).unwrap_err().contains("expanded"));
        let invalid=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],"meshes":[{"primitives":[{"attributes":{"POSITION":999}}]}]});
        assert!(validate_resource_glb(&resource_glb(invalid)).is_err());
        let mut morph=doc(3);morph["meshes"][0]["primitives"][0]["targets"]=serde_json::json!([{}]);
        assert!(validate_resource_glb(&resource_glb_with_bin(morph,vec![0;39])).is_err());
    }
    #[test]
    fn dedicated_resource_glb_charges_animation_channels_and_checks_targets() {
        let count=100_000;let size=count*16;
        let channel=serde_json::json!({"sampler":0,"target":{"node":0,"path":"translation"}});
        let mut document=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":size}],
            "bufferViews":[{"buffer":0,"byteLength":count*4},{"buffer":0,"byteOffset":count*4,"byteLength":count*12}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":count,"type":"SCALAR"},
                {"bufferView":1,"componentType":5126,"count":count,"type":"VEC3"}],"nodes":[{}],
            "animations":[{"samplers":[{"input":0,"output":1}],"channels":vec![channel;11]}]});
        assert!(validate_resource_glb(&resource_glb_with_bin(document.clone(),vec![0;size])).is_err());
        document["animations"][0]["channels"][0]["target"]["node"]=serde_json::json!(99);
        assert!(validate_resource_glb(&resource_glb_with_bin(document,vec![0;size])).is_err());
    }
    #[test]
    fn authored_materials_cast_shadows_and_opacity_is_instance_local() {
        use bevy::camera::visibility::RenderLayers;
        let mut world=World::new();
        world.init_resource::<Assets<StandardMaterial>>();
        let source=world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
            metallic:0.8,perceptual_roughness:0.23,double_sided:true,cull_mode:None,
            base_color:Color::srgba(0.2,0.3,0.4,0.6),alpha_mode:AlphaMode::Mask(0.3),..default()
        });
        let original=world.spawn(MeshMaterial3d(source.clone())).id();
        let faded=world.spawn(MeshMaterial3d(source.clone())).id();
        let mut cache=BTreeMap::new();
        prepare_mesh(&mut world,original,1.,&mut cache);
        prepare_mesh(&mut world,faded,0.5,&mut cache);
        assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(original).unwrap().0,source);
        for entity in [original,faded] {
            assert!(world.get::<RenderLayers>(entity).unwrap().intersects(&RenderLayers::layer(28)));
        }
        let assets=world.resource::<Assets<StandardMaterial>>();
        assert_eq!(assets.get(&source).unwrap().base_color.alpha(),0.6);
        assert_eq!(assets.get(&source).unwrap().alpha_mode,AlphaMode::Mask(0.3));
        let material=assets.get(&world.get::<MeshMaterial3d<StandardMaterial>>(faded).unwrap().0).unwrap();
        assert_eq!(material.base_color.alpha(),0.3);
        assert_eq!(material.metallic,0.8);
        assert_eq!(material.perceptual_roughness,0.23);
        assert!(material.double_sided);
        assert!(material.cull_mode.is_none());
    }

    #[test] fn bind_delta_does_not_accumulate_or_discard_authored_scale() {
        let authored=Transform::from_xyz(2.,3.,4.).with_scale(Vec3::splat(2.));
        let mut state=NodeState::default(); state.transform.position=[0.,0.2,0.];
        state.transform.rotation=Quat::from_rotation_y(0.4).to_array();
        let a=node_transform(authored,&state,0.);
        let b=node_transform(authored,&state,0.);
        assert_eq!(a,b); assert_eq!(a.scale,Vec3::splat(2.));
        assert!((a.translation.y-3.2).abs()<1e-5);
    }
    #[test] fn node_extrapolation_freezes_at_one_tenth_second() {
        let mut state=NodeState::default();state.angular_velocity=[20.,0.,0.];
        assert_eq!(node_transform(Transform::IDENTITY,&state,1.),node_transform(Transform::IDENTITY,&state,0.1));
    }
}

#[cfg(test)]
mod deformation_tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    #[test]
    fn deformation_is_instance_local_and_keeps_glb_materials_and_rigid_parts() {
        let mut world=World::new();world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        let source=world.resource_mut::<Assets<Mesh>>().add(Cuboid::new(1.,1.,1.));
        let material=world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {metallic:0.9,..default()});
        let root=world.spawn(Transform::IDENTITY).id();
        let panel=world.spawn((Name::new("panel"),Transform::from_xyz(0.5,0.,0.),ChildOf(root),
            Mesh3d(source.clone()),MeshMaterial3d(material.clone()))).id();
        let rigid=world.spawn((Name::new("rigid"),Transform::IDENTITY,ChildOf(root),Mesh3d(source.clone()))).id();
        let mut owned=Owned {entity:root,mesh:None,material:None,body:Some("object".into()),
            definition:GraphicsDefinition {path:String::new(),body:Some("object".into()),color:[1.;3],opacity:1.,deform_nodes:vec!["panel".into()]},
            transform:TransformState {position:[0.25,0.,0.],scale:[2.,1.,1.],..default()},visible:true,serial:1,
            nodes:BTreeMap::new(),bindings:BTreeMap::new(),ambiguous:BTreeSet::new(),warned:BTreeSet::new(),ready:true,
            deformation:default()};
        let field=skate_dynamics::deformation::Field {min:[-4.;3],max:[4.;3],resolution:[2;3],offsets:vec![[0.,0.,-0.2];8],revision:1};
        super::super::graphics_deformation::sync(&mut world,&mut owned,1,&field,[0.25,0.,0.]);
        let handle=world.get::<Mesh3d>(panel).unwrap().0.clone();
        assert_ne!(handle,source);
        assert_eq!(world.get::<Mesh3d>(rigid).unwrap().0,source);
        assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(panel).unwrap().0,material);
        let meshes=world.resource::<Assets<Mesh>>();
        let Some(VertexAttributeValues::Float32x3(original))=meshes.get(&source).unwrap().attribute(Mesh::ATTRIBUTE_POSITION) else {panic!()};
        let Some(VertexAttributeValues::Float32x3(damaged))=meshes.get(&handle).unwrap().attribute(Mesh::ATTRIBUTE_POSITION) else {panic!()};
        for (a,b) in original.iter().zip(damaged) {assert!((b[2]-a[2]+0.2).abs()<1e-5);assert_eq!(b[0],a[0]);}
        super::super::graphics_deformation::sync(&mut world,&mut owned,1,&field,[0.25,0.,0.]);
        assert_eq!(world.get::<Mesh3d>(panel).unwrap().0,handle);
        // New replica layout rebinds weights to retained ORIGINAL vertices.
        let repaired=skate_dynamics::deformation::Field {min:[-4.;3],max:[4.;3],resolution:[3;3],offsets:vec![[0.;3];27],revision:0};
        super::super::graphics_deformation::sync(&mut world,&mut owned,2,&repaired,[0.25,0.,0.]);
        let meshes=world.resource::<Assets<Mesh>>();
        assert_eq!(meshes.get(&handle).unwrap().attribute(Mesh::ATTRIBUTE_POSITION),meshes.get(&source).unwrap().attribute(Mesh::ATTRIBUTE_POSITION));
        owned.deformation.clear(&mut world);
        assert!(world.resource::<Assets<Mesh>>().get(&handle).is_none());
        assert!(world.resource::<Assets<Mesh>>().get(&source).is_some());
    }
}

#[cfg(test)]
mod deformation_timing {
    use super::*;
    #[test]
    #[ignore = "manual actual Skyline asset timing probe"]
    fn actual_asset_deformation_cpu_cost() {
        let path=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/Skyline_Drive_Mod/skyline.glb");
        let geometry=skate_mods::model::read_geometry_file(&path,"skyline_mesh",&Default::default()).unwrap();
        let vertices=geometry.vertices.len();
        let mut source=Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList,bevy::asset::RenderAssetUsages::default());
        source.insert_attribute(Mesh::ATTRIBUTE_POSITION,geometry.vertices);
        source.insert_indices(bevy::mesh::Indices::U32(geometry.triangles.into_iter().flatten().collect()));
        source.compute_normals();
        let mut world=World::new();world.init_resource::<Assets<Mesh>>();
        let source=world.resource_mut::<Assets<Mesh>>().add(source);
        let root=world.spawn(Transform::IDENTITY).id();
        world.spawn((Name::new("shell"),Transform::IDENTITY,ChildOf(root),Mesh3d(source)));
        let mut owned=Owned {entity:root,mesh:None,material:None,body:Some("object".into()),
            definition:GraphicsDefinition {path:String::new(),body:Some("object".into()),color:[1.;3],opacity:1.,deform_nodes:vec!["shell".into()]},
            transform:default(),visible:true,serial:1,nodes:BTreeMap::new(),bindings:BTreeMap::new(),ambiguous:BTreeSet::new(),warned:BTreeSet::new(),ready:true,deformation:default()};
        let mut field=skate_dynamics::deformation::Field {min:[-2.,-1.,-3.],max:[2.,2.,3.],resolution:[9,5,17],offsets:vec![[0.;3];9*5*17],revision:1};
        for i in 0..field.offsets.len() {let p=field.rest(i);if p.z>1. {field.offsets[i][2]=-0.2;}}
        let t=std::time::Instant::now();super::super::graphics_deformation::sync(&mut world,&mut owned,1,&field,[0.;3]);
        let initial=t.elapsed();
        field.revision+=1;for p in &mut field.offsets {p[2]*=1.2;}
        let t=std::time::Instant::now();super::super::graphics_deformation::sync(&mut world,&mut owned,1,&field,[0.;3]);let update=t.elapsed();
        let t=std::time::Instant::now();for _ in 0..10000 {super::super::graphics_deformation::sync(&mut world,&mut owned,1,&field,[0.;3]);}
        eprintln!("Skyline welded vertices={vertices}; initial mesh bind/update={initial:?}; subsequent damage={update:?}; 10000 unchanged checks={:?}; excludes GPU upload/tangents",t.elapsed());
    }
}
