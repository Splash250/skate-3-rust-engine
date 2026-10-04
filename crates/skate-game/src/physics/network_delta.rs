//! Choose one local movement owner for each accepted server velocity change.
pub(crate) fn route_to_ground(ground: bool, sliding: &mut [f32; 4], pending: &mut [f32; 3]) {
    if ground {
        for (velocity, delta) in sliding[..3]
            .iter_mut()
            .zip(std::mem::replace(pending, [0.; 3]))
        {
            *velocity += delta;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ground_consumes_delta_once_so_rigid_body_solve_cannot_apply_it_again() {
        let mut sliding = [0.5, 0., 0., 0.];
        let mut pending = [2., 0., -1.];
        route_to_ground(true, &mut sliding, &mut pending);
        assert_eq!(sliding, [2.5, 0., -1., 0.]);
        assert_eq!(pending, [0.; 3]);
        route_to_ground(true, &mut sliding, &mut pending);
        assert_eq!(sliding, [2.5, 0., -1., 0.]);
    }
    #[test]
    fn ragdoll_or_skating_keeps_delta_for_the_physical_solver() {
        let mut sliding = [0.; 4];
        let mut pending = [2., 1.5, -1.];
        route_to_ground(false, &mut sliding, &mut pending);
        assert_eq!(pending, [2., 1.5, -1.]);
        assert_eq!(sliding, [0.; 4]);
    }
}
