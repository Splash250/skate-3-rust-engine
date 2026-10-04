//! The real fixed-tick native pipeline, reusable by the trusted authority worker
//! and full-state prediction replay. No renderer, device polling or mod callbacks.
use super::{GamePhysics, PlayerControls, SkaterRuntime};
use crate::{camera::CameraRuntime, difficulty::Difficulty, graph_runtime::StockGraphs};
use skate_net::native_authority::{Admission, Input, InputLog, Reply, Request, Score, Snapshot};
use std::{
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
};

pub(crate) struct Simulation {
    pub physics: GamePhysics,
    pub skater: SkaterRuntime,
    pub controls: PlayerControls,
    pub camera: CameraRuntime,
    pub graphs: StockGraphs,
    pub log: InputLog,
    failed: bool,
}
impl Simulation {
    pub fn load(
        root: &Path,
        map: Option<&Path>,
        difficulty: Difficulty,
        admission: Admission,
    ) -> Result<Self, String> {
        let map = map.map(skate_data::skate_map::SkateMap::load).transpose()?;
        Self::load_world(root, map.as_ref(), difficulty, admission)
    }
    fn load_world(
        root: &Path,
        map: Option<&skate_data::skate_map::SkateMap>,
        difficulty: Difficulty,
        admission: Admission,
    ) -> Result<Self, String> {
        let log = InputLog::new(admission)?;
        let assets = skate_data::GameAssets::load(root).map_err(|e| e.to_string())?;
        let graphs = StockGraphs::load(root, &assets)?;
        if let Some(map) = map {
            skate_data::resource_world::validate(map)?;
        }
        let mut physics = GamePhysics::load_with_difficulty(root, map, difficulty)?;
        physics.network_active = true;
        // This contract is exactly sixty physical ticks per host second.
        if (physics.settings.step.simulation.time_step - 1. / 60.).abs() > 1e-7 {
            return Err("Native authority requires the stock 60Hz timestep".into());
        }
        let skater = SkaterRuntime::load(root, &graphs, &physics, difficulty.profile_key())?;
        let controls = PlayerControls::load(root)?;
        let camera = CameraRuntime::load(root)?;
        Ok(Self {
            physics,
            skater,
            controls,
            camera,
            graphs,
            log,
            failed: false,
        })
    }
    pub fn step(&mut self, input: Input) -> Result<Snapshot, String> {
        if self.failed {
            return Err("Native authority simulation failed; a new admission is required".into());
        }
        let actions = input.actions;
        if self.log.append(input)? {
            let result = advance(
                &mut self.physics,
                &mut self.skater,
                &mut self.controls,
                &self.graphs,
                &mut self.camera,
                actions,
            );
            if result.is_err() {
                self.failed = true;
            }
            result?;
        }
        Ok(self.snapshot())
    }
    pub fn snapshot(&self) -> Snapshot {
        snapshot(&self.physics, &self.skater, &self.log)
    }
    #[cfg(test)]
    pub fn replay(&mut self, inputs: &[Input]) -> Result<(), String> {
        for input in inputs {
            self.step(input.clone())?;
        }
        Ok(())
    }
}

pub(crate) fn advance(
    physics: &mut GamePhysics,
    skater: &mut SkaterRuntime,
    controls: &mut PlayerControls,
    graphs: &StockGraphs,
    camera: &mut CameraRuntime,
    values: [f32; 18],
) -> Result<(), String> {
    let mut actions = skate_core::input::gameplay_map::GameplayActions::from_values(values);
    controls.update_for_physics(&mut actions, physics, skater, camera)?;
    controls.publish_gestures(
        physics.animation_profile.physics_mode,
        skater.player_input.physical.state.state_16,
    );
    super::frame::advance(
        physics,
        skater,
        controls,
        graphs,
        &mut actions,
        true,
        camera,
    )
}

pub(crate) fn snapshot(physics: &GamePhysics, skater: &SkaterRuntime, log: &InputLog) -> Snapshot {
    let body = super::network::capture_body(physics, skater);
    let score = &skater.scoring;
    Snapshot {
        admission: log.admission(),
        tick: log.len() as u64,
        root: body.root,
        enabled: body.enabled,
        bodies: body.bodies,
        state: skater.player_state.current() as u32,
        score: Score {
            trick_seq: score.trick_seq(),
            trick: score.trick_name().into(),
            landing_seq: score.landing_seq,
            landed_trick: score.landed_trick.clone(),
            bail_seq: score.bail_seq,
            sequence: score.sequence_score(),
            awarded: score.session.holder.awarded_total(),
            publications: score.session.holder.publication_count(),
            line: score.line_score(),
            completed_lines: score.session.holder.snapshot.completed_lines,
            multiplier: score.multiplier(),
            clean: score.clean(),
            sketchy: score.sketchy(),
        },
        history_digest: log.digest(),
    }
}

/// Enter before normal configuration/updater startup. Assets and map paths are
/// trusted operator arguments, never fields supplied by a connected player.
pub(crate) fn entry() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--native-authority")) {
        return None;
    }
    Some(match run(args.collect()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("NATIVE_AUTHORITY_ERROR {error}");
            1
        }
    })
}
fn run(args: Vec<std::ffi::OsString>) -> Result<(), String> {
    let mut root = None;
    let mut map = None;
    let mut difficulty = Difficulty::Easy;
    let mut selected_world = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if !seen.insert(arg.clone()) {
            return Err("Repeated native authority option".into());
        }
        match arg.to_str() {
            Some("--assets") => root = Some(PathBuf::from(args.next().ok_or("--assets requires a path")?)),
            Some("--map") => {
                if selected_world { return Err("Select one native authority world".into()); }
                selected_world = true;
                map = Some(PathBuf::from(args.next().ok_or("--map requires a path")?));
            }
            Some("--test-world") => {
                if selected_world { return Err("Select one native authority world".into()); }
                selected_world = true;
            }
            Some("--difficulty") => difficulty = Difficulty::parse(args.next().as_deref().and_then(|s|s.to_str()).ok_or("--difficulty requires a mode")?)?,
            _ => return Err("Usage: skate3rust --native-authority --assets PATH (--map MAP.skate | --test-world) [--difficulty easy|normal|hardcore]".into()),
        }
    }
    if !selected_world {
        return Err("Native authority requires an explicit trusted world".into());
    }
    if !matches!(
        difficulty,
        Difficulty::Easy | Difficulty::Normal | Difficulty::Hardcore
    ) {
        return Err("Native authority supports stock easy, normal and hardcore only".into());
    }
    let root = root.ok_or("Native authority requires --assets PATH")?;
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let mut simulation = None;
    let mut started = false;
    loop {
        let mut line = Vec::new();
        let count = reader
            .by_ref()
            .take((skate_net::native_authority::MAX_REQUEST_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if count > skate_net::native_authority::MAX_REQUEST_BYTES || !line.ends_with(b"\n") {
            return Err("Native authority request exceeds its framing bound".into());
        }
        let request = serde_json::from_slice::<Request>(&line)
            .map_err(|e| format!("Native authority request: {e}"));
        let result = match request {
            Ok(Request::Stop) => break,
            Ok(Request::Start { admission }) if !started => {
                started = true;
                Simulation::load(&root, map.as_deref(), difficulty, admission).map(|loaded| {
                    let snapshot = loaded.snapshot();
                    simulation = Some(loaded);
                    Reply::Ready { snapshot }
                })
            }
            Ok(Request::Start { .. }) => Err("A worker admits exactly one native session".into()),
            Ok(Request::Step { input }) => simulation
                .as_mut()
                .ok_or_else(|| "Native authority is not initialized".to_string())
                .and_then(|s| s.step(input))
                .map(|snapshot| Reply::Advanced { snapshot }),
            Err(error) => Err(error),
        };
        let failed = result.is_err();
        let reply = result.unwrap_or_else(|error| Reply::Rejected { error });
        let bytes = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
        if bytes.len() + 1 > skate_net::native_authority::MAX_REPLY_BYTES {
            return Err("Native authority reply exceeds bounds".into());
        }
        writer
            .write_all(&bytes)
            .and_then(|_| writer.write_all(b"\n"))
            .and_then(|_| writer.flush())
            .map_err(|e| e.to_string())?;
        // A rejected worker cannot publish another score with partially advanced
        // native state. Its host must retire the epoch and issue a new admission.
        if failed {
            return Err("Native authority session rejected".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires owned stock native data via SKATE3_ASSET_ROOT"]
    fn ordinary_inputs_derive_grab_grind_landings_and_banked_combo_totals() {
        let root = PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("asset root"));
        for (scenario, bytes, epoch) in [
            (
                "grab",
                include_bytes!("../tests/fixtures/native-authority-grab.skate").as_slice(),
                4,
            ),
            (
                "rail",
                include_bytes!("../tests/fixtures/native-authority-rail.skate").as_slice(),
                5,
            ),
            (
                "combo",
                include_bytes!("../tests/fixtures/native-authority-grab.skate").as_slice(),
                6,
            ),
        ] {
            let rail = scenario == "rail";
            let combo = scenario == "combo";
            let map = skate_data::skate_map::SkateMap::parse(bytes).unwrap();
            let admission = Admission {
                version: 1,
                epoch,
                instance: 41,
                generation: 1,
            };
            let mut simulation =
                Simulation::load_world(&root, Some(&map), Difficulty::Normal, admission).unwrap();
            let mut preload = None;
            let mut grab = None;
            let mut combo_start = None;
            let mut publications = Vec::new();
            let mut saw_semantic = false;
            let mut peak_multiplier = 1.0_f32;
            for tick in 1..=900 {
                let previous = simulation.snapshot();
                let mut actions = [0.; 18];
                if rail {
                    if tick > 30 && preload.is_none() {
                        actions[16] = 1.;
                        if previous.root.p[2] >= -7.5 {
                            preload = Some(tick);
                        }
                    }
                    if let Some(start) = preload {
                        let age = tick - start;
                        if age < 24 {
                            actions[4] = -1.;
                        } else if age < 27 {
                            actions[4] = 1.;
                        }
                    }
                } else {
                    if grab.is_none() {
                        if (200..300).contains(&previous.state) {
                            grab = Some(tick);
                        } else if tick > 30 {
                            actions[16] = 1.;
                        }
                    }
                    if grab.is_some_and(|start| tick - start < 35) {
                        actions[6] = 1.;
                    }
                    if combo {
                        if combo_start.is_none()
                            && previous.score.landing_seq == 1
                            && previous.state == 100
                        {
                            combo_start = Some(tick);
                        }
                        if let Some(start) = combo_start {
                            match tick - start {
                                0..150 => actions[17] = 1., // ordinary foot brake
                                180..210 => actions[4] = -1.,
                                210..213 => {
                                    actions[3] = -0.89442;
                                    actions[4] = 0.44722;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                let state = simulation
                    .step(Input {
                        epoch: admission.epoch,
                        tick,
                        actions,
                    })
                    .unwrap();
                saw_semantic |= if rail {
                    (400..=405).contains(&state.state) && state.score.trick.contains("GRIND")
                } else {
                    state.score.trick.contains("GRAB")
                };
                peak_multiplier = peak_multiplier.max(state.score.multiplier);
                if state.score.publications > previous.score.publications {
                    publications.push((tick, previous.score.multiplier, state.score.clone()));
                }
            }
            let result = simulation.snapshot();
            eprintln!(
                "NATIVE_AUTHORITY_ACTION scenario={scenario} {:?}",
                result.score
            );
            assert!(
                saw_semantic,
                "ordinary actions must derive the actual native action family"
            );
            assert_eq!(
                result.state, 100,
                "the skater must exit and settle on the floor"
            );
            assert_eq!(
                result.score.landing_seq,
                if combo { 2 } else { 1 },
                "each ordinary action must produce its own successful landing"
            );
            assert_eq!(result.score.bail_seq, 0);
            assert!(result.score.awarded >= f64::from(result.score.completed_lines));
            assert!(result.score.publications > 0);
            assert!(
                result.score.completed_lines > 0. && peak_multiplier > 1.,
                "the native combo must bank: {:?}",
                result.score
            );
            assert_eq!(
                result.score.line, 0.,
                "the completed combo must have left the live line"
            );
            if combo {
                assert_eq!(result.score.publications, 2);
                assert_eq!(publications.len(), 2);
                let (_, _, first) = &publications[0];
                let (_, multiplier, second) = &publications[1];
                assert!(
                    first.clean && second.clean,
                    "both native landings must be clean"
                );
                assert!(!first.sketchy && !second.sketchy);
                assert!(first.landed_trick.contains("GRAB"));
                assert!(second.landed_trick.contains("HEELFLIP"));
                assert_eq!(
                    *multiplier, 1.5,
                    "the second publication must use the active combo"
                );
                assert_eq!(second.sequence, 33.);
                assert_eq!(second.awarded - first.awarded, 33.);
                assert_eq!(
                    result.score.awarded, second.awarded,
                    "line expiry cannot award again"
                );
                eprintln!("NATIVE_AUTHORITY_COMBO_PUBLICATIONS {publications:?}");
            }
            let mut replay =
                Simulation::load_world(&root, Some(&map), Difficulty::Normal, admission).unwrap();
            replay.replay(simulation.log.inputs()).unwrap();
            assert_eq!(
                replay.snapshot().state_digest(),
                result.state_digest(),
                "replaying every accepted raw input must restore the exact final native state"
            );
        }
    }
    #[test]
    #[ignore = "requires owned stock native data via SKATE3_ASSET_ROOT"]
    fn full_native_replay_rebuilds_identical_solver_graph_camera_and_scores() {
        let root = PathBuf::from(std::env::var_os("SKATE3_ASSET_ROOT").expect("asset root"));
        let admission = Admission {
            version: 1,
            epoch: 3,
            instance: 0,
            generation: 1,
        };
        let mut predicted = Simulation::load(&root, None, Difficulty::Normal, admission).unwrap();
        let mut authority = Simulation::load(&root, None, Difficulty::Normal, admission).unwrap();
        let mut controller = crate::input::ControllerInput::default();
        for tick in 1..=300 {
            controller.sample_raw_for_test(skate_core::input::xbox::XboxState {
                buttons: 0,
                triggers: [0; 2],
                left: [0; 2],
                right: if (120..150).contains(&tick) {
                    [0, -32767]
                } else if (150..153).contains(&tick) {
                    [-32767, 16384]
                } else {
                    [0; 2]
                },
            });
            let actions = *controller.player_actions().values();
            let input = Input {
                epoch: admission.epoch,
                tick,
                actions,
            };
            let local = predicted.step(input.clone()).unwrap();
            let remote = authority.step(input).unwrap();
            assert_eq!(
                local.state_digest(),
                remote.state_digest(),
                "native tick{tick}"
            );
        }
        let outcome = authority.snapshot();
        eprintln!("NATIVE_AUTHORITY_OUTCOME {:?}", outcome.score);
        assert!(
            outcome.score.landing_seq > 0,
            "ordinary flick must produce a native banked landing"
        );
        assert!(
            outcome.score.sequence > 0.,
            "native scorer must bank points without any score input"
        );
        assert_eq!(
            outcome.score.awarded, 22.,
            "the isolated native heelflip must remain in the attempt tally even without an active line"
        );
        let history = predicted.log.inputs().to_vec();
        let expected = predicted.snapshot().state_digest();
        drop(predicted);
        let mut rebuilt = Simulation::load(&root, None, Difficulty::Normal, admission).unwrap();
        rebuilt.replay(&history).unwrap();
        assert_eq!(
            expected,
            rebuilt.snapshot().state_digest(),
            "reconstruction must restore hidden solver/graph history too"
        );
        for tick in 301..=330 {
            let input = Input {
                epoch: admission.epoch,
                tick,
                actions: [0.; 18],
            };
            assert_eq!(
                rebuilt.step(input.clone()).unwrap().state_digest(),
                authority.step(input).unwrap().state_digest()
            );
        }
    }
}
