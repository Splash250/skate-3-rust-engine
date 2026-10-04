use crate::{
    wire::{Frame, Playback, Policy},
    *,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy)]
pub struct Player {
    pub actor: u64,
    pub peer: u64,
    pub instance: u64,
    pub epoch: u64,
    pub position: [f32; 3],
}
struct Owner {
    generation: u64,
    active: bool,
    channels: BTreeMap<String, BTreeSet<u64>>,
    muted: BTreeSet<u64>,
    radius: Option<f32>,
}
struct Sender {
    sequence: u64,
    window: u128,
    credit: f64,
    last: u64,
    talking: u64,
    channel: String,
}
struct Receiver {
    queues: BTreeMap<u64, VecDeque<Vec<u8>>>,
    sender_cursor: u64,
    control: Option<Vec<u8>>,
    last_control: u64,
}
impl Receiver {
    fn next(&mut self) -> Option<Vec<u8>> {
        use std::ops::Bound::{Excluded, Unbounded};
        let actor = self
            .queues
            .range((Excluded(self.sender_cursor), Unbounded))
            .next()
            .or_else(|| self.queues.iter().next())
            .map(|(id, _)| *id)?;
        let queue = self.queues.get_mut(&actor).unwrap();
        let packet = queue.pop_front();
        if queue.is_empty() {
            self.queues.remove(&actor);
        }
        self.sender_cursor = actor;
        packet
    }
    fn pending(&self) -> usize {
        self.queues.values().map(VecDeque::len).sum()
    }
}
#[derive(Default, Debug, Clone, Copy, Serialize)]
pub struct Metrics {
    pub accepted: u64,
    pub rejected: u64,
    pub queue_dropped: u64,
    pub sent: u64,
    pub bytes: u64,
}
pub struct Router {
    session: u64,
    server: u64,
    revision: u64,
    owners: BTreeMap<String, Owner>,
    muted: BTreeSet<u64>,
    radius: f32,
    players: BTreeMap<u64, Player>,
    senders: BTreeMap<u64, Sender>,
    receivers: BTreeMap<u64, Receiver>,
    cursor: u64,
    pub metrics: Metrics,
}
impl Router {
    pub fn new(session: u64, server: u64) -> Result<Self> {
        if session == 0 || server == 0 {
            return Err("voice session/server identity must be nonzero".into());
        }
        Ok(Self {
            session,
            server,
            revision: 1,
            owners: BTreeMap::new(),
            muted: BTreeSet::new(),
            radius: 20.,
            players: BTreeMap::new(),
            senders: BTreeMap::new(),
            receivers: BTreeMap::new(),
            cursor: 0,
            metrics: Metrics::default(),
        })
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    fn changed(&mut self) {
        self.muted = self
            .owners
            .values()
            .filter(|o| o.active)
            .flat_map(|o| o.muted.iter().copied())
            .collect();
        self.radius = self
            .owners
            .values()
            .filter(|o| o.active)
            .filter_map(|o| o.radius)
            .reduce(f32::min)
            .unwrap_or(20.);
        self.revision = self.revision.saturating_add(1);
        for receiver in self.receivers.values_mut() {
            receiver.queues.clear();
            receiver.sender_cursor = 0;
            receiver.control = None;
            receiver.last_control = 0;
        }
    }
    pub fn activate(&mut self, resource: &str, generation: u64) -> Result<()> {
        if !name(resource) || generation == 0 {
            return Err("invalid voice resource owner".into());
        }
        if let Some(owner) = self.owners.get(resource) {
            if owner.generation >= generation {
                return Err("stale voice owner generation".into());
            }
        } else if self.owners.len() >= 128 {
            return Err("voice resource owner capacity reached".into());
        }
        self.owners.insert(
            resource.into(),
            Owner {
                generation,
                active: true,
                channels: BTreeMap::new(),
                muted: BTreeSet::new(),
                radius: None,
            },
        );
        self.changed();
        Ok(())
    }
    pub fn revoke(&mut self, resource: &str, generation: u64) {
        if let Some(owner) = self.owners.get_mut(resource) {
            if owner.generation == generation && owner.active {
                owner.active = false;
                owner.channels.clear();
                owner.muted.clear();
                owner.radius = None;
                self.changed();
            }
        }
    }
    pub fn command(
        &mut self,
        resource: &str,
        generation: u64,
        command: ServerCommand,
    ) -> Result<()> {
        command.validate()?;
        let total_channels: usize = self.owners.values().map(|o| o.channels.len()).sum();
        let owner = self
            .owners
            .get_mut(resource)
            .filter(|o| o.active && o.generation == generation)
            .ok_or("inactive or stale voice resource owner")?;
        match command {
            ServerCommand::Channel {
                name: channel,
                members,
            } => {
                if !name(&channel) || members.len() > 64 {
                    return Err("radio channel requires a valid name and at most64 members".into());
                }
                if !owner.channels.contains_key(&channel)
                    && (owner.channels.len() >= 16 || total_channels >= 64)
                {
                    return Err("radio channel capacity reached".into());
                }
                let members = members
                    .iter()
                    .map(|s| actor_id(s))
                    .collect::<Result<BTreeSet<_>>>()?;
                owner.channels.insert(channel, members);
            }
            ServerCommand::RemoveChannel { name: channel } => {
                if !name(&channel) {
                    return Err("invalid radio channel name".into());
                }
                owner.channels.remove(&channel);
            }
            ServerCommand::Mute { player, muted } => {
                let id = actor_id(&player)?;
                if muted {
                    if owner.muted.len() >= 64 && !owner.muted.contains(&id) {
                        return Err("voice mute capacity reached".into());
                    }
                    owner.muted.insert(id);
                } else {
                    owner.muted.remove(&id);
                }
            }
            ServerCommand::Proximity { meters } => {
                if !meters.is_finite() || !(1.0..=100.).contains(&meters) {
                    return Err("voice proximity requires1..100 meters".into());
                }
                owner.radius = Some(meters);
            }
        }
        self.changed();
        Ok(())
    }
    pub fn sync(&mut self, players: &[Player], now: u64) -> Result<()> {
        if players.len() > 64 {
            return Err("voice supports at most64 admitted players".into());
        }
        let mut next = BTreeMap::new();
        for player in players {
            if player.actor == 0
                || player.epoch == 0
                || !player.position.iter().all(|n| n.is_finite())
                || next.insert(player.actor, *player).is_some()
            {
                return Err("invalid or duplicate admitted voice player".into());
            }
        }
        let changed = next.len() != self.players.len()
            || next.iter().any(|(id, p)| {
                self.players.get(id).is_none_or(|old| {
                    (old.instance, old.epoch, old.peer) != (p.instance, p.epoch, p.peer)
                })
            });
        self.players = next;
        self.senders.retain(|id, _| self.players.contains_key(id));
        self.receivers.retain(|id, _| self.players.contains_key(id));
        for id in self.players.keys() {
            self.receivers.entry(*id).or_insert(Receiver {
                queues: BTreeMap::new(),
                sender_cursor: 0,
                control: None,
                last_control: 0,
            });
        }
        if changed {
            self.changed();
        }
        for (id, receiver) in &mut self.receivers {
            if receiver.last_control == 0 || now.saturating_sub(receiver.last_control) >= 100 {
                let player = self.players[id];
                receiver.control = Some(
                    Policy {
                        session: self.session,
                        recipient_epoch: player.epoch,
                        instance: player.instance,
                        revision: self.revision,
                    }
                    .encode(self.server),
                );
                receiver.last_control = now.max(1);
            }
        }
        Ok(())
    }
    pub fn receive(&mut self, peer: u64, bytes: &[u8], now: u64) -> bool {
        let accepted = self.route(peer, bytes, now);
        if accepted {
            self.metrics.accepted += 1;
        } else {
            self.metrics.rejected += 1;
        }
        accepted
    }
    fn route(&mut self, peer: u64, bytes: &[u8], now: u64) -> bool {
        let Some(frame) = Frame::decode(bytes) else {
            return false;
        };
        let Some(speaker) = self.players.get(&frame.actor).copied() else {
            return false;
        };
        if frame.session != self.session
            || frame.epoch != speaker.epoch
            || frame.revision != self.revision
            || speaker.peer != peer
            || self.muted.contains(&frame.actor)
        {
            return false;
        }
        let members = if frame.channel.is_empty() {
            None
        } else {
            let Some((resource, channel)) = frame.channel.split_once('/') else {
                return false;
            };
            let Some(members) = self
                .owners
                .get(resource)
                .filter(|o| o.active)
                .and_then(|o| o.channels.get(channel))
            else {
                return false;
            };
            if !members.contains(&frame.actor) {
                return false;
            }
            Some(members.clone())
        };
        let sender = self.senders.entry(frame.actor).or_insert(Sender {
            sequence: 0,
            window: 0,
            credit: 5.,
            last: now,
            talking: now,
            channel: frame.channel.clone(),
        });
        if now < sender.last {
            return false;
        }
        sender.credit = (sender.credit + (now - sender.last) as f64 * 0.05).min(5.);
        sender.last = now;
        if sender.credit < 1.
            || frame.sequence == 0
            || (frame.sequence <= sender.sequence
                && (sender.sequence - frame.sequence >= 128
                    || sender.window & (1u128 << (sender.sequence - frame.sequence)) != 0))
        {
            return false;
        }
        sender.credit -= 1.;
        sender.talking = now;
        sender.channel = frame.channel.clone();
        if frame.sequence > sender.sequence {
            let shift = frame.sequence - sender.sequence;
            sender.window = if shift >= 128 {
                1
            } else {
                (sender.window << shift) | 1
            };
            sender.sequence = frame.sequence;
        } else {
            sender.window |= 1u128 << (sender.sequence - frame.sequence);
        }
        let radius = self.radius;
        for listener in self.players.values() {
            if listener.actor == speaker.actor || listener.instance != speaker.instance {
                continue;
            }
            let distance = distance(speaker.position, listener.position);
            if members
                .as_ref()
                .is_some_and(|m| !m.contains(&listener.actor))
                || members.is_none() && distance >= radius
            {
                continue;
            }
            // At most eight currently transmitting speakers per receiver, ordered
            // by distance then stable actor ID. Radio memberships use same bound.
            let closer = self
                .senders
                .iter()
                .filter(|(id, s)| **id != listener.actor && now.saturating_sub(s.talking) <= 500)
                .filter(|(id, sender)| {
                    let Some(p) = self.players.get(id) else {
                        return false;
                    };
                    if p.instance != listener.instance || self.muted.contains(id) {
                        return false;
                    }
                    let d = distance_fn(p.position, listener.position);
                    let audible = if sender.channel.is_empty() {
                        d < radius
                    } else {
                        sender
                            .channel
                            .split_once('/')
                            .and_then(|(owner, channel)| {
                                self.owners
                                    .get(owner)
                                    .filter(|o| o.active)
                                    .and_then(|o| o.channels.get(channel))
                            })
                            .is_some_and(|m| m.contains(id) && m.contains(&listener.actor))
                    };
                    if !audible {
                        return false;
                    }
                    d < distance || (d == distance && **id < speaker.actor)
                })
                .take(MAX_TALKERS)
                .count();
            if closer >= MAX_TALKERS {
                continue;
            }
            let gain = if members.is_some() {
                1.
            } else {
                (1. - distance / radius).clamp(0., 1.)
            };
            let packet = Playback {
                session: self.session,
                actor: speaker.actor,
                recipient_epoch: listener.epoch,
                sender_epoch: speaker.epoch,
                revision: self.revision,
                sequence: frame.sequence,
                gain,
                data: frame.data.clone(),
            }
            .encode();
            let receiver = self.receivers.get_mut(&listener.actor).unwrap();
            if !receiver.queues.contains_key(&speaker.actor) && receiver.queues.len() >= MAX_TALKERS
            {
                // Newly selected nearer talkers replace the farthest buffered
                // sender. Drained senders do not consume an allocation slot.
                let victim = receiver
                    .queues
                    .keys()
                    .max_by(|a, b| {
                        let distance = |id: &u64| {
                            self.players
                                .get(id)
                                .map_or(f32::MAX, |p| distance_fn(p.position, listener.position))
                        };
                        distance(a).total_cmp(&distance(b)).then(a.cmp(b))
                    })
                    .copied()
                    .unwrap();
                self.metrics.queue_dropped += receiver.queues.remove(&victim).unwrap().len() as u64;
            }
            let queue = receiver.queues.entry(speaker.actor).or_default();
            if queue.len() >= 2 {
                queue.pop_front();
                self.metrics.queue_dropped += 1;
            }
            queue.push_back(packet);
        }
        true
    }
    /// Fair capped draining is scheduled after gameplay packets by the host.
    pub fn drain(&mut self, limit: usize) -> Vec<(u64, Vec<u8>)> {
        let mut out = Vec::new();
        let mut actors = self.receivers.keys().copied().collect::<Vec<_>>();
        if actors.is_empty() {
            return out;
        }
        let pivot = actors.partition_point(|id| *id <= self.cursor);
        actors.rotate_left(pivot);
        for _ in 0..(MAX_TALKERS * 2 + 1) {
            for id in &actors {
                if out.len() >= limit.min(512) {
                    return out;
                }
                let receiver = self.receivers.get_mut(id).unwrap();
                if let Some(packet) = receiver.control.take().or_else(|| receiver.next()) {
                    self.cursor = *id;
                    self.metrics.sent += 1;
                    self.metrics.bytes += packet.len() as u64;
                    out.push((self.players[id].peer, packet));
                }
            }
        }
        out
    }
    pub fn pending(&self) -> usize {
        self.receivers
            .values()
            .map(|r| r.pending() + usize::from(r.control.is_some()))
            .sum()
    }
}
fn name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn actor_id(s: &str) -> Result<u64> {
    s.parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && id.to_string() == s)
        .ok_or_else(|| "player must be a canonical nonzero decimal actor ID".into())
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    distance_fn(a, b)
}
fn distance_fn(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (*a - b).powi(2))
        .sum::<f32>()
        .sqrt()
}
