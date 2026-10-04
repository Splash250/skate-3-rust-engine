//! Bounded mesh residency. CPU source geometry and collision remain owned by
//! the mounted world; only render mesh assets enter and leave GPU residency.
use bevy::prelude::*;
use serde::{Deserialize,Serialize};

#[derive(Component,Clone,Copy)]
pub(crate) struct Cell {pub min:[f32;3],pub max:[f32;3],pub near:f32,pub far:f32}
#[derive(Clone,Copy,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub(crate) struct Options {pub distance:f32,pub upload_bytes_per_frame:usize,pub uploads_per_frame:usize}
impl Default for Options {fn default()->Self {Self {distance:600.,upload_bytes_per_frame:4*1024*1024,uploads_per_frame:2}}}
impl Options {
    pub(crate) fn validate(self)->Result<Self,String> {
        if !self.distance.is_finite() || !(100. ..=10000.).contains(&self.distance)
            || !(1..=16).contains(&self.uploads_per_frame)
            || !(65536..=64*1024*1024).contains(&self.upload_bytes_per_frame) {
            return Err("world-streaming.json requires distance100..10000m, uploads1..16 and bytes65536..67108864".into());
        } Ok(self)
    }
    pub(crate) fn read(root:&std::path::Path)->Result<Self,String> {
        use std::io::Read;
        let path=root.join("world-streaming.json");
        let meta=match std::fs::symlink_metadata(&path) {
            Ok(meta)=>meta,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(Self::default()),Err(e)=>return Err(e.to_string()),
        };
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len()>16384 {return Err("world-streaming.json must be a regular local file of at most16KiB".into());}
        let mut bytes=Vec::new();std::fs::File::open(path).map_err(|e|e.to_string())?.take(16385).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        if bytes.len()>16384 {return Err("world-streaming.json exceeds16KiB".into());}
        let value:serde_json::Value=serde_json::from_slice(&bytes).map_err(|e|format!("World streaming options: {e}"))?;
        if !value.is_object() {return Err("world-streaming.json must contain an object".into());}
        serde_json::from_value::<Self>(value).map_err(|e|format!("World streaming options: {e}"))?.validate()
    }
}
struct Chunk {entity:Entity,handle:Option<Handle<Mesh>>,source:Mesh,cell:Cell,bytes:usize,resident:bool}
#[derive(Resource)]
struct Residency {chunks:Vec<Chunk>,options:Options,ready:bool,origin:[f32;3]}
fn distance(cell:Cell,origin:[f32;3])->f32 {
    (0..3).map(|i| (cell.min[i]-origin[i]).max(origin[i]-cell.max[i]).max(0.).powi(2)).sum::<f32>().sqrt()
}
pub(crate) fn install(world:&mut World,options:Options,origin:[f32;3])->Result<(),String> {
    let options=options.validate()?;
    clear(world);
    let entries=world.query::<(Entity,&Mesh3d,&Cell)>().iter(world).map(|(entity,mesh,cell)|(entity,mesh.0.clone(),*cell)).collect::<Vec<_>>();
    let mut chunks=Vec::new();
    for (entity,handle,mut cell) in entries {
        let source=world.resource_mut::<Assets<Mesh>>().remove(handle.id()).ok_or("Required streamed mesh was not published")?;
        let bytes=source.get_vertex_buffer_size()+source.get_index_buffer_bytes().map_or(0,<[u8]>::len);
        if cell.far==0. {cell.far=options.distance;}
        cell.far=cell.far.min(options.distance);
        world.entity_mut(entity).remove::<Mesh3d>();
        chunks.push(Chunk {entity,handle:None,source,cell,bytes,resident:false});
    }
    world.insert_resource(Residency {chunks,options,ready:false,origin});
    step(world,origin)
}
pub(crate) fn step(world:&mut World,origin:[f32;3])->Result<(),String> {
    let Some(mut state)=world.remove_resource::<Residency>() else {return Ok(());};
    let result=(|| {
        state.origin=origin;
        let mut wanted=Vec::new();
        for (i,chunk) in state.chunks.iter_mut().enumerate() {
            let d=distance(chunk.cell,origin);
            let visible=d>=chunk.cell.near && d<chunk.cell.far;
            let keep=d>=(chunk.cell.near-20.).max(0.) && d<chunk.cell.far+20. && chunk.cell.near<chunk.cell.far;
            if chunk.resident && !keep {
                world.entity_mut(chunk.entity).remove::<Mesh3d>();
                if let Some(handle)=chunk.handle.take() {world.resource_mut::<Assets<Mesh>>().remove(handle.id());}
                chunk.resident=false;
            }
            if !visible {world.entity_mut(chunk.entity).remove::<Mesh3d>();}
            if visible && chunk.resident {world.entity_mut(chunk.entity).insert(Mesh3d(chunk.handle.as_ref().unwrap().clone()));}
            if visible && !chunk.resident {wanted.push((d,i));}
        }
        wanted.sort_by(|(a,i),(b,j)|a.total_cmp(b).then(i.cmp(j)));
        let mut uploaded=0;let mut count=0;
        for (_,index) in wanted {
            let chunk=&mut state.chunks[index];
            if count==state.options.uploads_per_frame || count>0 && uploaded+chunk.bytes>state.options.upload_bytes_per_frame {break;}
            // One bounded spatial leaf may exceed the requested byte quantum;
            // always allow one leaf so a large legal cell cannot starve forever.
            // Bevy frees GPU assets on last-handle Unused, not Removed. A fresh
            // strong handle is necessary after eviction; never retain it in CPU source storage.
            chunk.handle=Some(world.resource_mut::<Assets<Mesh>>().add(chunk.source.clone()));
            world.entity_mut(chunk.entity).insert(Mesh3d(chunk.handle.as_ref().unwrap().clone()));chunk.resident=true;
            uploaded+=chunk.bytes;count+=1;
        }
        state.ready=state.chunks.iter().all(|chunk| {
            let d=distance(chunk.cell,origin);d<chunk.cell.near || d>=chunk.cell.far || chunk.resident
        });
        Ok(())
    })();
    world.insert_resource(state);result
}
pub(crate) fn update(world:&mut World) {
    if !world.contains_resource::<Residency>() {return;}
    let position=world.query_filtered::<&Transform,With<crate::world::PlayerRoot>>().iter(world).next()
        .map(|t|t.translation.to_array()).unwrap_or(world.resource::<Residency>().origin);
    if let Err(error)=step(world,position) {error!("Required world streaming failed: {error}");}
}
pub(crate) fn ready(world:&World)->bool {world.get_resource::<Residency>().is_none_or(|state|state.ready)}
pub(crate) fn clear(world:&mut World) {
    if let Some(state)=world.remove_resource::<Residency>() {
        for chunk in state.chunks {
            if let Ok(mut entity)=world.get_entity_mut(chunk.entity) {entity.remove::<Mesh3d>();}
            if let Some(handle)=chunk.handle {world.resource_mut::<Assets<Mesh>>().remove(handle.id());}
        }
    }
}

#[cfg(test)] mod tests {
    use super::*;
    fn cell(world:&mut World,x:f32)->(Entity,Handle<Mesh>) {
        let handle=world.resource_mut::<Assets<Mesh>>().add(Cuboid::new(2.,2.,2.));
        let entity=world.spawn((super::super::MapEntity,Mesh3d(handle.clone()),Cell {min:[x-1.,-1.,-1.],max:[x+1.,1.,1.],near:0.,far:0.})).id();
        (entity,handle)
    }
    #[test] fn resource_cells_evict_restore_and_bound_per_frame_mesh_uploads() {
        let mut world=World::new();world.init_resource::<Assets<Mesh>>();
        let (near,near_mesh)=cell(&mut world,0.);let (other,other_mesh)=cell(&mut world,5.);let (far,far_mesh)=cell(&mut world,1000.);
        install(&mut world,Options {distance:100.,uploads_per_frame:1,..Default::default()},[0.;3]).unwrap();
        assert!(world.get::<Mesh3d>(near).is_some());
        assert!(world.get::<Mesh3d>(other).is_none(),"upload count must bound one frame");
        assert!(world.get::<Mesh3d>(far).is_none());
        assert!(world.resource::<Assets<Mesh>>().get(far_mesh.id()).is_none(),"far asset must leave residency");
        assert!(!ready(&world),"nearby required geometry is still pending");
        step(&mut world,[0.;3]).unwrap();assert!(ready(&world));
        step(&mut world,[1000.,0.,0.]).unwrap();
        assert!(world.get::<Mesh3d>(far).is_some());
        assert!(world.resource::<Assets<Mesh>>().get(near_mesh.id()).is_none());
        assert!(world.resource::<Assets<Mesh>>().get(other_mesh.id()).is_none());
        step(&mut world,[0.;3]).unwrap();assert_ne!(world.get::<Mesh3d>(near).unwrap().0.id(),near_mesh.id(),"GPU eviction drops the old strong handle");
        clear(&mut world);
        assert!(world.resource::<Assets<Mesh>>().is_empty());
    }
    #[test] fn streaming_options_reject_unbounded_distance_and_uploads() {
        assert!(Options {distance:f32::NAN,..Default::default()}.validate().is_err());
        assert!(Options {uploads_per_frame:1000,..Default::default()}.validate().is_err());
        assert!(Options {upload_bytes_per_frame:0,..Default::default()}.validate().is_err());
    }
    #[test] fn authored_lod_switches_mesh_residency_without_changing_world_identity() {
        let mut world=World::new();world.init_resource::<Assets<Mesh>>();
        let (near,_)=cell(&mut world,0.);let (far,_)=cell(&mut world,0.);
        world.get_mut::<Cell>(near).unwrap().far=100.;
        world.get_mut::<Cell>(far).unwrap().near=100.;
        install(&mut world,Options::default(),[0.;3]).unwrap();
        assert!(world.get::<Mesh3d>(near).is_some());assert!(world.get::<Mesh3d>(far).is_none());
        step(&mut world,[200.,0.,0.]).unwrap();
        assert!(world.get::<Mesh3d>(near).is_none());assert!(world.get::<Mesh3d>(far).is_some());
        assert_eq!(world.query::<&super::super::MapEntity>().iter(&world).count(),2);
    }
}
