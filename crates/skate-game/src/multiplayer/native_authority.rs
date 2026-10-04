//! Responsive native input prediction and bounded full-state reconstruction.
//! BODY is never used as a replacement for hidden graph/solver history.
use super::Multiplayer;
use crate::{
    app::SimulationSet,
    physics::{
        GamePhysics, PlayerControls, SkaterRuntime,
        native_authority::{self, Simulation},
    },
};
use bevy::prelude::*;
use skate_net::native_authority::{
    INPUT_KEY, Input, InputLog, InputPacket, MAX_PACKET_INPUTS, MAX_TICKS, State, Status,
};
use std::thread::JoinHandle;

struct Replayed {
    simulation: Simulation,
    proofs: Vec<String>,
    terminal: bool,
}
#[derive(Resource, Default)]
pub(crate) struct Prediction {
    active: bool,
    state: Option<State>,
    log: Option<InputLog>,
    proofs: Vec<String>,
    acknowledged: u64,
    checked: Option<u64>,
    finished_epoch: Option<u64>,
    job: Option<JoinHandle<Result<Replayed, String>>>,
    failure: Option<String>,
}
impl Prediction {
    fn retire(&mut self) {
        self.active = false;
        self.log = None;
        self.proofs.clear();
        self.state = None;
        self.checked = None;
        self.finished_epoch = None;
        self.acknowledged = 0;
    }
    fn may_begin(&self, epoch: u64) -> bool {
        self.state.is_none() && self.finished_epoch != Some(epoch)
    }
}
pub(crate) fn ordinary(prediction: Option<Res<Prediction>>) -> bool {
    prediction.is_none_or(|p| !p.active)
}
pub(crate) fn active(world: &World) -> bool {
    world.get_resource::<Prediction>().is_some_and(|p| p.active)
}
#[cfg(test)]
pub(crate) fn active_prediction() -> Prediction {
    Prediction {
        active: true,
        ..Default::default()
    }
}
pub(crate) fn install(app: &mut App) {
    app.init_resource::<Prediction>()
        .add_systems(
            PreUpdate,
            synchronize
                .after(super::receive)
                .after(crate::map_transition::MapTransitionSet),
        )
        .add_systems(FixedUpdate, simulate.in_set(SimulationSet::Physics));
}

fn host_state(world: &World) -> Option<State> {
    let net = world.get_resource::<Multiplayer>()?;
    if !net.is_dedicated() {
        return None;
    }
    let lobby = net.lobby.as_ref()?;
    let value = &lobby
        .actors
        .get(&lobby.host_actor()?)?
        .application
        .get(&skate_net::native_authority::state_key(lobby.local))?
        .value;
    let state: State = serde_json::from_slice(value).ok()?;
    if state.admission.epoch != lobby.movement_epoch()
        || state.admission.version != 1
        || !(1..=MAX_TICKS).contains(&state.ticks)
        || state.tick > state.ticks
    {
        return None;
    }
    let host = world
        .get_resource::<crate::modding::Mods>()?
        .manager
        .resources
        .as_ref()?;
    if host.generation(&state.resource) != Some(state.admission.generation)
        || !host.running(&state.resource)
    {
        return None;
    }
    Some(state)
}

fn begin_replay(
    world: &World,
    prediction: &mut Prediction,
    state: &State,
    terminal: bool,
) -> Result<(), String> {
    crate::modding::native_authority_ready(world)?;
    let root = world.resource::<crate::config::Config>().asset_root.clone();
    let map = world
        .resource::<crate::map_transition::CurrentMap>()
        .path
        .clone()
        .ok_or("Native authority requires the mounted verified resource world")?;
    let difficulty = crate::difficulty::Difficulty::parse(&state.difficulty)?;
    if !matches!(
        difficulty,
        crate::difficulty::Difficulty::Easy
            | crate::difficulty::Difficulty::Normal
            | crate::difficulty::Difficulty::Hardcore
    ) {
        return Err("Unsupported native authority difficulty".into());
    }
    let expected = state.clone();
    let mut inputs = prediction
        .log
        .as_ref()
        .map_or_else(Vec::new, |l| l.inputs().to_vec());
    if terminal {
        inputs.truncate(state.tick as usize);
    }
    if state.tick > inputs.len() as u64 {
        return Err("Authority acknowledged unknown native input".into());
    }
    bevy::log::info!(
        "NATIVE_AUTHORITY_REPLAY epoch={} acknowledged={} through={} terminal={}",
        state.admission.epoch,
        state.tick,
        inputs.len(),
        terminal
    );
    prediction.active = true;
    prediction.state = Some(state.clone());
    prediction.job = Some(std::thread::Builder::new().name("native-replay".into()).spawn(move || {
        let bytes = std::fs::read(&map).map_err(|e|format!("Native authority world: {e}"))?;
        if skate_resources::digest_bytes(&bytes) != expected.world {
            return Err("Native authority world identity differs from the mounted world".into());
        }
        let mut simulation = Simulation::load(&root, Some(&map), difficulty, expected.admission)?;
        let mut proofs = vec![simulation.snapshot().state_digest()];
        for input in inputs { proofs.push(simulation.step(input)?.state_digest()); }
        if proofs.get(expected.tick as usize) != Some(&expected.state_digest) {
            return Err("Native replay differs from authority; asset/settings/platform parity is unavailable".into());
        }
        Ok(Replayed { simulation, proofs, terminal })
    }).map_err(|e|e.to_string())?);
    Ok(())
}

fn synchronize(world: &mut World) {
    let Some(mut prediction) = world.remove_resource::<Prediction>() else {
        return;
    };
    let state = host_state(world);
    let retired = prediction.state.as_ref().is_some_and(|old| {
        state.as_ref().is_none_or(|s| {
            s.admission != old.admission
                || s.resource != old.resource
                || matches!(s.status, Status::Cancelled | Status::Rejected)
        })
    });
    if retired {
        prediction.retire();
        end_native(world);
    }
    if prediction.job.as_ref().is_some_and(|j| j.is_finished()) {
        let result = prediction
            .job
            .take()
            .unwrap()
            .join()
            .unwrap_or_else(|_| Err("Native replay thread failed".into()));
        if !retired && prediction.active {
            match result {
                Ok(replayed) => {
                    let simulation = replayed.simulation;
                    let admission = simulation.log.admission();
                    let travel = simulation.skater.travel_generation;
                    prediction.log = Some(simulation.log);
                    prediction.proofs = replayed.proofs;
                    bevy::log::info!(
                        "NATIVE_AUTHORITY_READY epoch={} tick={} terminal={}",
                        admission.epoch,
                        prediction.log.as_ref().unwrap().len(),
                        replayed.terminal
                    );
                    prediction.checked = None;
                    world
                        .resource_mut::<Time<Fixed>>()
                        .set_timestep(simulation.physics.period());
                    world.insert_resource(simulation.physics);
                    world.insert_resource(simulation.skater);
                    world.insert_resource(simulation.controls);
                    world.insert_resource(simulation.camera);
                    world.insert_resource(simulation.graphs);
                    world.insert_resource(crate::presentation::Presentation::default());
                    // A manual-camera release latch and a prior presentation
                    // replay belong to the previous timeline. Neither can
                    // suppress actions in the newly reconstructed native run.
                    world.insert_resource(crate::debug_cam::DebugCam::default());
                    world.insert_resource(crate::replay::Replay::default());
                    let mut net = world.resource_mut::<Multiplayer>();
                    net.dedicated.native_begin(admission.epoch, travel);
                    if let Some(lobby) = &mut net.lobby {
                        lobby.complete_movement_reset(admission.epoch);
                    }
                    if replayed.terminal {
                        prediction.active = false;
                        prediction.finished_epoch = Some(admission.epoch);
                        end_native(world);
                        bevy::log::info!(
                            "NATIVE_AUTHORITY_COMPLETED epoch={} tick={} reconciled=true",
                            admission.epoch,
                            prediction.log.as_ref().unwrap().len()
                        );
                    }
                }
                Err(error) => prediction.failure = Some(error),
            }
        }
    }
    if let Some(state) = &state {
        if prediction.job.is_none() && prediction.failure.is_none() {
            if state.status == Status::Running && prediction.may_begin(state.admission.epoch) {
                prediction.acknowledged = 0;
                if let Err(error) = begin_replay(world, &mut prediction, state, false) {
                    prediction.failure = Some(error);
                }
            } else if prediction.active
                && matches!(state.status, Status::Running | Status::Completed)
            {
                let tick = state.tick as usize;
                let length = prediction.log.as_ref().map_or(0, InputLog::len);
                if tick > length {
                    prediction.failure = Some("Authority acknowledged unsent native input".into());
                } else if prediction.checked != Some(state.tick)
                    || state.status == Status::Completed
                {
                    let mismatch = prediction.proofs.get(tick) != Some(&state.state_digest);
                    let terminal = state.status == Status::Completed;
                    if mismatch || terminal && length != tick {
                        if let Err(error) = begin_replay(world, &mut prediction, state, terminal) {
                            prediction.failure = Some(error);
                        }
                    } else {
                        if prediction.acknowledged != state.tick {
                            bevy::log::info!(
                                "NATIVE_AUTHORITY_ACK epoch={} tick={} predicted={} matched=true",
                                state.admission.epoch,
                                state.tick,
                                length
                            );
                        }
                        prediction.acknowledged = state.tick;
                        prediction.checked = Some(state.tick);
                        prediction.state = Some(state.clone());
                        if terminal {
                            prediction.active = false;
                            prediction.finished_epoch = Some(state.admission.epoch);
                            end_native(world);
                            bevy::log::info!(
                                "NATIVE_AUTHORITY_COMPLETED epoch={} tick={} reconciled=true",
                                state.admission.epoch,
                                state.tick
                            );
                        }
                    }
                }
            }
        }
    }
    if prediction.active && prediction.job.is_none() && prediction.failure.is_none() {
        if let Some(log) = &prediction.log {
            let start = prediction.acknowledged as usize;
            let inputs = log.inputs()[start..log.len().min(start + MAX_PACKET_INPUTS)].to_vec();
            if !inputs.is_empty() {
                if let Ok(bytes) = (InputPacket { inputs }).encode() {
                    world
                        .resource_mut::<Multiplayer>()
                        .publish_application(INPUT_KEY, bytes);
                }
            }
        }
    }
    if let Some(error) = prediction.failure.take() {
        prediction.active = false;
        prediction.log = None;
        end_native(world);
        let mut net = world.resource_mut::<Multiplayer>();
        net.leave();
        net.status = format!("Native authority stopped: {error}");
        bevy::log::error!("{}", net.status);
    }
    world.insert_resource(prediction);
}

fn end_native(world: &mut World) {
    let difficulty = world.resource::<crate::config::Config>().difficulty;
    let period = {
        let mut physics = world.resource_mut::<GamePhysics>();
        physics.set_difficulty(difficulty);
        physics.period()
    };
    world.resource_mut::<Time<Fixed>>().set_timestep(period);
    let travel = world.resource::<SkaterRuntime>().travel_generation;
    world
        .resource_mut::<Multiplayer>()
        .dedicated
        .native_end(travel);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_epoch_deduplication_does_not_survive_connection_retirement() {
        let mut prediction = Prediction {
            finished_epoch: Some(2),
            acknowledged: 300,
            ..Default::default()
        };
        assert!(
            !prediction.may_begin(2),
            "repeated terminal record must not restart the same attempt"
        );
        prediction.retire();
        assert!(
            prediction.may_begin(2),
            "a new server connection can legitimately reuse an epoch number"
        );
        assert_eq!(prediction.acknowledged, 0);
    }
}

fn simulate(
    mut prediction: ResMut<Prediction>,
    input: Res<crate::input::PublishedTickInput>,
    mut physics: ResMut<GamePhysics>,
    mut skater: ResMut<SkaterRuntime>,
    mut controls: ResMut<PlayerControls>,
    graphs: Res<crate::graph_runtime::StockGraphs>,
    mut camera: ResMut<crate::camera::CameraRuntime>,
) {
    if !prediction.active || prediction.job.is_some() || prediction.failure.is_some() {
        return;
    }
    let Some(state) = &prediction.state else {
        return;
    };
    let limit = state.ticks;
    let acknowledged = prediction.acknowledged;
    let Some(log) = &mut prediction.log else {
        return;
    };
    let tick = log.len() as u64 + 1;
    // Two seconds of unacknowledged prediction is the maximum retained lead.
    // Stop advancing on missing authority instead of inventing neutral inputs.
    if tick > limit || tick > acknowledged.saturating_add(120) {
        return;
    }
    let actions = *input.0.actions().values();
    let next = Input {
        epoch: log.admission().epoch,
        tick,
        actions,
    };
    if let Err(error) = log.append(next).and_then(|_| {
        native_authority::advance(
            &mut physics,
            &mut skater,
            &mut controls,
            &graphs,
            &mut camera,
            actions,
        )
    }) {
        prediction.failure = Some(error);
        return;
    }
    let proof = native_authority::snapshot(&physics, &skater, log).state_digest();
    prediction.proofs.push(proof);
}
