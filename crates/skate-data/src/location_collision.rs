//! Bounded authored shell decoding. Column-major transforms match glTF.
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Shell {
    version: u32,
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
}
pub fn decode(bytes: &[u8], m: [[f32; 4]; 4]) -> Result<Vec<[[f32; 3]; 3]>, String> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("collision shell exceeds 4 MiB".into());
    }
    if m.iter().flatten().any(|v| !v.is_finite())
        || m[0][3] != 0.
        || m[1][3] != 0.
        || m[2][3] != 0.
        || m[3][3] != 1.
    {
        return Err("collision transform is not finite affine".into());
    }
    let det = m[0][0] as f64 * (m[1][1] as f64 * m[2][2] as f64 - m[1][2] as f64 * m[2][1] as f64)
        - m[1][0] as f64 * (m[0][1] as f64 * m[2][2] as f64 - m[0][2] as f64 * m[2][1] as f64)
        + m[2][0] as f64 * (m[0][1] as f64 * m[1][2] as f64 - m[0][2] as f64 * m[1][1] as f64);
    if det.abs() < 1e-12 {
        return Err("singular collision transform".into());
    }
    let s: Shell = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if s.version != 1
        || s.positions.is_empty()
        || s.positions.len() > 65536
        || s.triangles.is_empty()
        || s.triangles.len() > 32768
    {
        return Err("invalid collision version/count".into());
    }
    let positions = s
        .positions
        .iter()
        .map(|p| {
            let q = std::array::from_fn(|axis| {
                m[0][axis] * p[0] + m[1][axis] * p[1] + m[2][axis] * p[2] + m[3][axis]
            });
            if p.iter()
                .chain(q.iter())
                .any(|v| !v.is_finite() || v.abs() > 100_000.)
            {
                Err("nonfinite collision position".to_string())
            } else {
                Ok(q)
            }
        })
        .collect::<Result<Vec<[f32; 3]>, String>>()?;
    s.triangles
        .iter()
        .map(|indices| {
            let mut tri = [[0.; 3]; 3];
            for j in 0..3 {
                tri[j] = *positions
                    .get(indices[j] as usize)
                    .ok_or("collision index outside positions")?;
            }
            let a: [f64; 3] = std::array::from_fn(|k| tri[1][k] as f64 - tri[0][k] as f64);
            let b: [f64; 3] = std::array::from_fn(|k| tri[2][k] as f64 - tri[0][k] as f64);
            let cross = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            if cross.iter().map(|v| v * v).sum::<f64>() <= 1e-20 {
                return Err("degenerate collision triangle".into());
            }
            // Reflection changes authored winding; keep the original front face.
            if det < 0. {
                tri.swap(1, 2);
            }
            Ok(tri)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: [[f32; 4]; 4] = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];
    fn shell() -> serde_json::Value {
        serde_json::json!({"version":1,"positions":[[0,0,0],[0,0,2],[2,0,0],[0,2,0]],"triangles":[[0,1,2],[0,3,1]]})
    }
    #[test]
    fn collision_shell_rejects_indices_and_nonfinite_transforms() {
        let good = serde_json::to_vec(&shell()).unwrap();
        assert_eq!(decode(&good, ID).unwrap().len(), 2);
        let mut bad = shell();
        bad["triangles"][0][0] = serde_json::json!(4);
        assert!(decode(&serde_json::to_vec(&bad).unwrap(), ID).is_err());
        let mut transform = ID;
        transform[3][0] = f32::NAN;
        assert!(decode(&good, transform).is_err());
        transform = ID;
        transform[0][3] = 1.;
        assert!(decode(&good, transform).is_err());
        transform = ID;
        transform[1][1] = 0.;
        assert!(decode(&good, transform).is_err());
        assert!(decode(&vec![b' '; 4 * 1024 * 1024 + 1], ID).is_err());
    }
    #[test]
    fn location_collision_floor_and_wall_transform_exactly() {
        let mut transform = ID;
        transform[3] = [4096., 100., 4096., 1.];
        let actual = decode(&serde_json::to_vec(&shell()).unwrap(), transform).unwrap();
        assert_eq!(
            actual[0],
            [
                [4096., 100., 4096.],
                [4096., 100., 4098.],
                [4098., 100., 4096.]
            ]
        );
        assert_eq!(actual[1][1], [4096., 102., 4096.]);
    }
}

/// Build an additive native world without reinterpreting base retail metadata.
/// Call during preparation, before binding any moving external query provider.
pub fn compose(
    base: &skate_core::physics::board_world::BoardWorld,
    shells: &[Vec<[[f32; 3]; 3]>],
    material: skate_core::physics::contact::RetailContactMaterial,
) -> Result<skate_core::physics::board_world::BoardWorld, String> {
    use skate_core::{
        math::Vector3,
        physics::{
            board_world::{
                WorldTriangle,
                query_metadata::{Bounds, QueryMesh, QueryPool},
            },
            drive_frames::RetailAffineTransform,
        },
    };
    let mut triangles = base.triangles().to_vec();
    let mut metadata = base.query_metadata().map_err(str::to_owned)?.clone();
    if shells.len() > 16 || shells.iter().any(|s| s.is_empty() || s.len() > 32768) {
        return Err("additive collision shell count exceeds catalog bounds".into());
    }
    let mut geometry = metadata
        .meshes
        .iter()
        .map(|m| m.geometry)
        .max()
        .unwrap_or(0);
    for shell in shells {
        geometry = geometry
            .checked_add(1)
            .ok_or("collision geometry identity exhausted")?;
        let start = triangles.len();
        for points in shell {
            triangles.push(
                WorldTriangle::from_vertices(
                    points.map(|p| Vector3::new(p[0], p[1], p[2])),
                    material,
                    0,
                    skate_core::physics::collision::TriangleFeature::ONE_SIDED
                        | skate_core::physics::collision::TriangleFeature::USE_EDGE_COSINES
                        | 0xe0,
                    [1.; 3],
                    0.,
                )
                .ok_or("invalid additive collision triangle")?,
            );
            metadata.packed_surfaces.push(0);
        }
        for offset in (start..triangles.len()).step_by(64) {
            let end = (offset + 64).min(triangles.len());
            let local_bounds = Bounds::from_points(
                triangles[offset..end]
                    .iter()
                    .flat_map(|t| t.triangle.vertices),
            )
            .ok_or("invalid shell bounds")?;
            metadata.meshes.push(QueryMesh {
                triangle_range: offset..end,
                local_to_world: RetailAffineTransform::IDENTITY,
                world_to_local: RetailAffineTransform::IDENTITY,
                local_bounds,
                matching_group: -1,
                rejection_flags: 0,
                geometry,
                pool: QueryPool::Ground,
            });
        }
    }
    base.rebuild_static(triangles, metadata)
        .map_err(str::to_owned)
}

#[cfg(test)]
mod composition_tests {
    use super::*;
    use skate_core::{
        math::Vector3,
        physics::{
            board_world::{
                BoardWorld, WorldTriangle,
                query_metadata::{Bounds, QueryMesh, QueryMetadata, QueryPool},
            },
            contact::RetailContactMaterial,
            drive_frames::RetailAffineTransform,
        },
    };
    fn material() -> RetailContactMaterial {
        RetailContactMaterial {
            static_friction: 0.8,
            dynamic_friction: 0.6,
            restitution: 0.,
        }
    }
    pub(super) fn base(pool: QueryPool) -> BoardWorld {
        let t = WorldTriangle::from_vertices(
            [
                Vector3::new(0., 0., 0.),
                Vector3::new(0., 0., 4.),
                Vector3::new(4., 0., 0.),
            ],
            material(),
            17,
            0,
            [1.; 3],
            0.,
        )
        .unwrap();
        let bounds = Bounds::from_points(t.triangle.vertices).unwrap();
        BoardWorld::with_query_metadata(
            vec![t],
            QueryMetadata {
                packed_surfaces: vec![17],
                meshes: vec![QueryMesh {
                    triangle_range: 0..1,
                    local_to_world: RetailAffineTransform::IDENTITY,
                    world_to_local: RetailAffineTransform::IDENTITY,
                    local_bounds: bounds,
                    matching_group: -1,
                    rejection_flags: 0,
                    geometry: 29,
                    pool,
                }],
                static_edges: vec![],
                island_flags: 7,
            },
        )
        .unwrap()
    }
    #[test]
    fn location_composition_preserves_external_query_provider() {
        use skate_core::physics::board_world::{ExternalLineHit, ExternalQueries};
        struct Provider;
        impl ExternalQueries for Provider {
            fn line(&self, _: Vector3, _: Vector3, _: f32) -> Option<ExternalLineHit> {
                None
            }
            fn nearby(&self, _: Vector3, _: f32) -> Vec<[Vector3; 3]> {
                vec![[Vector3::ZERO; 3]]
            }
        }
        let mut base = base(QueryPool::Ground);
        base.set_external_queries(Some(std::sync::Arc::new(Provider)));
        let composed = compose(&base, &[], material()).unwrap();
        assert_eq!(composed.external_nearby(Vector3::ZERO, 1.).len(), 1);
    }
    #[test]
    fn authored_and_rwcm_metadata_survives_location_composition() {
        for pool in [QueryPool::Ground, QueryPool::Island] {
            let base = base(pool);
            let shell = vec![[[10., 2., 0.], [10., 2., 4.], [14., 2., 0.]]];
            let composed = compose(&base, &[shell], material()).unwrap();
            assert_eq!(base.triangles().len(), 1);
            let meta = composed.query_metadata().unwrap();
            assert_eq!(meta.meshes[0].geometry, 29);
            assert_eq!(meta.meshes[0].pool, pool);
            assert_eq!(meta.island_flags, 7);
            assert_eq!(meta.packed_surfaces[0], 17);
            assert_ne!(meta.meshes[1].geometry, 29);
            let ray = |w: &BoardWorld, x| {
                w.query_thin_line(Vector3::new(x, 5., 1.), Vector3::new(x, -2., 1.))
                    .unwrap()
                    .unwrap()
                    .geometry
            };
            assert_eq!(ray(&base, 1.), ray(&composed, 1.));
            assert!(
                composed
                    .query_thin_line(Vector3::new(11., 5., 1.), Vector3::new(11., 0., 1.))
                    .unwrap()
                    .is_some()
            );
        }
    }
}

#[cfg(test)]
mod snapshot_tests {
    #[test]
    fn location_snapshot_shares_immutable_geometry_for_background_preparation() {
        let world = super::composition_tests::base(
            skate_core::physics::board_world::query_metadata::QueryPool::Ground,
        );
        let snapshot = world.clone();
        assert_eq!(world.triangles().as_ptr(), snapshot.triangles().as_ptr());
    }
}

#[cfg(test)]
mod contact_regression {
    use super::*;
    use skate_core::{
        math::Vector3,
        physics::{
            board_world::BoardWorld,
            collision::{Sphere, WorldContactSettings},
            contact::RetailContactMaterial,
            world_contact::{ContactPrimitive, primitive_triangle_world_contacts},
        },
    };
    #[test]
    fn additive_floor_does_not_collide_with_actor_above_its_back_face() {
        let material = RetailContactMaterial {
            static_friction: 0.8,
            dynamic_friction: 0.6,
            restitution: 0.,
        };
        let shell = vec![[[-2., 2., -2.], [2., 2., -2.], [-2., 2., 2.]]]; // downward-facing ceiling
        let world = compose(
            &BoardWorld::with_query_metadata(
                vec![],
                skate_core::physics::board_world::query_metadata::QueryMetadata {
                    packed_surfaces: vec![],
                    meshes: vec![],
                    static_edges: vec![],
                    island_flags: 0,
                },
            )
            .unwrap(),
            &[shell],
            material,
        )
        .unwrap();
        let query = WorldContactSettings {
            volume_padding: 0.05,
            maximum_separating_distance: 0.1,
            edge_cos_bend_normal_threshold: -1.,
            convexity_epsilon: 0.,
            is_object: false,
        };
        let sphere = ContactPrimitive::Sphere(Sphere {
            center: Vector3::new(-0.5, 2.05, -0.5),
            radius: 0.1,
        });
        assert!(
            primitive_triangle_world_contacts(
                sphere,
                world.triangles()[0].triangle,
                Vector3::ZERO,
                query
            )
            .is_none()
        );
    }
}
