//! Background preparation owns bytes, native collision and render assets together.
use bevy::prelude::*;
use skate_core::physics::board_world::BoardWorld;
use skate_resources::locations::PreparedCatalog;
use std::sync::Arc;

pub struct Prepared {
    pub package: Arc<PreparedCatalog>,
    pub collision: BoardWorld,
    pub models: Vec<(String, super::model::Model)>,
}
pub fn prepare(
    base: &BoardWorld,
    material: skate_core::physics::contact::RetailContactMaterial,
    package: Arc<PreparedCatalog>,
) -> Result<Prepared, String> {
    let shells = package
        .catalog
        .interiors
        .iter()
        .map(|i| skate_data::location_collision::decode(&package.files[&i.collision], i.transform))
        .collect::<Result<Vec<_>, _>>()?;
    let collision = skate_data::location_collision::compose(base, &shells, material)?;
    for i in &package.catalog.interiors {
        support(&collision, i.spawn)?;
        support(&collision, i.exit.position)?;
    }
    for l in &package.catalog.locations {
        support(base, l.position)?;
        support(base, l.return_position)?;
        let d = Vec2::new(
            l.position[0] - l.return_position[0],
            l.position[2] - l.return_position[2],
        );
        if d.length() < l.style.radius + 0.75 {
            return Err(format!("Return for {} intersects its entrance", l.key));
        }
    }
    let models = package
        .catalog
        .interiors
        .iter()
        .map(|i| {
            super::model::prepare(&package.files[&i.model], i.transform).map(|m| (i.key.clone(), m))
        })
        .collect::<Result<_, _>>()?;
    Ok(Prepared {
        package,
        collision,
        models,
    })
}
/// Floor support and standing room are checked against the real query world.
pub fn support(world: &BoardWorld, p: [f32; 3]) -> Result<(), String> {
    use skate_core::math::Vector3;
    let ray = |a: [f32; 3], b: [f32; 3]| {
        world
            .query_thin_line(
                Vector3::new(a[0], a[1], a[2]),
                Vector3::new(b[0], b[1], b[2]),
            )
            .map_err(str::to_owned)
    };
    if ray([p[0], p[1] + 0.35, p[2]], [p[0], p[1] - 0.35, p[2]])?
        .is_none_or(|hit| hit.geometry.normal.y < 0.7)
    {
        return Err(format!("No native floor support at {p:?}"));
    }
    // Sweeping a body-sized sphere catches walls parallel to the vertical
    // clearance rays, including a wall already intersecting the standing body.
    let a = Vector3::new(p[0], p[1] + 0.46, p[2]);
    let b = Vector3::new(p[0], p[1] + 1.5, p[2]);
    if world
        .query_swept_line(a, b, 0.3)
        .map_err(str::to_owned)?
        .is_some()
        || world
            .query_swept_line(b, a, 0.3)
            .map_err(str::to_owned)?
            .is_some()
    {
        return Err(format!("Standing body obstructed at {p:?}"));
    }
    for (dx, dz) in [(0., 0.), (0.3, 0.), (-0.3, 0.), (0., 0.3), (0., -0.3)] {
        // Test both directions because native thin rays respect face winding.
        let a = [p[0] + dx, p[1] + 0.15, p[2] + dz];
        let b = [p[0] + dx, p[1] + 1.8, p[2] + dz];
        if ray(a, b)?.is_some() || ray(b, a)?.is_some() {
            return Err(format!("Standing room obstructed at {p:?}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_or_obstructed_spawn_is_rejected() {
        let material = skate_core::physics::contact::RetailContactMaterial {
            static_friction: 0.8,
            dynamic_friction: 0.6,
            restitution: 0.,
        };
        let base = crate::physics::ground::Terrain::Flat.world(material);
        assert!(support(&base, [0., 0., 0.]).is_ok());
        assert!(support(&base, [0., 10., 0.]).is_err());
        let ceiling = vec![[[-2., 1., -2.], [2., 1., -2.], [-2., 1., 2.]]];
        let blocked = skate_data::location_collision::compose(&base, &[ceiling], material).unwrap();
        assert!(support(&blocked, [-0.5, 0., -0.5]).is_err());
        let wall = vec![
            [[0.15, 0., -2.], [0.15, 2., -2.], [0.15, 2., 2.]],
            [[0.15, 0., -2.], [0.15, 2., 2.], [0.15, 0., 2.]],
        ];
        let blocked = skate_data::location_collision::compose(&base, &[wall], material).unwrap();
        assert!(
            support(&blocked, [0., 0., 0.]).is_err(),
            "wall through spawn body must reject"
        );
        assert!(support(&blocked, [-1., 0., 0.]).is_ok());
    }
    #[test]
    #[ignore = "requires owned Downtown and prepared apartment paths"]
    fn installed_downtown_and_apartment_support_survey() {
        use skate_core::math::Vector3;
        let material = skate_core::physics::contact::RetailContactMaterial {
            static_friction: 0.8,
            dynamic_friction: 0.6,
            restitution: 0.,
        };
        let path = std::env::var("SKATE_LOCATION_MAP").expect("SKATE_LOCATION_MAP");
        let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(&path)).unwrap();
        let base = crate::skate_world::collision_world(&map, material).unwrap();
        for x in [142., 143., 144., 145., 146., 147., 148., 149.] {
            for z in [294., 295., 296., 297., 298.] {
                if let Some(hit) = base
                    .query_thin_line(Vector3::new(x, 24., z), Vector3::new(x, 20., z))
                    .unwrap()
                {
                    let p = hit.geometry.position;
                    println!(
                        "EXTERIOR x={} y={} z={} clear={}",
                        p.x,
                        p.y,
                        p.z,
                        support(&base, [p.x, p.y, p.z]).is_ok()
                    );
                }
            }
        }
        for z in [294., 295., 296., 297., 298.] {
            if let Some(hit) = base
                .query_thin_line(Vector3::new(149., 23.5, z), Vector3::new(135., 23.5, z))
                .unwrap()
            {
                println!("DOOR_WALL {:?}", hit.geometry.position);
            }
        }
        let root = std::path::PathBuf::from(
            std::env::var("SKATE_LOCATION_RUNTIME").expect("SKATE_LOCATION_RUNTIME"),
        );
        let package = PreparedCatalog::read(&root, "catalog.json").unwrap();
        prepare(&base, material, Arc::new(package))
            .expect("all installed entry/exit/spawn/return anchors must be safe");
        let transform = Mat4::from_translation(Vec3::new(4096., 100., 4096.)).to_cols_array_2d();
        let shell = skate_data::location_collision::decode(
            &std::fs::read(root.join("collision.json")).unwrap(),
            transform,
        )
        .unwrap();
        let composed = skate_data::location_collision::compose(&base, &[shell], material).unwrap();
        let mut clear = 0;
        for x in [-4., -3., -2., -1.5, -1., -0.5, 0., 1.] {
            for z in [-3., -2., -1., 0., 1.] {
                let p = [4096. + x, 100., 4096. + z];
                if support(&composed, p).is_ok() {
                    println!("INTERIOR_CLEAR {p:?}");
                    clear += 1;
                }
            }
        }
        assert!(clear > 0, "Apartment must have supported standing room");
        super::super::model::prepare(
            &std::fs::read(root.join("apartment.glb")).unwrap(),
            transform,
        )
        .unwrap();
    }
    #[test]
    #[ignore = "requires owned map and candidate catalog"]
    fn installed_multi_apartment_catalog_validates_models_and_anchors() {
        use skate_core::math::Vector3;
        let material = skate_core::physics::contact::RetailContactMaterial {
            static_friction: 0.8,
            dynamic_friction: 0.6,
            restitution: 0.,
        };
        let map = skate_data::skate_map::SkateMap::load(std::path::Path::new(
            &std::env::var("SKATE_LOCATION_MAP").unwrap(),
        ))
        .unwrap();
        let base = crate::skate_world::collision_world(&map, material).unwrap();
        if let Ok(raw) = std::env::var("SKATE_LOCATION_PROBES") {
            let probes: Vec<[f32; 3]> = serde_json::from_str(&raw).unwrap();
            for p in probes {
                if let Some(hit) = base
                    .query_thin_line(
                        Vector3::new(p[0], p[1] + 3., p[2]),
                        Vector3::new(p[0], p[1] - 3., p[2]),
                    )
                    .unwrap()
                {
                    let v = hit.geometry.position;
                    println!(
                        "PROBE {:?} clear={}",
                        [v.x, v.y, v.z],
                        support(&base, [v.x, v.y, v.z]).is_ok()
                    );
                    for (dx, dz) in [(15., 0.), (-15., 0.), (0., 15.), (0., -15.)] {
                        if let Some(wall) = base
                            .query_thin_line(
                                Vector3::new(v.x, v.y + 1., v.z),
                                Vector3::new(v.x + dx, v.y + 1., v.z + dz),
                            )
                            .unwrap()
                        {
                            println!("WALL {:?}", wall.geometry.position);
                        }
                    }
                }
            }
        }
        let root = std::path::PathBuf::from(std::env::var("SKATE_LOCATION_RUNTIME").unwrap());
        let package = PreparedCatalog::read(&root, "catalog.json").unwrap();
        assert!(package.catalog.interiors.len() >= 4);
        assert!(package.catalog.locations.len() >= 2);
        let ready = prepare(&base, material, Arc::new(package)).unwrap();
        assert_eq!(ready.models.len(), ready.package.catalog.interiors.len());
        println!(
            "CATALOG_VALID interiors={} entries={}",
            ready.models.len(),
            ready.package.catalog.locations.len()
        );
    }

}
