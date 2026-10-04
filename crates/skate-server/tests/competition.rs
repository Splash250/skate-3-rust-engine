use serde_json::json;
use skate_net::{
    Body, Pose,
    dedicated::{Config, Server},
    lobby::{Info, Session},
    packed::{self, BodyState, Packed},
};
use skate_server::{
    competition::{Competition, OwnedEvent},
    world::Terrain,
};
use std::{collections::BTreeMap, sync::Arc};

fn terrain(wall: bool) -> Terrain {
    let mut triangles = vec![
        [[-20., 0., -20.], [20., 0., 20.], [20., 0., -20.]],
        [[-20., 0., -20.], [-20., 0., 20.], [20., 0., 20.]],
    ];
    if wall {
        triangles.extend([
            [[2., 0., -10.], [2., 4., 10.], [2., 0., 10.]],
            [[2., 0., -10.], [2., 4., -10.], [2., 4., 10.]],
        ]);
    }
    Terrain {
        revision: format!("synthetic-{wall}"),
        triangles: Arc::new(triangles),
        spawn: [0.; 3],
        heading: 0.,
    }
}
struct Run {
    server: Server,
    client: Session,
    competition: Competition,
    now: u64,
    position: [f32; 3],
    entities: Vec<skate_net::entities::Entity>,
}
impl Run {
    fn new(wall: bool) -> Self {
        let server = Server::new(Config {
            session: 7,
            server_id: 99,
            map: 1,
            max_players: 16,
        })
        .unwrap();
        let mut client = Session::dedicated_client(
            7,
            Info {
                id: 2,
                map: 1,
                rig: 3,
                physics: 4,
                appearance: 5,
            },
            99,
        );
        client.set_loopback(true);
        let mut competition = Competition::default();
        competition.set_terrain(Some(&terrain(wall))).unwrap();
        competition.sync_resources(BTreeMap::from([("course".into(), 1)]));
        let mut run = Self {
            server,
            client,
            competition,
            now: 0,
            position: [0.; 3],
            entities: Vec::new(),
        };
        for _ in 0..15 {
            run.tick([0.; 3]);
        }
        assert_eq!(run.server.player_count(), 1);
        run
    }
    fn tick(&mut self, position: [f32; 3]) -> Vec<OwnedEvent> {
        self.now += 20;
        if let Some(reset) = self.client.pending_movement_reset() {
            // The fixture's native simulation accepts the approved spawn; a
            // subsequent forged sample still goes through normal wire validation.
            self.client.complete_movement_reset(reset.epoch);
        }
        self.position = position;
        let pose = Pose {
            p: position,
            q: [0., 0., 0., 1.],
        };
        self.client.publish(
            packed::BODY,
            Packed::body(&BodyState {
                root: pose,
                enabled: (1 << 33) - 1,
                bodies: vec![
                    Body {
                        pose,
                        velocity: [0.; 3],
                        angular: [0.; 3]
                    };
                    33
                ],
            })
            .unwrap(),
            self.now,
        );
        for packet in self.client.service(self.now) {
            self.server.receive(11, &packet.data, self.now);
        }
        for packet in self.server.service(self.now) {
            self.client.receive(99, &packet.data, self.now);
        }
        self.competition.step(&mut self.server, &self.entities)
    }
    fn start(&mut self) {
        self.competition.command("course",1,json!({"kind":"define","course":{"name":"sprint","instance":"0","checkpoints":[{"position":[2,0,0],"radius":0.5,"points":50},{"position":[4,0,0],"radius":0.5,"points":75}],"limits":{"max_speed":5,"max_acceleration":40,"max_gap_ms":500,"max_airborne_ms":2500}}}),&mut self.server).unwrap();
        self.competition
            .command(
                "course",
                1,
                json!({"kind":"start","name":"sprint","player":"2"}),
                &mut self.server,
            )
            .unwrap();
        // Deliver the authoritative baseline reset before publishing its next sample.
        for packet in self.server.service(self.now) {
            self.client.receive(99, &packet.data, self.now);
        }
    }
}

#[test]
fn pickups_and_server_entity_contacts_award_once_and_require_the_owned_instance() {
    use skate_net::entities::{BodyType, Definition, Entity, Shape};
    let mut run = Run::new(false);
    run.competition.command("course",1,json!({"kind":"define","course":{"name":"contacts","instance":"0","pickups":[{"position":[1,0,0],"radius":0.5,"points":10}],"contacts":[{"entity":"target","points":20}]}}),&mut run.server).unwrap();
    let entity = Entity {
        id: 4,
        resource: "course".into(),
        generation: 1,
        key: "target".into(),
        instance: 0,
        controller: None,
        definition: Definition {
            shape: Shape::Sphere { radius: 0.3 },
            body_type: BodyType::Static,
            mass: 1.,
            friction: 0.5,
            color: [1.; 4],
        },
        tick: 1,
        epoch: 1,
        position: [2., 0.85, 0.],
        rotation: [0., 0., 0., 1.],
        velocity: [0.; 3],
        angular: [0.; 3],
    };
    let mut foreign = entity.clone();
    foreign.resource = "unrelated".into();
    run.entities.push(foreign);
    run.competition
        .command(
            "course",
            1,
            json!({"kind":"start","name":"contacts","player":"2"}),
            &mut run.server,
        )
        .unwrap();
    for packet in run.server.service(run.now) {
        run.client.receive(99, &packet.data, run.now);
    }
    for n in 1..=40 {
        assert!(run.tick([n as f32 * 0.04, 0., 0.]).is_empty());
    }
    run.entities = vec![entity];
    let mut events = Vec::new();
    for n in 41..=45 {
        events.extend(run.tick([n as f32 * 0.04, 0., 0.]));
    }
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].value["kind"], "completed");
    assert_eq!(events[0].value["score"], 30);
    assert_eq!(events[0].value["pickups"], 1);
    assert_eq!(events[0].value["contacts"], 1);
    assert!(run.tick([1.84, 0., 0.]).is_empty());
}
#[test]
fn legitimate_course_is_scored_from_server_time_and_trajectory_not_claimed_score() {
    let mut run = Run::new(false);
    run.start();
    let started = run.now;
    run.client.publish_application(
        skate_net::dedicated::GAMEPLAY_KEY,
        serde_json::to_vec(&skate_net::dedicated::Gameplay {
            sequence_score: 9_999_999,
            line_score: 9_999_999,
            ..Default::default()
        })
        .unwrap(),
        run.now,
    );
    let mut result = None;
    for n in 1..=110 {
        for event in run.tick([n as f32 * 0.04, 0., 0.]) {
            assert_ne!(event.value["kind"], "rejected", "{}", event.value);
            result = Some(event);
        }
    }
    let result = result.expect("legitimate route did not finish");
    assert_eq!(result.resource, "course");
    assert_eq!(result.generation, 1);
    assert_eq!(result.value["kind"], "completed");
    assert_eq!(result.value["score"], 125);
    let elapsed = result.value["elapsed_ms"].as_u64().unwrap();
    assert!(elapsed >= 1500 && elapsed <= run.now - started);
    assert!(
        run.tick([4.4, 0., 0.]).is_empty(),
        "completion cannot be replayed"
    );
    assert!(
        run.competition
            .command(
                "course",
                1,
                json!({"kind":"score","player":"2","score":999999}),
                &mut run.server
            )
            .is_err()
    );
}
#[test]
fn forged_jump_and_terrain_crossing_are_rejected_and_corrected_to_approved_spawn() {
    let mut jump = Run::new(false);
    jump.start();
    let epoch = jump.client.movement_epoch();
    // Below the generic protocol's10m allowance but far above this course's
    // speed bound, so the stricter competition validator must reject it.
    let mut events = Vec::new();
    for _ in 0..5 {
        events = jump.tick([4., 0., 0.]);
        if !events.is_empty() {
            break;
        }
    }
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].value["kind"], "rejected");
    assert_eq!(events[0].value["score"], 0);
    assert!(
        jump.server.competition_players()[0].epoch > epoch,
        "rejection issues a fresh server correction epoch"
    );
    assert_eq!(jump.server.competition_players()[0].position, [0.; 3]);
    let mut wall = Run::new(true);
    wall.start();
    let mut rejected = None;
    for n in 1..=80 {
        let events = wall.tick([n as f32 * 0.04, 0., 0.]);
        if !events.is_empty() {
            rejected = Some(events[0].value.clone());
            break;
        }
    }
    let rejected = rejected.expect("wall crossing was not rejected");
    assert_eq!(rejected["kind"], "rejected");
    assert_eq!(rejected["reason"], "terrain_collision");
    assert_eq!(rejected["score"], 0);
}
#[test]
fn missing_freshness_instance_change_owner_retirement_and_untrusted_schema_fail_closed() {
    let mut run = Run::new(false);
    run.start();
    run.now += 501;
    run.server.service(run.now);
    let events = run.competition.step(&mut run.server, &[]);
    assert_eq!(events[0].value["reason"], "observation_timeout");
    assert!(
        run.competition
            .command(
                "course",
                0,
                json!({"kind":"start","name":"sprint","player":"2"}),
                &mut run.server
            )
            .is_err()
    );
    assert!(
        run.competition
            .command(
                "course",
                1,
                json!({"kind":"start","name":"sprint","player":"02"}),
                &mut run.server
            )
            .is_err()
    );
    run.competition
        .sync_resources(BTreeMap::from([("course".into(), 2)]));
    assert!(
        run.competition
            .command(
                "course",
                2,
                json!({"kind":"start","name":"sprint","player":"2"}),
                &mut run.server
            )
            .is_err(),
        "new generation cannot reuse retired rules"
    );
    assert!(run.competition.command("course",2,json!({"kind":"define","course":{"name":"sprint","instance":"0","checkpoints":[{"position":[0,0,0],"radius":0.5,"points":1}],"score":999}}),&mut run.server).is_err());
}

#[test]
fn tiny_packet_steps_cannot_accumulate_tolerance_and_authorized_teleports_do_not_finish() {
    let mut run = Run::new(false);
    run.competition.command("course",1,json!({"kind":"define","course":{"name":"slow","instance":"0","checkpoints":[{"position":[10,0,0],"radius":0.5,"points":1}],"limits":{"max_speed":1,"max_acceleration":80}}}),&mut run.server).unwrap();
    run.competition
        .command(
            "course",
            1,
            json!({"kind":"start","name":"slow","player":"2"}),
            &mut run.server,
        )
        .unwrap();
    for packet in run.server.service(run.now) {
        run.client.receive(99, &packet.data, run.now);
    }
    let mut rejected = None;
    for n in 1..=20 {
        let events = run.tick([n as f32 * 0.04, 0., 0.]);
        if !events.is_empty() {
            rejected = Some(events[0].value.clone());
            break;
        }
    }
    assert_eq!(rejected.unwrap()["reason"], "window_displacement");
    let mut teleported = Run::new(false);
    teleported.start();
    teleported
        .server
        .teleport_now(
            2,
            skate_net::dedicated::TeleportDestination {
                position: [4., 0., 0.],
                heading: 0.,
                velocity: [0.; 3],
                instance: 0,
            },
        )
        .unwrap();
    let events = teleported.competition.step(&mut teleported.server, &[]);
    assert_eq!(events[0].value["kind"], "cancelled");
    assert_eq!(events[0].value["reason"], "movement_epoch_changed");
}

#[test]
fn approved_teleports_cancel_attempts_without_overriding_the_new_baseline() {
    for instance in [0, 7] {
        let mut run = Run::new(false);
        run.start();
        assert!(run.tick([0.;3]).is_empty());
        let destination = skate_net::dedicated::TeleportDestination {
            position: [12., 0., 5.], heading: 0.7, velocity: [0.;3], instance,
        };
        let epoch = run.server.teleport_now(2, destination).unwrap();
        let events = run.competition.step(&mut run.server, &[]);
        assert_eq!(run.server.movement_epoch_of(2), Some(epoch), "course must not create a stale correction epoch");
        assert_eq!(run.server.instance_of(2), Some(instance));
        assert_eq!(run.server.competition_players()[0].position, destination.position);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].value["kind"], "cancelled");
        assert_eq!(events[0].value["reason"], "movement_epoch_changed");
        for packet in run.server.service(run.now) { run.client.receive(99, &packet.data, run.now); }
        assert_eq!(run.client.pending_movement_reset().unwrap().destination, destination);
        for _ in 0..30 { assert!(run.tick(destination.position).is_empty()); }
        assert_eq!(run.server.movement_epoch_of(2), Some(epoch));
    }
}

#[test]
fn admission_reset_cancels_course_before_missing_samples_can_time_out() {
    let mut run = Run::new(false);
    run.start();
    run.server.configure_resources("a".repeat(64), 31030, BTreeMap::from([("course".into(),1)])).unwrap();
    let epoch = run.server.movement_epoch_of(2);
    assert!(run.server.competition_players().is_empty(), "readmission has no eligible body sample");
    let events = run.competition.step(&mut run.server, &[]);
    assert_eq!(events.len(), 1, "trusted admission reset cancels immediately without a body observation");
    assert_eq!(events[0].value["kind"], "cancelled");
    assert_eq!(events[0].value["reason"], "movement_epoch_changed");
    run.now += 501;
    run.server.service(run.now);
    assert!(run.competition.step(&mut run.server, &[]).is_empty());
    assert_eq!(run.server.movement_epoch_of(2), epoch, "old attempt cannot reset the new admission epoch");
}

#[test]
fn larger_world_and_incompatible_spawn_disable_only_course_verification() {
    let mut run = Run::new(false);
    run.start();
    let mut large = terrain(false);
    large.revision = "large-valid-world".into();
    large.triangles = Arc::new(vec![large.triangles[0]; 262145]);
    run.competition
        .set_terrain(Some(&large))
        .expect("competition bound must not reject a valid world");
    let error = run
        .competition
        .command(
            "course",
            1,
            json!({"kind":"start","name":"sprint","player":"2"}),
            &mut run.server,
        )
        .unwrap_err();
    assert!(error.contains("262144"));
    let mut blocked = terrain(false);
    blocked.revision = "blocked-spawn".into();
    blocked.spawn = [0., -0.5, 0.];
    run.competition.set_terrain(Some(&blocked)).unwrap();
    let error = run
        .competition
        .command(
            "course",
            1,
            json!({"kind":"start","name":"sprint","player":"2"}),
            &mut run.server,
        )
        .unwrap_err();
    assert!(error.contains("spawn intersects"));
}

// The actual native BODY root is animation_to_world, not a feet position.
// Native course run 20261004a settled at y=-0.13076536 on the park's y=0 floor
// after the approved y=1 spawn, before any push input. Reproduce that endpoint
// through a bounded smooth trajectory; this fixture does not invent native poses.
fn settled_native_root(wall: bool) -> Run {
    settled_native_root_on(terrain(wall))
}
fn settled_native_root_on(mut world: Terrain) -> Run {
    let mut run = Run::new(false);
    world.revision.push_str("-native-root");
    world.spawn = [0., 1., 0.];
    run.competition.set_terrain(Some(&world)).unwrap();
    run.start();
    for n in 0..=75 {
        let t = n as f32 / 75.;
        let y = 1. - 1.13076536 * (1. - (std::f32::consts::PI * t).cos()) * 0.5;
        assert!(run.tick([0., y, 0.]).is_empty(), "native root settling rejected at y={y}");
    }
    run
}

#[test]
fn native_animation_root_can_settle_and_complete_a_supported_course() {
    let mut run = settled_native_root(false);
    let mut events = Vec::new();
    for n in 1..=110 {
        events.extend(run.tick([n as f32 * 0.04, -0.13076536, 0.]));
    }
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].value["kind"], "completed");
    assert_eq!(events[0].value["score"], 125);
}

#[test]
fn native_root_support_adjustment_cannot_hide_walls_or_deep_below_floor_motion() {
    let mut wall = settled_native_root(true);
    let rejected = (1..=110).find_map(|n| wall.tick([n as f32 * 0.04, -0.13076536, 0.]).into_iter().next()).unwrap();
    assert_eq!(rejected.value["kind"], "rejected");
    assert_eq!(rejected.value["reason"], "terrain_collision");
    assert!(wall.position[0] < 2., "wall must stop the capsule before its root crosses");
    let mut below = settled_native_root(false);
    let rejected = (1..=40).find_map(|n| below.tick([0., -0.13076536 - n as f32 * 0.01, 0.]).into_iter().next()).unwrap();
    assert_eq!(rejected.value["kind"], "rejected");
    assert_eq!(rejected.value["reason"], "terrain_collision");
    assert!(below.position[1] > -0.25, "support adjustment is not an underground movement allowance");
}

#[test]
fn native_root_support_adjustment_still_checks_the_full_height_below_a_ceiling() {
    let mut world = terrain(false);
    let mut triangles = world.triangles.as_ref().clone();
    triangles.extend([
        [[1., 1.4, -10.], [10., 1.4, -10.], [10., 1.4, 10.]],
        [[1., 1.4, -10.], [10., 1.4, 10.], [1., 1.4, 10.]],
    ]);
    world.triangles = Arc::new(triangles);
    let mut run = settled_native_root_on(world);
    let rejected = (1..=110).find_map(|n| run.tick([n as f32 * 0.04, -0.13076536, 0.]).into_iter().next()).unwrap();
    assert_eq!(rejected.value["reason"], "terrain_collision");
    assert!(run.position[0] < 1., "full capsule must collide before entering the low ceiling");
}
