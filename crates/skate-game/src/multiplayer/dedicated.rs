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
    pub(super) native_active: bool,
    server: Option<(u64, u64)>,
    observed_travel: Option<u64>,
    awaiting_respawn: bool,
    respawn_request: u64,
    scheduled_reset: u64,
    reset_travel_generation: u64,
    effects: ClientEffects,
    input: ShoveInput,
    published: Vec<u8>,
    gameplay: BTreeMap<u64, Gameplay>,
}
impl Client {
    pub(super) fn native_begin(&mut self, epoch: u64, travel: u64) {
        self.native_active = true;
        self.scheduled_reset = epoch;
        self.observed_travel = Some(travel);
        self.awaiting_respawn = false;
        self.effects.consume(&EffectBatch {
            epoch,
            effects: vec![],
        });
    }
    pub(super) fn native_end(&mut self, travel: u64) {
        self.native_active = false;
        self.observed_travel = Some(travel);
        self.awaiting_respawn = false;
    }
    fn observe_server(&mut self, server: u64, connection: u64) -> bool {
        if self.server == Some((server, connection)) {
            return false;
        }
        *self = Self {
            server: Some((server, connection)),
            ..Self::default()
        };
        true
    }
    pub(super) fn waiting_for_respawn(&self) -> bool {
        self.awaiting_respawn
    }
    fn observe_travel(
        &mut self,
        generation: u64,
        epoch: u64,
        reset_pending: bool,
    ) -> Option<skate_net::dedicated::TeleportRequest> {
        if reset_pending || epoch == 0 {
            return None;
        }
        let previous = self.observed_travel.replace(generation);
        if self.awaiting_respawn || previous.is_none_or(|old| old == generation) {
            return None;
        }
        self.respawn_request = self
            .respawn_request
            .checked_add(1)
            .expect("Respawn sequence exhausted");
        self.awaiting_respawn = true;
        Some(skate_net::dedicated::TeleportRequest {
            epoch,
            id: self.respawn_request,
            destination: skate_net::dedicated::RESPAWN_DESTINATION.into(),
        })
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
    if net.dedicated.native_active {
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
    let connection = net.lobby.as_ref().unwrap().connection_generation();
    if net.dedicated.observe_server(host, connection) {
        physics.network_delta_velocity = [0.; 3];
    }
    if let Some(reset) = net.lobby.as_ref().and_then(|l| l.pending_movement_reset()) {
        if net.dedicated.scheduled_reset != reset.epoch {
            let mut transform = Mat4::from_rotation_translation(
                Quat::from_rotation_y(reset.destination.heading),
                Vec3::from_array(reset.destination.position),
            )
            .to_cols_array_2d();
            transform[3][3] = 0.;
            if skater.player_input.pending_teleport().is_some() {
                return;
            }
            if let Err(error) = skater.travel(transform, Some(reset.destination.velocity)) {
                net.leave();
                net.status = format!("Dedicated teleport stopped: {error}");
                return;
            }
            physics.network_delta_velocity = [0.; 3];
            net.dedicated.effects.consume(&EffectBatch {
                epoch: reset.epoch,
                effects: Vec::new(),
            });
            net.dedicated.input = ShoveInput::default();
            net.dedicated.scheduled_reset = reset.epoch;
            net.dedicated.reset_travel_generation = skater.travel_generation;
        } else {
            // The normal actor-reset pipeline owns the physical relocation.
            // The native callback marks actual body relocation; use that commit
            // instead of a distance heuristic (high-speed spawns may already
            // have moved several metres by this publication boundary).
            if skater.travel_generation > net.dedicated.reset_travel_generation
                && skater.player_input.pending_teleport().is_none()
            {
                net.lobby
                    .as_mut()
                    .unwrap()
                    .complete_movement_reset(reset.epoch);
                net.dedicated.observed_travel = Some(skater.travel_generation);
                net.dedicated.awaiting_respawn = false;
            }
        }
        return;
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

pub(super) fn publish(net: &mut Multiplayer, skater: &SkaterRuntime) -> bool {
    if !net.is_dedicated() {
        return true;
    }
    let lobby = net.lobby.as_ref().unwrap();
    let (epoch, reset_pending) = (
        lobby.movement_epoch(),
        lobby.pending_movement_reset().is_some(),
    );
    if !net.dedicated.native_active {
        if let Some(request) =
            net.dedicated
                .observe_travel(skater.travel_generation, epoch, reset_pending)
        {
            let bytes = serde_json::to_vec(&request).expect("Bounded respawn request");
            net.publish_application(skate_net::dedicated::TELEPORT_KEY, bytes);
        }
    }
    if net.dedicated.awaiting_respawn {
        net.lobby.as_mut().unwrap().retry_owner_body();
        return false;
    }
    if reset_pending {
        return false;
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
        return true;
    };
    if bytes != net.dedicated.published && net.publish_application(GAMEPLAY_KEY, bytes.clone()) {
        net.dedicated.published = bytes;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dedicated_host_identity_change_resets_epoch_but_same_host_keeps_deduplication() {
        let mut client = Client::default();
        assert!(client.observe_server(11, 1));
        client.effects.consume(&EffectBatch {
            epoch: 100,
            effects: vec![],
        });
        assert!(!client.observe_server(11, 1));
        assert_eq!(client.effects.ack().epoch, 100);
        assert!(client.observe_server(22, 2));
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
    #[test]
    fn native_travel_requests_only_server_recovery_and_server_travel_does_not_loop() {
        let mut client = Client::default();
        assert!(client.observe_travel(7, 100, false).is_none());
        let request = client.observe_travel(8, 100, false).unwrap();
        assert_eq!(
            request.destination,
            skate_net::dedicated::RESPAWN_DESTINATION
        );
        assert_eq!(request.epoch, 100);
        assert!(client.waiting_for_respawn());
        assert!(client.observe_travel(8, 100, false).is_none());
        assert!(client.observe_travel(9, 101, true).is_none());
        client.observed_travel = Some(9);
        client.awaiting_respawn = false;
        assert!(client.observe_travel(9, 101, false).is_none());
        assert!(client.observe_server(11, 2));
        assert!(!client.waiting_for_respawn());
        assert!(client.observe_travel(10, 200, false).is_none());
    }
}
