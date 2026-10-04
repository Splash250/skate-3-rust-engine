//! Reliable, ordered resource messages carried by bounded application records.
//! Each connection has an epoch; each resource has a generation. No transport
//! endpoints, content paths or Lua values are trusted as sender identities.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const CLIENT_KEY: &str = "resource:client";
pub const MAX_PENDING: usize = 64;
pub const MAX_PAYLOAD: usize = 16 * 1024;
pub const SMALL_PAYLOAD: usize = 384;
/// Negotiated operational budgets, always below wire hard ceilings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Budgets {
    pub value_bytes: usize,
    pub pending: usize,
    pub queue_bytes: usize,
    pub state_keys: usize,
    pub state_bytes: usize,
    pub resources: usize,
    pub events_per_second: u32,
}
impl Default for Budgets {
    fn default() -> Self {
        Self {
            value_bytes: MAX_PAYLOAD,
            pending: MAX_PENDING,
            queue_bytes: 1024 * 1024,
            state_keys: 256,
            state_bytes: 1024 * 1024,
            resources: 128,
            events_per_second: 60,
        }
    }
}
impl Budgets {
    pub fn validate(self) -> Result<(), String> {
        if !(SMALL_PAYLOAD..=256 * 1024).contains(&self.value_bytes)
            || !(1..=256).contains(&self.pending)
            || !(self.value_bytes + 1024..=16 * 1024 * 1024).contains(&self.queue_bytes)
            || !(1..=4096).contains(&self.state_keys)
            || !(self.value_bytes..=16 * 1024 * 1024).contains(&self.state_bytes)
            || !(1..=256).contains(&self.resources)
            || !(1..=1000).contains(&self.events_per_second)
        {
            return Err("Invalid resource network budgets".into());
        }
        Ok(())
    }
    fn negotiate(self, other: Self) -> Self {
        Self {
            value_bytes: self.value_bytes.min(other.value_bytes),
            pending: self.pending.min(other.pending),
            queue_bytes: self.queue_bytes.min(other.queue_bytes),
            state_keys: self.state_keys.min(other.state_keys),
            state_bytes: self.state_bytes.min(other.state_bytes),
            resources: self.resources.min(other.resources),
            events_per_second: self.events_per_second.min(other.events_per_second),
        }
    }
    fn bulk(self) -> crate::bulk::Limits {
        crate::bulk::Limits {
            max_message: self.value_bytes + 1024,
            max_queued_bytes: self.queue_bytes,
            max_pending: self.pending,
        }
    }
}

pub fn server_key(actor: u64) -> String {
    format!("resource:server:{actor}")
}
pub(crate) fn is_server_key(key: &str) -> bool {
    key.strip_prefix("resource:server:")
        .is_some_and(|v| v.parse::<u64>().is_ok_and(|v| v != 0))
}
fn identifier(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
}
fn digest(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    #[serde(default)]
    pub budgets: Budgets,
    pub revision: String,
    pub port: u16,
    pub epoch: u64,
}
impl Offer {
    fn valid(&self) -> bool {
        self.budgets.validate().is_ok()
            && digest(&self.revision)
            && self.port != 0
            && self.epoch != 0
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Event,
    State,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scope {
    #[default]
    Resource,
    Instance {
        #[serde(
            serialize_with = "scope_id",
            deserialize_with = "crate::entities::input_id"
        )]
        id: u64,
    },
    Player {
        #[serde(
            serialize_with = "scope_id",
            deserialize_with = "crate::entities::input_id"
        )]
        id: u64,
    },
    Entity {
        #[serde(
            serialize_with = "scope_id",
            deserialize_with = "crate::entities::input_id"
        )]
        id: u64,
    },
}
fn scope_id<S: serde::Serializer>(id: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(id)
}
impl Scope {
    pub fn is_resource(&self) -> bool {
        matches!(self, Self::Resource)
    }
    fn valid(&self) -> bool {
        !matches!(self, Self::Player { id: 0 } | Self::Entity { id: 0 })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    #[serde(default, skip_serializing_if = "Scope::is_resource")]
    pub scope: Scope,
    pub id: u64,
    pub resource: String,
    pub generation: u64,
    pub kind: Kind,
    pub name: String,
    pub value: Value,
}
impl Message {
    fn bytes(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |b| b.len())
    }
    fn fits(&self, budgets: Budgets) -> bool {
        self.valid()
            && serde_json::to_vec(&self.value).is_ok_and(|b| b.len() <= budgets.value_bytes)
    }
    fn small(&self) -> bool {
        serde_json::to_vec(&self.value).is_ok_and(|b| b.len() <= SMALL_PAYLOAD)
            && self.bytes() <= 600
    }
    pub fn valid(&self) -> bool {
        self.id != 0
            && self.scope.valid()
            && self.generation != 0
            && identifier(&self.resource)
            && identifier(&self.name)
            && serde_json::to_vec(&self.value).is_ok_and(|v| v.len() <= 256 * 1024)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientRecord {
    #[serde(default)]
    pub budgets: Budgets,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bulk: Option<crate::bulk::Frame>,
    #[serde(default)]
    pub bulk_ack: crate::bulk::Ack,
    pub epoch: u64,
    pub revision: String,
    pub ready: bool,
    pub ack: u64,
    pub message: Option<Message>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bulk: Option<crate::bulk::Frame>,
    #[serde(default)]
    pub bulk_ack: crate::bulk::Ack,
    pub offer: Offer,
    pub ack: u64,
    pub message: Option<Message>,
}
#[derive(Clone, Debug)]
pub struct Incoming {
    pub sender: u64,
    pub message: Message,
}
/// Host-only handle bound to an admitted recipient and activation epoch.
/// Scripts receive their own local request key, never this transport authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LargeTicket {
    pub recipient: u64,
    pub epoch: u64,
    pub id: u64,
}
pub(crate) fn valid_client(bytes: &[u8]) -> bool {
    bytes.len() <= crate::lobby::MAX_APP_VALUE
        && serde_json::from_slice::<ClientRecord>(bytes).is_ok_and(|r| {
            r.budgets.validate().is_ok()
                && r.epoch != 0
                && digest(&r.revision)
                && r.message.as_ref().is_none_or(|m| {
                    m.valid() && m.small() && m.kind == Kind::Event && m.scope.is_resource()
                })
        })
}
#[derive(Default)]
struct Queue {
    next: u64,
    acked: u64,
    received: u64,
    pending: VecDeque<Message>,
}
impl Queue {
    fn push(&mut self, mut msg: Message) -> Result<(), String> {
        if self.pending.len() >= 256 {
            return Err("Resource message queue is full".into());
        }
        msg.id = self
            .next
            .checked_add(1)
            .ok_or("Resource sequence exhausted")?;
        if !msg.valid() || !msg.small() {
            return Err("Invalid small resource message".into());
        }
        self.next = msg.id;
        self.pending.push_back(msg);
        Ok(())
    }
    fn ack(&mut self, through: u64) -> Result<(), String> {
        // Only one message is transmitted at a time. An ACK must never discard
        // queued messages which have not been sent to the remote runtime.
        let sent = self.pending.front().map_or(self.acked, |m| m.id);
        if through > sent {
            return Err("Resource acknowledgement exceeds sent sequence".into());
        }
        self.acked = self.acked.max(through);
        while self.pending.front().is_some_and(|m| m.id <= through) {
            self.pending.pop_front();
        }
        Ok(())
    }
    fn consume(&mut self, msg: &Message) -> Result<bool, String> {
        if !msg.valid() {
            return Err("Invalid resource message".into());
        }
        if msg.id <= self.received {
            return Ok(false);
        }
        if self.received.checked_add(1) != Some(msg.id) {
            return Err("Resource message sequence gap".into());
        }
        self.received = msg.id;
        Ok(true)
    }
}
#[derive(Default)]
pub struct Client {
    local_budgets: Budgets,
    budgets: Budgets,
    bulk: crate::bulk::Channel,
    bulk_turn: std::sync::atomic::AtomicBool,
    offer: Option<Offer>,
    ready: bool,
    queue: Queue,
    incoming: Vec<Message>,
}
impl Client {
    pub fn with_budgets(budgets: Budgets) -> Result<Self, String> {
        budgets.validate()?;
        Ok(Self {
            local_budgets: budgets,
            budgets,
            bulk: crate::bulk::Channel::new(budgets.bulk())?,
            ..Self::default()
        })
    }
    pub fn budgets(&self) -> Budgets {
        self.budgets
    }
    fn check_queue(&self, bytes: usize) -> Result<(), String> {
        let count = self.queue.pending.len() + self.bulk.pending_count();
        let used =
            self.queue.pending.iter().map(Message::bytes).sum::<usize>() + self.bulk.queued_bytes();
        if count >= self.budgets.pending || used.saturating_add(bytes) > self.budgets.queue_bytes {
            return Err("Resource backpressure: aggregate queue budget exhausted".into());
        }
        Ok(())
    }
    pub fn cancel_large(&mut self, transfer: u64) -> Result<(), String> {
        self.bulk.cancel(transfer)
    }
    pub fn start_large(&mut self, resource:&str, generation:u64, name:&str, value:Value) -> Result<LargeTicket,String> {
        if !self.ready {return Err("Resource connection is not ready".into());}
        let id=self.emit_large(resource,generation,name,value)?;
        Ok(LargeTicket {recipient:0,epoch:self.offer.as_ref().unwrap().epoch,id})
    }
    pub fn large_progress(&self, ticket:LargeTicket) -> Result<crate::bulk::Progress,String> {
        if ticket.recipient!=0 || self.offer.as_ref().map(|o|o.epoch)!=Some(ticket.epoch) {
            return Err("Large transfer connection retired".into());
        }
        self.bulk.progress(ticket.id)
    }
    pub fn cancel_ticket(&mut self, ticket:LargeTicket) -> Result<(),String> {
        self.large_progress(ticket)?;
        self.bulk.cancel(ticket.id)
    }
    pub fn emit_large(
        &mut self,
        resource: &str,
        generation: u64,
        name: &str,
        value: Value,
    ) -> Result<u64, String> {
        if self.offer.is_none() {
            return Err("No resource connection".into());
        }
        let msg = Message {
            scope: Default::default(),
            id: 1,
            resource: resource.into(),
            generation,
            kind: Kind::Event,
            name: name.into(),
            value,
        };
        if !msg.fits(self.budgets) {
            return Err("Message exceeds negotiated value budget".into());
        }
        let bytes = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
        self.check_queue(bytes.len())?;
        self.bulk.send(bytes)
    }
    pub fn offer(&self) -> Option<&Offer> {
        self.offer.as_ref()
    }
    /// Returns true when the server requires a different activation epoch/set.
    pub fn receive(&mut self, record: &ServerRecord) -> Result<bool, String> {
        if !record.offer.valid() {
            return Err("Invalid resource offer".into());
        }
        let changed = self.offer.as_ref() != Some(&record.offer);
        if changed {
            // A lower epoch from a delayed packet cannot replace a newer one.
            if self
                .offer
                .as_ref()
                .is_some_and(|o| record.offer.epoch < o.epoch)
            {
                return Ok(false);
            }
            let local = self.local_budgets;
            let selected = local.negotiate(record.offer.budgets);
            *self = Self::with_budgets(local)?;
            self.budgets = selected;
            self.bulk = crate::bulk::Channel::new(selected.bulk())?;
            self.offer = Some(record.offer.clone());
        }
        self.queue.ack(record.ack)?;
        self.bulk.acknowledge(record.bulk_ack)?;
        if let Some(frame) = &record.bulk {
            if !self.ready {
                return Err("Bulk message before client activation".into());
            }
            if let Some(bytes) = self.bulk.receive(frame)? {
                let msg: Message = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if !msg.fits(self.budgets)
                    || self.incoming.len() >= self.budgets.pending
                    || self
                        .incoming
                        .iter()
                        .map(Message::bytes)
                        .sum::<usize>()
                        .saturating_add(msg.bytes())
                        > self.budgets.queue_bytes
                {
                    return Err("Invalid bulk message or incoming budget exhausted".into());
                }
                self.incoming.push(msg);
            }
        }
        if let Some(msg) = &record.message {
            if !self.ready {
                return Err("Resource message before client activation".into());
            }
            if !msg.fits(self.budgets) || !msg.small() {
                return Err("Invalid small resource message".into());
            }
            // Preflight before advancing the acknowledgement. Retransmissions
            // consume no additional space; newly admitted messages share the
            // same incoming byte budget as completed bulk messages.
            if msg.id > self.queue.received
                && (self.incoming.len() >= self.budgets.pending
                    || self
                        .incoming
                        .iter()
                        .map(Message::bytes)
                        .sum::<usize>()
                        .saturating_add(msg.bytes())
                        > self.budgets.queue_bytes)
            {
                return Err("Resource incoming queue budget exhausted".into());
            }
            if self.queue.consume(msg)? {
                self.incoming.push(msg.clone());
            }
        }
        Ok(changed)
    }
    pub fn set_ready(&mut self, ready: bool) {
        self.ready = ready;
    }
    pub fn ready(&self) -> bool {
        self.ready
    }
    pub fn emit(
        &mut self,
        resource: &str,
        generation: u64,
        name: &str,
        value: Value,
    ) -> Result<(), String> {
        if self.offer.is_none() {
            return Err("No resource connection".into());
        }
        let message = Message {
            scope: Scope::Resource,
            id: 1,
            resource: resource.into(),
            generation,
            kind: Kind::Event,
            name: name.into(),
            value,
        };
        if !message.small() {
            self.emit_large(resource, generation, name, message.value)?;
            return Ok(());
        }
        self.check_queue(message.bytes())?;
        self.queue.push(message)
    }
    pub fn take_incoming(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.incoming)
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let offer = self.offer.as_ref().ok_or("No resource connection")?;
        let send_bulk = self.ready
            && self.bulk.frame().is_some()
            && (self.queue.pending.is_empty()
                || !self
                    .bulk_turn
                    .fetch_xor(true, std::sync::atomic::Ordering::Relaxed));
        let r = ClientRecord {
            budgets: self.budgets,
            bulk: if send_bulk { self.bulk.frame() } else { None },
            bulk_ack: self.bulk.ack(),
            epoch: offer.epoch,
            revision: offer.revision.clone(),
            ready: self.ready,
            ack: self.queue.received,
            message: if self.ready && !send_bulk {
                self.queue.pending.front().cloned()
            } else {
                None
            },
        };
        serde_json::to_vec(&r).map_err(|e| e.to_string())
    }
}
struct Peer {
    instance: u64,
    position: Option<[f32; 3]>,
    visible_states: BTreeSet<StateKey>,
    budgets: Budgets,
    bulk: crate::bulk::Channel,
    bulk_turn: bool,
    epoch: u64,
    ready: bool,
    queue: Queue,
    window: u64,
    requests: u32,
    pending_states: BTreeMap<StateKey, Message>,
    state_order: VecDeque<StateKey>,
    events: VecDeque<Message>,
    last_state: bool,
}
type StateKey = (String, Scope, String);
fn visible(
    id: u64,
    p: &Peer,
    message: &Message,
    entities: &BTreeMap<u64, crate::entities::Entity>,
) -> bool {
    match message.scope {
        Scope::Resource => true,
        Scope::Instance { id } => p.instance == id,
        Scope::Player { id: owner } => id == owner,
        Scope::Entity { id } => entities.get(&id).is_some_and(|e| {
            e.resource == message.resource
                && e.generation == message.generation
                && e.instance == p.instance
                && p.position.is_none_or(|position| {
                    position
                        .iter()
                        .zip(e.position)
                        .map(|(a, b)| (a - b).powi(2))
                        .sum::<f32>()
                        <= 500_f32.powi(2)
                })
        }),
    }
}
fn enqueue_state(p: &mut Peer, key: StateKey, message: Message) -> bool {
    let previous = p.pending_states.get(&key).map_or(0, Message::bytes);
    if !message.fits(p.budgets)
        || p.queued_bytes()
            .saturating_sub(previous)
            .saturating_add(message.bytes())
            > p.budgets.queue_bytes
    {
        return false;
    }
    if !p.pending_states.contains_key(&key) {
        if p.pending_states.len() >= p.budgets.resources * p.budgets.state_keys {
            return false;
        }
        p.state_order.push_back(key.clone());
    }
    p.pending_states.insert(key, message);
    true
}
impl Peer {
    fn queued_bytes(&self) -> usize {
        self.pending_states
            .values()
            .chain(&self.events)
            .chain(&self.queue.pending)
            .map(Message::bytes)
            .sum::<usize>()
            + self.bulk.queued_bytes()
    }
}
#[derive(Default)]
pub(crate) struct Authority {
    budgets: Budgets,
    pub revision: String,
    pub port: u16,
    generations: BTreeMap<String, u64>,
    peers: BTreeMap<u64, Peer>,
    incoming: Vec<Incoming>,
    states: BTreeMap<StateKey, Message>,
    entities: BTreeMap<u64, crate::entities::Entity>,
    disconnect: Vec<u64>,
}
impl Authority {
    pub fn set_budgets(&mut self, budgets: Budgets) -> Result<(), String> {
        budgets.validate()?;
        if !self.peers.is_empty() {
            return Err("Configure budgets before admission".into());
        }
        self.budgets = budgets;
        Ok(())
    }
    pub fn configure(
        &mut self,
        revision: String,
        port: u16,
        generations: BTreeMap<String, u64>,
    ) -> Result<(), String> {
        if !digest(&revision)
            || port == 0
            || generations.len() > self.budgets.resources
            || generations.iter().any(|(k, v)| !identifier(k) || *v == 0)
        {
            return Err("Invalid resource offer".into());
        }
        self.revision = revision;
        self.port = port;
        self.generations = generations;
        self.peers.clear();
        self.incoming.clear();
        self.states.clear();
        Ok(())
    }
    pub(crate) fn scope_targets(&self) -> (Vec<u64>, Vec<(String, u64, u64)>) {
        (
            self.peers.keys().copied().collect(),
            self.entities
                .values()
                .map(|e| (e.resource.clone(), e.id, e.generation))
                .collect(),
        )
    }
    pub fn active(&self) -> bool {
        !self.revision.is_empty()
    }
    pub fn ready(&self, id: u64) -> bool {
        !self.active() || self.peers.get(&id).is_some_and(|p| p.ready)
    }
    pub fn retain(&mut self, ids: &[u64]) {
        self.peers.retain(|id, _| ids.contains(id));
        self.incoming.retain(|event| ids.contains(&event.sender));
        self.states
            .retain(|(_, scope, _), _| !matches!(scope,Scope::Player{id} if !ids.contains(id)));
    }
    pub fn add(&mut self, id: u64, epoch: u64) {
        if self.peers.get(&id).is_some_and(|p| p.epoch != epoch) {
            self.peers.remove(&id);
            self.incoming.retain(|event| event.sender != id);
        }
        self.peers.entry(id).or_insert_with(|| Peer {
            instance: 0,
            position: None,
            visible_states: BTreeSet::new(),
            budgets: self.budgets,
            bulk: crate::bulk::Channel::new(self.budgets.bulk()).unwrap(),
            bulk_turn: false,
            epoch,
            ready: false,
            queue: Queue::default(),
            window: 0,
            requests: 0,
            pending_states: BTreeMap::new(),
            state_order: VecDeque::new(),
            events: VecDeque::new(),
            last_state: false,
        });
    }
    pub fn context(&mut self, id: u64, instance: u64, position: Option<[f32; 3]>) {
        if let Some(peer) = self.peers.get_mut(&id) {
            peer.instance = instance;
            peer.position = position;
        }
        self.refresh_visibility(id);
    }
    pub fn entities(&mut self, objects: &[crate::entities::Entity]) {
        self.entities = objects.iter().map(|e| (e.id, e.clone())).collect();
        self.states.retain(|_,m|!matches!(m.scope,Scope::Entity{id} if self.entities.get(&id).is_none_or(|e|e.resource!=m.resource || e.generation!=m.generation)));
        for id in self.peers.keys().copied().collect::<Vec<_>>() {
            self.refresh_visibility(id);
        }
    }
    fn refresh_visibility(&mut self, id: u64) {
        let Some(p) = self.peers.get(&id).filter(|p| p.ready) else {
            return;
        };
        let current: BTreeSet<_> = self
            .states
            .iter()
            .filter(|(_, m)| visible(id, p, m, &self.entities))
            .map(|(key, _)| key.clone())
            .collect();
        let p = self.peers.get_mut(&id).unwrap();
        let entered: Vec<_> = current.difference(&p.visible_states).cloned().collect();
        let left: Vec<_> = p.visible_states.difference(&current).cloned().collect();
        for key in entered {
            if !enqueue_state(p, key.clone(), self.states[&key].clone()) {
                self.disconnect.push(id);
            }
        }
        for key in left {
            if let Some(&generation) = self.generations.get(&key.0) {
                let message = Message {
                    id: 1,
                    resource: key.0.clone(),
                    scope: key.1.clone(),
                    generation,
                    kind: Kind::State,
                    name: key.2.clone(),
                    value: Value::Null,
                };
                if !enqueue_state(p, key, message) {
                    self.disconnect.push(id);
                }
            }
        }
        p.visible_states = current;
        // Unscheduled private events cannot follow a departed interest scope.
        p.events.retain(|m| match m.scope {
            Scope::Resource => true,
            Scope::Player { id: target } => target == id,
            Scope::Instance { id: target } => target == p.instance,
            Scope::Entity { id } => self.entities.get(&id).is_some_and(|e| {
                e.instance == p.instance
                    && p.position.is_none_or(|pos| {
                        pos.iter()
                            .zip(e.position)
                            .map(|(a, b)| (a - b).powi(2))
                            .sum::<f32>()
                            <= 500_f32.powi(2)
                    })
            }),
        });
    }
    pub fn receive(&mut self, id: u64, record: &[u8], now: u64) {
        if !valid_client(record) {
            return;
        }
        let r: ClientRecord = serde_json::from_slice(record).unwrap();
        let peer_count = self.peers.len();
        let Some(p) = self.peers.get_mut(&id) else {
            return;
        };
        if r.epoch != p.epoch || r.revision != self.revision {
            return;
        }
        if p.queue.ack(r.ack).is_err() {
            self.disconnect.push(id);
            return;
        }
        let selected = self.budgets.negotiate(r.budgets);
        if !p.ready && p.budgets != selected {
            p.bulk = crate::bulk::Channel::new(selected.bulk()).unwrap();
            p.budgets = selected;
        }
        if p.ready && p.budgets != selected {
            self.disconnect.push(id);
            return;
        }
        if p.bulk.acknowledge(r.bulk_ack).is_err() {
            self.disconnect.push(id);
            return;
        }
        if r.ready && !p.ready {
            let states: BTreeMap<_, _> = self
                .states
                .iter()
                .filter(|(_, m)| visible(id, p, m, &self.entities))
                .map(|(k, m)| (k.clone(), m.clone()))
                .collect();
            let bytes = states.values().map(Message::bytes).sum::<usize>();
            if bytes > p.budgets.queue_bytes || states.values().any(|m| !m.fits(p.budgets)) {
                self.disconnect.push(id);
                return;
            }
            p.ready = true;
            p.state_order = states.keys().cloned().collect();
            p.visible_states = states.keys().cloned().collect();
            p.pending_states = states;
        }
        if !r.ready {
            p.ready = false;
            return;
        }
        let mut messages = Vec::new();
        if let Some(frame) = r.bulk {
            match p.bulk.receive(&frame) {
                Ok(Some(bytes)) => match serde_json::from_slice::<Message>(&bytes) {
                    Ok(msg)
                        if msg.kind == Kind::Event
                            && msg.scope.is_resource()
                            && msg.fits(p.budgets) =>
                    {
                        messages.push(msg)
                    }
                    _ => {
                        self.disconnect.push(id);
                        return;
                    }
                },
                Ok(None) => {}
                Err(_) => {
                    self.disconnect.push(id);
                    return;
                }
            }
        }
        if let Some(msg) = r.message {
            if p.queue.consume(&msg) == Ok(true) {
                messages.push(msg);
            }
        }
        for msg in messages {
            if now.saturating_sub(p.window) >= 1000 {
                p.window = now;
                p.requests = 0;
            }
            p.requests += 1;
            if p.requests > p.budgets.events_per_second {
                self.disconnect.push(id);
                return;
            }
            if self.generations.get(&msg.resource) != Some(&msg.generation) {
                return;
            }
            if self.incoming.len() >= self.budgets.pending * peer_count.max(1)
                || self
                    .incoming
                    .iter()
                    .map(|m| m.message.bytes())
                    .sum::<usize>()
                    .saturating_add(msg.bytes())
                    > self.budgets.queue_bytes
            {
                self.disconnect.push(id);
                return;
            }
            self.incoming.push(Incoming {
                sender: id,
                message: msg,
            });
        }
    }
    pub fn record(&mut self, id: u64) -> Option<Vec<u8>> {
        let p = self.peers.get_mut(&id)?;
        if p.ready && p.queue.pending.is_empty() && p.bulk.pending_count() < p.budgets.pending {
            // Assign sequence IDs when scheduling, so a sustained event stream
            // cannot starve authoritative state (and vice versa).
            if !p.state_order.is_empty() && (p.events.is_empty() || !p.last_state) {
                if let Some(key) = p.state_order.pop_front() {
                    if let Some(message) = p.pending_states.remove(&key) {
                        if message.kind == Kind::Event && message.small() {
                            let _ = p.queue.push(message);
                        } else if let Ok(bytes) = serde_json::to_vec(&message) {
                            if p.bulk.send(bytes).is_err() {
                                self.disconnect.push(id);
                            }
                        }
                        p.last_state = true;
                    }
                }
            } else if let Some(message) = p.events.pop_front() {
                if message.kind == Kind::Event && message.small() {
                    let _ = p.queue.push(message);
                } else if let Ok(bytes) = serde_json::to_vec(&message) {
                    if p.bulk.send(bytes).is_err() {
                        self.disconnect.push(id);
                    }
                }
                p.last_state = false;
            }
        }
        let send_bulk =
            p.ready && p.bulk.frame().is_some() && (p.queue.pending.is_empty() || !p.bulk_turn);
        p.bulk_turn = send_bulk;
        serde_json::to_vec(&ServerRecord {
            bulk: if send_bulk { p.bulk.frame() } else { None },
            bulk_ack: p.bulk.ack(),
            offer: Offer {
                budgets: self.budgets,
                revision: self.revision.clone(),
                port: self.port,
                epoch: p.epoch,
            },
            ack: p.queue.received,
            message: if p.ready && !send_bulk {
                p.queue.pending.front().cloned()
            } else {
                None
            },
        })
        .ok()
    }
    pub fn send(&mut self, recipient: Option<u64>, mut message: Message) -> Result<(), String> {
        message.id = 1;
        if !message.fits(self.budgets)
            || self.generations.get(&message.resource) != Some(&message.generation)
        {
            return Err("Unknown resource generation or invalid message".into());
        }
        if matches!(message.scope,Scope::Entity{id} if self.entities.get(&id).is_none_or(|e|e.resource!=message.resource || e.generation!=message.generation))
            || matches!(message.scope,Scope::Player{id} if !self.peers.contains_key(&id))
        {
            return Err(
                "Scope target is unavailable or not owned by this resource generation".into(),
            );
        }
        if message.kind == Kind::State {
            if recipient.is_some() {
                return Err("Resource state is server-owned and broadcast".into());
            }
            let key = (
                message.resource.clone(),
                message.scope.clone(),
                message.name.clone(),
            );
            if !message.value.is_null()
                && !self.states.contains_key(&key)
                && self
                    .states
                    .keys()
                    .filter(|(resource, _, _)| resource == &message.resource)
                    .count()
                    >= self.budgets.state_keys
            {
                return Err("Resource state key budget exhausted".into());
            }
            let state_bytes = self
                .states
                .iter()
                .filter(|(k, _)| *k != &key)
                .map(|(_, m)| m.bytes())
                .sum::<usize>();
            if !message.value.is_null()
                && state_bytes.saturating_add(message.bytes()) > self.budgets.state_bytes
            {
                return Err("Resource state byte budget exhausted".into());
            }
            if message.value.is_null() {
                self.states.remove(&key);
            } else {
                self.states.insert(key.clone(), message.clone());
            }
            for (&id, p) in self.peers.iter_mut().filter(|(_, p)| p.ready) {
                if !visible(id, p, &message, &self.entities) {
                    continue;
                }
                if message.value.is_null() {
                    p.visible_states.remove(&key);
                } else {
                    p.visible_states.insert(key.clone());
                }
                if !enqueue_state(p, key.clone(), message.clone()) {
                    self.disconnect.push(id);
                }
            }
            return Ok(());
        }
        for (&id, p) in &mut self.peers {
            if p.ready
                && recipient.is_none_or(|to| id == to)
                && visible(id, p, &message, &self.entities)
            {
                if !message.fits(p.budgets) {
                    return Err(format!("Recipient {id} negotiated a smaller value budget"));
                }
                if p.events.len() + p.queue.pending.len() + p.bulk.pending_count()
                    >= p.budgets.pending
                    || p.queued_bytes().saturating_add(message.bytes()) > p.budgets.queue_bytes
                {
                    self.disconnect.push(id);
                } else {
                    p.events.push_back(message.clone());
                }
            }
        }
        Ok(())
    }
    pub fn start_large(&mut self, recipient:u64, mut message:Message) -> Result<LargeTicket,String> {
        message.id=1;
        if message.kind!=Kind::Event || !message.scope.is_resource()
            || self.generations.get(&message.resource)!=Some(&message.generation) {
            return Err("Large transfer requires a live resource event and explicit recipient".into());
        }
        let p=self.peers.get_mut(&recipient).filter(|p|p.ready).ok_or("Large transfer recipient is not ready")?;
        if !message.fits(p.budgets) {return Err("Message exceeds recipient value budget".into());}
        let bytes=serde_json::to_vec(&message).map_err(|e|e.to_string())?;
        if p.events.len()+p.queue.pending.len()+p.bulk.pending_count()>=p.budgets.pending
            || p.queued_bytes().saturating_add(bytes.len())>p.budgets.queue_bytes {
            return Err("Large transfer backpressure: recipient queue budget exhausted".into());
        }
        Ok(LargeTicket {recipient,epoch:p.epoch,id:p.bulk.send(bytes)?})
    }
    pub fn large_progress(&self, ticket:LargeTicket) -> Result<crate::bulk::Progress,String> {
        let p=self.peers.get(&ticket.recipient).filter(|p|p.ready && p.epoch==ticket.epoch)
            .ok_or("Large transfer recipient connection retired")?;
        p.bulk.progress(ticket.id)
    }
    pub fn cancel_large(&mut self, ticket:LargeTicket) -> Result<(),String> {
        self.large_progress(ticket)?;
        self.peers.get_mut(&ticket.recipient).unwrap().bulk.cancel(ticket.id)
    }
    pub fn drain(&mut self) -> Vec<Incoming> {
        std::mem::take(&mut self.incoming)
    }
    pub fn disconnects(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.disconnect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_large_handles_are_epoch_bound_and_report_ack_progress() {
        let mut host=Authority::default();
        host.configure("a".repeat(64),31030,BTreeMap::from([("app".into(),1)])).unwrap();
        host.add(2,7);
        let mut client=Client::default();
        client.receive(&serde_json::from_slice(&host.record(2).unwrap()).unwrap()).unwrap();
        client.set_ready(true);host.receive(2,&client.encode().unwrap(),0);
        let message=Message {scope:Scope::Resource,id:1,resource:"app".into(),generation:1,kind:Kind::Event,name:"large".into(),value:Value::String("x".repeat(2000))};
        assert!(host.start_large(99,message.clone()).is_err());
        let server_ticket=host.start_large(2,message.clone()).unwrap();
        let client_ticket=client.start_large("app",1,"large",message.value).unwrap();
        for tick in 1..100 {
            client.receive(&serde_json::from_slice(&host.record(2).unwrap()).unwrap()).unwrap();
            host.receive(2,&client.encode().unwrap(),tick*20);
            // Avoid the bounded receiver queues being mistaken for progress storage.
            if matches!(host.large_progress(server_ticket).unwrap(),crate::bulk::Progress::Delivered{..})
                && matches!(client.large_progress(client_ticket).unwrap(),crate::bulk::Progress::Delivered{..}) {break;}
        }
        assert!(matches!(host.large_progress(server_ticket).unwrap(),crate::bulk::Progress::Delivered{..}));
        assert!(matches!(client.large_progress(client_ticket).unwrap(),crate::bulk::Progress::Delivered{..}));
        assert_eq!(host.drain().len(),1);assert_eq!(client.take_incoming().len(),1);
        host.add(2,8);
        assert!(host.large_progress(server_ticket).is_err());
        client.receive(&serde_json::from_slice(&host.record(2).unwrap()).unwrap()).unwrap();
        assert!(client.large_progress(client_ticket).is_err());
        assert!(client.cancel_ticket(client_ticket).is_err());
    }
    #[test]
    fn continuous_events_do_not_starve_pending_state() {
        let mut host = Authority::default();
        host.configure("a".repeat(64), 31030, BTreeMap::from([("app".into(), 1)]))
            .unwrap();
        host.add(2, 7);
        let client = ClientRecord {
            budgets: Budgets::default(),
            bulk: None,
            bulk_ack: Default::default(),
            epoch: 7,
            revision: "a".repeat(64),
            ready: true,
            ack: 0,
            message: None,
        };
        host.receive(2, &serde_json::to_vec(&client).unwrap(), 0);
        host.send(
            None,
            Message {
                scope: Default::default(),
                id: 1,
                resource: "app".into(),
                generation: 1,
                kind: Kind::State,
                name: "state".into(),
                value: Value::Bool(true),
            },
        )
        .unwrap();
        let mut delivered = false;
        let mut runtime = Client::default();
        runtime
            .receive(&ServerRecord {
                offer: Offer {
                    budgets: Budgets::default(),
                    revision: "a".repeat(64),
                    port: 31030,
                    epoch: 7,
                },
                ack: 0,
                message: None,
                bulk: None,
                bulk_ack: Default::default(),
            })
            .unwrap();
        runtime.set_ready(true);
        for now in 0..8 {
            host.send(
                Some(2),
                Message {
                    scope: Default::default(),
                    id: 1,
                    resource: "app".into(),
                    generation: 1,
                    kind: Kind::Event,
                    name: "tick".into(),
                    value: Value::Null,
                },
            )
            .unwrap();
            let record: ServerRecord = serde_json::from_slice(&host.record(2).unwrap()).unwrap();
            runtime.receive(&record).unwrap();
            delivered |= runtime
                .take_incoming()
                .iter()
                .any(|m| m.kind == Kind::State);
            host.receive(2, &runtime.encode().unwrap(), now);
        }
        assert!(delivered, "continuous events starved authoritative state");
    }
}

#[cfg(test)]
mod budget_regressions {
    use super::*;
    #[test]
    fn tombstones_cannot_escape_pending_state_byte_budget() {
        let budget = Budgets {
            value_bytes: 1024,
            queue_bytes: 2048,
            resources: 1,
            ..Default::default()
        };
        let mut authority = Authority::default();
        authority.set_budgets(budget).unwrap();
        authority
            .configure("a".repeat(64), 31030, BTreeMap::from([("app".into(), 1)]))
            .unwrap();
        authority.add(2, 1);
        let record = ClientRecord {
            budgets: budget,
            bulk: None,
            bulk_ack: Default::default(),
            epoch: 1,
            revision: "a".repeat(64),
            ready: true,
            ack: 0,
            message: None,
        };
        authority.receive(2, &serde_json::to_vec(&record).unwrap(), 0);
        for n in 0..100 {
            for value in [Value::Bool(true), Value::Null] {
                authority
                    .send(
                        None,
                        Message {
                            scope: Default::default(),
                            id: 1,
                            resource: "app".into(),
                            generation: 1,
                            kind: Kind::State,
                            name: format!("key{n}"),
                            value,
                        },
                    )
                    .unwrap();
            }
            assert!(authority.peers[&2].queued_bytes() <= budget.queue_bytes);
        }
        assert!(!authority.disconnects().is_empty());
    }
    #[test]
    fn client_count_budget_covers_both_lanes_and_can_exceed_old_sixty_four() {
        let budget = Budgets {
            pending: 65,
            ..Default::default()
        };
        let mut client = Client::with_budgets(budget).unwrap();
        client
            .receive(&ServerRecord {
                offer: Offer {
                    budgets: budget,
                    revision: "a".repeat(64),
                    port: 31030,
                    epoch: 1,
                },
                ack: 0,
                message: None,
                bulk: None,
                bulk_ack: Default::default(),
            })
            .unwrap();
        client
            .emit_large("app", 1, "large", Value::String("x".repeat(1000)))
            .unwrap();
        for _ in 0..64 {
            client.emit("app", 1, "small", Value::Null).unwrap();
        }
        assert!(client.emit("app", 1, "small", Value::Null).is_err());
        assert!(
            client
                .emit_large("app", 1, "large", Value::String("x".repeat(1000)))
                .is_err()
        );
    }
}

#[cfg(test)]
mod scope_regressions {
    use super::*;
    fn setup() -> (Authority, BTreeMap<u64, Client>) {
        let mut host = Authority::default();
        host.configure("a".repeat(64), 31030, BTreeMap::from([("app".into(), 1)]))
            .unwrap();
        let mut clients = BTreeMap::new();
        for id in [2, 3] {
            host.add(id, id + 10);
            let mut c = Client::default();
            c.receive(&serde_json::from_slice(&host.record(id).unwrap()).unwrap())
                .unwrap();
            c.set_ready(true);
            host.receive(id, &c.encode().unwrap(), 0);
            clients.insert(id, c);
        }
        (host, clients)
    }
    fn pump(host: &mut Authority, clients: &mut BTreeMap<u64, Client>) -> Vec<(u64, Message)> {
        let mut incoming = Vec::new();
        for now in 1..20 {
            for (&id, c) in clients.iter_mut() {
                c.receive(&serde_json::from_slice(&host.record(id).unwrap()).unwrap())
                    .unwrap();
                incoming.extend(c.take_incoming().into_iter().map(|m| (id, m)));
                host.receive(id, &c.encode().unwrap(), now);
            }
        }
        incoming
    }
    fn message(scope: Scope, kind: Kind) -> Message {
        Message {
            scope,
            id: 1,
            resource: "app".into(),
            generation: 1,
            kind,
            name: "private".into(),
            value: Value::String("secret".into()),
        }
    }
    #[test]
    fn player_scoped_state_never_enters_other_players_resource_channel() {
        let (mut host, mut clients) = setup();
        host.send(None, message(Scope::Player { id: 2 }, Kind::State))
            .unwrap();
        let messages = pump(&mut host, &mut clients);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].0, 2);
    }
    #[test]
    fn entity_scoped_state_requires_current_entity_owned_by_the_sending_resource() {
        let (mut host, _) = setup();
        assert!(
            host.send(None, message(Scope::Entity { id: 9 }, Kind::State))
                .is_err()
        );
    }
    #[test]
    fn new_resource_epoch_discards_queued_small_and_bulk_data_before_readmission() {
        let (mut host, mut clients) = setup();
        host.send(Some(2), message(Scope::Resource, Kind::Event))
            .unwrap();
        let mut large = message(Scope::Resource, Kind::Event);
        large.value = Value::String("x".repeat(2000));
        host.send(Some(2), large).unwrap();
        host.record(2);
        host.add(2, 100);
        let record: ServerRecord = serde_json::from_slice(&host.record(2).unwrap()).unwrap();
        assert_eq!(record.offer.epoch, 100);
        assert!(record.message.is_none() && record.bulk.is_none());
        assert!(clients.get_mut(&2).unwrap().receive(&record).unwrap());
        assert!(clients[&2].incoming.is_empty());
    }
    #[test]
    fn instance_state_names_do_not_alias_and_readmission_replays_only_new_instance() {
        let (mut host, mut clients) = setup();
        host.context(3, 7, None);
        let mut zero = message(Scope::Instance { id: 0 }, Kind::State);
        zero.value = Value::String("public room".into());
        let mut seven = message(Scope::Instance { id: 7 }, Kind::State);
        seven.value = Value::String("private room".into());
        host.send(None, zero).unwrap();
        host.send(None, seven).unwrap();
        let initial = pump(&mut host, &mut clients);
        assert_eq!(initial.len(), 2);
        assert!(
            initial
                .iter()
                .any(|(id, m)| *id == 2 && m.value == "public room")
        );
        assert!(
            initial
                .iter()
                .any(|(id, m)| *id == 3 && m.value == "private room")
        );
        host.add(2, 100);
        host.context(2, 7, None);
        let record = serde_json::from_slice(&host.record(2).unwrap()).unwrap();
        clients.get_mut(&2).unwrap().receive(&record).unwrap();
        clients.get_mut(&2).unwrap().set_ready(true);
        host.receive(2, &clients[&2].encode().unwrap(), 50);
        let changed = pump(&mut host, &mut clients);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0, 2);
        assert_eq!(changed[0].1.value, "private room");
    }
    #[test]
    fn entity_interest_initializes_state_on_entry_and_retires_state_on_deletion() {
        let (mut host, mut clients) = setup();
        let entity:crate::entities::Entity=serde_json::from_value(serde_json::json!({
            "id":9,"resource":"app","generation":1,"key":"box","instance":0,"controller":null,
            "definition":{"shape":{"type":"box","half_extents":[0.5,0.5,0.5]},"body_type":"dynamic","mass":1,"friction":0.7,"color":[1,1,1,1]},
            "tick":1,"epoch":1,"position":[0,1,0],"rotation":[0,0,0,1],"velocity":[0,0,0],"angular":[0,0,0]})).unwrap();
        host.entities(&[entity]);
        host.context(3, 0, Some([1000., 0., 0.]));
        host.send(None, message(Scope::Entity { id: 9 }, Kind::State))
            .unwrap();
        let initial = pump(&mut host, &mut clients);
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].0, 2);
        host.context(3, 0, Some([0., 0., 0.]));
        let entered = pump(&mut host, &mut clients);
        assert_eq!(entered.len(), 1);
        assert_eq!(entered[0].0, 3);
        host.entities(&[]);
        let removed = pump(&mut host, &mut clients);
        assert_eq!(removed.len(), 2);
        assert!(removed.iter().all(|(_, m)| m.value.is_null()));
        assert!(host.states.is_empty());
    }
}
