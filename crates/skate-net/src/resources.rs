//! Reliable, ordered resource messages carried by bounded application records.
//! Each connection has an epoch; each resource has a generation. No transport
//! endpoints, content paths or Lua values are trusted as sender identities.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};

pub const CLIENT_KEY: &str = "resource:client";
pub const MAX_PENDING: usize = 64;
pub const MAX_PAYLOAD: usize = 384;
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
    pub revision: String,
    pub port: u16,
    pub epoch: u64,
}
impl Offer {
    fn valid(&self) -> bool {
        digest(&self.revision) && self.port != 0 && self.epoch != 0
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Event,
    State,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub id: u64,
    pub resource: String,
    pub generation: u64,
    pub kind: Kind,
    pub name: String,
    pub value: Value,
}
impl Message {
    pub fn valid(&self) -> bool {
        self.id != 0
            && self.generation != 0
            && identifier(&self.resource)
            && identifier(&self.name)
            && serde_json::to_vec(&self.value).is_ok_and(|v| v.len() <= MAX_PAYLOAD)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientRecord {
    pub epoch: u64,
    pub revision: String,
    pub ready: bool,
    pub ack: u64,
    pub message: Option<Message>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerRecord {
    pub offer: Offer,
    pub ack: u64,
    pub message: Option<Message>,
}
#[derive(Clone, Debug)]
pub struct Incoming {
    pub sender: u64,
    pub message: Message,
}
pub(crate) fn valid_client(bytes: &[u8]) -> bool {
    bytes.len() <= crate::lobby::MAX_APP_VALUE
        && serde_json::from_slice::<ClientRecord>(bytes).is_ok_and(|r| {
            r.epoch != 0
                && digest(&r.revision)
                && r.message
                    .as_ref()
                    .is_none_or(|m| m.valid() && m.kind == Kind::Event)
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
        if self.pending.len() >= MAX_PENDING {
            return Err("Resource message queue is full".into());
        }
        msg.id = self
            .next
            .checked_add(1)
            .ok_or("Resource sequence exhausted")?;
        if !msg.valid() {
            return Err("Invalid resource message or payload exceeds 384 bytes".into());
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
    offer: Option<Offer>,
    ready: bool,
    queue: Queue,
    incoming: Vec<Message>,
}
impl Client {
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
            *self = Self {
                offer: Some(record.offer.clone()),
                ..Self::default()
            };
        }
        self.queue.ack(record.ack)?;
        if let Some(msg) = &record.message {
            if !self.ready {
                return Err("Resource message before client activation".into());
            }
            if self.queue.consume(msg)? {
                if self.incoming.len() >= MAX_PENDING {
                    return Err("Resource incoming queue full".into());
                }
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
        self.queue.push(Message {
            id: 0,
            resource: resource.into(),
            generation,
            kind: Kind::Event,
            name: name.into(),
            value,
        })
    }
    pub fn take_incoming(&mut self) -> Vec<Message> {
        std::mem::take(&mut self.incoming)
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let offer = self.offer.as_ref().ok_or("No resource connection")?;
        let r = ClientRecord {
            epoch: offer.epoch,
            revision: offer.revision.clone(),
            ready: self.ready,
            ack: self.queue.received,
            message: if self.ready {
                self.queue.pending.front().cloned()
            } else {
                None
            },
        };
        serde_json::to_vec(&r).map_err(|e| e.to_string())
    }
}
struct Peer {
    epoch: u64,
    ready: bool,
    queue: Queue,
    window: u64,
    requests: u32,
    pending_states: BTreeMap<(String, String), Message>,
    state_order: VecDeque<(String, String)>,
    events: VecDeque<Message>,
    last_state: bool,
}
#[derive(Default)]
pub(crate) struct Authority {
    pub revision: String,
    pub port: u16,
    generations: BTreeMap<String, u64>,
    peers: BTreeMap<u64, Peer>,
    incoming: Vec<Incoming>,
    states: BTreeMap<(String, String), Message>,
    disconnect: Vec<u64>,
}
impl Authority {
    pub fn configure(
        &mut self,
        revision: String,
        port: u16,
        generations: BTreeMap<String, u64>,
    ) -> Result<(), String> {
        if !digest(&revision)
            || port == 0
            || generations.len() > 32
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
    pub fn active(&self) -> bool {
        !self.revision.is_empty()
    }
    pub fn ready(&self, id: u64) -> bool {
        !self.active() || self.peers.get(&id).is_some_and(|p| p.ready)
    }
    pub fn retain(&mut self, ids: &[u64]) {
        self.peers.retain(|id, _| ids.contains(id));
    }
    pub fn add(&mut self, id: u64, epoch: u64) {
        self.peers.entry(id).or_insert(Peer {
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
    pub fn receive(&mut self, id: u64, record: &[u8], now: u64) {
        if !valid_client(record) {
            return;
        }
        let r: ClientRecord = serde_json::from_slice(record).unwrap();
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
        if r.ready && !p.ready {
            p.ready = true;
            p.pending_states = self.states.clone();
            p.state_order = self.states.keys().cloned().collect();
        }
        if !r.ready {
            p.ready = false;
            return;
        }
        if let Some(msg) = r.message {
            if p.queue.consume(&msg) != Ok(true) {
                return;
            }
            if now.saturating_sub(p.window) >= 1000 {
                p.window = now;
                p.requests = 0;
            }
            p.requests += 1;
            if p.requests > 20 {
                self.disconnect.push(id);
                return;
            }
            if self.generations.get(&msg.resource) != Some(&msg.generation) {
                return;
            }
            if self.incoming.len() >= MAX_PENDING {
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
        if p.ready && p.queue.pending.is_empty() {
            // Assign sequence IDs when scheduling, so a sustained event stream
            // cannot starve authoritative state (and vice versa).
            if !p.state_order.is_empty() && (p.events.is_empty() || !p.last_state) {
                if let Some(key) = p.state_order.pop_front() {
                    if let Some(message) = p.pending_states.remove(&key) {
                        let _ = p.queue.push(message);
                        p.last_state = true;
                    }
                }
            } else if let Some(message) = p.events.pop_front() {
                let _ = p.queue.push(message);
                p.last_state = false;
            }
        }
        serde_json::to_vec(&ServerRecord {
            offer: Offer {
                revision: self.revision.clone(),
                port: self.port,
                epoch: p.epoch,
            },
            ack: p.queue.received,
            message: if p.ready {
                p.queue.pending.front().cloned()
            } else {
                None
            },
        })
        .ok()
    }
    pub fn send(&mut self, recipient: Option<u64>, mut message: Message) -> Result<(), String> {
        message.id = 1;
        if !message.valid() || self.generations.get(&message.resource) != Some(&message.generation)
        {
            return Err("Unknown resource generation or invalid message".into());
        }
        if message.kind == Kind::State {
            if recipient.is_some() {
                return Err("Resource state is server-owned and broadcast".into());
            }
            let key = (message.resource.clone(), message.name.clone());
            if !message.value.is_null()
                && !self.states.contains_key(&key)
                && self
                    .states
                    .keys()
                    .filter(|(resource, _)| resource == &message.resource)
                    .count()
                    >= 64
            {
                return Err("Resource state limit (64 keys per resource)".into());
            }
            if message.value.is_null() {
                self.states.remove(&key);
            } else {
                self.states.insert(key.clone(), message.clone());
            }
            for (&id, p) in self.peers.iter_mut().filter(|(_, p)| p.ready) {
                if !p.pending_states.contains_key(&key) {
                    if p.pending_states.len() >= 32 * 64 {
                        self.disconnect.push(id);
                        continue;
                    }
                    p.state_order.push_back(key.clone());
                }
                p.pending_states.insert(key.clone(), message.clone());
            }
            return Ok(());
        }
        for (&id, p) in &mut self.peers {
            if p.ready && recipient.is_none_or(|to| id == to) {
                if p.events.len() + p.queue.pending.len() >= MAX_PENDING {
                    self.disconnect.push(id);
                } else {
                    p.events.push_back(message.clone());
                }
            }
        }
        Ok(())
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
    fn continuous_events_do_not_starve_pending_state() {
        let mut host = Authority::default();
        host.configure("a".repeat(64), 31030, BTreeMap::from([("app".into(), 1)]))
            .unwrap();
        host.add(2, 7);
        let mut client = ClientRecord {
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
        for now in 0..4 {
            host.send(
                Some(2),
                Message {
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
            let message = record.message.unwrap();
            delivered |= message.kind == Kind::State;
            client.ack = message.id;
            host.receive(2, &serde_json::to_vec(&client).unwrap(), now);
        }
        assert!(delivered, "continuous events starved authoritative state");
    }
}
