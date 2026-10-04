//! Server-derived course results. These rules verify a bounded movement envelope;
//! they do not reproduce the native articulated skating/trick simulation.
use crate::world::Terrain;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use skate_dynamics::rapier3d::{
    parry::query::{self, Ray, ShapeCastOptions},
    prelude::{Pose, SharedShape, Vector},
};
use skate_net::{
    dedicated::{CompetitionPlayer, Server, TeleportDestination},
    entities::{BodyType, Entity, Shape},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
type Result<T> = std::result::Result<T, String>;
const MAX_COURSES: usize = 32;
const MAX_ATTEMPTS: usize = 64;
const OFFSET: f32 = 0.85;
// BODY.root is the native animation frame, which subtracts the authored board
// translation (SkeletonRootFrames::update), not a feet position. Permit only a
// bounded support-relative reference adjustment: the observed stock settle is
// 0.131m below its floor. This does not enlarge the capsule or movement budgets.
const MAX_ROOT_BELOW_SUPPORT: f32 = 0.20;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub position: [f32; 3],
    pub radius: f32,
    pub points: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    pub entity: String,
    pub points: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_speed: f32,
    pub max_acceleration: f32,
    pub max_gap_ms: u64,
    pub max_airborne_ms: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_speed: 15.,
            max_acceleration: 40.,
            max_gap_ms: 500,
            max_airborne_ms: 2500,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Course {
    pub name: String,
    pub instance: String,
    #[serde(default)]
    pub checkpoints: Vec<Zone>,
    #[serde(default)]
    pub pickups: Vec<Zone>,
    #[serde(default)]
    pub contacts: Vec<Contact>,
    #[serde(default)]
    pub limits: Limits,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Define { course: Course },
    Start { name: String, player: String },
    Cancel { player: String },
    Remove { name: String },
}
#[derive(Clone, Debug)]
pub struct OwnedEvent {
    pub resource: String,
    pub generation: u64,
    pub value: Value,
}
struct OwnedCourse {
    generation: u64,
    course: Course,
}
struct Collision {
    mesh: SharedShape,
    spawn: [f32; 3],
    heading: f32,
}
struct Attempt {
    owner: String,
    generation: u64,
    course: String,
    epoch: u64,
    instance: u64,
    started: u64,
    received: u64,
    seq: Option<u32>,
    position: [f32; 3],
    velocity: [f32; 3],
    airborne: Option<u64>,
    travel: VecDeque<(u64, f32)>,
    checkpoint: usize,
    pickups: BTreeSet<usize>,
    contacts: BTreeSet<usize>,
    score: u32,
}
#[derive(Default)]
pub struct Competition {
    owners: BTreeMap<String, u64>,
    courses: BTreeMap<(String, String), OwnedCourse>,
    attempts: BTreeMap<u64, Attempt>,
    terrain: Option<Collision>,
    world_revision: Option<String>,
    unavailable: Option<String>,
    events: VecDeque<OwnedEvent>,
}
impl Course {
    pub fn validate(&self) -> Result<()> {
        if !label(&self.name)
            || decimal(&self.instance).is_none()
            || self.checkpoints.len() > 32
            || self.pickups.len() > 32
            || self.contacts.len() > 16
            || self.checkpoints.len() + self.pickups.len() + self.contacts.len() == 0
        {
            return Err("Course requires a valid name/instance and1..80 bounded targets".into());
        }
        for zone in self.checkpoints.iter().chain(&self.pickups) {
            if !position(zone.position)
                || !zone.radius.is_finite()
                || !(0.5..=5.).contains(&zone.radius)
                || zone.points > 10000
            {
                return Err(
                    "Course zones require finite positions,0.5..5m radius and at most10000 points"
                        .into(),
                );
            }
        }
        let mut keys = BTreeSet::new();
        for contact in &self.contacts {
            if !label(&contact.entity) || contact.points > 10000 || !keys.insert(&contact.entity) {
                return Err(
                    "Course contacts require distinct owned entity keys and bounded points".into(),
                );
            }
        }
        let l = &self.limits;
        if !l.max_speed.is_finite()
            || !(1.0..=25.).contains(&l.max_speed)
            || !l.max_acceleration.is_finite()
            || !(1.0..=80.).contains(&l.max_acceleration)
            || !(100..=500).contains(&l.max_gap_ms)
            || !(100..=4000).contains(&l.max_airborne_ms)
        {
            return Err("Course movement limits exceed supported verification bounds".into());
        }
        Ok(())
    }
}
impl Competition {
    pub fn active(&self, actor: u64) -> bool { self.attempts.contains_key(&actor) }
    /// Build static collision once on a trusted resource-world transition.
    pub fn set_terrain(&mut self, terrain: Option<&Terrain>) -> Result<()> {
        if self.world_revision.as_ref() == terrain.map(|t| &t.revision) {
            return Ok(());
        }
        let mut unavailable = None;
        let collision = terrain
            .map(|t| -> Result<Option<Collision>> {
                if t.triangles.is_empty()
                    || !position(t.spawn)
                    || !t.heading.is_finite()
                {
                    return Err("Competition terrain exceeds bounds or has invalid spawn".into());
                }
                if t.triangles.len()>262144 {
                    unavailable=Some("Course verification unsupported: terrain exceeds262144 triangles".into());
                    return Ok(None);
                }
                let mut vertices = Vec::with_capacity(t.triangles.len() * 3);
                let mut indices = Vec::with_capacity(t.triangles.len());
                for tri in t.triangles.iter() {
                    if !tri.iter().all(|p| position(*p)) {
                        return Err("Competition terrain contains invalid geometry".into());
                    }
                    let index = vertices.len() as u32;
                    vertices.extend(tri.iter().map(|p| Vector::new(p[0], p[1], p[2])));
                    indices.push([index, index + 1, index + 2]);
                }
                let mesh = SharedShape::trimesh(vertices, indices)
                    .map_err(|e| format!("Competition collision: {e}"))?;
                let collision = Collision {
                    mesh,
                    spawn: t.spawn,
                    heading: t.heading,
                };
                if !collision.clear(t.spawn, t.spawn) {
                    unavailable=Some("Course verification unsupported: approved spawn intersects movement envelope".into());
                    return Ok(None);
                }
                Ok(Some(collision))
            })
            .transpose()?.flatten();
        self.terrain = collision;
        self.world_revision = terrain.map(|t| t.revision.clone());
        self.unavailable = unavailable;
        for (actor, attempt) in std::mem::take(&mut self.attempts) {
            self.emit(
                &attempt,
                actor,
                json!({"kind":"cancelled","reason":"world_changed"}),
            );
        }
        Ok(())
    }
    pub fn sync_resources(&mut self, owners: BTreeMap<String, u64>) {
        self.courses
            .retain(|(owner, _), c| owners.get(owner) == Some(&c.generation));
        self.attempts
            .retain(|_, a| owners.get(&a.owner) == Some(&a.generation));
        self.events
            .retain(|e| owners.get(&e.resource) == Some(&e.generation));
        self.owners = owners;
    }
    pub fn command(
        &mut self,
        owner: &str,
        generation: u64,
        operation: Value,
        server: &mut Server,
    ) -> Result<Value> {
        if self.owners.get(owner) != Some(&generation) || generation == 0 {
            return Err("Competition resource generation is not active".into());
        }
        if serde_json::to_vec(&operation)
            .map_err(|e| e.to_string())?
            .len()
            > 16384
        {
            return Err("Competition command exceeds16KiB".into());
        }
        let command: Command = serde_json::from_value(operation)
            .map_err(|e| format!("Invalid competition command: {e}"))?;
        match command {
            Command::Define { course } => {
                course.validate()?;
                let key = (owner.to_owned(), course.name.clone());
                if !self.courses.contains_key(&key)
                    && (self.courses.len() >= MAX_COURSES
                        || self.courses.keys().filter(|(id, _)| id == owner).count() >= 4)
                {
                    return Err("Competition course capacity reached".into());
                }
                if self
                    .attempts
                    .values()
                    .any(|a| a.owner == owner && a.course == course.name)
                {
                    return Err("Cancel active attempts before redefining their course".into());
                }
                self.courses.insert(key, OwnedCourse { generation, course });
                Ok(json!({"defined":true}))
            }
            Command::Start { name, player } => {
                let actor = actor(&player)?;
                let course = &self
                    .courses
                    .get(&(owner.into(), name.clone()))
                    .ok_or("Unknown owned competition course")?
                    .course;
                let instance = decimal(&course.instance).unwrap();
                if self.attempts.contains_key(&actor) || self.attempts.len() >= MAX_ATTEMPTS {
                    return Err("Player already competing or attempt capacity reached".into());
                }
                if !server.resource_ready(actor) || server.instance_of(actor) != Some(instance) {
                    return Err("Competition player is not admitted to the course instance".into());
                }
                let terrain = self.terrain.as_ref().ok_or_else(|| {
                    self.unavailable
                        .clone()
                        .unwrap_or_else(|| "Competition requires a verified resource world".into())
                })?;
                let epoch = server.teleport_now(
                    actor,
                    TeleportDestination {
                        position: terrain.spawn,
                        heading: terrain.heading,
                        velocity: [0.; 3],
                        instance,
                    },
                )?;
                let now = server.now_ms();
                self.attempts.insert(
                    actor,
                    Attempt {
                        owner: owner.into(),
                        generation,
                        course: name,
                        epoch,
                        instance,
                        started: now,
                        received: now,
                        seq: None,
                        position: terrain.spawn,
                        velocity: [0.; 3],
                        airborne: None,
                        travel: VecDeque::new(),
                        checkpoint: 0,
                        pickups: BTreeSet::new(),
                        contacts: BTreeSet::new(),
                        score: 0,
                    },
                );
                Ok(json!({"started":true,"player":player,"movement_epoch":epoch.to_string()}))
            }
            Command::Cancel { player } => {
                let id = actor(&player)?;
                if self
                    .attempts
                    .get(&id)
                    .is_none_or(|a| a.owner != owner || a.generation != generation)
                {
                    return Err("Unknown owned competition attempt".into());
                }
                self.attempts.remove(&id);
                Ok(json!({"cancelled":true}))
            }
            Command::Remove { name } => {
                if !label(&name) {
                    return Err("Invalid competition course name".into());
                }
                self.courses.remove(&(owner.into(), name.clone()));
                self.attempts
                    .retain(|_, a| a.owner != owner || a.course != name);
                Ok(json!({"removed":true}))
            }
        }
    }
    /// Samples come only from the admitted authority and server receipt clock.
    pub fn step(&mut self, server: &mut Server, entities: &[Entity]) -> Vec<OwnedEvent> {
        // Admission resets can hide BODY observations. Check trusted authority
        // first so an old attempt cannot correct a newer approved destination.
        let cancelled: Vec<_> = self
            .attempts
            .iter()
            .filter(|(actor, attempt)| {
                server.movement_epoch_of(**actor) != Some(attempt.epoch)
                    || server.instance_of(**actor) != Some(attempt.instance)
            })
            .map(|(actor, _)| *actor)
            .collect();
        for actor in cancelled {
            let attempt = self.attempts.remove(&actor).unwrap();
            self.emit(
                &attempt,
                actor,
                json!({"kind":"cancelled","reason":"movement_epoch_changed"}),
            );
        }
        let now = server.now_ms();
        let players = server.competition_players();
        let corrections = self.observe(now, &players, entities);
        for (actor, instance) in corrections {
            if let Some(t) = &self.terrain {
                let _ = server.teleport_now(
                    actor,
                    TeleportDestination {
                        position: t.spawn,
                        heading: t.heading,
                        velocity: [0.; 3],
                        instance,
                    },
                );
            }
        }
        self.events.drain(..).collect()
    }
    fn observe(
        &mut self,
        now: u64,
        players: &[CompetitionPlayer],
        entities: &[Entity],
    ) -> Vec<(u64, u64)> {
        let mut corrections = vec![];
        for actor in self.attempts.keys().copied().collect::<Vec<_>>() {
            let mut attempt = self.attempts.remove(&actor).unwrap();
            let Some(course) = self
                .courses
                .get(&(attempt.owner.clone(), attempt.course.clone()))
            else {
                continue;
            };
            let Some(terrain) = &self.terrain else {
                continue;
            };
            let sample = players.iter().find(|p| p.actor == actor);
            let result = match sample {
                Some(sample) => {
                    advance(&mut attempt, &course.course, terrain, sample, entities, now)
                }
                None if now.saturating_sub(attempt.received) > course.course.limits.max_gap_ms => {
                    Err("observation_timeout")
                }
                None => Ok(false),
            };
            match result {
                Err("movement_epoch_changed")=>self.emit(&attempt,actor,json!({"kind":"cancelled","reason":"movement_epoch_changed"})),
                Err(reason)=>{corrections.push((actor,attempt.instance));self.emit(&attempt,actor,json!({"kind":"rejected","reason":reason,"score":0}));},
                Ok(true)=>self.emit(&attempt,actor,json!({"kind":"completed","score":attempt.score,"elapsed_ms":now.saturating_sub(attempt.started),"checkpoints":attempt.checkpoint,"pickups":attempt.pickups.len(),"contacts":attempt.contacts.len(),"verified_rules":"course-v1"})),
                Ok(false)=>{self.attempts.insert(actor,attempt);},
            }
        }
        corrections
    }
    fn emit(&mut self, attempt: &Attempt, actor: u64, mut value: Value) {
        value["player"] = json!(actor.to_string());
        value["course"] = json!(attempt.course);
        if self.events.len() >= 128 {
            self.events.pop_front();
        }
        self.events.push_back(OwnedEvent {
            resource: attempt.owner.clone(),
            generation: attempt.generation,
            value,
        });
    }
}
fn advance(
    a: &mut Attempt,
    c: &Course,
    terrain: &Collision,
    s: &CompetitionPlayer,
    entities: &[Entity],
    now: u64,
) -> std::result::Result<bool, &'static str> {
    if s.instance != a.instance || s.epoch != a.epoch {
        return Err("movement_epoch_changed");
    }
    if !position(s.position) || s.received > now || s.received < a.received {
        return Err("invalid_observation");
    }
    if now.saturating_sub(a.received) > c.limits.max_gap_ms {
        return Err("observation_timeout");
    }
    if a.seq == Some(s.seq) {
        return Ok(false);
    }
    let elapsed = s.received - a.received;
    // Coalesce samples below20ms; do not mint displacement tolerance per packet.
    if elapsed < 20 {
        return Ok(false);
    }
    if elapsed > c.limits.max_gap_ms {
        return Err("observation_gap");
    }
    let dt = elapsed as f32 * 0.001;
    let delta = sub(s.position, a.position);
    let distance = length(delta);
    let velocity = delta.map(|v| v / dt);
    if distance > c.limits.max_speed * dt + 0.08 {
        return Err("speed");
    }
    if length(sub(velocity, a.velocity)) > c.limits.max_acceleration * dt + 1.5 {
        return Err("acceleration");
    }
    while a
        .travel
        .front()
        .is_some_and(|(t, _)| s.received.saturating_sub(*t) > 1000)
    {
        a.travel.pop_front();
    }
    let window_start = a.travel.front().map_or(a.received, |(t, _)| *t);
    let travelled = a.travel.iter().map(|(_, d)| *d).sum::<f32>() + distance;
    if travelled > c.limits.max_speed * (s.received - window_start) as f32 * 0.001 + 0.15 {
        return Err("window_displacement");
    }
    if !terrain.clear(a.position, s.position) {
        return Err("terrain_collision");
    }
    if terrain.supported(s.position) {
        a.airborne = None;
    } else {
        let airborne = *a.airborne.get_or_insert(a.received);
        if s.received.saturating_sub(airborne) > c.limits.max_airborne_ms {
            return Err("unsupported_flight");
        }
    }
    let previous = a.position;
    a.travel.push_back((a.received, distance));
    while a.travel.len() > 64 {
        a.travel.pop_front();
    }
    a.received = s.received;
    a.seq = Some(s.seq);
    a.position = s.position;
    a.velocity = velocity;
    // One ordered checkpoint per fresh accepted sample prevents instant chains
    // of overlapping targets. All awards originate in server-owned definitions.
    if let Some(zone) = c.checkpoints.get(a.checkpoint) {
        if hits_zone(previous, s.position, zone) {
            a.checkpoint += 1;
            a.score += zone.points;
        }
    }
    for (i, zone) in c.pickups.iter().enumerate() {
        if !a.pickups.contains(&i) && hits_zone(previous, s.position, zone) {
            a.pickups.insert(i);
            a.score += zone.points;
        }
    }
    for (i, contact) in c.contacts.iter().enumerate() {
        if a.contacts.contains(&i) {
            continue;
        }
        if let Some(entity) = entities.iter().find(|e| {
            e.resource == a.owner
                && e.generation == a.generation
                && e.key == contact.entity
                && e.instance == a.instance
        }) {
            // Use the same support-relative capsule as terrain verification.
            if hits_entity(terrain.reference(previous).unwrap(), terrain.reference(s.position).unwrap(), entity) {
                a.contacts.insert(i);
                a.score += contact.points;
            }
        }
    }
    Ok(a.checkpoint == c.checkpoints.len()
        && a.pickups.len() == c.pickups.len()
        && a.contacts.len() == c.contacts.len())
}
impl Collision {
    fn clear(&self, from: [f32; 3], to: [f32; 3]) -> bool {
        let (Some(from), Some(to)) = (self.reference(from), self.reference(to)) else {
            return false;
        };
        let body = capsule();
        let pose = capsule_pose(from);
        if query::intersection_test(&pose, body.as_ref(), &Pose::IDENTITY, self.mesh.as_ref())
            .unwrap_or(true)
        {
            return false;
        }
        let velocity = vector(sub(to, from));
        query::cast_shapes(
            &pose,
            velocity,
            body.as_ref(),
            &Pose::IDENTITY,
            Vector::ZERO,
            self.mesh.as_ref(),
            ShapeCastOptions {
                max_time_of_impact: 1.,
                stop_at_penetration: true,
                ..Default::default()
            },
        )
        .is_ok_and(|hit| hit.is_none())
    }
    /// Resolve the animation origin against server terrain, never client state
    /// flags. Keep airborne roots unchanged; shallow native ground offsets move
    /// only the collision reference up to support. Deeper roots fail closed.
    fn reference(&self, mut position: [f32; 3]) -> Option<[f32; 3]> {
        if let Some(height) = self.support_height(position) {
            let depth = height - position[1];
            if depth > MAX_ROOT_BELOW_SUPPORT {
                return None;
            }
            if depth > 0. {
                position[1] = height;
            }
        }
        Some(position)
    }
    fn supported(&self, position: [f32; 3]) -> bool {
        self.support_height(position).is_some()
    }
    fn support_height(&self, position: [f32; 3]) -> Option<f32> {
        let ray = Ray::new(
            vector([position[0], position[1] + 0.3, position[2]]),
            Vector::NEG_Y,
        );
        self.mesh
            .cast_ray_and_get_normal(&Pose::IDENTITY, &ray, 1.8, true)
            .filter(|hit| hit.normal.y.abs() > 0.4)
            .map(|hit| position[1] + 0.3 - hit.time_of_impact)
    }
}
fn hits_entity(from: [f32; 3], to: [f32; 3], entity: &Entity) -> bool {
    // Only server-simulated bodies qualify; a leased client kinematic object is
    // not an independently simulated contact target.
    if entity.definition.body_type == BodyType::Kinematic {
        return false;
    }
    let target = match entity.definition.shape {
        Shape::Box { half_extents: p } => SharedShape::cuboid(p[0], p[1], p[2]),
        Shape::Sphere { radius } => SharedShape::ball(radius),
        Shape::Capsule {
            half_height,
            radius,
        } => SharedShape::capsule_y(half_height, radius),
    };
    let rotation = skate_dynamics::rapier3d::prelude::Rotation::from_xyzw(
        entity.rotation[0],
        entity.rotation[1],
        entity.rotation[2],
        entity.rotation[3],
    );
    let target_pose = Pose::from_parts(vector(entity.position), rotation);
    let body = capsule();
    query::cast_shapes(
        &capsule_pose(from),
        vector(sub(to, from)),
        body.as_ref(),
        &target_pose,
        Vector::ZERO,
        target.as_ref(),
        ShapeCastOptions::with_max_time_of_impact(1.),
    )
    .is_ok_and(|hit| hit.is_some())
}
fn capsule() -> SharedShape {
    SharedShape::capsule_y(0.5, 0.3)
}
fn capsule_pose(p: [f32; 3]) -> Pose {
    Pose::translation(p[0], p[1] + OFFSET, p[2])
}
fn hits_zone(from: [f32; 3], to: [f32; 3], zone: &Zone) -> bool {
    let direction = sub(to, from);
    let denominator = dot(direction, direction);
    let t = if denominator > 1e-8 {
        (dot(sub(zone.position, from), direction) / denominator).clamp(0., 1.)
    } else {
        0.
    };
    length(sub(
        std::array::from_fn(|i| from[i] + direction[i] * t),
        zone.position,
    )) <= zone.radius
}
fn label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn decimal(value: &str) -> Option<u64> {
    value.parse::<u64>().ok().filter(|n| n.to_string() == value)
}
fn actor(value: &str) -> Result<u64> {
    decimal(value)
        .filter(|n| *n > 0)
        .ok_or_else(|| "Player must be a canonical decimal actor ID".into())
}
fn position(p: [f32; 3]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() < 100000.)
}
fn vector(p: [f32; 3]) -> Vector {
    Vector::new(p[0], p[1], p[2])
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
fn length(v: [f32; 3]) -> f32 {
    dot(v, v).sqrt()
}
