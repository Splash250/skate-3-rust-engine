//! Headless session authority for owner-predicted movement and shared interactions.
//!
//! The server validates snapshots and resolves conservative player capsules. It
//! does not run the recovered world or articulated skeleton simulation. Owner
//! delta histories remain untouched; reliable effects reconcile the owner.
use crate::{
    lobby::{Info, Outgoing, Revision, Session, Stats},
    packed::{BodyState, Packed},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const SESSION: u64 = 48_031_030;
pub const MAX_PLAYERS: usize = 64;
pub const GAMEPLAY_KEY: &str = "builtin:gameplay";
pub const SHOVE_KEY: &str = "builtin:shove";
pub const RESPAWN_DESTINATION: &str = "builtin:respawn";
pub const TELEPORT_KEY: &str = "builtin:teleport";
pub const EFFECT_ACK_KEY: &str = "builtin:effect-ack";
pub const SHOVE_REACH: f32 = 2.5;
pub const SHOVE_FACING: f32 = 0.25;
const EFFECT_PREFIX: &str = "builtin:effects:";
const FRESH_MS: u64 = 250;
const SHOVE_COOLDOWN_MS: u64 = 800;
const MAX_PENDING: usize = 64;
const BATCH_SIZE: usize = 4;

pub fn effects_key(actor: u64) -> String {
    format!("{EFFECT_PREFIX}{actor}")
}
pub(crate) fn server_key(key: &str) -> bool {
    crate::resources::is_server_key(key) || key.strip_prefix(crate::native_authority::AUTHORITY_PREFIX)
        .is_some_and(|id| id.parse::<u64>().is_ok_and(|id| id != 0)) || key.strip_prefix(EFFECT_PREFIX)
        .is_some_and(|id| id.parse::<u64>().is_ok_and(|id| id != 0))
}

/// A trusted server destination. World collision/asset admission remains the host's
/// responsibility; the protocol enforces finite coordinates and bounded velocity.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeleportDestination {
    pub position: [f32; 3],
    pub heading: f32,
    pub velocity: [f32; 3],
    pub instance: u64,
}
impl TeleportDestination {
    pub(crate) fn relocate(&self, body: &mut BodyState) {
        let [x,y,z,w] = body.root.q;
        let next = [0., (self.heading * 0.5).sin(), 0., (self.heading * 0.5).cos()];
        let rotation = multiply(next, [-x,-y,-z,w]);
        for part in &mut body.bodies {
            let offset = sub(part.pose.p, body.root.p);
            let rotated = multiply(multiply(rotation,[offset[0],offset[1],offset[2],0.]),[-rotation[0],-rotation[1],-rotation[2],rotation[3]]);
            part.pose.p = std::array::from_fn(|i| self.position[i] + rotated[i]);
            part.pose.q = multiply(rotation,part.pose.q);
            part.velocity = self.velocity; part.angular = [0.;3];
        }
        body.root = crate::Pose { p:self.position, q:next };
    }
    pub fn valid(&self) -> bool {
        self.position.iter().all(|v| v.is_finite() && v.abs() < 100_000.)
            && self.heading.is_finite() && self.heading.abs() <= std::f32::consts::TAU
            && self.velocity.iter().all(|v| v.is_finite() && v.abs() <= 200.)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeleportRequest {
    pub epoch: u64,
    pub id: u64,
    pub destination: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementReset {
    pub actor: u64,
    pub epoch: u64,
    pub destination: TeleportDestination,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerMode {
    #[default]
    Skating,
    Offboard,
    Ragdoll,
}

/// Owner-reported presentation and scoring metadata. Scores are not verified by
/// authoritative skating simulation and must not be used as trusted rankings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gameplay {
    pub mode: PlayerMode,
    pub trick_seq: u64,
    pub trick: String,
    pub landed_seq: u64,
    pub landed_trick: String,
    pub bail_seq: u64,
    pub sequence_score: i64,
    pub line_score: i64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShoveRequest {
    pub epoch: u64,
    pub id: u64,
    pub target: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectKind {
    Collision,
    Shove,
    AttackAccepted,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    /// Monotonic per recipient, with no gaps within an epoch.
    pub id: u64,
    pub source: u64,
    /// Recipient. For AttackAccepted this is the attacking player.
    pub target: u64,
    pub delta_velocity: [f32; 3],
    pub kind: EffectKind,
    /// Contact location; AttackAccepted carries the victim's world position.
    pub position: [f32; 3],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectBatch {
    pub epoch: u64,
    pub effects: Vec<Effect>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectAck {
    pub epoch: u64,
    pub through: u64,
}

/// Only feed batches read from the dedicated control actor's application record.
/// Reset on connection/host identity changes. Publish ack only after applying the
/// returned effects, so receipt acknowledgements cannot discard unapplied hits.
#[derive(Default)]
pub struct ClientEffects {
    ack: EffectAck,
}
impl ClientEffects {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn ack(&self) -> EffectAck {
        self.ack
    }
    pub fn consume(&mut self, batch: &EffectBatch) -> Vec<Effect> {
        if batch.epoch == 0
            || batch.epoch < self.ack.epoch
            || batch.effects.len() > BATCH_SIZE
            || !batch.effects.iter().all(|effect| {
                effect.id != 0
                    && effect.source != 0
                    && effect.target != 0
                    && effect
                        .delta_velocity
                        .iter()
                        .all(|v| v.is_finite() && v.abs() <= 16.)
                    && effect
                        .position
                        .iter()
                        .all(|v| v.is_finite() && v.abs() < 100_000.)
            })
        {
            return vec![];
        }
        if batch.epoch != self.ack.epoch {
            // A fresh connection starts at effect 1; never adopt a partial queue.
            if batch.effects.first().is_some_and(|e| e.id != 1) {
                return vec![];
            }
            self.ack = EffectAck {
                epoch: batch.epoch,
                through: 0,
            };
        }
        let mut effects = Vec::new();
        for effect in &batch.effects {
            if effect.id <= self.ack.through {
                continue;
            }
            if Some(effect.id) != self.ack.through.checked_add(1) {
                break;
            }
            self.ack.through = effect.id;
            effects.push(effect.clone());
        }
        effects
    }
}

fn label(value: &str, max: usize) -> bool {
    value.len() <= max && !value.chars().any(char::is_control)
}
pub(crate) fn valid_client_application(key: &str, bytes: &[u8]) -> bool {
    if bytes.len() > crate::lobby::MAX_APP_VALUE {
        return false;
    }
    match key {
        "mp:name" => std::str::from_utf8(bytes)
            .is_ok_and(|name| label(name, 64) && name.chars().count() <= 16),
        "mp:ping" => bytes
            .try_into()
            .ok()
            .is_some_and(|value: [u8; 8]| u64::from_le_bytes(value) <= 60_000),
        GAMEPLAY_KEY => serde_json::from_slice::<Gameplay>(bytes).is_ok_and(|state| {
            label(&state.trick, 128)
                && label(&state.landed_trick, 128)
                && (0..=i32::MAX as i64).contains(&state.sequence_score)
                && (0..=i32::MAX as i64).contains(&state.line_score)
        }),
        crate::native_authority::INPUT_KEY => crate::native_authority::InputPacket::decode(bytes).is_ok(),
        TELEPORT_KEY => serde_json::from_slice::<TeleportRequest>(bytes).is_ok_and(|request|
            request.epoch != 0 && request.id != 0 && !request.destination.is_empty() && label(&request.destination, 96)),
        SHOVE_KEY => serde_json::from_slice::<ShoveRequest>(bytes)
            .is_ok_and(|request| request.epoch != 0 && request.id != 0 && request.target != 0),
        crate::resources::CLIENT_KEY => crate::resources::valid_client(bytes),
        EFFECT_ACK_KEY => {
            serde_json::from_slice::<EffectAck>(bytes).is_ok_and(|ack| ack.epoch != 0)
        }
        _ => false,
    }
}

pub(crate) fn plausible_body(state: &Packed, previous: Option<&Revision>, now: u64) -> bool {
    let Some(body) = state.unpack_body() else {
        return false;
    };
    let Some(previous) = previous else {
        return true;
    };
    if state.captured < previous.state.captured
        || now.saturating_sub(previous.received)
            > state
                .captured
                .saturating_sub(previous.state.captured)
                .saturating_add(FRESH_MS)
    {
        return false;
    }
    let Some(old) = previous.state.unpack_body() else {
        return false;
    };
    // Arrival time prevents a forged source timestamp granting unlimited travel.
    // Published physical speed preserves the game's legitimate high-speed moves.
    let elapsed = now.saturating_sub(previous.received).min(2000) as f32 / 1000.;
    let speed = old
        .bodies
        .iter()
        .chain(&body.bodies)
        .map(|body| length(body.velocity))
        .fold(0., f32::max);
    length(sub(body.root.p, old.root.p)) <= 10. + speed * (elapsed + 0.1) * 1.25
}

#[derive(Clone, Debug)]
pub struct Config {
    pub session: u64,
    pub server_id: u64,
    pub map: u64,
    pub max_players: usize,
}
struct Player {
    epoch: u64,
    effect_epoch: u64,
    spawn: Option<TeleportDestination>,
    world_spawn_pending: bool,
    next_effect: u64,
    sent_through: u64,
    acked: u64,
    effects: VecDeque<Effect>,
    last_request: u64,
    last_teleport: u64,
    last_teleport_at: Option<u64>,
    last_shove: Option<u64>,
}
impl Player {
    fn new(epoch: u64) -> Self {
        Self {
            epoch,
            effect_epoch: epoch,
            spawn: None,
            world_spawn_pending: false,
            next_effect: 1,
            sent_through: 0,
            acked: 0,
            effects: VecDeque::new(),
            last_request: 0,
            last_teleport: 0,
            last_teleport_at: None,
            last_shove: None,
        }
    }
}

/// A fresh accepted owner sample for explicitly server-verified competitions.
/// Sequence and receive clock distinguish new observations from repeated reads.
#[derive(Clone,Copy,Debug)]
pub struct CompetitionPlayer {
    pub actor:u64,pub instance:u64,pub epoch:u64,pub position:[f32;3],pub seq:u32,pub received:u64,
}

pub struct Server {
    session: Session,
    players: BTreeMap<u64, Player>,
    overlaps: BTreeSet<(u64, u64)>,
    last_contact: BTreeMap<(u64, u64), u64>,
    blocked: BTreeMap<u64, u64>,
    epoch: u64,
    resources: crate::resources::Authority,
    destinations: BTreeMap<String, TeleportDestination>,
    base_world_spawn: Option<TeleportDestination>,
    world_spawn: Option<TeleportDestination>,
    resource_readmission_resets: BTreeMap<u64, (u64, u64)>,
}
impl Server {
    /// Explicitly permit player requests to this destination. No destinations
    /// are enabled by default; ordinary players cannot choose arbitrary points.
    pub fn allow_teleport_destination(&mut self, id: String, destination: TeleportDestination) -> Result<(), String> {
        if id.is_empty() || id == RESPAWN_DESTINATION || !label(&id, 96) || !destination.valid() || (self.destinations.len() >= 256 && !self.destinations.contains_key(&id)) {
            return Err("Invalid or excessive teleport destination".into());
        }
        self.destinations.insert(id, destination);
        Ok(())
    }
    pub fn revoke_teleport_destination(&mut self, id: &str) { self.destinations.remove(id); }
    pub fn instance_of(&self, actor: u64) -> Option<u64> { self.session.actors.get(&actor).map(|a| a.instance) }
    /// Trusted authority even while resource admission hides movement samples.
    pub fn movement_epoch_of(&self, actor: u64) -> Option<u64> { self.session.actors.get(&actor).map(|a| a.movement_epoch) }
    pub fn native_input(&self, actor: u64) -> Option<crate::native_authority::InputPacket> {
        if !self.resource_ready(actor) { return None; }
        let record = self.session.actors.get(&actor)?.application.get(crate::native_authority::INPUT_KEY)?;
        let packet = crate::native_authority::InputPacket::decode(&record.value).ok()?;
        (packet.valid() && packet.inputs.iter().all(|i| Some(i.epoch) == self.movement_epoch_of(actor))).then_some(packet)
    }
    pub fn publish_native(&mut self, actor: u64, state: crate::native_authority::State) -> Result<(), String> {
        if !self.players.contains_key(&actor) || Some(state.admission.epoch) != self.movement_epoch_of(actor)
            || Some(state.admission.instance) != self.instance_of(actor) {
            return Err("Native authority player epoch or instance changed".into());
        }
        let value = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
        if !self.session.publish_application(&crate::native_authority::state_key(actor), value, self.now_ms()) {
            return Err("Native authority publication exceeds application bounds".into());
        }
        Ok(())
    }
    pub fn clear_native(&mut self, actor: u64) {
        self.session.actors.get_mut(&self.session.local).unwrap().application.remove(&crate::native_authority::state_key(actor));
    }
    pub fn native_instance_solitary(&self, actor: u64, instance: u64) -> bool {
        self.players.keys().all(|id| *id == actor || self.instance_of(*id) != Some(instance))
    }
    /// Trusted host moderation. Revoke any account/session credential alongside
    /// this removal; the core backs off the current endpoint for five seconds.
    pub fn kick(&mut self, actor:u64, now:u64)->bool {
        if !self.players.contains_key(&actor) {return false;}
        self.disconnect_abusive(actor,now); self.sync_players(); true
    }
    pub fn peer_for_actor(&self, actor:u64)->Option<u64> {self.session.peer_for_actor(actor)}
    /// Trusted host snapshots; clients have no corresponding update operation.
    pub fn publish_entities(&mut self, objects: Vec<crate::entities::Entity>) -> Result<(),String> {
        self.session.publish_entities(objects.clone())?;self.resources.entities(&objects);Ok(())
    }
    pub fn entity_players(&self) -> Vec<crate::entities::Player> {
        self.players.keys().filter_map(|&actor| {
            let body=self.body(actor,self.session.service_clock())?;
            let state=&self.session.actors[&actor];
            Some(crate::entities::Player {actor,instance:state.instance,epoch:state.movement_epoch,
                position:body.root.p,rotation:body.root.q,velocity:self.entity_root_velocity(actor,&body)})
        }).collect()
    }
    /// The coarse contact capsule follows the animation root, not an articulated
    /// wheel that may already be bouncing in the opposite direction. This only
    /// derives its bounded proxy velocity; it does not verify full-rig movement.
    fn entity_root_velocity(&self, actor: u64, current: &BodyState) -> [f32;3] {
        let history=&self.session.actors[&actor].body.history;
        let mut recent=history.iter().rev();
        let Some(latest)=recent.next() else {return [0.;3]};
        let Some(previous)=recent.next() else {return [0.;3]};
        // Epoch changes and readmission clear this history. A reset's synthetic
        // seq=0 placement carries an old capture clock, never a motion baseline.
        if latest.seq==0 || previous.seq==0 || self.now_ms().saturating_sub(previous.received)>FRESH_MS {
            return [0.;3];
        }
        let Some(captured)=latest.state.captured.checked_sub(previous.state.captured) else {return [0.;3]};
        let Some(received)=latest.received.checked_sub(previous.received) else {return [0.;3]};
        if captured==0 || captured>FRESH_MS || received>FRESH_MS {return [0.;3];}
        // Source time survives packet bunching; arrival time prevents a tiny
        // client timestamp from amplifying the observed displacement.
        let span=captured.max(received) as f32/1000.;
        let Some(old)=previous.state.unpack_body() else {return [0.;3]};
        let rig_speed=old.bodies.iter().chain(&current.bodies)
            .map(|body|length(body.velocity)).fold(0.,f32::max);
        let delta=sub(current.root.p,old.root.p);
        let distance=length(delta);
        // Match the conservative short-offset envelope used by player contact:
        // a tolerated discontinuity does not imply motion through nearby objects.
        if distance>1.+rig_speed*(span+0.02)*1.5 {return [0.;3];}
        let speed=distance/span;
        if !speed.is_finite() || speed==0. {return [0.;3];}
        // An owner root remains an observation, not new authority to add energy.
        // Do not exceed observed rig speed or the shared-object velocity bound.
        scale(delta,rig_speed.min(200.).min(speed)/distance)
    }
    pub fn now_ms(&self)->u64 {self.session.service_clock()}
    pub fn competition_players(&self)->Vec<CompetitionPlayer> {
        self.players.keys().filter_map(|&id| {
            let body=self.body(id,self.now_ms())?;
            let actor=&self.session.actors[&id];let sample=actor.body.latest()?;
            Some(CompetitionPlayer {actor:id,instance:actor.instance,epoch:actor.movement_epoch,
                position:body.root.p,seq:sample.seq,received:sample.received})
        }).collect()
    }
    pub fn entity_observations(&self)->serde_json::Value {self.session.entity_observations()}
    /// Live targets for pruning obsolete server VM private state. Instance state
    /// is intentionally independent of currently connected players.
    pub fn resource_scope_targets(&self)->(Vec<u64>,Vec<(String,u64,u64)>) {self.resources.scope_targets()}
    pub fn teleport_now(&mut self, actor: u64, destination: TeleportDestination) -> Result<u64,String> {
        self.teleport(actor, destination, self.session.service_clock())
    }
    /// Trusted host operation; capability checks belong to the resource host.
    pub fn teleport(&mut self, actor: u64, destination: TeleportDestination, now: u64) -> Result<u64, String> {
        if !destination.valid() || !self.players.contains_key(&actor) || !self.resource_ready(actor) {
            return Err("Invalid destination or unavailable player".into());
        }
        let instance_changed=self.instance_of(actor)!=Some(destination.instance);
        self.session.advance_movement_epoch(self.epoch);
        let epoch = self.session.reset_movement(actor, destination, now)?;
        self.epoch = self.epoch.max(epoch);
        self.overlaps.retain(|(a,b)| *a != actor && *b != actor);
        self.last_contact.retain(|(a,b),_| *a != actor && *b != actor);
        // Pre-relocation impulses and requests must never act in the new instance.
        let player = self.players.get_mut(&actor).unwrap();
        player.spawn = Some(TeleportDestination { velocity:[0.;3], ..destination });
        player.effects.clear();
        player.effect_epoch = epoch;
        player.next_effect = 1;
        player.acked = 0;
        player.sent_through = 0;
        player.last_request = 0;
        if instance_changed && self.resources.active() {
            player.epoch=epoch;
            self.resources.add(actor,epoch);
            self.resources.context(actor,destination.instance,Some(destination.position));
            self.session.resource_admission(actor,false);
            // Replaces the old application snapshot immediately. Earlier
            // packets are rejected by the client's reset-derived epoch floor.
            if let Some(bytes)=self.resources.record(actor) {self.session.publish_application(&crate::resources::server_key(actor),bytes,now);}
        }
        Ok(epoch)
    }
    fn teleports(&mut self, now: u64) {
        // Admission already accepts an initial owner body. Retain that baseline
        // as the default recovery point; later trusted host travel replaces it.
        // A recovery request never contains a client-proposed destination.
        for (&id, player) in &mut self.players {
            if player.spawn.is_none() {
                if let Some(body) = self.session.actors[&id].body.latest().and_then(|r| r.state.unpack_body()) {
                    let [x,y,z,w] = body.root.q;
                    player.spawn = Some(TeleportDestination {
                        position:body.root.p, heading:(2.*(w*y+x*z)).atan2(1.-2.*(x*x+y*y)),
                        velocity:[0.;3], instance:self.session.actors[&id].instance,
                    });
                }
            }
        }
        let requests: Vec<_> = self.players.keys().filter_map(|&id| {
            let value = &self.session.actors[&id].application.get(TELEPORT_KEY)?.value;
            Some((id, serde_json::from_slice::<TeleportRequest>(value).ok()?))
        }).collect();
        for (actor, request) in requests {
            let player = self.players.get_mut(&actor).unwrap();
            if request.epoch != self.session.actors[&actor].movement_epoch || request.id <= player.last_teleport { continue; }
            let destination = if request.destination == RESPAWN_DESTINATION {
                // The initial reliable request can precede its first movement
                // keyframe; keep waiting rather than consume and strand it.
                let Some(spawn) = player.spawn else {continue;};
                Some(spawn)
            } else { self.destinations.get(&request.destination).copied() };
            if player.last_teleport_at.is_some_and(|at| now.saturating_sub(at) < 500) { continue; }
            player.last_teleport = request.id;
            if let Some(destination) = destination {
                player.last_teleport_at = Some(now);
                let _ = self.teleport(actor, destination, now);
            }
        }
    }
    /// Trusted host bootstrap metadata, never a client observation. Used only
    /// when removing a required world; ordinary initial admission is unchanged.
    pub fn set_base_world_spawn(&mut self, spawn: TeleportDestination) -> Result<(), String> {
        if !spawn.valid() {return Err("Invalid base-world spawn".into());}
        self.base_world_spawn=Some(TeleportDestination {velocity:[0.;3],..spawn});
        Ok(())
    }
    /// Configure the trusted spawn of the validated required world before
    /// advertising its revision. Removal resets existing players to the trusted
    /// base world after readiness; their private instance remains unchanged.
    pub fn set_world_spawn(&mut self, spawn:Option<TeleportDestination>)->Result<(),String> {
        if spawn.is_some_and(|s|!s.valid()) {return Err("Invalid required-world spawn".into());}
        let removed_world=self.world_spawn.is_some() && spawn.is_none();
        if removed_world && self.base_world_spawn.is_none() {return Err("Required-world removal needs a trusted base-world spawn".into());}
        self.world_spawn=spawn;
        for player in self.players.values_mut() {
            player.world_spawn_pending |= spawn.is_some() || removed_world;
            if removed_world {player.spawn=None;}
        }
        Ok(())
    }
    pub fn set_resource_budgets(&mut self, budgets: crate::resources::Budgets) -> Result<(), String> { self.resources.set_budgets(budgets) }
    pub fn configure_resources(&mut self, revision: String, port: u16, generations: BTreeMap<String,u64>) -> Result<(),String> {
        self.resources.configure(revision,port,generations)?;
        self.session.require_resources();
        // Require a new readiness claim even if a resource changed without reconnecting.
        for player in self.players.values_mut() {
            self.epoch = self.epoch.checked_add(1).ok_or("Resource epoch exhausted")?;
            let spawn = player.spawn;
            let pending = player.world_spawn_pending;
            *player = Player::new(self.epoch);
            player.spawn = spawn;
            player.world_spawn_pending=pending || self.world_spawn.is_some();
        }
        self.overlaps.clear(); self.last_contact.clear();
        Ok(())
    }
    /// Trusted host-only epoch transitions caused by required-world readiness.
    /// Bounded by server capacity; never populated by client movement/reset ACKs.
    pub fn drain_resource_readmission_resets(&mut self)->BTreeMap<u64,(u64,u64)> {
        std::mem::take(&mut self.resource_readmission_resets)
    }
    pub fn resource_ready(&self, actor:u64)->bool {self.resources.ready(actor)}
    pub fn drain_resource_events(&mut self)->Vec<crate::resources::Incoming>{self.resources.drain()}
    pub fn send_resource(&mut self, recipient:Option<u64>, message:crate::resources::Message)->Result<(),String>{self.resources.send(recipient,message)}
    pub fn start_large_resource(&mut self,recipient:u64,message:crate::resources::Message)->Result<crate::resources::LargeTicket,String>{self.resources.start_large(recipient,message)}
    pub fn large_resource_progress(&self,ticket:crate::resources::LargeTicket)->Result<crate::bulk::Progress,String>{self.resources.large_progress(ticket)}
    pub fn cancel_large_resource(&mut self,ticket:crate::resources::LargeTicket)->Result<(),String>{self.resources.cancel_large(ticket)}
    pub fn player_observations(&self) -> serde_json::Value {
        serde_json::Value::Array(self.players.keys().filter(|id|self.resource_ready(**id)).map(|id| {
            let actor=&self.session.actors[id];
            let position=actor.body.latest().map(|r|r.state.position());
            serde_json::json!({"id":id.to_string(),"position":position,"instance":actor.instance.to_string(),"movement_epoch":actor.movement_epoch.to_string(),"gameplay":actor.application.get(GAMEPLAY_KEY).and_then(|r|serde_json::from_slice::<Gameplay>(&r.value).ok()).unwrap_or_default()})
        }).collect())
    }
    fn service_resources(&mut self, now:u64) {
        if !self.resources.active(){return}
        let ids:Vec<_>=self.players.keys().copied().collect();
        self.resources.retain(&ids);
        for id in ids {
            self.resources.add(id,self.players[&id].epoch);
            let actor=&self.session.actors[&id];
            self.resources.context(id,actor.instance,actor.body.latest().map(|r|r.state.position()));
            if let Some(record)=self.session.actors[&id].application.get(crate::resources::CLIENT_KEY){
                self.resources.receive(id,&record.value,now);
            }
            let ready=self.resources.ready(id);
            self.session.resource_admission(id,ready);
            if ready && self.players[&id].world_spawn_pending {
                if let Some(mut destination)=self.world_spawn.or(self.base_world_spawn) {
                    destination.instance=self.instance_of(id).unwrap_or(0);
                    let previous=self.movement_epoch_of(id).unwrap();
                    if let Ok(epoch)=self.teleport(id,destination,now) {
                        self.players.get_mut(&id).unwrap().world_spawn_pending=false;
                        self.resource_readmission_resets.retain(|actor,_|self.players.contains_key(actor));
                        if self.resource_readmission_resets.len()<MAX_PLAYERS || self.resource_readmission_resets.contains_key(&id) {
                            let original=self.resource_readmission_resets.get(&id).filter(|(_,latest)|*latest==previous).map_or(previous,|(first,_)|*first);
                            self.resource_readmission_resets.insert(id,(original,epoch));
                        }
                    }
                }
            }
            if let Some(bytes)=self.resources.record(id){self.session.publish_application(&crate::resources::server_key(id),bytes,now);}
        }
        for id in self.resources.disconnects(){self.disconnect_abusive(id,now);}
    }
    pub fn new(config: Config) -> Result<Self, String> {
        if config.session == 0
            || config.server_id == 0
            || !(1..=MAX_PLAYERS).contains(&config.max_players)
        {
            return Err(
                "Dedicated server requires nonzero session/server IDs and 1..=64 players".into(),
            );
        }
        // Each process/rejoin gets a new ordered effect epoch. Source capture
        // clocks remain unrelated to this identity and to the server tick clock.
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "System clock is before the Unix epoch")?
            .as_nanos() as u64;
        Ok(Self {
            session: Session::dedicated_host(
                config.session,
                Info {
                    id: config.server_id,
                    map: config.map,
                    rig: 0,
                    physics: 0,
                    appearance: 0,
                },
                config.max_players,
                epoch,
            ),
            players: BTreeMap::new(),
            overlaps: BTreeSet::new(),
            last_contact: BTreeMap::new(),
            blocked: BTreeMap::new(),
            epoch,
            resources: crate::resources::Authority::default(),
            destinations: BTreeMap::new(),
            base_world_spawn: None,
            world_spawn: None,
            resource_readmission_resets: BTreeMap::new(),
        })
    }
    pub fn player_count(&self) -> usize {
        self.session.player_ids().iter().filter(|id|self.resource_ready(**id)).count()
    }
    /// Includes connections still downloading content, which occupy real slots.
    pub fn connection_count(&self) -> usize { self.session.connection_count() }
    pub fn capacity(&self) -> usize { self.session.capacity() }
    pub fn set_capacity(&mut self, capacity: usize) -> Result<(), String> {
        if !(1..=MAX_PLAYERS).contains(&capacity) { return Err("Capacity must be 1..64".into()); }
        self.session.set_capacity(capacity); Ok(())
    }
    pub fn map_fingerprint(&self) -> u64 { self.session.actors[&self.session.local].info.map }
    pub fn server_id(&self) -> u64 { self.session.local }
    pub fn session_id(&self) -> u64 { self.session.session }
    pub fn stats(&self) -> &Stats {
        &self.session.stats
    }
    pub fn record_send(&mut self, len: usize, success: bool) {
        self.session.record_send(len, success);
    }
    pub fn receive(&mut self, peer: u64, packet: &[u8], now: u64) {
        if self.blocked.get(&peer).is_some_and(|until| now < *until) {
            return;
        }
        self.session.expire_connections(now);
        self.sync_players();
        self.session.receive(peer, packet, now);
        self.sync_players();
    }
    pub fn service(&mut self, now: u64) -> Vec<Outgoing> {
        self.blocked.retain(|_, until| *until > now);
        let mut output = self.session.service(now);
        self.sync_players();
        self.service_resources(now);
        self.sync_players();
        self.teleports(now);
        self.read_acks();
        self.shoves(now);
        self.collisions(now);
        self.publish_effects();
        output.extend(self.session.service(now));
        output
    }
    fn sync_players(&mut self) {
        let ids = self.session.player_ids();
        let removed: Vec<_> = self
            .players
            .keys()
            .copied()
            .filter(|id| !ids.contains(id))
            .collect();
        for id in removed {
            self.players.remove(&id);
            self.clear_native(id);
            self.session.actors.get_mut(&self.session.local).unwrap().application.remove(&crate::resources::server_key(id));
            self.session
                .actors
                .get_mut(&self.session.local)
                .unwrap()
                .application
                .remove(&effects_key(id));
        }
        for id in ids {
            if !self.players.contains_key(&id) {
                self.epoch = self
                    .epoch
                    .checked_add(1)
                    .expect("Dedicated session epochs exhausted");
                let mut player=Player::new(self.epoch);player.world_spawn_pending=self.world_spawn.is_some();
                self.players.insert(id, player);
            }
        }
        self.overlaps
            .retain(|(a, b)| self.players.contains_key(a) && self.players.contains_key(b));
        self.last_contact
            .retain(|(a, b), _| self.players.contains_key(a) && self.players.contains_key(b));
    }
    fn body(&self, id: u64, now: u64) -> Option<BodyState> {
        if !self.resource_ready(id) {return None;}
        let revision = self.session.actors.get(&id)?.body.latest()?;
        if now.saturating_sub(revision.received) > FRESH_MS {
            return None;
        }
        let body = revision.state.unpack_body()?;
        // Native occupied/suspended bodies have no enabled player collision parts.
        if body.enabled & ((1u64 << 33) - 1) == 0 {
            return None;
        }
        Some(body)
    }
    fn read_acks(&mut self) {
        for (&id, player) in &mut self.players {
            let Some(record) = self.session.actors[&id].application.get(EFFECT_ACK_KEY) else {
                continue;
            };
            let Ok(ack) = serde_json::from_slice::<EffectAck>(&record.value) else {
                continue;
            };
            if ack.epoch != player.effect_epoch
                || ack.through < player.acked
                || ack.through > player.sent_through
            {
                continue;
            }
            player.acked = ack.through;
            while player
                .effects
                .front()
                .is_some_and(|effect| effect.id <= ack.through)
            {
                player.effects.pop_front();
            }
        }
    }
    fn eligible(&self, id: u64, body: &BodyState) -> bool {
        if body.enabled & (1u64 << 63) != 0 {
            return false;
        }
        self.session.actors[&id]
            .application
            .get(GAMEPLAY_KEY)
            .and_then(|record| serde_json::from_slice::<Gameplay>(&record.value).ok())
            .is_none_or(|gameplay| gameplay.mode != PlayerMode::Ragdoll)
    }
    fn shoves(&mut self, now: u64) {
        let requests: Vec<_> = self
            .players
            .keys()
            .filter_map(|&id| {
                let record = self.session.actors[&id].application.get(SHOVE_KEY)?;
                Some((
                    id,
                    serde_json::from_slice::<ShoveRequest>(&record.value).ok()?,
                ))
            })
            .collect();
        for (source, request) in requests {
            let Some(player) = self.players.get_mut(&source) else {
                continue;
            };
            if request.epoch != player.effect_epoch || request.id <= player.last_request {
                continue;
            }
            player.last_request = request.id;
            if player
                .last_shove
                .is_some_and(|at| now.saturating_sub(at) < SHOVE_COOLDOWN_MS)
                || source == request.target
                || !self.players.contains_key(&request.target)
                || self.instance_of(source) != self.instance_of(request.target)
            {
                continue;
            }
            let (Some(a), Some(b)) = (self.body(source, now), self.body(request.target, now))
            else {
                continue;
            };
            if !self.eligible(source, &a) || !self.eligible(request.target, &b) {
                continue;
            }
            let delta = sub(b.root.p, a.root.p);
            let horizontal = [delta[0], 0., delta[2]];
            let distance = length(horizontal);
            if distance > SHOVE_REACH || distance < 0.05 || delta[1].abs() > 1.5 {
                continue;
            }
            let direction = scale(horizontal, 1. / distance);
            let [x, y, z, w] = a.root.q;
            let forward = [2. * (x * z + w * y), 0., 1. - 2. * (x * x + y * y)];
            if dot(forward, direction) < SHOVE_FACING {
                continue;
            }
            if !self.reserve(&[source, request.target], now) {
                continue;
            }
            self.players.get_mut(&source).unwrap().last_shove = Some(now);
            self.enqueue(
                source,
                source,
                EffectKind::AttackAccepted,
                [0.; 3],
                b.root.p,
            );
            self.enqueue(
                source,
                request.target,
                EffectKind::Shove,
                [direction[0] * 5., 1.5, direction[2] * 5.],
                a.root.p,
            );
        }
    }
    fn collisions(&mut self, now: u64) {
        let bodies: Vec<_> = self
            .players
            .keys()
            .filter_map(|&id| self.body(id, now).map(|body| (id, body)))
            .collect();
        let mut overlaps = BTreeSet::new();
        for (index, (a_id, a)) in bodies.iter().enumerate() {
            for (b_id, b) in &bodies[index + 1..] {
                if self.instance_of(*a_id) != self.instance_of(*b_id) { continue; }
                let difference = sub(b.root.p, a.root.p);
                let horizontal = [difference[0], 0., difference[2]];
                let distance = length(horizontal);
                let contact = if distance < 0.9 && difference[1].abs() < 1.8 {
                    let normal = if distance > 0.001 {
                        scale(horizontal, 1. / distance)
                    } else {
                        [1., 0., 0.]
                    };
                    Some((normal, scale(add(a.root.p, b.root.p), 0.5), distance))
                } else {
                    self.swept_contact(*a_id, a, *b_id, b, now)
                };
                let Some((normal, point, distance)) = contact else {
                    continue;
                };
                let pair = (*a_id, *b_id);
                overlaps.insert(pair);
                if self.overlaps.contains(&pair)
                    || self
                        .last_contact
                        .get(&pair)
                        .is_some_and(|at| now.saturating_sub(*at) < 300)
                {
                    continue;
                }
                // Body 7 is the first skeleton body, avoiding detached board speed.
                let closing = dot(sub(a.bodies[7].velocity, b.bodies[7].velocity), normal);
                let impulse = (closing.max(0.) * 0.525 + (0.9 - distance) * 0.5).min(8.);
                if !self.reserve(&[*a_id, *b_id], now) {
                    continue;
                }
                self.enqueue(
                    *b_id,
                    *a_id,
                    EffectKind::Collision,
                    scale(normal, -impulse),
                    point,
                );
                self.enqueue(
                    *a_id,
                    *b_id,
                    EffectKind::Collision,
                    scale(normal, impulse),
                    point,
                );
                self.last_contact.insert(pair, now);
            }
        }
        self.overlaps = overlaps;
    }
    fn swept_contact(
        &self,
        a_id: u64,
        a: &BodyState,
        b_id: u64,
        b: &BodyState,
        now: u64,
    ) -> Option<([f32; 3], [f32; 3], f32)> {
        let previous = |id: u64, current: &BodyState| -> Option<BodyState> {
            let stream = &self.session.actors.get(&id)?.body;
            let latest = stream.latest()?;
            let old = stream.history.iter().rev().nth(1)?;
            if now.saturating_sub(old.received) > FRESH_MS {
                return None;
            }
            let body = old.state.unpack_body()?;
            let span = latest.received.saturating_sub(old.received) as f32 / 1000.;
            let speed = length(body.bodies[7].velocity).max(length(current.bodies[7].velocity));
            // Short native root offsets are fine. Large zero-speed relocation
            // is a teleport, not a collider moving through everyone en route.
            if length(sub(current.root.p, body.root.p)) > 1. + speed * (span + 0.02) * 1.5 {
                return None;
            }
            Some(body)
        };
        let old_a = previous(a_id, a)?;
        let old_b = previous(b_id, b)?;
        let initial = sub(old_b.root.p, old_a.root.p);
        let final_offset = sub(b.root.p, a.root.p);
        let travel = sub(final_offset, initial);
        let c = initial[0] * initial[0] + initial[2] * initial[2] - 0.9 * 0.9;
        if c <= 0. {
            return None;
        }
        let aa = travel[0] * travel[0] + travel[2] * travel[2];
        if aa < 0.000001 {
            return None;
        }
        let bb = 2. * (initial[0] * travel[0] + initial[2] * travel[2]);
        let discriminant = bb * bb - 4. * aa * c;
        if discriminant < 0. {
            return None;
        }
        let t = (-bb - discriminant.sqrt()) / (2. * aa);
        if !(0. ..=1.).contains(&t) {
            return None;
        }
        let offset = add(initial, scale(travel, t));
        if offset[1].abs() >= 1.8 {
            return None;
        }
        let normal = scale([offset[0], 0., offset[2]], 1. / 0.9);
        let at_a = add(old_a.root.p, scale(sub(a.root.p, old_a.root.p), t));
        let at_b = add(old_b.root.p, scale(sub(b.root.p, old_b.root.p), t));
        Some((normal, scale(add(at_a, at_b), 0.5), 0.9))
    }
    /// Never silently lose an impulse: disconnect/back off non-acknowledging
    /// recipients before committing a shared action that cannot be retained.
    fn disconnect_abusive(&mut self,id:u64,now:u64) {
            if let Some(peer) = self.session.remove_player(id) {
                if self.blocked.len() >= 64 {
                    if let Some(peer) = self
                        .blocked
                        .iter()
                        .min_by_key(|(_, until)| **until)
                        .map(|(&peer, _)| peer)
                    {
                        self.blocked.remove(&peer);
                    }
                }
                self.blocked.insert(peer, now.saturating_add(5000));
            }
    }
    fn reserve(&mut self, recipients: &[u64], now: u64) -> bool {
        let full: Vec<_> = recipients
            .iter()
            .copied()
            .filter(|id| {
                self.players
                    .get(id)
                    .is_none_or(|player| player.effects.len() >= MAX_PENDING)
            })
            .collect();
        if full.is_empty() {
            return true;
        }
        for id in full {
            self.disconnect_abusive(id,now);
        }
        self.sync_players();
        false
    }
    fn enqueue(
        &mut self,
        source: u64,
        target: u64,
        kind: EffectKind,
        delta_velocity: [f32; 3],
        position: [f32; 3],
    ) {
        let player = self.players.get_mut(&target).unwrap();
        let id = player.next_effect;
        player.next_effect = id
            .checked_add(1)
            .expect("Dedicated effect sequence exhausted");
        player.effects.push_back(Effect {
            id,
            source,
            target,
            delta_velocity,
            kind,
            position,
        });
    }
    fn publish_effects(&mut self) {
        for (&id, player) in &mut self.players {
            // Bound each record below the existing application MTU. Keep the
            // whole queue until the game, not merely the network, acknowledges it.
            let mut batch = EffectBatch {
                epoch: player.effect_epoch,
                effects: player.effects.iter().take(BATCH_SIZE).cloned().collect(),
            };
            let bytes = loop {
                let bytes = serde_json::to_vec(&batch).expect("Finite built-in effects serialize");
                if bytes.len() <= crate::lobby::MAX_APP_VALUE {
                    break bytes;
                }
                batch.effects.pop();
            };
            if let Some(last) = batch.effects.last() {
                player.sent_through = player.sent_through.max(last.id);
            }
            let published = self.session.publish_application(&effects_key(id), bytes, 0);
            debug_assert!(
                published,
                "Bounded per-player effect record fits the control actor"
            );
        }
    }
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn scale(a: [f32; 3], factor: f32) -> [f32; 3] {
    a.map(|v| v * factor)
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn multiply([x,y,z,w]:[f32;4], [a,b,c,d]:[f32;4]) -> [f32;4] {
    [w*a+x*d+y*c-z*b, w*b-x*c+y*d+z*a, w*c+x*b-y*a+z*d, w*d-x*a-y*b-z*c]
}
