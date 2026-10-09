//! Markers are presentation-only cylinders tested against actual native shapes.
use skate_core::physics::world_contact::ContactPrimitive;
use skate_resources::locations::MarkerStyle;
pub fn intersect_marker(
    volumes: &[ContactPrimitive],
    position: [f32; 3],
    style: &MarkerStyle,
) -> bool {
    use skate_dynamics::rapier3d::{
        parry::query::intersection_test,
        prelude::{Pose, SharedShape, Vector},
    };
    if style.validate().is_err() || position.iter().any(|p| !p.is_finite()) {
        return false;
    }
    let cylinder = SharedShape::cylinder(style.height / 2., style.radius);
    let pose = Pose::from_translation(Vector::new(
        position[0],
        position[1] + style.height / 2.,
        position[2],
    ));
    volumes.iter().any(|p| {
        crate::physics::solid_contacts::shape(*p).is_some_and(|(shape, at)| {
            intersection_test(&at, &*shape, &pose, &*cylinder).unwrap_or(false)
        })
    })
}
pub fn nearest<'a>(
    center: [f32; 3],
    markers: impl IntoIterator<Item = (&'a str, [f32; 3])>,
) -> Option<&'a str> {
    markers
        .into_iter()
        .filter(|(_, p)| p.iter().all(|v| v.is_finite()))
        .min_by(|(ak, a), (bk, b)| {
            let distance = |p: [f32; 3]| {
                p.iter()
                    .zip(center)
                    .map(|(v, c)| (*v as f64 - c as f64).powi(2))
                    .sum::<f64>()
            };
            distance(*a)
                .total_cmp(&distance(*b))
                .then_with(|| ak.cmp(bk))
        })
        .map(|(key, _)| key)
}
#[cfg(test)]
mod tests {
    use super::*;
    use skate_core::{
        math::Vector3,
        physics::{collision::Sphere, world_contact::ContactPrimitive},
    };
    #[test]
    fn intersection_uses_physical_volume_not_root_point() {
        let style = skate_resources::locations::MarkerStyle::default();
        let volume = ContactPrimitive::Sphere(Sphere {
            center: Vector3::new(1.3, 1., 0.),
            radius: 0.4,
        });
        assert!(intersect_marker(&[volume], [0., 0., 0.], &style));
        assert!(!intersect_marker(&[volume], [0., 4., 0.], &style));
    }
    #[test]
    fn nearest_overlap_is_stable() {
        assert_eq!(
            nearest([0., 0., 0.], [("b", [1., 0., 0.]), ("a", [-1., 0., 0.])]),
            Some("a")
        );
    }
}
