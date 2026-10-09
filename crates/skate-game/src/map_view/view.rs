use super::geometry::OverviewBounds;
use bevy::prelude::*;
#[derive(Clone)]
pub(super) struct ViewFrame {
    pub transform: Transform,
    pub projection: PerspectiveProjection,
}
#[cfg(test)]
pub(super) fn frame(
    bounds: OverviewBounds,
    focus: Vec3,
    expanded: bool,
    aspect: f32,
    zoom: f32,
) -> ViewFrame {
    frame_orbit(
        bounds,
        focus,
        expanded,
        aspect,
        zoom,
        0.,
        50_f32.to_radians(),
    )
}

pub(super) fn frame_orbit(
    bounds: OverviewBounds,
    focus: Vec3,
    expanded: bool,
    aspect: f32,
    zoom: f32,
    yaw: f32,
    pitch: f32,
) -> ViewFrame {
    if !expanded {
        // A nearby wide lens preserves perspective and local height. Fitting
        // the entire city and zooming its lens made compact mode almost flat.
        let focus = if focus.is_finite() {
            focus
        } else {
            bounds.min * 0.5 + bounds.max * 0.5
        };
        let yaw = if yaw.is_finite() { yaw } else { 0. };
        let pitch = if pitch.is_finite() {
            pitch.clamp(55_f32.to_radians(), 80_f32.to_radians())
        } else {
            65_f32.to_radians()
        };
        let rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-pitch);
        return ViewFrame {
            transform: Transform::from_translation(focus + rotation * Vec3::Z * 100.)
                .with_rotation(rotation),
            projection: PerspectiveProjection {
                fov: 55_f32.to_radians(),
                aspect_ratio: if aspect.is_finite() && aspect > 0. {
                    aspect
                } else {
                    4. / 3.
                },
                near: 0.1,
                far: 600.,
                ..default()
            },
        };
    }
    build_frame(
        bounds, focus, expanded, aspect, zoom, yaw, pitch, false, false,
    )
}
pub(super) fn frame_free(
    bounds: OverviewBounds,
    focus: Vec3,
    aspect: f32,
    zoom: f32,
    yaw: f32,
    pitch: f32,
) -> ViewFrame {
    build_frame(bounds, focus, true, aspect, zoom, yaw, pitch, true, false)
}
pub(super) fn frame_interior(
    bounds: OverviewBounds,
    focus: Vec3,
    aspect: f32,
    zoom: f32,
    yaw: f32,
    pitch: f32,
) -> ViewFrame {
    build_frame(bounds, focus, true, aspect, zoom, yaw, pitch, true, true)
}
fn build_frame(
    bounds: OverviewBounds,
    focus: Vec3,
    expanded: bool,
    aspect: f32,
    zoom: f32,
    yaw: f32,
    pitch: f32,
    free: bool,
    interior: bool,
) -> ViewFrame {
    let center = bounds.min * 0.5 + bounds.max * 0.5;
    let zoom = if zoom.is_finite() {
        zoom.clamp(1., 64.)
    } else {
        1.
    };
    let aspect = if aspect.is_finite() && aspect > 0. {
        aspect
    } else {
        4. / 3.
    };
    let focus = if (expanded && zoom <= 1. && !free) || !focus.is_finite() {
        center
    } else {
        focus
    };
    let pitch = pitch.clamp(25_f32.to_radians(), std::f32::consts::FRAC_PI_2);
    let direction = Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    );
    let rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-pitch);
    let tan_vertical = (35_f32.to_radians() / 2.).tan();
    let mut distance = 0_f32;
    for x in [bounds.min.x, bounds.max.x] {
        for y in [bounds.min.y, bounds.max.y] {
            for z in [bounds.min.z, bounds.max.z] {
                let p = rotation.inverse() * (Vec3::new(x, y, z) - center);
                distance = distance
                    .max(p.z + (p.x.abs() / (tan_vertical * aspect)).max(p.y.abs() / tan_vertical));
            }
        }
    }
    let radius = (bounds.max - bounds.min)
        .length()
        .max(if interior { 1. } else { 120. });
    // Zoom the lens rather than entering the buildings: all terrain stays in front
    // of the camera, including high towers when focusing on a street-level player.
    distance = (distance * 1.1).max(radius + if interior { 0.5 } else { 100. });
    let tan_fov = if expanded {
        tan_vertical / zoom
    } else {
        60. / aspect / distance
    };
    ViewFrame {
        transform: Transform::from_translation(focus + direction * distance)
            .with_rotation(rotation),
        projection: PerspectiveProjection {
            fov: 2. * tan_fov.atan(),
            aspect_ratio: aspect,
            near: 0.1,
            far: distance + radius * 2. + 100.,
            ..default()
        },
    }
}

impl ViewFrame {
    pub fn ground_point(&self, uv: Vec2, height: f32) -> Option<Vec3> {
        use bevy::camera::CameraProjection;
        let p = self
            .projection
            .get_clip_from_view()
            .inverse()
            .project_point3(Vec3::new(uv.x * 2. - 1., 1. - uv.y * 2., 1.));
        let ray = self.transform.rotation * p.normalize();
        if !ray.is_finite() || ray.y.abs() < 1e-6 {
            return None;
        }
        let t = (height - self.transform.translation.y) / ray.y;
        (t.is_finite() && t > 0.).then_some(self.transform.translation + ray * t)
    }
}

pub(super) fn project_unclipped(frame: &ViewFrame, point: Vec3) -> Option<Vec2> {
    use bevy::camera::CameraProjection;
    if !point.is_finite() {
        return None;
    }
    let ndc = (frame.projection.get_clip_from_view() * frame.transform.to_matrix().inverse())
        .project_point3(point);
    if !ndc.is_finite() || !(0. ..=1.).contains(&ndc.z) {
        return None;
    }
    Some(Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5))
}
pub(super) fn project(frame: &ViewFrame, point: Vec3) -> Option<Vec2> {
    let p = project_unclipped(frame, point)?;
    ((0. ..=1.).contains(&p.x) && (0. ..=1.).contains(&p.y)).then_some(p)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_camera_uses_local_perspective_independent_of_city_size() {
        let focus = Vec3::new(25., 8., -30.);
        for extent in [100., 10000.] {
            let f = frame_orbit(
                OverviewBounds {
                    min: Vec3::splat(-extent),
                    max: Vec3::splat(extent),
                },
                focus,
                false,
                4. / 3.,
                1.,
                1.,
                65_f32.to_radians(),
            );
            assert!(f.transform.translation.distance(focus) < 200.);
            assert!(f.projection.fov > 30_f32.to_radians());
            let ahead = focus + Vec3::new(-1_f32.sin(), 0., -1_f32.cos()) * 15.;
            let uv = project(&f, ahead).unwrap();
            assert!((uv.x - 0.5).abs() < 0.001 && uv.y < 0.5);
        }
    }
    #[test]
    fn miniature_has_perspective_depth() {
        use bevy::camera::CameraProjection;
        let f = frame(
            OverviewBounds {
                min: Vec3::splat(-100.),
                max: Vec3::splat(100.),
            },
            Vec3::ZERO,
            true,
            4. / 3.,
            1.,
        );
        assert_eq!(f.projection.get_clip_from_view().w_axis.w, 0.);
    }
    #[test]
    fn orbit_keeps_whole_world_fitted_and_player_overlay_centered() {
        let bounds = OverviewBounds {
            min: Vec3::new(-1000., -30., -300.),
            max: Vec3::new(1000., 400., 500.),
        };
        for yaw in [0., 1., 3., 5.] {
            for pitch in [25_f32.to_radians(), 80_f32.to_radians()] {
                let f = frame_orbit(bounds, Vec3::ZERO, true, 4. / 3., 1., yaw, pitch);
                for x in [bounds.min.x, bounds.max.x] {
                    for y in [bounds.min.y, bounds.max.y] {
                        for z in [bounds.min.z, bounds.max.z] {
                            assert!(project(&f, Vec3::new(x, y, z)).is_some());
                        }
                    }
                }
                let zoom = frame_orbit(bounds, Vec3::ZERO, true, 4. / 3., 16., yaw, pitch);
                assert!(
                    project(&zoom, Vec3::ZERO)
                        .unwrap()
                        .distance(Vec2::splat(0.5))
                        < 0.001
                );
            }
        }
    }
    #[test]
    fn expanded_frames_every_corner_with_padding() {
        let bounds = OverviewBounds {
            min: Vec3::new(-1200., -50., 800.),
            max: Vec3::new(100., 350., 1300.),
        };
        for aspect in [0.5, 4. / 3., 3.] {
            let f = frame(bounds, Vec3::ZERO, true, aspect, 1.);
            for x in [bounds.min.x, bounds.max.x] {
                for y in [bounds.min.y, bounds.max.y] {
                    for z in [bounds.min.z, bounds.max.z] {
                        let uv = project(&f, Vec3::new(x, y, z)).unwrap();
                        assert!(uv.x > 0. && uv.x < 1. && uv.y > 0. && uv.y < 1., "{uv:?}");
                    }
                }
            }
        }
    }
    #[test]
    fn follows_player_and_projects_height_and_clips() {
        let p = Vec3::new(1200., 30., -800.);
        let f = frame(
            OverviewBounds {
                min: p - Vec3::splat(20.),
                max: p + Vec3::splat(20.),
            },
            p,
            false,
            4. / 3.,
            1.,
        );
        assert!(project(&f, p).unwrap().distance(Vec2::splat(0.5)) < 0.001);
        assert!(project(&f, p + Vec3::Y * 10.).unwrap().y < 0.5);
        assert!(project(&f, p + Vec3::X * 10000.).is_none());
        assert!(project(&f, Vec3::NAN).is_none());
        let flat = frame(OverviewBounds { min: p, max: p }, p, true, 4. / 3., 1.);
        assert!(project(&flat, p).is_some());
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    #[test]
    fn top_down_has_stable_north_and_ground_unprojection() {
        let b = OverviewBounds {
            min: Vec3::splat(-100.),
            max: Vec3::splat(100.),
        };
        let f = frame_orbit(
            b,
            Vec3::ZERO,
            true,
            4. / 3.,
            4.,
            0.,
            std::f32::consts::FRAC_PI_2,
        );
        assert!(f.transform.forward().as_vec3().distance(-Vec3::Y) < 0.001);
        let p = Vec3::new(15., 0., -10.);
        let uv = project(&f, p).unwrap();
        assert!(f.ground_point(uv, 0.).unwrap().distance(p) < 0.01);
    }
    #[test]
    fn free_center_is_respected_at_whole_map_zoom() {
        let b = OverviewBounds {
            min: Vec3::splat(-100.),
            max: Vec3::splat(100.),
        };
        let center = Vec3::new(50., 0., 30.);
        let f = frame_free(b, center, 4. / 3., 1., 0., 50_f32.to_radians());
        assert!(project(&f, center).unwrap().distance(Vec2::splat(0.5)) < 0.001);
    }
}
