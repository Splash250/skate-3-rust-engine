//! Built-in dedicated gameplay. The server owns shared collisions and shoves;
//! the local client predicts skating and publishes its body, pose and tricks.
use super::{
    Multiplayer,
    dedicated_input::{ShoveInput, trick_label},
};
use crate::physics::{GamePhysics, PlayerControls, SkaterRuntime, network};
use bevy::prelude::*;
use skate_net::dedicated::{
    ClientEffects, EFFECT_ACK_KEY, EffectBatch, GAMEPLAY_KEY, Gameplay, PlayerMode, SHOVE_KEY,
    ShoveRequest, effects_key,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Client {
    server: Option<u64>,
    effects: ClientEffects,
    input: ShoveInput,
    published: Vec<u8>,
    gameplay: BTreeMap<u64, Gameplay>,
}
impl Client {
    fn observe_server(&mut self, server: u64) -> bool {
        if self.server == Some(server) {
            return false;
        }
        *self = Self {
            server: Some(server),
            ..Self::default()
        };
        true
    }
    pub fn activity(&self, actor: u64) -> Option<String> {
        let state = self.gameplay.get(&actor)?;
        if state.mode == PlayerMode::Ragdoll {
            return Some("Bailed".into());
        }
        if !state.trick.is_empty() {
            return Some(
                state
                    .trick
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(64)
                    .collect(),
            );
        }
        if !state.landed_trick.is_empty() {
            return Some(format!(
                "Landed {}",
                state
                    .landed_trick
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(64)
                    .collect::<String>()
            ));
        }
        None
    }
}

/// Before local physics, receive only the connected server endpoint's effect stream.
/// Receiver state and pending impulses are scoped to this connection identity.
pub(super) fn fixed(
    mut net: ResMut<Multiplayer>,
    mut physics: ResMut<GamePhysics>,
    mut skater: ResMut<SkaterRuntime>,
    controls: Res<PlayerControls>,
) {
    if !net.is_dedicated() {
        physics.network_delta_velocity = [0.; 3];
        return;
    }
    if physics.failed || physics.ticks == 0 {
        return;
    }
    let Some((local, host)) = net
        .lobby
        .as_ref()
        .and_then(|l| l.host_actor().map(|h| (l.local, h)))
    else {
        return;
    };
    // A new control actor is a new authority, even if its wall clock moved
    // backwards. Transient roster absence alone must not reset deduplication.
    if net.dedicated.observe_server(host) {
        physics.network_delta_velocity = [0.; 3];
    }
    let batch = net
        .lobby
        .as_ref()
        .and_then(|l| l.actors.get(&host))
        .and_then(|a| a.application.get(&effects_key(local)))
        .and_then(|r| serde_json::from_slice::<EffectBatch>(&r.value).ok());
    if let Some(batch) = batch {
        let effects = net.dedicated.effects.consume(&batch);
        for effect in effects {
            if effect.target != local {
                continue;
            }
            if let Err(error) = network::apply_server_effect(&mut physics, &mut skater, &effect) {
                // Do not acknowledge a hit the physics adapter could not apply.
                net.leave();
                physics.network_delta_velocity = [0.; 3];
                net.status = format!("Dedicated gameplay stopped: {error}");
                return;
            }
        }
        if let Ok(bytes) = serde_json::to_vec(&net.dedicated.effects.ack()) {
            net.publish_application(EFFECT_ACK_KEY, bytes);
        }
    }
    let remote_states = net
        .lobby
        .as_ref()
        .map(|l| {
            l.actors
                .iter()
                .filter(|(id, _)| **id != local && **id != host)
                .filter_map(|(&id, a)| {
                    a.application
                        .get(GAMEPLAY_KEY)
                        .and_then(|r| serde_json::from_slice::<Gameplay>(&r.value).ok())
                        .map(|s| (id, s))
                })
                .collect()
        })
        .unwrap_or_default();
    net.dedicated.gameplay = remote_states;

    let root = network::capture_body(&physics, &skater).root;
    let forward = (Quat::from_array(root.q) * Vec3::Z).to_array();
    let held = controls.controller.words()[13] & (1 << 28) != 0;
    let targets: Vec<_> = net
        .remotes
        .iter()
        .filter(|(_, r)| r.body_at.elapsed().as_millis() <= 250)
        .filter(|(_, r)| r.body.enabled & (1u64 << 63) == 0)
        .filter(|_| {
            matches!(skater.player_input.physical.state.category_12, 100 | 500)
                && !skater.skeleton_collision.is_ragdoll
                && !skater
                    .animation
                    .motion
                    .animation
                    .channels
                    .has("RetrieveBoard")
        })
        .map(|(&id, r)| (id, r.body.root.p))
        .collect();
    if let Some((id, target)) = net.dedicated.input.sample(held, root.p, forward, targets) {
        let epoch = net.dedicated.effects.ack().epoch;
        if epoch == 0 {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec(&ShoveRequest { epoch, id, target }) {
            net.publish_application(SHOVE_KEY, bytes);
        }
    }
}

pub(super) fn publish(net: &mut Multiplayer, skater: &SkaterRuntime) {
    if !net.is_dedicated() {
        return;
    }
    let scoring = &skater.scoring;
    let state = Gameplay {
        mode: if skater.skeleton_collision.is_ragdoll {
            PlayerMode::Ragdoll
        } else if skater.player_input.physical.state.category_12 == 500 {
            PlayerMode::Offboard
        } else {
            PlayerMode::Skating
        },
        trick_seq: scoring.trick_seq() as u64,
        trick: trick_label(scoring.trick_name()),
        landed_seq: scoring.landing_seq as u64,
        landed_trick: trick_label(&scoring.landed_trick),
        bail_seq: scoring.bail_seq as u64,
        sequence_score: scoring.sequence_score().round().clamp(0., 1_000_000_000.) as i64,
        line_score: scoring.line_score().round().clamp(0., 1_000_000_000.) as i64,
    };
    let Ok(bytes) = serde_json::to_vec(&state) else {
        return;
    };
    if bytes != net.dedicated.published && net.publish_application(GAMEPLAY_KEY, bytes.clone()) {
        net.dedicated.published = bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dedicated_host_identity_change_resets_epoch_but_same_host_keeps_deduplication() {
        let mut client = Client::default();
        assert!(client.observe_server(11));
        client.effects.consume(&EffectBatch {
            epoch: 100,
            effects: vec![],
        });
        assert!(!client.observe_server(11));
        assert_eq!(client.effects.ack().epoch, 100);
        assert!(client.observe_server(22));
        client.effects.consume(&EffectBatch {
            epoch: 50,
            effects: vec![],
        });
        assert_eq!(
            client.effects.ack().epoch,
            50,
            "New server identity must accept a fresh, lower clock epoch"
        );
    }
}
