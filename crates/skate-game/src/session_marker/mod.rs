//! Manual session markers. Automatic bail recovery owns a different checkpoint.
mod effect;
mod hud;
mod noise;
mod state;
mod validation;
use crate::{
    app::SimulationSet,
    input::ControllerInput,
    map_transition::CurrentMap,
    physics::{GamePhysics, SkaterRuntime},
};
use bevy::prelude::*;
use skate_core::physics::{board::BodyId, skeleton_animation_record::AnimationPartTransform};

#[derive(Clone, Copy, Debug)]
struct Marker {
    transform: AnimationPartTransform,
    on_board: bool,
    foot_forward: bool,
    generation: u64,
}

#[derive(Resource, Default)]
pub(crate) struct SessionMarker {
    marker: Option<Marker>,
    hold: state::Hold,
    generation: u64,
    pub visible: bool,
    pub can_place: bool,
    pub can_return: bool,
    pub progress: f32,
    blocked_until_release: bool,
    ui_time: f64,
    last_batch: u64,
    /// The modifier (LB) as last seen: its press opens the marker menu (retail's cellphone
    /// activate sound, `ui_audio::SessionMarkerAction::Opened`).
    modifier: bool,
}

pub(crate) struct SessionMarkerPlugin;
impl Plugin for SessionMarkerPlugin {
    fn build(&self, app: &mut App) {
        let root = &app.world().resource::<crate::config::Config>().asset_root;
        match validation::Validation::load(root) {
            Ok(settings) => {
                app.insert_resource(settings);
            }
            Err(e) => {
                error!("Session marker validation data: {e}");
                return;
            }
        }
        app.init_resource::<SessionMarker>()
            .add_systems(
                PreUpdate,
                suspend.after(crate::map_transition::MapTransitionSet),
            )
            .add_systems(
                FixedUpdate,
                update
                    .after(SimulationSet::Input)
                    .before(SimulationSet::Controls)
                    .run_if(crate::graphics_menu::gameplay_active)
                    .run_if(crate::multiplayer::native_authority::ordinary),
            )
            .add_systems(Update, publish_teleport_effect.before(crate::ui_audio::UiAudioSet));
        hud::install(app);
        effect::install(app);
    }
}

fn suspend(
    mut marker: ResMut<SessionMarker>,
    map: Res<CurrentMap>,
    menu: Res<crate::graphics_menu::Menu>,
    replay: Res<crate::replay::Replay>,
    time: Res<Time<Real>>,
    native: Option<Res<crate::multiplayer::native_authority::Prediction>>,
) {
    if marker.generation != map.generation {
        *marker = SessionMarker {
            generation: map.generation,
            blocked_until_release: true,
            ..default()
        };
    }
    if !crate::graphics_menu::gameplay_active(Some(menu))
        || replay.active
        || !crate::multiplayer::native_authority::ordinary(native)
    {
        marker.hold.cancel();
        marker.visible = false;
        marker.progress = 0.;
        marker.blocked_until_release = true;
        marker.ui_time = 0.;
    } else if !marker.blocked_until_release {
        // PlayerUI runs on the UI clock, independently of physics substeps.
        marker.ui_time += time.delta_secs_f64();
    }
}

/// The hold's teleport effect amount for the frame's readers (`ui_audio::TeleportEffect`: the screen
/// static and the skater's Class_Treatment). PlayerUI sends `cMsgTeleportEffectAmount` on each UI tick
/// of the hold and on the relocation tick and two more (progress 1); `progress` is 0 on every other
/// tick, and the last tick's value holds until the next.
fn publish_teleport_effect(session: Res<SessionMarker>, effect: Option<ResMut<crate::ui_audio::TeleportEffect>>) {
    let Some(mut effect) = effect else { return };
    let amount = (session.progress > 0.).then_some(session.progress);
    if effect.engine != amount {
        effect.engine = amount;
    }
}

fn update(
    mut session: ResMut<SessionMarker>,
    input: Res<ControllerInput>,
    map: Res<CurrentMap>,
    physics: Res<GamePhysics>,
    mut skater: ResMut<SkaterRuntime>,
    validation: Res<validation::Validation>,
    replay: Res<crate::replay::Replay>,
    mut events: MessageWriter<crate::ui_audio::SessionMarkerEvent>,
) {
    use crate::ui_audio::{SessionMarkerAction, SessionMarkerEvent};
    if replay.active {
        return;
    }
    let (modifier, set, held) = input.session_marker_actions();
    let pressed = modifier && !session.modifier;
    session.modifier = modifier;
    if session.blocked_until_release {
        if !modifier {
            session.blocked_until_release = false;
        }
        return;
    }
    // The cellphone UI (sub_826682B0): LB opens the menu with its activate sound.
    if pressed {
        events.write(SessionMarkerEvent { action: SessionMarkerAction::Opened });
    }
    let p = &skater.player_input.physical;
    let processed = &skater.player_input.processed;
    let on_board = p.state.category_12 != 500;
    let deck = physics.board.part_transforms()[BodyId::Deck.index()];
    let mut transform = skater.animated_skeleton.roots.animation_to_world;
    if on_board {
        for i in 0..3 {
            transform[i][..3].copy_from_slice(&deck.basis.columns[i]);
        }
        transform[3] = [
            deck.translation.x,
            deck.translation.y + 0.2,
            deck.translation.z,
            0.,
        ];
    }
    transform = crate::physics::facing_from_visual(
        transform,
        processed.flags_2468,
        processed.flags_2476,
        on_board,
    );
    let state = p.state.state_16;
    let state_allowed = (p.state.category_12 == 100
        && p.collision.wheel_count_0 >= 2
        && state != 104
        && deck.basis.columns[1][1] > 0.71)
        || (p.state.category_12 == 500 && state == 500);
    session.can_place = modifier
        && state_allowed
        && p.surface_default_mode != 8
        && validation.check(physics.world(), transform[3]);
    session.can_return = session
        .marker
        .is_some_and(|m| m.generation == map.generation)
        && !matches!(state, 104 | 502);
    session.visible = modifier;
    // A retained Pad publication must not turn one .pressed into repeated sets.
    if set && session.last_batch != input.consumed_batches {
        // sub_82898FC8: every Place Marker press sounds, the place sound when the marker went
        // down, the error sound when it could not.
        if session.can_place {
            session.marker = Some(Marker {
                transform,
                on_board,
                foot_forward: skater.animation.foot_forward(),
                generation: map.generation,
            });
            events.write(SessionMarkerEvent { action: SessionMarkerAction::Placed });
        } else {
            events.write(SessionMarkerEvent { action: SessionMarkerAction::Refused });
        }
    }
    session.last_batch = input.consumed_batches;
    let ready = p.state.flag_69 == 0
        && state != 702
        && !(physics.board_wiping_out && p.skeleton.teleport_pending_604 != 0)
        && skater.player_input.pending_teleport().is_none();
    let distance = session.marker.map_or(0., |m| {
        // 82DB6EC0 -> 82BE1AE8 publishes animation-to-world at output+368;
        // UpdateSessionMarker reads its translation at output+416.
        Vec3::from_slice(&skater.animated_skeleton.roots.animation_to_world[3][..3])
            .distance(Vec3::from_slice(&m.transform[3][..3]))
    });
    let usable = session.can_return;
    while session.ui_time >= 1. / 60. {
        session.ui_time -= 1. / 60.;
        let step = session.hold.update(held, usable, distance, ready);
        session.progress = step.progress;
        if step.relocate {
            if let Some(target) = session.marker {
                match skater.player_input.request_teleport(target.transform) {
                    Ok(()) => {
                        // sub_82898FC8: the go-to sound in the relocation tick.
                        events.write(SessionMarkerEvent { action: SessionMarkerAction::Returned });
                        skater.animation.restore_foot_forward(target.foot_forward);
                        skater
                            .teleport_state
                            .request_manual(target.transform, target.on_board);
                    }
                    Err(e) => {
                        warn!("Session marker return rejected: {e}");
                        session.hold.cancel();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn authority_cancels_a_pending_marker_hold_until_controls_are_released() {
        let mut world = World::new();
        let mut marker = SessionMarker {
            visible: true,
            progress: 0.9,
            ui_time: 0.1,
            ..default()
        };
        // The next ordinary UI tick would fire this previously held return.
        for _ in 0..12 {
            assert!(!marker.hold.update(true, true, 20., true).relocate);
        }
        let mut menu = crate::graphics_menu::Menu::transition_test_menu();
        menu.open = false;
        world.insert_resource(marker);
        world.insert_resource(menu);
        world.insert_resource(CurrentMap {
            path: None,
            name: "test".into(),
            spawn: [0.; 3],
            heading: 0.,
            generation: 0,
        });
        world.insert_resource(crate::replay::Replay::default());
        world.insert_resource(Time::<Real>::default());
        world.insert_resource(crate::multiplayer::native_authority::active_prediction());
        world.run_system_once(suspend).unwrap();
        let mut marker = world.resource_mut::<SessionMarker>();
        assert!(!marker.visible);
        assert_eq!(marker.progress, 0.);
        assert_eq!(marker.ui_time, 0.);
        assert!(marker.blocked_until_release);
        assert!(!marker.hold.update(true, true, 20., true).relocate);
        world.remove_resource::<crate::multiplayer::native_authority::Prediction>();
        world.run_system_once(suspend).unwrap();
        assert!(world.resource::<SessionMarker>().blocked_until_release);
    }
}
