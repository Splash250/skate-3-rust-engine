//! Actual native adapters on a synthetic flat world using locally owned stock
//! animation/physics collections. No renderer, network socket or mod runtime.
use super::*;
use crate::physics::{PlayerControls, frame};
use skate_core::{input::xbox::XboxState, player::state::PhysicalStateId};
use skate_net::dedicated::{Effect, EffectKind};

struct Fixture {
    physics: GamePhysics,
    skater: SkaterRuntime,
    controls: PlayerControls,
    input: crate::input::ControllerInput,
    camera: crate::camera::CameraRuntime,
    graphs: crate::graph_runtime::StockGraphs,
}
impl Fixture {
    fn load() -> Self {
        let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
        let root = std::path::Path::new(&root);
        let assets = skate_data::GameAssets::load(root).unwrap();
        let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
        let mut physics = GamePhysics::load(root).unwrap();
        physics.network_active = true;
        let skater =
            SkaterRuntime::load(root, &graphs, &physics, crate::physics::PHYSICS_MODE).unwrap();
        Self {
            physics,
            skater,
            graphs,
            controls: PlayerControls::default(),
            input: crate::input::ControllerInput::default(),
            camera: crate::camera::CameraRuntime::load(root).unwrap(),
        }
    }
    fn tick(&mut self, buttons: u16, effect: Option<&Effect>) {
        self.input.sample_raw_for_test(XboxState {
            buttons,
            triggers: [0; 2],
            left: [0; 2],
            right: [0; 2],
        });
        let mut actions = self.input.player_actions();
        self.controls
            .update_for_physics(&mut actions, &self.physics, &self.skater, &self.camera)
            .unwrap();
        if let Some(effect) = effect {
            apply_server_effect(&mut self.physics, &mut self.skater, effect).unwrap();
        }
        frame::advance(
            &mut self.physics,
            &mut self.skater,
            &mut self.controls,
            &self.graphs,
            &mut actions,
            true,
            &mut self.camera,
        )
        .unwrap();
        assert!(
            self.skater
                .render_pose
                .iter()
                .flatten()
                .flatten()
                .all(|v| v.is_finite())
        );
        assert_eq!(self.skater.pose_generation, self.physics.ticks);
    }
    fn velocities(&self) -> Vec<[f32; 3]> {
        self.physics
            .board
            .bodies()
            .iter()
            .chain(self.skater.skeleton.bodies())
            .map(|body| xyz(body.rates.linear_velocity))
            .collect()
    }
}
fn effect(kind: EffectKind, delta_velocity: [f32; 3], position: [f32; 3]) -> Effect {
    Effect {
        id: 1,
        source: 2,
        target: 1,
        kind,
        delta_velocity,
        position,
    }
}

#[test]
#[ignore = "requires locally owned stock animation banks and collections via SKATE3_ASSET_ROOT"]
fn dedicated_shove_enters_native_ragdoll_and_applies_impulse_once() {
    let mut f = Fixture::load();
    for _ in 0..90 {
        f.tick(0, None);
    }
    assert_eq!(
        f.skater.player_state.current(),
        PhysicalStateId::PhysicsGround
    );
    let delta = [2., 1.5, -1.];
    apply_server_effect(
        &mut f.physics,
        &mut f.skater,
        &effect(EffectKind::Shove, delta, [0.; 3]),
    )
    .unwrap();
    assert_eq!(
        f.skater.player_state.current(),
        PhysicalStateId::WipeoutGround
    );
    assert!(f.skater.skeleton_collision.is_ragdoll);
    assert_eq!(f.physics.board.collision_group(), 7);
    assert_eq!(
        f.physics.board.hook().drive.dynamics,
        [0, 0, 0, 2, 0, 0, 0, 2]
    );
    prepare_server_movement(&mut f.physics, &mut f.skater);
    assert_eq!(f.physics.network_delta_velocity, delta);
    let before = f.velocities();
    apply_server_velocity(&mut f.physics, &mut f.skater);
    let after = f.velocities();
    for (old, new) in before.iter().zip(&after) {
        for axis in 0..3 {
            assert!((new[axis] - old[axis] - delta[axis]).abs() < 0.00001);
        }
    }
    assert_eq!(f.physics.network_delta_velocity, [0.; 3]);
    apply_server_velocity(&mut f.physics, &mut f.skater);
    assert_eq!(
        f.velocities(),
        after,
        "The next solve must not repeat an accepted impulse"
    );
    f.tick(0, None);
    assert_eq!(
        f.skater.player_state.current(),
        PhysicalStateId::WipeoutGround,
        "Native state selection must preserve the server-accepted wipeout"
    );
    for _ in 0..30 {
        f.tick(0, None);
    }
}

#[test]
#[ignore = "requires locally owned stock animation banks and collections via SKATE3_ASSET_ROOT"]
fn dedicated_walking_collision_has_one_owner_and_accepted_attack_plays_stock_shove() {
    let mut f = Fixture::load();
    for tick in 0..200 {
        f.tick(if tick == 20 { 0x8000 } else { 0 }, None);
    }
    assert_eq!(
        f.skater.player_state.current(),
        PhysicalStateId::BipedGround
    );
    let old = f.skater.biped_ground.controller.state.sliding.velocity_528;
    apply_server_effect(
        &mut f.physics,
        &mut f.skater,
        &effect(EffectKind::Collision, [1., 0., 0.], [0.; 3]),
    )
    .unwrap();
    prepare_server_movement(&mut f.physics, &mut f.skater);
    let sliding = f.skater.biped_ground.controller.state.sliding.velocity_528;
    assert!((sliding[0] - old[0] - 1.).abs() < 0.00001);
    assert_eq!(
        f.physics.network_delta_velocity, [0.; 3],
        "Walking consumed the impulse; the rigid body solver must not apply it again"
    );
    prepare_server_movement(&mut f.physics, &mut f.skater);
    assert_eq!(
        f.skater.biped_ground.controller.state.sliding.velocity_528,
        sliding
    );
    f.tick(0, None);
    let root = matrix(capture_body(&f.physics, &f.skater).root);
    let target = root.transform_point3(Vec3::new(0., 0., 1.2)).to_array();
    // Simulate a quick RB tap and a delayed server response after release.
    // No attack animation may start from the unconfirmed local input alone.
    f.tick(0x0200, None);
    for _ in 0..10 {
        f.tick(0, None);
    }
    assert!(!f.skater.animation.motion.animation.channels.has("Shove"));
    f.tick(
        0,
        Some(&effect(EffectKind::AttackAccepted, [0.; 3], target)),
    );
    assert!(
        f.skater.animation.motion.animation.channels.has("Shove"),
        "An accepted dedicated attack must start the recovered Shove animation channel"
    );
    for _ in 0..30 {
        f.tick(0, None);
    }
}
