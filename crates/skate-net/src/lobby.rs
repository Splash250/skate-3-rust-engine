//! Transport-neutral ten-player star lobby. The host admits and forwards;
//! each actor retains local physics ownership. No platform SDK or wall clock.
use crate::packed::{self, BODY, HEADER, POSE, Packed, Reader};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
pub const MAX_PLAYERS: usize = 10;
pub const HELLO: u8 = 3;
pub const ROSTER: u8 = 4;
pub const ACK: u8 = 5;
pub const PING: u8 = 6;
pub const PONG: u8 = 7;
pub const REJECT: u8 = 8;
pub const GOODBYE: u8 = 9;
pub const APPLICATION: u8 = 10;
pub const APPLICATION_ACK: u8 = 11;
pub const DEDICATED_HELLO: u8 = 19;
pub const DEDICATED_ROSTER: u8 = 20;
pub const MOVEMENT_RESET: u8 = 21;
pub const MOVEMENT_RESET_ACK: u8 = 22;
pub const SHARED_ENTITY: u8 = 23;
const ROSTER_CHUNK: usize = 20;
#[derive(Default)]
struct RosterParts {
    sequence: u32,
    incarnation: u64,
    parts: Vec<Option<Vec<(Info, u64, u64)>>>,
}
pub const MAX_APP_KEYS: usize = 256;
pub const MAX_APP_VALUE: usize = 1024;
#[derive(Clone, Debug)]
pub struct Application { pub seq: u32, pub value: Vec<u8>, pub received: u64 }

pub const DEFAULT_LINK_BUDGET: f64 = 180_000.;
pub const DEFAULT_HOST_BUDGET: f64 = 1_000_000.;
const LINK_CREDIT_CAP: f64 = 3_600.;
const SESSION_CREDIT_CAP: f64 = 12_000.;
/// Local two-client hosting is not an internet path. Burst player pose too.
const LOOPBACK_LINK_BUDGET: f64 = 2_000_000.;
const LOOPBACK_CREDIT_CAP: f64 = 48_000.;
/// Snapshot records (collider fragments, scenes, Lua net) are rare and must not
/// drip behind pose. Steam hosting is not UDP loopback, so this budget is
/// independent of the physics link cap.
const APP_BUDGET: f64 = 800_000.;
const APP_CREDIT_CAP: f64 = 48_000.;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub id: u64,
    pub map: u64,
    pub rig: u64,
    pub physics: u64,
    pub appearance: u64,
}
#[derive(Clone, Debug)]
pub struct Revision {
    pub seq: u32,
    pub state: Packed,
    pub received: u64,
    interest: Vec<[f32; 3]>,
}
#[derive(Default)]
pub struct Stream {
    pub history: VecDeque<Revision>,
    pub changed: u64,
}
impl Stream {
    pub fn latest(&self) -> Option<&Revision> {
        self.history.back()
    }
    fn at(&self, seq: u32) -> Option<&Packed> {
        self.history.iter().find(|r| r.seq == seq).map(|r| &r.state)
    }
    fn push(&mut self, seq: u32, state: Packed, now: u64) {
        if self.latest().is_none_or(|r| !r.state.same_state(&state)) {
            self.changed = now;
        }
        let mut interest = vec![state.position()];
        if state.rows.len() == 33 {
            if let Some(body) = state.unpack_body() {
                interest.extend(body.bodies.iter().take(7).map(|b| b.pose.p));
            }
        }
        self.history.push_back(Revision {
            interest,
            seq,
            state,
            received: now,
        });
        while self.history.len() > 64 {
            self.history.pop_front();
        }
    }
}
pub struct Actor {
    pub movement_epoch: u64,
    pub instance: u64,
    reset_anchor: Option<([f32;3], u64)>,
    // Admission retires replication history, but cannot authorize a new
    // location. This private reference bounds the first new-epoch sample.
    admission_baseline: Option<Revision>,
    pub info: Info,
    pub body: Stream,
    pub pose: Stream,
    pub application: BTreeMap<String, Application>,
}
impl Actor {
    fn new(info: Info) -> Self {
        Self {
            info,
            movement_epoch: 0,
            instance: 0,
            reset_anchor: None,
            admission_baseline: None,
            body: Stream::default(),
            pose: Stream::default(),
            application: BTreeMap::new(),
        }
    }
    pub fn stream(&self, kind: u8) -> &Stream {
        if kind == BODY { &self.body } else { &self.pose }
    }
    fn stream_mut(&mut self, kind: u8) -> &mut Stream {
        if kind == BODY {
            &mut self.body
        } else {
            &mut self.pose
        }
    }
}
struct Link {
    entities: crate::entities::Publisher,
    actor: u64,
    incarnation: u64,
    reset_acks: BTreeMap<u64,u64>,
    reset_sent: BTreeMap<u64,u64>,
    seen: u64,
    acks: BTreeMap<(u64, u8), u32>,
    sent: BTreeMap<(u64, u8), (u64, u32)>,
    keys: BTreeMap<(u64, u8), u64>,
    credits: f64,
    app_credits: f64,
    app_sent: BTreeMap<(u64, String), (u64, u32)>,
    app_acks: BTreeMap<(u64,String),u32>,
    app_round: usize,
    physics_round: [usize; 2],
    last_ack: u64,
    last_ping: u64,
    rtt: u64,
}
impl Link {
    fn new(actor: u64, now: u64) -> Self {
        Self {
            entities: Default::default(),
            actor,
            incarnation: 0,
            reset_acks: BTreeMap::new(),
            reset_sent: BTreeMap::new(),
            seen: now,
            acks: BTreeMap::new(),
            sent: BTreeMap::new(),
            keys: BTreeMap::new(),
            credits: 2400.,
            app_credits: APP_CREDIT_CAP,
            app_sent: BTreeMap::new(), app_acks: BTreeMap::new(), app_round: 0,
            physics_round: [0;2],
            last_ack: 0,
            last_ping: 0,
            rtt: 0,
        }
    }
}
#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub decoded: u64,
    pub late: u64,
    pub baseline_miss: u64,
    pub invalid: u64,
    pub budget_skips: u64,
    pub send_errors: u64,
    pub rtt_ms: u64,
}
pub struct Outgoing {
    pub peer: u64,
    pub data: Vec<u8>,
}
pub struct Session {
    pub entities: crate::entities::Replica,
    shared_objects: Vec<crate::entities::Entity>,
    pub local: u64,
    pub session: u64,
    pub actors: BTreeMap<u64, Actor>,
    pub stats: Stats,
    pub notice: String,
    host: Option<u64>,
    dedicated: bool,
    resources_required: bool,
    resource_ready: BTreeSet<u64>,
    max_players: usize,
    links: BTreeMap<u64, Link>,
    pending: Vec<Outgoing>,
    last_hello: u64,
    last_roster: u64,
    roster_seq: u32,
    received_roster: u32,
    roster_parts: RosterParts,
    resets: BTreeMap<u64, crate::dedicated::MovementReset>,
    local_reset: Option<crate::dedicated::MovementReset>,
    resource_epoch_floor: u64,
    received_incarnation: u64,
    next_incarnation: u64,
    application_seq: u32,
    last_service: u64,
    credits: f64,
    budget: f64,
    link_budget: f64,
    round: usize,
    loopback: bool,
    pub blobs: crate::blob::Blobs,
}
impl Session {
    pub fn new(session: u64, info: Info, host: Option<u64>) -> Self {
        let mut actors = BTreeMap::new();
        actors.insert(info.id, Actor::new(info));
        let mut links = BTreeMap::new();
        if let Some(peer) = host {
            links.insert(peer, Link::new(0, 0));
        }
        Self {
            entities: Default::default(),
            shared_objects: Vec::new(),
            local: info.id,
            session,
            actors,
            host,
            dedicated: false,
            resources_required: false,
            resource_ready: BTreeSet::new(),
            max_players: MAX_PLAYERS - 1,
            links,
            stats: Stats::default(),
            notice: String::new(),
            pending: vec![],
            last_hello: 0,
            last_roster: 0,
            roster_seq: 0,
            received_roster: 0,
            roster_parts: RosterParts::default(),
            resets: BTreeMap::new(),
            local_reset: None,
            resource_epoch_floor: 0,
            received_incarnation: 0,
            next_incarnation: 0,
            application_seq: 0,
            last_service: 0,
            credits: 12_000.,
            budget: if host.is_none() {
                DEFAULT_HOST_BUDGET
            } else {
                DEFAULT_LINK_BUDGET
            },
            link_budget: DEFAULT_LINK_BUDGET,
            round: 0,
            loopback: false,
            blobs: Default::default(),
        }
    }
    /// Connect only to the dedicated handshake, never a peer-hosted lobby.
    pub fn dedicated_client(session: u64, info: Info, host: u64) -> Self {
        let mut result = Self::new(session, info, Some(host));
        result.dedicated = true;
        result.max_players = crate::dedicated::MAX_PLAYERS;
        result
    }
    pub(crate) fn dedicated_host(session: u64, info: Info, max_players: usize, incarnation: u64) -> Self {
        let mut result = Self::new(session, info, None);
        result.dedicated = true;
        result.max_players = max_players;
        // Preserve per-slot throughput as dedicated capacity grows; the per-link
        // ceiling and global token bucket still bound bursts and sustained bytes.
        result.budget = DEFAULT_HOST_BUDGET * (max_players as f64 / 16.).max(1.);
        result.next_incarnation = incarnation;
        result
    }
    pub fn pending_movement_reset(&self) -> Option<crate::dedicated::MovementReset> { self.local_reset }
    /// Call only after the owner's simulation has accepted the travel command.
    pub fn complete_movement_reset(&mut self, epoch: u64) {
        if self.local_reset.is_some_and(|reset| reset.epoch == epoch) { self.local_reset = None; }
    }
    pub(crate) fn service_clock(&self) -> u64 { self.last_service }
    pub(crate) fn peer_for_actor(&self, actor:u64)->Option<u64> {self.links.iter().find(|(_,link)|link.actor==actor).map(|(&peer,_)|peer)}
    pub(crate) fn publish_entities(&mut self, objects: Vec<crate::entities::Entity>) -> Result<(),String> {
        if !self.dedicated || !self.is_host() || objects.len() > crate::entities::MAX_ENTITIES
            || objects.iter().any(|e| !e.valid())
            || objects.iter().map(|e| e.id).collect::<BTreeSet<_>>().len() != objects.len() {
            return Err("Invalid shared entity snapshot".into());
        }
        self.shared_objects=objects; self.shared_objects.sort_by_key(|e|e.id); Ok(())
    }
    pub(crate) fn entity_observations(&self)->serde_json::Value {
        serde_json::Value::Array(self.shared_objects.iter().map(crate::entities::Entity::observation).collect())
    }
    /// Retry the last pre-recovery sample while prediction is paused. This
    /// allows an admitted owner whose initial keyframe was lost to establish
    /// its server-owned recovery point without submitting the new local jump.
    pub fn retry_owner_body(&mut self) {
        if !self.dedicated || self.is_host() {return;}
        for link in self.links.values_mut() {
            link.sent.remove(&(self.local,BODY));
            link.keys.remove(&(self.local,BODY));
        }
    }
    pub fn movement_epoch(&self) -> u64 { self.actors[&self.local].movement_epoch }
    pub fn resource_epoch_floor(&self)->u64 {self.resource_epoch_floor}
    pub fn resource_scope_epoch(&self)->u64 {self.received_incarnation.max(self.resource_epoch_floor)}
    pub fn visible_resource_scopes(&self)->Vec<crate::resources::Scope> {
        use crate::resources::Scope;
        if !self.dedicated || !self.connected() {return Vec::new();}
        let mut result=vec![Scope::Resource,Scope::Player{id:self.local},Scope::Instance{id:self.actors[&self.local].instance}];
        result.extend(self.entities.entities().keys().map(|&id|Scope::Entity{id})); result
    }
    pub(crate) fn advance_movement_epoch(&mut self, epoch: u64) { self.next_incarnation = self.next_incarnation.max(epoch); }
    pub(crate) fn reset_movement(&mut self, actor: u64, destination: crate::dedicated::TeleportDestination, now: u64) -> Result<u64,String> {
        if !self.dedicated || !self.is_host() || !self.actors.contains_key(&actor) || actor == self.local || !destination.valid() { return Err("Invalid movement reset".into()); }
        self.next_incarnation = self.next_incarnation.checked_add(1).ok_or("Movement epoch exhausted")?;
        let reset = crate::dedicated::MovementReset { actor, epoch: self.next_incarnation, destination };
        self.apply_movement_reset(reset, now);
        self.resets.insert(actor, reset);
        self.last_roster = 0;
        Ok(reset.epoch)
    }
    fn apply_movement_reset(&mut self, reset: crate::dedicated::MovementReset, now: u64) {
        let actor = self.actors.get_mut(&reset.actor).unwrap();
        let visibility_changed=actor.instance!=reset.destination.instance;
        if reset.actor==self.local && visibility_changed {self.resource_epoch_floor=reset.epoch;}
        let old = actor.body.latest().map(|r| (r.state.captured, r.state.unpack_body()));
        actor.body = Stream::default(); actor.pose = Stream::default();
        actor.admission_baseline = None;
        actor.movement_epoch = reset.epoch; actor.instance = reset.destination.instance;
        actor.reset_anchor = Some((reset.destination.position,now));
        if let Some((captured, Some(mut state))) = old {
            // Preserve articulated offsets, but never retain a previous sample for
            // collision sweep or interpolation across this authorized discontinuity.
            reset.destination.relocate(&mut state);
            if let Some(mut packed) = Packed::body(&state) { packed.captured = captured; actor.body.push(0, packed, now); }
        }
        for link in self.links.values_mut() {
            link.reset_acks.remove(&reset.actor);
            link.reset_sent.remove(&reset.actor);
            link.acks.retain(|(id,_),_| *id != reset.actor);
            link.sent.retain(|(id,_),_| *id != reset.actor);
            link.keys.retain(|(id,_),_| *id != reset.actor);
            if visibility_changed {
                // A filtered roster retires remote metadata on the receiver.
                // Replay it when either endpoint returns to this instance.
                link.app_acks.retain(|(id,_),_| *id==self.local || (*id!=reset.actor && link.actor!=reset.actor));
                link.app_sent.retain(|(id,_),_| *id==self.local || (*id!=reset.actor && link.actor!=reset.actor));
            }
        }
        if reset.actor == self.local { self.local_reset = Some(reset);self.entities.reset(reset.epoch,reset.destination.instance); }
    }
    pub fn is_dedicated(&self) -> bool { self.dedicated }
    pub(crate) fn require_resources(&mut self) {
        self.resources_required = true;
        // service() publishes membership before the new resource authority
        // writes its first offer. Retire old ACK/effect snapshots before that
        // first batch can combine them with a new connection incarnation.
        self.actors.get_mut(&self.local).unwrap().application.retain(|key,_|!crate::dedicated::server_key(key));
        self.resource_ready.clear();
        self.resets.clear();
        // Hot reload creates an admission boundary for movement as well as Lua
        // events. Old datagrams cannot repopulate newly cleared histories.
        for link in self.links.values_mut() {
            self.next_incarnation = self.next_incarnation.checked_add(1).expect("Connection generations exhausted");
            link.incarnation = self.next_incarnation;
            link.acks.clear(); link.sent.clear(); link.keys.clear();
            link.app_acks.clear(); link.app_sent.clear();
            link.reset_acks.clear(); link.reset_sent.clear();
            let actor = self.actors.get_mut(&link.actor).unwrap();
            actor.movement_epoch = link.incarnation;
            actor.admission_baseline = actor.body.latest().cloned().or_else(||actor.admission_baseline.take());
            actor.body = Stream::default(); actor.pose = Stream::default();
            for key in [crate::dedicated::TELEPORT_KEY,crate::dedicated::SHOVE_KEY,crate::dedicated::EFFECT_ACK_KEY,crate::resources::CLIENT_KEY,crate::native_authority::INPUT_KEY] { actor.application.remove(key); }
        }
        self.last_roster = 0;
    }
    pub(crate) fn resource_admission(&mut self, actor:u64, ready:bool) {
        if ready { self.resource_ready.insert(actor); } else { self.resource_ready.remove(&actor); }
    }
    pub fn connection_generation(&self) -> u64 { self.received_incarnation }

    /// The dedicated control actor is a server identity, never a player.
    pub fn player_ids(&self) -> Vec<u64> {
        let server = self.dedicated.then(|| self.host_actor()).flatten();
        self.actors.keys().copied().filter(|id| Some(*id) != server).collect()
    }
    pub(crate) fn remove_player(&mut self, id: u64) -> Option<u64> {
        let peer = self.links.iter().find_map(|(&peer, link)| (link.actor == id).then_some(peer))?;
        self.links.remove(&peer);
        self.actors.remove(&id);
        self.last_roster = 0;
        Some(peer)
    }
    fn roster_limit(&self) -> usize {
        if self.dedicated { crate::dedicated::MAX_PLAYERS + 1 } else { MAX_PLAYERS }
    }
    /// An external membership authority chooses the new endpoint. Preserve our
    /// actor/capture history; rebuild membership and recipient delta baselines.
    pub fn migrate(&mut self, host: Option<u64>, now: u64) {
        self.host = host;
        self.links.clear();
        if let Some(peer) = host {
            self.links.insert(peer, Link::new(0, now));
        }
        self.actors.retain(|id, _| *id == self.local);
        self.pending.clear();
        self.roster_parts = RosterParts::default();
        self.resets.clear(); self.local_reset = None;
        self.received_roster = 0;
        self.received_incarnation = 0;
        self.roster_seq = 0;
        self.last_hello = 0;
        self.last_roster = 0;
        self.last_service = now;
        self.budget = if host.is_none() {
            DEFAULT_HOST_BUDGET
        } else {
            DEFAULT_LINK_BUDGET
        };
        self.credits = 12_000.;
        self.notice = if host.is_none() {
            String::new()
        } else {
            "Host changed; reconnecting players...".into()
        };
    }
    pub fn set_loopback(&mut self, enabled: bool) {
        self.loopback = enabled;
        let link = if enabled { LOOPBACK_LINK_BUDGET } else { DEFAULT_LINK_BUDGET };
        self.link_budget = link;
        if self.host.is_some() {
            self.budget = link;
        }
    }
    pub fn is_host(&self) -> bool {
        self.host.is_none()
    }
    /// Authenticated player identity from the host roster, not its transport endpoint.
    pub fn host_actor(&self) -> Option<u64> {
        match self.host {
            None => Some(self.local),
            Some(peer) => self.links.get(&peer).map(|link| link.actor).filter(|id| *id != 0),
        }
    }
    pub fn host_peer(&self) -> u64 {
        self.host.unwrap_or(self.local)
    }
    pub fn connected(&self) -> bool {
        self.is_host() || self.received_roster != 0
    }
    pub fn set_congested(&mut self, congested: bool) {
        let base = if self.loopback { LOOPBACK_LINK_BUDGET } else { DEFAULT_LINK_BUDGET };
        self.link_budget = if congested { base * 0.5 } else { base };
    }
    fn session_credit_cap(&self) -> f64 {
        if self.loopback { LOOPBACK_CREDIT_CAP } else if self.dedicated && self.is_host() {
            SESSION_CREDIT_CAP * (self.max_players as f64 / 16.).max(1.)
        } else { SESSION_CREDIT_CAP }
    }
    fn link_credit_cap(&self) -> f64 {
        if self.loopback { LOOPBACK_CREDIT_CAP } else { LINK_CREDIT_CAP }
    }
    pub fn publish(&mut self, kind: u8, mut state: Packed, now: u64) {
        if self.dedicated && self.local_reset.is_some() { return; }
        state.captured = now;
        let stream = self.actors.get_mut(&self.local).unwrap().stream_mut(kind);
        let seq = stream.latest().map_or(1, |r| {
            r.seq.checked_add(1).expect("Session sequence exhausted")
        });
        stream.push(seq, state, now);
    }
    /// Latest owner-authenticated state. Empty values are persistent tombstones.
    /// Repeated full records recover loss and initialize late joiners without replaying Lua.
    pub fn publish_application(&mut self, key: &str, value: Vec<u8>, now: u64) -> bool {
        if key.is_empty() || key.len() > 128 || value.len() > MAX_APP_VALUE { return false; }
        if self.dedicated && !(if self.is_host() {
            crate::dedicated::server_key(key)
        } else { crate::dedicated::valid_client_application(key, &value) }) { return false; }
        let records = &mut self.actors.get_mut(&self.local).unwrap().application;
        if records.len() >= MAX_APP_KEYS && !records.contains_key(key) { return false; }
        if records.get(key).is_some_and(|r| r.value == value) { return true; }
        let seq = if self.dedicated && self.host.is_none() {
            let Some(next) = self.application_seq.checked_add(1) else { return false; };
            self.application_seq = next;
            next
        } else { records.get(key).map_or(1, |r| r.seq.saturating_add(1)) };
        records.insert(key.into(), Application {seq, value, received: now});
        true
    }
    pub fn record_send(&mut self, len: usize, success: bool) {
        if success {
            self.stats.tx_bytes += len as u64;
        } else {
            self.stats.send_errors += 1;
        }
    }
    fn queue(&mut self, peer: u64, data: Vec<u8>) {
        if self.pending.len() < (crate::dedicated::MAX_PLAYERS + 1) * 5 && data.len() <= packed::MTU {
            self.pending.push(Outgoing { peer, data });
        }
    }
    fn local_info(&self) -> Info {
        self.actors[&self.local].info
    }
    pub fn receive(&mut self, peer: u64, data: &[u8], now: u64) {
        self.stats.rx_bytes += data.len() as u64;
        let Some((session, actor, kind, seq)) = packed::envelope(data) else {
            self.stats.invalid += 1;
            return;
        };
        if session != self.session || actor == 0 {
            return;
        }
        let mut r = Reader(&data[HEADER..]);
        let hello = if self.dedicated { DEDICATED_HELLO } else { HELLO };
        let roster = if self.dedicated { DEDICATED_ROSTER } else { ROSTER };
        if kind == hello && self.is_host() {
            let Some(info) = read_info(&mut r) else {
                return;
            };
            if !r.0.is_empty() || info.id != actor || actor == self.local {
                return;
            }
            // Map, rig and physics fingerprints describe peers; they do not gate admission.
            let own = self.local_info();
            let error = if self.dedicated && info.map != own.map { 1 }
            else if self.dedicated && (info.rig == 0 || info.physics == 0
                || (own.rig != 0 && (info.rig != own.rig || info.physics != own.physics))) { 2 }
            else if !self.links.contains_key(&peer) && self.links.len() >= self.max_players { 3 }
            else { 0 };
            if error != 0 {
                let mut b = packed::header(self.session, self.local, REJECT, 0);
                b.push(error);
                self.queue(peer, b);
                return;
            }
            if self
                .links
                .iter()
                .any(|(&endpoint, l)| endpoint != peer && l.actor == actor)
            {
                return;
            }
            if self.dedicated && self.local_info().rig == 0 {
                let own = &mut self.actors.get_mut(&self.local).unwrap().info;
                own.rig = info.rig;
                own.physics = info.physics;
            }
            let old = self.links.get(&peer).map(|l| l.actor);
            if old != Some(actor) {
                if let Some(old) = old {
                    self.actors.remove(&old);
                }
                let mut link = Link::new(actor, now);
                self.next_incarnation = self.next_incarnation.checked_add(1).expect("Connection generations exhausted");
                link.incarnation = self.next_incarnation;
                self.links.insert(peer, link);
                let mut admitted = Actor::new(info);
                admitted.movement_epoch = self.next_incarnation;
                self.actors.insert(actor, admitted);
                self.resets.remove(&actor);
                self.resource_ready.remove(&actor);
                self.last_roster = 0;
            }
            self.links.get_mut(&peer).unwrap().seen = now;
            self.notice.clear();
            return;
        }
        if self.host.is_some_and(|h| h != peer) {
            return;
        }
        if !self.links.contains_key(&peer) {
            return;
        }
        if kind == roster && self.host == Some(peer) {
            if !self.dedicated && seq <= self.received_roster {
                return;
            }
            let mut epochs = BTreeMap::new();
            let (infos, incarnation) = if self.dedicated {
                let (Some(incarnation), Some(part), Some(total), Some(count)) = (r.u64(), r.byte(), r.byte(), r.byte()) else { return; };
                if incarnation == 0 || total == 0 || total as usize > self.roster_limit().div_ceil(ROSTER_CHUNK)
                    || part >= total || count == 0 || count as usize > ROSTER_CHUNK
                    || incarnation < self.received_incarnation
                    || (incarnation == self.received_incarnation && seq <= self.received_roster) { return; }
                let mut entries = Vec::new();
                for _ in 0..count {
                    let (Some(info),Some(epoch),Some(instance)) = (read_info(&mut r),r.u64(),r.u64()) else {return;};
                    if info.id == 0 || (info.id != actor && epoch == 0) { return; }
                    entries.push((info,epoch,instance));
                }
                if !r.0.is_empty() { return; }
                if self.roster_parts.incarnation > incarnation || (self.roster_parts.incarnation == incarnation && self.roster_parts.sequence > seq) { return; }
                if self.roster_parts.incarnation != incarnation || self.roster_parts.sequence != seq {
                    self.roster_parts = RosterParts { sequence:seq, incarnation, parts:vec![None;total as usize] };
                }
                if self.roster_parts.parts.len() != total as usize {return;}
                self.roster_parts.parts[part as usize] = Some(entries);
                if self.roster_parts.parts.iter().any(Option::is_none) { return; }
                let all:Vec<_> = self.roster_parts.parts.iter().flatten().flatten().copied().collect();
                if all.len() > self.roster_limit() { return; }
                let mut infos = Vec::new();
                for (info,epoch,instance) in all {
                    if epochs.insert(info.id,(epoch,instance)).is_some() {return;}
                    infos.push(info);
                }
                (infos,incarnation)
            } else {
                let Some(count) = r.byte() else {return;};
                if count == 0 || count as usize > self.roster_limit() { return; }
                let mut infos = Vec::new();
                for _ in 0..count {
                    let Some(info) = read_info(&mut r) else {return;};
                    if info.id == 0 || infos.iter().any(|i:&Info| i.id == info.id) {return;}
                    infos.push(info);
                }
                (infos,0)
            };
            if !r.0.is_empty() || infos[0].id != actor || !infos.iter().any(|i| i.id == self.local)
            {
                return;
            }
            if self.dedicated && (incarnation < self.received_incarnation
                || (incarnation == self.received_incarnation && seq <= self.received_roster)) { return; }
            if self.dedicated && (self.received_incarnation != incarnation || self.links[&peer].actor != actor) {
                // Readmission can happen after one-way loss while the client
                // still sees server heartbeats. Reset both reliable-record ACKs
                // and snapshot baselines, and never resend a previous action.
                self.links.insert(peer, Link::new(actor, now));
                self.actors.retain(|id, _| *id == self.local);
                let local = self.actors.get_mut(&self.local).unwrap();
                local.application.remove(crate::dedicated::TELEPORT_KEY);
                local.application.remove(crate::dedicated::SHOVE_KEY);
                local.application.remove(crate::dedicated::EFFECT_ACK_KEY);
                local.application.remove(crate::resources::CLIENT_KEY);
                local.application.remove(crate::native_authority::INPUT_KEY);
                if self.received_incarnation != 0 { local.body = Stream::default(); local.pose = Stream::default(); }
                local.movement_epoch = 0;
                local.reset_anchor = None;
                local.admission_baseline = None;
                self.local_reset = None;
                self.resource_epoch_floor = 0;
                self.received_incarnation = incarnation;
            }
            self.received_roster = seq;
            self.links.get_mut(&peer).unwrap().actor = actor;
            self.links.get_mut(&peer).unwrap().seen = now;
            self.actors
                .retain(|id, _| infos.iter().any(|i| i.id == *id));
            for info in infos {
                let entry = self.actors.entry(info.id).or_insert_with(|| Actor::new(info));
                if self.dedicated && entry.movement_epoch == 0 {
                    let (epoch,instance) = epochs[&info.id];
                    entry.movement_epoch = epoch; entry.instance = instance;
                }
            }
            self.notice.clear();
            return;
        }
        if kind == REJECT && self.host == Some(peer) {
            self.notice = match r.byte() {
                Some(1) => "Map mismatch",
                Some(2) => "Physics definition mismatch",
                Some(3) => if self.dedicated { "Dedicated server full" } else { "Lobby full (10 players)" },
                _ => "Lobby rejected connection",
            }
            .into();
            return;
        }
        if matches!(kind, crate::blob::META | crate::blob::DATA | crate::blob::ACK) {
            if self.dedicated { return; }
            let valid = if kind == crate::blob::ACK {
                self.links[&peer].actor == actor
            } else {
                actor != self.local && self.actors.contains_key(&actor)
                    && (!self.is_host() || self.links[&peer].actor == actor)
            };
            if valid {
                self.blobs.receive(peer, actor, kind, seq, &data[HEADER..]);
                self.links.get_mut(&peer).unwrap().seen = now;
            }
            return;
        }
        if kind == MOVEMENT_RESET && self.dedicated && self.host == Some(peer) && self.links[&peer].actor == actor {
            let Ok(reset) = serde_json::from_slice::<crate::dedicated::MovementReset>(r.0) else {return;};
            if reset.epoch == 0 || !reset.destination.valid() { return; }
            let Some(target) = self.actors.get(&reset.actor) else {return;};
            if reset.epoch > target.movement_epoch || (reset.actor == self.local
                && reset.epoch == target.movement_epoch && target.reset_anchor.is_none()) {
                self.apply_movement_reset(reset,now);
            }
            let mut ack = packed::header(self.session,self.local,MOVEMENT_RESET_ACK,0);
            ack.extend(reset.actor.to_le_bytes()); ack.extend(reset.epoch.to_le_bytes());
            self.queue(peer,ack);
            return;
        }
        if kind == MOVEMENT_RESET_ACK && self.dedicated && self.is_host() && self.links[&peer].actor == actor {
            let (Some(target),Some(epoch)) = (r.u64(),r.u64()) else {return;};
            if r.0.is_empty() && self.resets.get(&target).is_some_and(|reset| reset.epoch == epoch) {
                self.links.get_mut(&peer).unwrap().reset_acks.insert(target,epoch);
            }
            return;
        }
        if kind == SHARED_ENTITY {
            if self.dedicated && self.host == Some(peer) && self.links[&peer].actor == actor && actor != 0 {
                let local=&self.actors[&self.local];
                self.entities.reset(local.movement_epoch,local.instance);
                if let Ok(frame)=serde_json::from_slice(r.0) { self.entities.receive(frame); }
            }
            return;
        }
        if self.links[&peer].actor != actor && !matches!(kind, BODY | POSE | APPLICATION) {
            return;
        }
        if kind == APPLICATION_ACK {
            let Some(origin)=r.u64() else {return;};
            if self.dedicated {
                let (Some(origin_epoch),Some(receiver_epoch))=(r.u64(),r.u64()) else {return;};
                let (Some(source),Some(receiver))=(self.actors.get(&origin),self.actors.get(&actor)) else {return;};
                // Both endpoints participate in visibility. In particular an
                // ACK delayed from before a round trip must not acknowledge a
                // replay that was lost after returning to the same instance.
                if source.movement_epoch!=origin_epoch || receiver.movement_epoch!=receiver_epoch
                    || (self.is_host() && origin!=self.local && source.instance!=receiver.instance) {return;}
            }
            let Ok(key)=std::str::from_utf8(r.0) else {return;};
            if self.actors.get(&origin).and_then(|a|a.application.get(key)).is_some_and(|record|seq<=record.seq)
                && self.links[&peer].app_sent.get(&(origin,key.into())).is_some_and(|&(_,sent)|seq<=sent) {
                let link=self.links.get_mut(&peer).unwrap();
                let ack=link.app_acks.entry((origin,key.into())).or_default();*ack=(*ack).max(seq);link.seen=now;
            }
            return;
        }
        if kind == APPLICATION {
            if actor == self.local || (self.is_host() && self.links[&peer].actor != actor) { return; }
            let host_actor = self.host_actor();
            let host_mode = self.is_host();
            let Some(a) = self.actors.get_mut(&actor) else { return; };
            let Some(length) = r.byte() else { return; };
            let length = length as usize;
            if length == 0 || length > 128 || r.0.len() < length || r.0.len() - length > MAX_APP_VALUE { return; }
            let Ok(key) = std::str::from_utf8(&r.0[..length]) else { return; };
            if self.dedicated {
                let value = &r.0[length..];
                let valid = if !host_mode && Some(actor) == host_actor {
                    crate::dedicated::server_key(key)
                } else { crate::dedicated::valid_client_application(key, value) };
                if !valid { return; }
            }
            if a.application.len() >= MAX_APP_KEYS && !a.application.contains_key(key) { return; }
            if a.application.get(key).is_none_or(|old| seq > old.seq) {
                a.application.insert(key.into(), Application { seq, value: r.0[length..].to_vec(), received: now });
            }
            let latest=a.application.get(key).map_or(seq,|r|r.seq);
            let mut ack=packed::header(self.session,self.local,APPLICATION_ACK,latest);
            ack.extend(actor.to_le_bytes());
            if self.dedicated {
                ack.extend(self.actors[&actor].movement_epoch.to_le_bytes());
                ack.extend(self.actors[&self.local].movement_epoch.to_le_bytes());
            }
            ack.extend(key.as_bytes());self.queue(peer,ack);
            self.links.get_mut(&peer).unwrap().seen = now;
            return;
        }
        if kind == GOODBYE {
            if self.is_host() {
                self.links.remove(&peer);
                self.actors.remove(&actor);
                self.last_roster = 0;
            } else {
                self.actors.retain(|id, _| *id == self.local);
                self.received_roster = 0;
                self.notice = "Host left. Join another lobby.".into();
            }
            return;
        }
        if kind == PING {
            let Some(stamp) = r.u64() else {
                return;
            };
            if !r.0.is_empty() {
                return;
            }
            let mut b = packed::header(self.session, self.local, PONG, 0);
            b.extend(stamp.to_le_bytes());
            self.queue(peer, b);
            self.links.get_mut(&peer).unwrap().seen = now;
            return;
        }
        if kind == PONG {
            let Some(stamp) = r.u64() else {
                return;
            };
            if !r.0.is_empty() || stamp > now || now - stamp > 5000 {
                return;
            }
            self.links.get_mut(&peer).unwrap().rtt = now - stamp;
            self.links.get_mut(&peer).unwrap().seen = now;
            return;
        }
        if kind == ACK {
            let Some(count) = r.byte() else {
                return;
            };
            if count as usize > self.roster_limit() {
                return;
            }
            let mut ack = vec![];
            for _ in 0..count {
                let (Some(id), Some(body), Some(pose)) = (r.u64(), r.u32(), r.u32()) else {
                    return;
                };
                let epoch = if self.dedicated { let Some(epoch)=r.u64() else{return;}; epoch } else {0};
                ack.push((id, body, pose, epoch));
            }
            if !r.0.is_empty() {
                return;
            }
            let link = self.links.get_mut(&peer).unwrap();
            link.seen = now;
            for (id, body, pose, epoch) in ack {
                if self.actors.get(&id).is_some_and(|a| !self.dedicated || epoch == a.movement_epoch) {
                    link.acks.insert((id, BODY), body);
                    link.acks.insert((id, POSE), pose);
                }
            }
            return;
        }
        if !matches!(kind, BODY | POSE) || actor == self.local || !self.actors.contains_key(&actor)
        {
            return;
        }
        if self.is_host() && self.links[&peer].actor != actor {
            return;
        }
        if self.resources_required && self.is_host() && !self.resource_ready.contains(&actor) { return; }
        let data = if self.dedicated {
            if data.len() < HEADER + 8 { return; }
            let split = data.len()-8;
            let epoch = u64::from_le_bytes(data[split..].try_into().unwrap());
            if epoch == 0 || epoch != self.actors[&actor].movement_epoch { self.stats.late += 1; return; }
            &data[..split]
        } else { data };
        let anchor = self.actors[&actor].reset_anchor;
        let admission_baseline = self.actors[&actor].admission_baseline.clone();
        let stream = self.actors.get_mut(&actor).unwrap().stream_mut(kind);
        if stream.latest().is_some_and(|s| seq <= s.seq) {
            self.stats.late += 1;
            return;
        }
        let Some(base) = packed::baseline(data) else {
            return;
        };
        let baseline = if base == 0 { None } else { stream.at(base) };
        if base != 0 && baseline.is_none() {
            self.stats.baseline_miss += 1;
            return;
        }
        let Some(state) = packed::apply(data, baseline) else {
            self.stats.invalid += 1;
            return;
        };
        if self.dedicated && self.host.is_none() && kind == BODY && stream.latest().is_none()
            && anchor.is_some_and(|(target, at)| distance(state.position(), target) > 10. + 200. * (now.saturating_sub(at).min(1000) as f32 / 1000.)) {
            self.stats.invalid += 1; return;
        }
        if self.dedicated && self.host.is_none() && kind == BODY
            && !crate::dedicated::plausible_body(&state, stream.latest().or(admission_baseline.as_ref()), now) {
            self.stats.invalid += 1;
            return;
        }
        stream.push(seq, state, now);
        if kind == BODY {self.actors.get_mut(&actor).unwrap().admission_baseline=None;}
        self.stats.decoded += 1;
        self.links.get_mut(&peer).unwrap().seen = now;
    }
    pub fn goodbye(&self) -> Vec<Outgoing> {
        self.links
            .keys()
            .map(|&peer| Outgoing {
                peer,
                data: packed::header(self.session, self.local, GOODBYE, 0),
            })
            .collect()
    }
    pub(crate) fn expire_connections(&mut self, now: u64) {
        if self.is_host() {
            let expired: Vec<_> = self
                .links
                .iter()
                .filter(|(_, l)| now.saturating_sub(l.seen) > 5000)
                .map(|(&p, l)| (p, l.actor))
                .collect();
            for (peer, id) in expired {
                self.links.remove(&peer);
                self.actors.remove(&id);
                self.last_roster = 0;
            }
        } else if self
            .links
            .values()
            .any(|l| now.saturating_sub(l.seen) > 5000)
        {
            self.actors.retain(|id, _| *id == self.local);
            self.received_roster = 0;
            if self.dedicated {
                self.received_incarnation = 0;
                self.last_hello = 0;
                let peer = self.host.unwrap();
                self.links.insert(peer, Link::new(0, now));
                let local = self.actors.get_mut(&self.local).unwrap();
                local.application.remove(crate::dedicated::TELEPORT_KEY);
                local.application.remove(crate::dedicated::SHOVE_KEY);
                local.application.remove(crate::dedicated::EFFECT_ACK_KEY);
                local.application.remove(crate::resources::CLIENT_KEY);
                local.application.remove(crate::native_authority::INPUT_KEY);
            }
            self.notice = "Host unavailable; waiting to reconnect".into();
        }
    }
    fn prune_links(&mut self) {
        self.resets.retain(|id,_| self.actors.contains_key(id));
        for link in self.links.values_mut() {
            link.reset_acks.retain(|id,_| self.actors.contains_key(id));
            link.reset_sent.retain(|id,_| self.actors.contains_key(id));
            link.acks.retain(|(id, _), _| self.actors.contains_key(id));
            link.sent.retain(|(id, _), _| self.actors.contains_key(id));
            link.keys.retain(|(id, _), _| self.actors.contains_key(id));
            link.app_sent.retain(|(id, key), _| self.actors.get(id).is_some_and(|a| a.application.contains_key(key)));
            link.app_acks.retain(|(id, key), _| self.actors.get(id).is_some_and(|a| a.application.contains_key(key)));
        }
    }
    pub fn service(&mut self, now: u64) -> Vec<Outgoing> {
        let local=&self.actors[&self.local];
        self.entities.reset(local.movement_epoch,local.instance);
        let dt = now.saturating_sub(self.last_service).min(1000) as f64 / 1000.;
        self.last_service = now;
        self.credits = (self.credits + dt * self.budget).min(self.session_credit_cap());
        let link_cap = self.link_credit_cap();
        for l in self.links.values_mut() {
            l.credits = (l.credits + dt * self.link_budget).min(link_cap);
            l.app_credits = (l.app_credits + dt * APP_BUDGET).min(APP_CREDIT_CAP);
        }
        self.expire_connections(now);
        self.prune_links();
        if let Some(peer) = self.host {
            if now.saturating_sub(self.last_hello) >= 500 || self.last_hello == 0 {
                let mut b = packed::header(self.session, self.local, if self.dedicated { DEDICATED_HELLO } else { HELLO }, 0);
                write_info(&mut b, self.local_info());
                self.queue(peer, b);
                self.last_hello = now.max(1);
            }
        } else if now.saturating_sub(self.last_roster) >= 500 || self.last_roster == 0 {
            if self.last_roster == 0 { self.roster_seq = self.roster_seq.checked_add(1).expect("Roster sequence exhausted"); }
            for peer in self.links.keys().copied().collect::<Vec<_>>() {
                let recipient = self.links[&peer].actor;
                let visible:Vec<_> = std::iter::once(&self.actors[&self.local]).chain(self.actors.iter()
                    .filter(|(id,a)| **id != self.local && (!self.dedicated || a.instance == self.actors[&recipient].instance)).map(|(_,a)|a)).collect();
                if self.dedicated {
                    let chunks = visible.len().div_ceil(ROSTER_CHUNK);
                    let packets:Vec<_> = visible.chunks(ROSTER_CHUNK).enumerate().map(|(index,entries)| {
                        let mut b = packed::header(self.session,self.local,DEDICATED_ROSTER,self.roster_seq);
                        b.extend(self.links[&peer].incarnation.to_le_bytes());
                        b.extend([index as u8,chunks as u8,entries.len() as u8]);
                        for a in entries {write_info(&mut b,a.info); b.extend(a.movement_epoch.to_le_bytes()); b.extend(a.instance.to_le_bytes());}
                        b
                    }).collect();
                    for b in packets {self.queue(peer,b);}
                } else {
                    let mut b=packed::header(self.session,self.local,ROSTER,self.roster_seq);
                    b.push(visible.len() as u8);
                    for a in visible {write_info(&mut b,a.info);}
                    self.queue(peer,b);
                }
            }
            self.last_roster = now.max(1);
        }
        let mut output = std::mem::take(&mut self.pending);
        let ids: Vec<_> = self.actors.keys().copied().collect();
        self.round = self.round.wrapping_add(1);
        let mut peers: Vec<_> = self.links.keys().copied().collect();
        if !peers.is_empty() {
            let offset = self.round % peers.len();
            peers.rotate_left(offset);
        }
        if self.dedicated && self.is_host() {
            for &peer in &peers {
                let link = self.links.get_mut(&peer).unwrap();
                for (&id,reset) in &self.resets {
                    if id != link.actor && self.actors[&id].instance != self.actors[&link.actor].instance { continue; }
                    if link.reset_acks.get(&id) == Some(&reset.epoch) || link.reset_sent.get(&id).is_some_and(|at| now.saturating_sub(*at)<100) {continue;}
                    let mut data=packed::header(self.session,self.local,MOVEMENT_RESET,0);
                    data.extend(serde_json::to_vec(reset).expect("Validated reset serializes"));
                    output.push(Outgoing{peer,data}); link.reset_sent.insert(id,now);
                }
            }
        }
        for &peer in &peers {
            let link = self.links.get_mut(&peer).unwrap();
            if now.saturating_sub(link.last_ack) >= 100 {
                let actors:Vec<_> = self.actors.iter().collect();
                for entries in actors.chunks(if self.dedicated {48} else {MAX_PLAYERS}) {
                    let mut b = packed::header(self.session, self.local, ACK, 0);
                    b.push(entries.len() as u8);
                    for &(id,a) in entries {
                        b.extend(id.to_le_bytes());
                        b.extend(a.body.latest().map_or(0, |r| r.seq).to_le_bytes());
                        b.extend(a.pose.latest().map_or(0, |r| r.seq).to_le_bytes());
                        if self.dedicated { b.extend(a.movement_epoch.to_le_bytes()); }
                    }
                    output.push(Outgoing { peer, data: b });
                }
                link.last_ack = now;
            }
            if now.saturating_sub(link.last_ping) >= 1000 {
                let mut b = packed::header(self.session, self.local, PING, 0);
                b.extend(now.to_le_bytes());
                output.push(Outgoing { peer, data: b });
                link.last_ping = now;
            }
        }
        // Player rigid bodies first, then the live APPLICATION snapshot (collider
        // fragments, scenes, net state), then optional pose detail with whatever
        // budget remains. Snapshot spawn is not leftover traffic.
        self.write_physics(&mut output, &peers, &ids, BODY, now);
        self.write_application(&mut output, now);
        self.write_entities(&mut output, now);
        self.write_physics(&mut output, &peers, &ids, POSE, now);
        let members = self.actors.keys().copied().collect();
        let peers: Vec<_> = self.links.iter().map(|(&peer, link)| (peer, link.actor, link.rtt)).collect();
        if !self.dedicated {
            output.extend(self.blobs.service(self.session, self.local, self.host.is_none(), &members, &peers, now));
        }
        self.stats.rtt_ms = self.links.values().map(|l| l.rtt).max().unwrap_or(0);
        output
    }
    fn write_physics(&mut self, output: &mut Vec<Outgoing>, peers: &[u64], ids: &[u64], kind: u8, now: u64) {
        for &peer in peers {
            let Some(link_actor) = self.links.get(&peer).map(|l| l.actor) else { continue };
            let stream_index = usize::from(kind != BODY);
            let offset = self.links[&peer].physics_round[stream_index];
            for step in 0..ids.len() {
                let index = (offset + step) % ids.len();
                let id = ids[index];
                if self.resources_required && self.host.is_none()
                    && (!self.resource_ready.contains(&link_actor) || !self.resource_ready.contains(&id)) { continue; }
                if self.dedicated && ((self.host.is_none() && id != self.local && self.actors[&id].instance != self.actors[&link_actor].instance)
                    || (id == self.local && (self.local_reset.is_some() || self.actors[&id].movement_epoch == 0))) { continue; }
                if id == link_actor || (self.host.is_some() && id != self.local) {
                    continue;
                }
                let actor = &self.actors[&id];
                let stream = actor.stream(kind);
                let Some(latest) = stream.latest() else {
                    continue;
                };
                let distance = if self.host.is_some() {
                    0.
                } else {
                    self.actors
                        .get(&link_actor)
                        .and_then(|a| a.body.latest())
                        .zip(actor.body.latest())
                        .map_or(0., |(a, b)| {
                            let roots = distance(a.interest[0], b.interest[0]);
                            a.interest
                                .iter()
                                .map(|&p| distance(p, b.interest[0]))
                                .chain(b.interest.iter().map(|&p| distance(p, a.interest[0])))
                                .fold(roots, f32::min)
                        })
                };
                let mut interval = match (kind, distance) {
                    (BODY, d) if d < 30. => 50,
                    (BODY, d) if d < 100. => 100,
                    (BODY, _) => 500,
                    (POSE, d) if d < 30. => {
                        if self.loopback { 50 } else { 100 }
                    }
                    (POSE, d) if d < 100. => 250,
                    _ => 1000,
                };
                if !self.loopback
                    && now.saturating_sub(stream.changed) > 250
                    && (kind == POSE || distance >= 8.)
                {
                    interval = interval.max(200);
                }
                let link = self.links.get_mut(&peer).unwrap();
                if let Some(&(sent, seq)) = link.sent.get(&(id, kind)) {
                    if seq == latest.seq || now.saturating_sub(sent) < interval {
                        continue;
                    }
                }
                let keyframe = link
                    .keys
                    .get(&(id, kind))
                    .is_none_or(|at| now.saturating_sub(*at) >= 2000);
                let baseline = link
                    .acks
                    .get(&(id, kind))
                    .and_then(|&seq| stream.at(seq).map(|s| (seq, s)))
                    .filter(|_| !keyframe);
                let mut data =
                    packed::delta(self.session, id, kind, latest.seq, &latest.state, baseline);
                if self.dedicated { data.extend(actor.movement_epoch.to_le_bytes()); }
                if data.len() > packed::MTU {
                    self.stats.invalid += 1;
                    continue;
                }
                let bytes = data.len() as f64;
                if link.credits < bytes || self.credits < bytes {
                    self.stats.budget_skips += 1;
                    continue;
                }
                link.credits -= bytes;
                self.credits -= bytes;
                link.sent.insert((id, kind), (now, latest.seq));
                link.physics_round[stream_index] = (index + 1) % ids.len();
                if baseline.is_none() {
                    link.keys.insert((id, kind), now);
                }
                output.push(Outgoing { peer, data });
            }
        }
    }
    fn write_entities(&mut self, output: &mut Vec<Outgoing>, now: u64) {
        if !self.dedicated || !self.is_host() {return;}
        for (&peer,link) in &mut self.links {
            let actor=&self.actors[&link.actor];
            let ready=!self.resources_required || self.resource_ready.contains(&link.actor);
            let position=actor.body.latest().map(|r|r.state.position());
            let objects:Vec<_>=self.shared_objects.iter().filter(|e|ready && e.instance==actor.instance
                && position.is_none_or(|p|distance(p,e.position)<=500.)).collect();
            for bytes in link.entities.frames(actor.movement_epoch,actor.instance,&objects,now) {
                let mut data=packed::header(self.session,self.local,SHARED_ENTITY,0); data.extend(bytes);
                output.push(Outgoing {peer,data});
            }
        }
    }
    fn write_application(&mut self, output: &mut Vec<Outgoing>, now: u64) {
        let mut app_peers: Vec<_> = self.links.keys().copied().collect();
        if !app_peers.is_empty() {
            let offset = self.round % app_peers.len();
            app_peers.rotate_left(offset);
        }
        let retry = if self.loopback || (self.stats.rtt_ms > 0 && self.stats.rtt_ms < 25) { 16 } else { 110 };
        let first_wait = if self.loopback || (self.stats.rtt_ms > 0 && self.stats.rtt_ms < 25) { 0 } else { 50 };
        let lan = retry == 16;
        for peer in app_peers {
            let link = self.links.get_mut(&peer).unwrap();
            link.app_sent.retain(|(id, _), _| self.actors.contains_key(id));
            link.app_acks.retain(|(id, _), _| self.actors.contains_key(id));
            let records: Vec<_> = self.actors.iter().filter(|(id, _)| **id != link.actor && (self.host.is_none() || **id == self.local))
                .flat_map(|(&id, a)| a.application.iter().map(move |(key, record)| (id, key, record)))
                .filter(|(id, key, _)| !self.dedicated || self.host.is_some()
                    || (*id == self.local && (**key == crate::dedicated::effects_key(link.actor) || **key == crate::resources::server_key(link.actor) || **key == crate::native_authority::state_key(link.actor)))
                    || (*id != self.local && self.actors[id].instance == self.actors[&link.actor].instance && (!self.resources_required || (self.resource_ready.contains(id) && self.resource_ready.contains(&link.actor))) && matches!(key.as_str(), crate::dedicated::GAMEPLAY_KEY | "mp:name" | "mp:ping")))
                .collect();
            let count = records.len();
            for offset in 0..count {
                let index = (link.app_round + offset) % count;
                let (id, key, record) = records[index];
                if link.app_acks.get(&(id, key.clone())).is_some_and(|&seq| seq == record.seq) { continue; }
                let previous = link.app_sent.get(&(id, key.clone()));
                if previous.is_some_and(|&(at, seq)| now.saturating_sub(at) < if seq == record.seq {
                    if lan { retry } else { retry + (self.round as u64).wrapping_add(id).wrapping_mul(17) % 130 }
                } else { first_wait }) { continue; }
                let mut data = packed::header(self.session, id, APPLICATION, record.seq);
                data.push(key.len() as u8); data.extend(key.as_bytes()); data.extend(&record.value);
                let bytes = data.len() as f64;
                if bytes > link.app_credits { link.app_round = index; break; }
                link.app_credits -= bytes;
                link.app_sent.insert((id, key.clone()), (now, record.seq));
                output.push(Outgoing { peer, data });
            }
        }
    }
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f32>()
        .sqrt()
}
fn write_info(b: &mut Vec<u8>, i: Info) {
    for v in [i.id, i.map, i.rig, i.physics, i.appearance] {
        b.extend(v.to_le_bytes());
    }
}
fn read_info(r: &mut Reader) -> Option<Info> {
    Some(Info {
        id: r.u64()?,
        map: r.u64()?,
        rig: r.u64()?,
        physics: r.u64()?,
        appearance: r.u64()?,
    })
}

#[cfg(test)]
mod dedicated_tests {
    use super::*;
    fn info(id:u64)->Info { Info{id,map:1,rig:2,physics:3,appearance:0} }
    fn pump(host:&mut Session, observer:&mut Session, visitor:&mut Session,now:u64) {
        for packet in observer.service(now) {host.receive(100,&packet.data,now);}
        for packet in visitor.service(now) {host.receive(200,&packet.data,now);}
        for packet in host.service(now) {
            match packet.peer {
                100=>observer.receive(1,&packet.data,now),
                200=>visitor.receive(1,&packet.data,now),
                _=>panic!("Unexpected endpoint"),
            }
        }
    }
    #[test]
    fn departed_players_and_deleted_effect_keys_do_not_accumulate_link_state() {
        let mut host=Session::dedicated_host(7,info(99),16,1000);
        let mut observer=Session::dedicated_client(7,info(2),1);
        let root=crate::Pose {p:[0.;3],q:[0.,0.,0.,1.]};
        let body=Packed::body(&packed::BodyState {root,enabled:1,bodies:vec![crate::Body{pose:root,velocity:[0.;3],angular:[0.;3]};33]}).unwrap();
        for id in 110..210 {
            let mut visitor=Session::dedicated_client(7,info(id),1);
            let begin=(id-110)*1000;
            visitor.publish(BODY,body.clone(),begin);
            visitor.publish_application("mp:name",b"Visitor".to_vec(),begin);
            let key=crate::dedicated::effects_key(id);
            host.publish_application(&key,b"{}".to_vec(),begin);
            for now in (begin..begin+800).step_by(10) {pump(&mut host,&mut observer,&mut visitor,now);}
            assert!(observer.actors[&id].body.latest().is_some(), "visitor {id}, host {:?}, observer {:?}", host.stats, observer.stats);
            for packet in visitor.goodbye() {host.receive(200,&packet.data,begin+800);}
            host.actors.get_mut(&99).unwrap().application.remove(&key);
            host.service(begin+800);
            for link in host.links.values() {
                assert!(link.acks.keys().all(|(id,_)|host.actors.contains_key(id)));
                assert!(link.sent.keys().all(|(id,_)|host.actors.contains_key(id)));
                assert!(link.keys.keys().all(|(id,_)|host.actors.contains_key(id)));
                assert!(link.app_sent.keys().all(|(id,key)|host.actors.get(id).is_some_and(|actor|actor.application.contains_key(key))));
                assert!(link.app_acks.keys().all(|(id,key)|host.actors.get(id).is_some_and(|actor|actor.application.contains_key(key))));
            }
        }
    }
}
