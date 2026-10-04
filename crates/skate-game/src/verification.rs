//! Opt-in startup verification, with no in-game tool UI.
use crate::{
    animation::AnimationStatus,
    app::FrameSet,
    config::Config,
    input::ControllerInput,
    physics::{GamePhysics, PlayerControls, SkaterRuntime},
};
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
};

#[derive(Resource, Default)]
struct Verification {
    elapsed: f32,
    requested: bool,
    captured: bool,
    grind_frames: u64,
    max_network_contacts: usize,
    periodic_captures: u8,
    last_contact_tick: Option<(u64, u64)>,
    contact_frames: std::collections::BTreeMap<u64, u64>,
}

pub(crate) struct VerificationPlugin;
impl Plugin for VerificationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Verification>()
            .add_systems(Update, verify.in_set(FrameSet::Verification));
    }
}
fn verify(
    mut commands: Commands,
    time: Res<Time<Real>>,
    config: Res<Config>,
    animation: Res<AnimationStatus>,
    input: Res<ControllerInput>,
    physics: Res<GamePhysics>,
    controls: Res<PlayerControls>,
    skater: Res<SkaterRuntime>,
    camera: Res<crate::camera::CameraRuntime>,
    mut replay: ResMut<crate::replay::Replay>,
    mut state: ResMut<Verification>,
    mut exit: MessageWriter<AppExit>,
    current_map: Res<crate::map_transition::CurrentMap>,
    multiplayer: Res<crate::multiplayer::Multiplayer>,
    shared: Res<crate::multiplayer::entities::SharedObjects>,
    runtime_status: (Res<crate::modding::animation::State>, Res<Time<Virtual>>, Option<Res<crate::graphics_menu::Menu>>),
) {
    let Some(path) = &config.verification_capture else {
        return;
    };
    state.elapsed += time.delta_secs();
    state.grind_frames += u64::from(skater.grind.active_name().is_some());
    state.max_network_contacts = state.max_network_contacts.max(physics.network_contacts);
    let contact_tick = (current_map.generation, physics.ticks);
    if state.last_contact_tick != Some(contact_tick) {
        state.last_contact_tick = Some(contact_tick);
        for &id in &physics.network_contact_ids {
            if state.contact_frames.contains_key(&id) || state.contact_frames.len() < 1024 {
                *state.contact_frames.entry(id).or_default() += 1;
            }
        }
    }
    let capture_after = std::env::var("SKATE_VERIFY_SECONDS").ok()
        .and_then(|s| s.parse::<f32>().ok()).filter(|n| n.is_finite())
        .unwrap_or(4.).clamp(4., 40.);
    // Opt-in visual smoke check of the replay HUD and presentation endpoints.
    if animation.ready && state.elapsed > 2.0 && !replay.active
        && std::env::var("SKATE_VERIFY_REPLAY").as_deref() == Ok("1") {
        replay.enter();
    }
    // SKATE_VERIFY_AT: capture time in seconds (default 4), e.g. to catch an effect.
    let capture_at = std::env::var("SKATE_VERIFY_AT").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(capture_after);
    let first_capture_due = animation.ready && state.elapsed > capture_at && !state.requested;
    if first_capture_due {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()))
            .observe(
                |_: On<ScreenshotCaptured>, mut state: ResMut<Verification>| {
                    state.captured = true;
                },
            );
        state.requested = true;
    }
    // The same opt-in verification path can capture bounded lifecycle phases.
    let interval = std::env::var("SKATE_VERIFY_INTERVAL_SECONDS").ok()
        .and_then(|s| s.parse::<f32>().ok()).filter(|n| n.is_finite() && *n >= 4.);
    let periodic_due = animation.ready && !first_capture_due && state.periodic_captures < 8
        && interval.is_some_and(|seconds| state.elapsed >= seconds * (state.periodic_captures + 1) as f32);
    let finished = state.captured && state.elapsed > capture_after + 2.;
    if periodic_due || finished {
        let report_path = if periodic_due {
            state.periodic_captures += 1;
            let capture = path.with_extension(format!("phase{:02}.png", state.periodic_captures));
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(capture.clone()));
            info!("GAME_VERIFY_PHASE {}", state.periodic_captures);
            capture
        } else { path.clone() };
        let input_report = format!(
            "Game integration capture (not a Skate 3 parity verdict)\nExecutable: {}\nPolls: {}\nConsumed batches: {}\nStatus: {:?}\nPacket numbers: {:?}\nActions 64..81 by device: {:?}\nDerived ticks: {}\nIntents: {:?}\nDifficulty index: {}\nPhysics ticks: {}\nContacts: {}\nBody positions: {:?}\nPhysical pose publications: {}\nGraph animation ticks: {}\nTruck targets: {:?}\nGround speed: {}\nGameplay camera shot: {}\nCamera frame: {:?}\n",
            std::env::current_exe()
                .map_or_else(|_| "unavailable".into(), |p| p.display().to_string()),
            input.publications,
            input.consumed_batches,
            input.status,
            input.packet_numbers,
            input.mapped_actions,
            controls.ticks,
            controls.intents,
            physics.difficulty_index(),
            physics.ticks,
            physics.contact_count,
            physics.board.bodies().map(|body| body.rates.position),
            skater.pose_generation,
            skater.animation.ticks,
            skater.ground.steering.targets,
            physics.riding.motion.ground_speed,
            camera.selected_shot(),
            camera.frame,
        );
        let input_report = format!(
            "{input_report}Native OnBoard/OffBoard clips available: {}\n",
            skater.animation.evaluator.frames.clip_count()
        );
        let input_report = format!(
            "{input_report}World: {} generation={} triangles={} native_grind_primitives={}\nMultiplayer: {}\nShared objects: {}\nObserved active grind frames: {}\nMaximum observed network contacts: {}\n",
            current_map.name, current_map.generation, physics.world_triangles().len(),
            physics.grind_provider().primitives().len(), multiplayer.diagnostic_summary(),
            shared.diagnostic_summary(), state.grind_frames, state.max_network_contacts,
        );
        let input_report = format!("{input_report}Replicated object inventory: {}\n", multiplayer.resource_entity_states());
        let input_report = format!("{input_report}Shared entity observed contact frames: {}\n",
            serde_json::to_string(&state.contact_frames).expect("bounded contact diagnostics"));
        let counts = runtime_status.0.diagnostics();
        let input_report = format!("{input_report}Resource animation: banks:{} layers:{} appearances:{} attachments:{} pending:{}\nCurrent shared entity contact ids: {:?}\n",
            counts.banks, counts.layers, counts.appearances, counts.attachments, counts.pending,
            physics.network_contact_ids);
        let input_report = format!("{input_report}Gameplay paused: {}\nPause menu open: {}\n",
            runtime_status.1.is_paused(), runtime_status.2.as_ref().is_some_and(|menu| menu.open));
        if let Err(error) = std::fs::write(report_path.with_extension("input.txt"), input_report) {
            eprintln!("Cannot save controller verification: {error}");
            exit.write(AppExit::error());
            return;
        }
        if finished {
            info!("GAME_VERIFY_OK");
            exit.write(AppExit::Success);
        }
    } else if state.elapsed > 45. {
        error!("Game startup/capture timed out");
        exit.write(AppExit::error());
    }
}
