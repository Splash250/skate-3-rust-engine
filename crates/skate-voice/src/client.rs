use crate::wire::{Playback, Policy};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    pub session: u64,
    pub host_peer: u64,
    pub actor: u64,
    pub epoch: u64,
    pub instance: u64,
}
pub enum Incoming {
    Policy(Policy),
    Playback { packet: Playback, changed: bool },
}
/// Validates the admitted server, recipient movement baseline and policy epoch
/// before any decoded samples reach the device worker.
#[derive(Default)]
pub struct ClientState {
    context: Option<Context>,
    revision: u64,
    replay: BTreeMap<u64, (u64, u64, u128)>,
}
impl ClientState {
    pub fn synchronize(&mut self, context: Option<Context>) -> bool {
        if self.context != context {
            self.context = context;
            self.revision = 0;
            self.replay.clear();
            true
        } else {
            false
        }
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn context(&self) -> Option<Context> {
        self.context
    }
    pub fn accept(&mut self, peer: u64, bytes: &[u8]) -> Option<Incoming> {
        let context = self.context?;
        if peer != context.host_peer {
            return None;
        }
        if let Some(policy) = Policy::decode(bytes) {
            if policy.session != context.session
                || policy.recipient_epoch != context.epoch
                || policy.instance != context.instance
                || policy.revision < self.revision
            {
                return None;
            }
            if policy.revision > self.revision {
                self.revision = policy.revision;
                self.replay.clear();
                return Some(Incoming::Policy(policy));
            }
            return None;
        }
        let packet = Playback::decode(bytes)?;
        if packet.session != context.session
            || packet.actor == context.actor
            || packet.recipient_epoch != context.epoch
            || packet.revision < self.revision
        {
            return None;
        }
        let changed = packet.revision > self.revision;
        if changed {
            self.revision = packet.revision;
            self.replay.clear();
        }
        if self.replay.len() >= 64 && !self.replay.contains_key(&packet.actor) {
            return None;
        }
        let replay = self
            .replay
            .entry(packet.actor)
            .or_insert((packet.sender_epoch, 0, 0));
        if replay.0 != packet.sender_epoch {
            *replay = (packet.sender_epoch, 0, 0);
        }
        if packet.sequence <= replay.1
            && (replay.1 - packet.sequence >= 128
                || replay.2 & (1u128 << (replay.1 - packet.sequence)) != 0)
        {
            return None;
        }
        if packet.sequence > replay.1 {
            let shift = packet.sequence - replay.1;
            replay.2 = if shift >= 128 {
                1
            } else {
                (replay.2 << shift) | 1
            };
            replay.1 = packet.sequence;
        } else {
            replay.2 |= 1u128 << (replay.1 - packet.sequence);
        }
        Some(Incoming::Playback { packet, changed })
    }
}
