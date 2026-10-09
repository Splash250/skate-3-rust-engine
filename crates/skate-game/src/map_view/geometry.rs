use bevy::prelude::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct OverviewBounds {
    pub min: Vec3,
    pub max: Vec3,
}
pub(super) struct OverviewGeometry {
    pub positions: Vec<[f32; 3]>,
    pub source_triangles: Vec<usize>,
    pub uvs: Vec<[f32; 2]>,
    pub material_indices: Vec<u32>,
    pub bounds: OverviewBounds,
}
// Shared quantized vertices preserve contiguous surfaces, unlike sampling faces.
// Both output and scratch storage are bounded; an oversized pass stops early and
// retries at a coarser grid. Y gets its own scale so wide maps retain elevation.
const OVERVIEW_TRIANGLE_BUDGET: usize = 262_144;
const DETAIL_TRIANGLE_BUDGET: usize = 2_000_000;
const INTERMEDIATE_TRIANGLE_BUDGET: usize = 750_000;
pub(super) struct MapLodSet {
    pub levels: Vec<(u8, OverviewGeometry)>,
}
pub(super) fn build_lods(map: &skate_data::skate_map::SkateMap) -> Option<MapLodSet> {
    let levels: Vec<_> = [
        from_map(map),
        from_geometry(&map.geometry, INTERMEDIATE_TRIANGLE_BUDGET),
        from_map_detail(map),
    ]
    .into_iter()
    .enumerate()
    .filter_map(|(level, g)| g.map(|g| (level as u8, g)))
    .collect();
    (!levels.is_empty()).then_some(MapLodSet { levels })
}

fn reduce<I: IntoIterator<Item = (usize, [Vec3; 3])>>(
    triangles: impl Fn() -> I,
) -> Option<OverviewGeometry> {
    reduce_with_budget(triangles, OVERVIEW_TRIANGLE_BUDGET)
}

fn reduce_with_budget<I: IntoIterator<Item = (usize, [Vec3; 3])>>(
    triangles: impl Fn() -> I,
    budget: usize,
) -> Option<OverviewGeometry> {
    reduce_grouped(triangles, budget, |_| 0)
}
fn reduce_grouped<I: IntoIterator<Item = (usize, [Vec3; 3])>>(
    triangles: impl Fn() -> I,
    budget: usize,
    group: impl Fn(usize) -> u32,
) -> Option<OverviewGeometry> {
    let valid = |t: &[Vec3; 3]| {
        t.iter().all(|v| v.is_finite()) && {
            let area = (t[1] - t[0]).cross(t[2] - t[0]).length_squared();
            area.is_finite() && area > 1e-12
        }
    };
    let mut bounds = OverviewBounds {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };
    let mut count = 0usize;
    for (_, t) in triangles().into_iter().filter(|(_, t)| valid(t)) {
        count += 1;
        for p in t {
            bounds.min = bounds.min.min(p);
            bounds.max = bounds.max.max(p);
        }
    }
    let extent = bounds.max - bounds.min;
    if !extent.is_finite() || !extent.length().is_finite() {
        return None;
    }
    if count <= budget {
        let mut source_triangles = Vec::with_capacity(count);
        let positions = triangles()
            .into_iter()
            .filter(|(_, t)| valid(t))
            .flat_map(|(source, t)| {
                source_triangles.push(source);
                t
            })
            .map(|p| p.to_array())
            .collect();
        return Some(OverviewGeometry {
            positions,
            source_triangles,
            uvs: Vec::new(),
            material_indices: Vec::new(),
            bounds,
        });
    }
    for resolution in [256u32, 128, 64, 32, 16, 8, 4, 2, 1] {
        let step = (extent / resolution as f32).max(Vec3::splat(0.01));
        let mut keys = std::collections::HashSet::new();
        let mut positions = Vec::with_capacity(budget * 3);
        let mut source_triangles = Vec::with_capacity(budget);
        let mut overflow = false;
        for (source_triangle, t) in triangles().into_iter().filter(|(_, t)| valid(t)) {
            let grid = t.map(|p| {
                ((p - bounds.min) / step)
                    .round()
                    .clamp(Vec3::ZERO, Vec3::splat(resolution as f32))
                    .as_uvec3()
            });
            // Nine bits per axis (coordinates 0..=256). Sorted corners make
            // coincident faces share one key regardless of source winding.
            let mut key = grid.map(|p| p.x | (p.y << 9) | (p.z << 18));
            key.sort_unstable();
            let normal = (t[1] - t[0]).cross(t[2] - t[0]);
            let axis = if normal.x.abs() > normal.y.abs() && normal.x.abs() > normal.z.abs() {
                0
            } else if normal.y.abs() > normal.z.abs() {
                1
            } else {
                2
            };
            let direction = axis * 2 + usize::from(normal[axis] < 0.);
            if !keys.insert((key, group(source_triangle), direction)) {
                continue;
            }
            if keys.len() > budget {
                overflow = true;
                break;
            }
            let snapped = grid.map(|p| bounds.min + p.as_vec3() * step);
            // Preserve one real triangle when an isolated small feature collapses;
            // otherwise an entire distant region could disappear from the map.
            let face = if valid(&snapped) { snapped } else { t };
            positions.extend(face.map(|p| p.to_array()));
            source_triangles.push(source_triangle);
        }
        if !overflow {
            return Some(OverviewGeometry {
                positions,
                source_triangles,
                uvs: Vec::new(),
                material_indices: Vec::new(),
                bounds,
            });
        }
    }
    // Material and face-direction boundaries can still exceed a budget.
    // Keep other independently prepared LODs instead of erasing those boundaries.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn triangle(x: f32) -> [Vec3; 3] {
        [
            Vec3::new(x, 2., 0.),
            Vec3::new(x + 1., 3., 0.),
            Vec3::new(x, 2., 1.),
        ]
    }
    #[test]
    fn preserves_distant_geometry_and_height() {
        let result = reduce(|| {
            vec![triangle(-1000.), triangle(4000.)]
                .into_iter()
                .enumerate()
        })
        .unwrap();
        assert_eq!(result.positions.len(), 6);
        assert_eq!(result.bounds.min, Vec3::new(-1000., 2., 0.));
        assert_eq!(result.bounds.max, Vec3::new(4001., 3., 1.));
    }
    #[test]
    fn ignores_invalid_and_degenerate_triangles() {
        assert!(reduce(Vec::new).is_none());
        assert!(
            reduce(|| vec![[Vec3::ZERO; 3], [Vec3::NAN; 3]]
                .into_iter()
                .enumerate())
            .is_none()
        );
        assert_eq!(
            reduce(|| vec![[Vec3::NAN; 3], triangle(5.)].into_iter().enumerate())
                .unwrap()
                .positions
                .len(),
            3
        );
    }
    #[test]
    fn reduction_is_bounded_and_preserves_sparse_regions() {
        let triangles = || (0..300_000).map(|_| triangle(0.)).chain([triangle(10000.)]);
        let result = reduce(|| triangles().enumerate()).unwrap();
        assert!(result.positions.len() <= 262_144 * 3);
        // Reduced vertices may move by half a grid cell. Require a complete
        // nondegenerate face in the distant region, not an exact source vertex.
        let tolerance = (result.bounds.max.x - result.bounds.min.x) / 256. / 2.;
        assert!(result.positions.chunks_exact(3).any(|face| {
            face.iter().all(|p| (p[0] - 10000.).abs() <= tolerance)
                && (Vec3::from_array(face[1]) - Vec3::from_array(face[0]))
                    .cross(Vec3::from_array(face[2]) - Vec3::from_array(face[0]))
                    .length_squared()
                    > 1e-12
        }));
        assert_eq!(
            result.positions,
            reduce(|| triangles().enumerate()).unwrap().positions
        );
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;
    #[test]
    fn invalid_indices_are_skipped_without_panicking() {
        let geometry = skate_data::skate_map::Geometry {
            vertices: vec![],
            indices: vec![0, 1, u32::MAX],
            collision: vec![],
        };
        assert!(from_geometry(&geometry, OVERVIEW_TRIANGLE_BUDGET).is_none());
    }
    #[test]
    fn procedural_world_has_finite_overview() {
        let overview = from_test_world().unwrap();
        assert!(overview.bounds.min.is_finite() && overview.bounds.max.is_finite());
        assert!(!overview.positions.is_empty());
    }
}
fn from_geometry(
    geometry: &skate_data::skate_map::Geometry,
    budget: usize,
) -> Option<OverviewGeometry> {
    let mut overview = reduce_grouped(
        || {
            geometry
                .indices
                .chunks_exact(3)
                .enumerate()
                .filter_map(|(source, indices)| {
                    Some([
                        Vec3::from_array(geometry.vertices.get(indices[0] as usize)?.position),
                        Vec3::from_array(geometry.vertices.get(indices[1] as usize)?.position),
                        Vec3::from_array(geometry.vertices.get(indices[2] as usize)?.position),
                    ])
                    .map(|triangle| (source, triangle))
                })
        },
        budget,
        |source| {
            geometry
                .indices
                .get(source * 3)
                .and_then(|i| geometry.vertices.get(*i as usize))
                .map_or(0, |v| v.material)
        },
    )?;
    for &source in &overview.source_triangles {
        let indices = geometry.indices.get(source * 3..source * 3 + 3)?;
        for &index in indices {
            let vertex = geometry.vertices.get(index as usize)?;
            overview.uvs.push(vertex.uv);
            overview
                .material_indices
                .push(vertex.material.saturating_sub(1));
        }
    }
    Some(overview)
}
pub(super) fn from_map(map: &skate_data::skate_map::SkateMap) -> Option<OverviewGeometry> {
    from_geometry(&map.geometry, OVERVIEW_TRIANGLE_BUDGET)
}
pub(super) fn from_map_detail(map: &skate_data::skate_map::SkateMap) -> Option<OverviewGeometry> {
    from_geometry(&map.geometry, DETAIL_TRIANGLE_BUDGET)
}
pub(super) fn from_test_world() -> Option<OverviewGeometry> {
    reduce(|| {
        {
            crate::physics::ground::surfaces()
                .into_iter()
                .flatten()
                .flat_map(|quad| {
                    [[0, 2, 1], [0, 3, 2]]
                        .map(|indices| indices.map(|i| Vec3::new(quad[i].x, quad[i].y, quad[i].z)))
                })
        }
        .enumerate()
    })
}

#[cfg(test)]
mod coverage_tests {
    use super::*;
    #[test]
    fn dense_flat_map_keeps_its_surface_coverage() {
        let triangles = || {
            (0..600).flat_map(|x| {
                (0..600).flat_map(move |z| {
                    let a = Vec3::new(x as f32, 0., z as f32);
                    [
                        [a, a + Vec3::X, a + Vec3::Z],
                        [a + Vec3::X, a + Vec3::X + Vec3::Z, a + Vec3::Z],
                    ]
                })
            })
        };
        let overview = reduce(|| triangles().enumerate()).unwrap();
        let area: f32 = overview
            .positions
            .chunks_exact(3)
            .map(|t| {
                let a = Vec3::from_array(t[0]);
                let b = Vec3::from_array(t[1]);
                let c = Vec3::from_array(t[2]);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum();
        assert!(
            area >= 600. * 600. * 0.95,
            "large contiguous surfaces must survive reduction: {area}"
        );
        assert!(overview.positions.len() <= 262144 * 3);
    }
}

#[cfg(test)]
mod height_tests {
    use super::*;
    #[test]
    fn wide_map_keeps_stacked_decks_separate() {
        let triangles = || {
            (0..300_000).map(|i| {
                let y = if i % 2 == 0 { 0. } else { 2. };
                [
                    Vec3::new(0., y, 0.),
                    Vec3::new(1000., y, 0.),
                    Vec3::new(0., y, 1000.),
                ]
            })
        };
        let overview = reduce(|| triangles().enumerate()).unwrap();
        assert!(overview.positions.iter().any(|p| p[1] == 0.));
        assert!(overview.positions.iter().any(|p| p[1] == 2.));
        assert_eq!(overview.positions.len(), 6);
    }
}

#[cfg(test)]
mod detail_tests {
    use super::*;
    #[test]
    fn detail_lod_preserves_downtown_sized_source_and_uvs() {
        let mut map = skate_data::skate_map::SkateMap::parse(include_bytes!(
            "../../../../maps/format-demo.skate"
        ))
        .unwrap();
        let count = 1_888_894;
        let face = map.geometry.indices[..3].to_vec();
        map.geometry.indices = face.repeat(count);
        let detail = from_map_detail(&map).unwrap();
        assert_eq!(detail.source_triangles.len(), count);
        assert_eq!(detail.uvs.len(), count * 3);
        assert_eq!(detail.material_indices.len(), count * 3);
    }
}

#[cfg(test)]
mod material_boundary_tests {
    use super::*;
    #[test]
    fn coarse_lod_keeps_coincident_material_boundaries() {
        let mut map = skate_data::skate_map::SkateMap::parse(include_bytes!(
            "../../../../maps/format-demo.skate"
        ))
        .unwrap();
        let face: Vec<_> = map.geometry.indices[..3]
            .iter()
            .map(|i| map.geometry.vertices[*i as usize].clone())
            .collect();
        map.geometry.vertices = face.iter().chain(face.iter()).cloned().collect();
        for (i, v) in map.geometry.vertices.iter_mut().enumerate() {
            v.material = if i < 3 { 1 } else { 2 };
        }
        map.geometry.indices = [0, 1, 2, 3, 4, 5].repeat(140_000);
        let overview = from_map(&map).unwrap();
        assert!(overview.material_indices.contains(&0));
        assert!(
            overview.material_indices.contains(&1),
            "distinct albedos must survive clustering"
        );
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;
    #[test]
    fn failed_overview_keeps_usable_intermediate_and_detail() {
        let mut map = skate_data::skate_map::SkateMap::parse(include_bytes!(
            "../../../../maps/format-demo.skate"
        ))
        .unwrap();
        let face: Vec<_> = map.geometry.indices[..3]
            .iter()
            .map(|i| map.geometry.vertices[*i as usize].clone())
            .collect();
        map.geometry.vertices.clear();
        map.geometry.indices.clear();
        // The geometry builder uses material identities, independent of their
        // separately loaded material records. Give every face a distinct identity.
        for i in 0..262_145 {
            for (j, v) in face.iter().enumerate() {
                let mut v = v.clone();
                v.material = i + 1;
                map.geometry.vertices.push(v);
                map.geometry.indices.push(i * 3 + j as u32);
            }
        }
        assert!(from_map(&map).is_none());
        assert!(
            build_lods(&map).is_some(),
            "one failed LOD must not discard valid representations"
        );
    }
}
