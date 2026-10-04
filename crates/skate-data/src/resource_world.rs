//! Runtime subset for downloaded, redistributable SKATE worlds. The same
//! validation precedes native client collision and headless server terrain.
use crate::skate_map::SkateMap;

/// Native procedural test-world anchor, shared by map transitions and trusted
/// dedicated-server recovery so removing a resource world cannot drift.
pub const TEST_WORLD_SPAWN: [f32; 3] = [0., -0.035, 0.];

pub fn validate(map: &SkateMap) -> Result<(), String> {
    if map.geometry.collision.is_empty() {return Err("Resource world requires collision geometry".into());}
    validate_render(map)
}
pub fn validate_render(map:&SkateMap)->Result<(),String> {
    if map.geometry.collision.len() > 2_000_000
        || map.geometry.vertices.len() > 4_000_000 || map.geometry.indices.len() > 12_000_000
        || map.materials.len() > 4096 || map.textures.len() > 4096 || map.rails.len() > 4096 {
        return Err("Resource world requires collision and exceeds a geometry/material/rail budget".into());
    }
    if !map.doors.is_empty() || !map.routes.is_empty() {
        return Err("Resource worlds do not support native doors or NPC routes; use shared entities for moving objects".into());
    }
    if map.extensions.iter().any(|e| !matches!(&e.tag,b"WMET"|b"WCFG"|b"BMAT")) {
        return Err("Resource world has unsupported extension; static triangle collision and authored/verified native rails are required".into());
    }
    let bounded = |p:[f32;3]|p.into_iter().all(|x|x.is_finite() && x.abs()<=99_999.);
    if !bounded(map.spawn) || !map.heading.is_finite()
        || map.geometry.vertices.iter().any(|v|!bounded(v.position))
        || map.geometry.collision.iter().any(|t|t.points.iter().any(|p|!bounded(*p)))
        || map.rails.iter().any(|r|r.points.len()<2 || r.points.len()>4096 || r.points.iter().any(|p|!bounded(*p)))
        || map.rails.iter().map(|r|r.points.len()).sum::<usize>()>65_536 {
        return Err("Resource world has invalid or over-budget coordinates/rail points".into());
    }
    for triangle in &map.geometry.collision {
        let [a,b,c]=triangle.points;
        let ab:[f32;3]=std::array::from_fn(|i|b[i]-a[i]);
        let ac:[f32;3]=std::array::from_fn(|i|c[i]-a[i]);
        let cross=[ab[1]*ac[2]-ab[2]*ac[1],ab[2]*ac[0]-ab[0]*ac[2],ab[0]*ac[1]-ab[1]*ac[0]];
        if cross.iter().map(|x|x*x).sum::<f32>()<1e-12 {return Err("Resource world contains degenerate collision triangle".into());}
    }
    Ok(())
}
