//! Server-selected resource terrain shared by authoritative entity simulation
//! and server gameplay validators. No rendering or client-reported geometry.
mod rails;
pub use rails::Rails;
use std::sync::Arc;

#[derive(Clone)]
pub struct Terrain {
    pub revision: String,
    pub triangles: Arc<Vec<[[f32; 3]; 3]>>,
    pub spawn: [f32; 3],
    pub heading: f32,
}
impl Terrain {
    pub fn from_published(published: &skate_resources::PublishedSet) -> Result<Option<Self>, String> {
        let mut worlds = published.set.resources.iter().filter(|r| r.manifest.world.is_some());
        let Some(resource) = worlds.next() else {return Ok(None);};
        if worlds.next().is_some() {return Err("More than one required resource world".into());}
        let world = resource.manifest.world.as_ref().unwrap();
        let file = resource.files.get(&world.map).ok_or("Required world content is absent")?;
        let bytes = published.blobs.get(&file.digest).ok_or("Required world blob is absent")?;
        let mut remaining=world.max_decoded_bytes as usize;
        let map = skate_data::skate_map::SkateMap::parse_budgeted(bytes,&mut remaining,false)?;
        skate_data::resource_world::validate(&map)?;
        for lod in &world.lods {
            let file=resource.files.get(&lod.map).ok_or("Required world LOD is absent")?;
            let bytes=published.blobs.get(&file.digest).ok_or("Required world LOD blob is absent")?;
            let far=skate_data::skate_map::SkateMap::parse_budgeted(bytes,&mut remaining,true)?;
            skate_data::resource_world::validate_render(&far)?;
        }
        Ok(Some(Self {revision:file.digest.clone(),spawn:map.spawn,heading:map.heading,
            triangles:Arc::new(map.geometry.collision.into_iter().map(|t|t.points).collect())}))
    }
}
