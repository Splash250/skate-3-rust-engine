//! Transport-neutral shared-object contracts. Only the dedicated authority may
//! publish transforms; a controller lease never grants client physics authority.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_ENTITIES: usize = 256;
pub const INVENTORY_CHUNK: usize = 32;
pub const MAX_INSTANCES: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Shape {
    Box { half_extents: [f32; 3] },
    Sphere { radius: f32 },
    Capsule { half_height: f32, radius: f32 },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyType {
    #[default]
    Dynamic,
    Kinematic,
    Static,
}
fn mass() -> f32 {
    1.
}
fn friction() -> f32 {
    0.7
}
fn color() -> [f32; 4] {
    [0.3, 0.6, 0.9, 1.]
}
fn rotation() -> [f32; 4] {
    [0., 0., 0., 1.]
}
pub(crate) fn input_id<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        Text(String),
        Number(u64),
    }
    match Id::deserialize(decoder)? {
        Id::Number(value) if value <= 9_007_199_254_740_991 => Ok(value),
        Id::Text(value) => value
            .parse::<u64>()
            .ok()
            .filter(|id| id.to_string() == value)
            .ok_or_else(|| {
                serde::de::Error::custom("Entity identity must be canonical decimal u64")
            }),
        _ => Err(serde::de::Error::custom(
            "Large identities require decimal strings",
        )),
    }
}
fn optional_input_id<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Option<u64>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(decoder)?;
    value
        .map(|value| input_id(value).map_err(serde::de::Error::custom))
        .transpose()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    pub shape: Shape,
    #[serde(default)]
    pub body_type: BodyType,
    #[serde(default = "mass")]
    pub mass: f32,
    #[serde(default = "friction")]
    pub friction: f32,
    #[serde(default = "color")]
    pub color: [f32; 4],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub key: String,
    #[serde(default, deserialize_with = "input_id")]
    pub instance: u64,
    #[serde(default, deserialize_with = "optional_input_id")]
    pub controller: Option<u64>,
    #[serde(flatten)]
    pub definition: Definition,
    pub position: [f32; 3],
    #[serde(default = "rotation")]
    pub rotation: [f32; 4],
    #[serde(default)]
    pub velocity: [f32; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Spawn(Spawn),
    Remove {
        key: String,
    },
    Impulse {
        key: String,
        impulse: [f32; 3],
    },
    Velocity {
        key: String,
        velocity: [f32; 3],
    },
    Pose {
        key: String,
        position: [f32; 3],
        rotation: [f32; 4],
    },
    Transfer {
        key: String,
        #[serde(default, deserialize_with = "optional_input_id")]
        controller: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub id: u64,
    pub resource: String,
    pub generation: u64,
    pub key: String,
    pub instance: u64,
    pub controller: Option<u64>,
    pub definition: Definition,
    /// Simulation sample sequence; strictly increasing across all object updates.
    pub tick: u64,
    /// Discontinuous host pose changes reset interpolation and collision history.
    pub epoch: u64,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub angular: [f32; 3],
}
/// Validated, recent owner observation used only as a server-side contact proxy.
#[derive(Clone, Debug)]
pub struct Player {
    pub actor: u64,
    pub instance: u64,
    pub epoch: u64,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
}
impl Definition {
    pub fn valid(&self) -> bool {
        let dimension = |x: f32| x.is_finite() && (0.01..=100.).contains(&x);
        let shape = match self.shape {
            Shape::Box { half_extents } => half_extents.into_iter().all(dimension),
            Shape::Sphere { radius } => dimension(radius),
            Shape::Capsule {
                half_height,
                radius,
            } => dimension(half_height) && dimension(radius),
        };
        shape
            && self.mass.is_finite()
            && (0.01..=100_000.).contains(&self.mass)
            && self.friction.is_finite()
            && (0. ..=2.).contains(&self.friction)
            && self
                .color
                .iter()
                .all(|c| c.is_finite() && (0. ..=1.).contains(c))
    }
}
pub fn label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}
pub fn vector(value: [f32; 3], limit: f32) -> bool {
    value.iter().all(|v| v.is_finite() && v.abs() <= limit)
}
pub fn quaternion(value: [f32; 4]) -> bool {
    value.iter().all(|v| v.is_finite())
        && (0.99..=1.01).contains(&value.iter().map(|v| v * v).sum::<f32>())
}
impl Spawn {
    pub fn valid(&self) -> bool {
        label(&self.key)
            && self.controller != Some(0)
            && self.definition.valid()
            && vector(self.position, 99_999.)
            && quaternion(self.rotation)
            && vector(self.velocity, 200.)
    }
}
impl Entity {
    /// Script-facing observations preserve u64 identity precision in JavaScript.
    pub fn observation(&self) -> serde_json::Value {
        serde_json::json!({"id":self.id.to_string(),"resource":self.resource,"generation":self.generation.to_string(),
            "key":self.key,"instance":self.instance.to_string(),"controller":self.controller.map(|id|id.to_string()),
            "tick":self.tick.to_string(),"epoch":self.epoch.to_string(),"position":self.position,"rotation":self.rotation,
            "velocity":self.velocity,"angular":self.angular,"definition":self.definition})
    }
    pub fn valid(&self) -> bool {
        self.id != 0
            && self.generation != 0
            && self.tick != 0
            && self.epoch != 0
            && label(&self.resource)
            && label(&self.key)
            && self.controller != Some(0)
            && self.definition.valid()
            && vector(self.position, 99_999.)
            && quaternion(self.rotation)
            && vector(self.velocity, 250.)
            && vector(self.angular, 250.)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    Inventory {
        epoch: u64,
        instance: u64,
        revision: u64,
        part: usize,
        total: usize,
        ids: Vec<u64>,
    },
    Upsert {
        epoch: u64,
        instance: u64,
        revision: u64,
        entity: Entity,
    },
}
#[derive(Default)]
pub struct Replica {
    scope: (u64, u64),
    revision: u64,
    pending_revision: u64,
    parts: Vec<Option<Vec<u64>>>,
    members: BTreeSet<u64>,
    entities: BTreeMap<u64, Entity>,
}
impl Replica {
    pub fn reset(&mut self, epoch: u64, instance: u64) {
        if self.scope != (epoch, instance) {
            *self = Self {
                scope: (epoch, instance),
                ..Default::default()
            };
        }
    }
    pub fn entities(&self) -> &BTreeMap<u64, Entity> {
        &self.entities
    }
    pub fn receive(&mut self, frame: Frame) -> bool {
        match frame {
            Frame::Inventory {
                epoch,
                instance,
                revision,
                part,
                total,
                ids,
            } => {
                if epoch == 0
                    || self.scope != (epoch, instance)
                    || revision == 0
                    || revision <= self.revision
                    || revision < self.pending_revision
                    || total == 0
                    || total > MAX_ENTITIES.div_ceil(INVENTORY_CHUNK)
                    || part >= total
                    || ids.len() > INVENTORY_CHUNK
                    || ids.contains(&0)
                    || (part + 1 < total && ids.len() != INVENTORY_CHUNK)
                {
                    return false;
                }
                if self.pending_revision != revision {
                    self.pending_revision = revision;
                    self.parts = vec![None; total];
                }
                if self.parts.len() != total {
                    return false;
                }
                if self.parts[part].as_ref().is_some_and(|old| old != &ids) {
                    return false;
                }
                self.parts[part] = Some(ids);
                if self.parts.iter().any(Option::is_none) {
                    return true;
                }
                let ids: Vec<_> = self.parts.iter().flatten().flatten().copied().collect();
                let members: BTreeSet<_> = ids.iter().copied().collect();
                if ids.len() != members.len() || ids.len() > MAX_ENTITIES {
                    return false;
                }
                self.members = members;
                self.revision = revision;
                self.entities.retain(|id, _| self.members.contains(id));
                self.parts.clear();
                true
            }
            Frame::Upsert {
                epoch,
                instance,
                revision,
                entity,
            } => {
                if self.scope != (epoch, instance)
                    || revision != self.revision
                    || revision == 0
                    || entity.instance != instance
                    || !self.members.contains(&entity.id)
                    || !entity.valid()
                    || self.entities.get(&entity.id).is_some_and(|old| {
                        old.tick >= entity.tick
                            || old.resource != entity.resource
                            || old.generation != entity.generation
                            || old.key != entity.key
                            || old.definition != entity.definition
                            || old.epoch > entity.epoch
                    })
                {
                    return false;
                }
                self.entities.insert(entity.id, entity);
                true
            }
        }
    }
}

/// One bounded scheduler per authenticated recipient. Membership revisions stay
/// stable through retries; transform samples may advance without invalidating an
/// in-flight inventory. No per-deleted-object tombstones accumulate.
pub(crate) struct Publisher {
    scope: (u64, u64),
    revision: u64,
    ids: Vec<u64>,
    inventory_part: usize,
    last_inventory: u64,
    round: usize,
    sent: BTreeMap<u64, (u64, u64)>,
    credits: f64,
    last: u64,
}
impl Default for Publisher {
    fn default() -> Self {
        Self {
            scope: (0, 0),
            revision: 0,
            ids: Vec::new(),
            inventory_part: 0,
            last_inventory: 0,
            round: 0,
            sent: BTreeMap::new(),
            credits: 2400.,
            last: 0,
        }
    }
}
impl Publisher {
    pub(crate) fn frames(
        &mut self,
        epoch: u64,
        instance: u64,
        objects: &[&Entity],
        now: u64,
    ) -> Vec<Vec<u8>> {
        let dt = now.saturating_sub(self.last).min(1000) as f64 / 1000.;
        self.last = now;
        self.credits = (self.credits + dt * 24_000.).min(2400.);
        let ids: Vec<_> = objects.iter().map(|e| e.id).collect();
        let parts = ids.len().div_ceil(INVENTORY_CHUNK).max(1);
        if self.scope != (epoch, instance) || self.ids != ids {
            self.scope = (epoch, instance);
            self.ids = ids;
            self.revision = self
                .revision
                .checked_add(1)
                .expect("Entity inventory sequence exhausted");
            self.inventory_part = 0;
            self.round = 0;
            self.sent.retain(|id, _| self.ids.contains(id));
        } else if self.inventory_part == parts && now.saturating_sub(self.last_inventory) >= 250 {
            self.inventory_part = 0;
        }
        let mut output = Vec::new();
        while self.inventory_part < parts {
            let start = self.inventory_part * INVENTORY_CHUNK;
            let ids = self.ids[start..(start + INVENTORY_CHUNK).min(self.ids.len())].to_vec();
            let frame = Frame::Inventory {
                epoch,
                instance,
                revision: self.revision,
                part: self.inventory_part,
                total: parts,
                ids,
            };
            if !self.push(frame, &mut output) {
                return output;
            }
            self.inventory_part += 1;
            self.last_inventory = now;
        }
        for offset in 0..objects.len() {
            let index = (self.round + offset) % objects.len();
            let entity = objects[index];
            if self.sent.get(&entity.id).is_some_and(|(tick, at)| {
                now.saturating_sub(*at) < if *tick == entity.tick { 250 } else { 50 }
            }) {
                continue;
            }
            if !self.push(
                Frame::Upsert {
                    epoch,
                    instance,
                    revision: self.revision,
                    entity: entity.clone(),
                },
                &mut output,
            ) {
                self.round = index;
                break;
            }
            self.sent.insert(entity.id, (entity.tick, now));
        }
        output
    }
    fn push(&mut self, frame: Frame, output: &mut Vec<Vec<u8>>) -> bool {
        let Ok(bytes) = serde_json::to_vec(&frame) else {
            return false;
        };
        let size = bytes.len() + crate::packed::HEADER;
        if size > crate::packed::MTU || size as f64 > self.credits {
            return false;
        }
        self.credits -= size as f64;
        output.push(bytes);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entity(id: u64, tick: u64) -> Entity {
        Entity {
            id,
            resource: "objects".into(),
            generation: 1,
            key: format!("box{id}"),
            instance: 0,
            controller: None,
            definition: Definition {
                shape: Shape::Box {
                    half_extents: [0.5; 3],
                },
                body_type: BodyType::Dynamic,
                mass: 5.,
                friction: 0.7,
                color: color(),
            },
            tick,
            epoch: 1,
            position: [0., 1., 0.],
            rotation: rotation(),
            velocity: [0.; 3],
            angular: [0.; 3],
        }
    }
    fn inventory(revision: u64, ids: Vec<u64>) -> Frame {
        Frame::Inventory {
            epoch: 7,
            instance: 0,
            revision,
            part: 0,
            total: 1,
            ids,
        }
    }
    fn upsert(revision: u64, entity: Entity) -> Frame {
        Frame::Upsert {
            epoch: 7,
            instance: 0,
            revision,
            entity,
        }
    }
    #[test]
    fn inventory_retires_objects_and_delayed_updates_cannot_resurrect_them() {
        let mut replica = Replica::default();
        replica.reset(7, 0);
        assert!(replica.receive(inventory(1, vec![1, 2])));
        assert!(replica.receive(upsert(1, entity(1, 1))));
        assert!(replica.receive(upsert(1, entity(2, 1))));
        assert!(replica.receive(inventory(2, vec![2])));
        assert!(!replica.entities().contains_key(&1));
        assert!(!replica.receive(upsert(1, entity(1, 99))));
        assert!(!replica.receive(inventory(1, vec![1, 2])));
        assert_eq!(replica.entities().len(), 1);
    }
    #[test]
    fn scope_changes_clear_replicas_and_reject_inflight_previous_instance_frames() {
        let mut replica = Replica::default();
        replica.reset(7, 0);
        replica.receive(inventory(1, vec![1]));
        replica.receive(upsert(1, entity(1, 1)));
        assert_eq!(replica.entities().len(), 1);
        replica.reset(8, 2);
        assert!(replica.entities().is_empty());
        assert!(!replica.receive(inventory(1, vec![1])));
        assert!(!replica.receive(upsert(1, entity(1, 2))));
    }
    #[test]
    fn definition_and_snapshot_validation_reject_nonfinite_and_oversized_geometry() {
        let mut state = entity(1, 1);
        assert!(state.valid());
        state.velocity[0] = f32::NAN;
        assert!(!state.valid());
        state.velocity[0] = 0.;
        state.definition.shape = Shape::Box {
            half_extents: [1000.; 3],
        };
        assert!(!state.valid());
    }
    #[test]
    fn script_commands_keep_u64_identities_exact_and_reject_unknown_fields() {
        let command:Command=serde_json::from_value(serde_json::json!({"op":"spawn","key":"box","instance":"18446744073709551615","controller":"18446744073709551614","shape":{"type":"box","half_extents":[0.5,0.5,0.5]},"position":[0,1,0]})).unwrap();
        let Command::Spawn(spawn) = command else {
            panic!()
        };
        assert_eq!(spawn.instance, u64::MAX);
        assert_eq!(spawn.controller, Some(u64::MAX - 1));
        assert!(spawn.valid());
        assert!(
            serde_json::from_value::<Command>(
                serde_json::json!({"op":"transfer","key":"box","controller":9007199254740992u64})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<Command>(
                serde_json::json!({"op":"remove","key":"box","owner":"other"})
            )
            .is_err()
        );
    }
    #[test]
    fn full_entity_budget_recovers_fragmented_inventory_and_fair_updates_under_loss() {
        let objects: Vec<_> = (1..=MAX_ENTITIES as u64)
            .map(|id| {
                let mut e = entity(id, 1);
                e.resource = "r".repeat(64);
                e.key = format!("{id:064}");
                e
            })
            .collect();
        let entries: Vec<_> = objects.iter().collect();
        let mut publisher = Publisher::default();
        let mut replica = Replica::default();
        replica.reset(7, 0);
        let mut packets = 0;
        for now in (0..20_000).step_by(10) {
            for bytes in publisher.frames(7, 0, &entries, now) {
                assert!(bytes.len() + crate::packed::HEADER <= crate::packed::MTU);
                packets += 1;
                if packets % 13 != 0 {
                    replica.receive(serde_json::from_slice(&bytes).unwrap());
                }
            }
        }
        assert_eq!(replica.entities().len(), MAX_ENTITIES);
        assert!(publisher.sent.len() <= MAX_ENTITIES);
    }
}
