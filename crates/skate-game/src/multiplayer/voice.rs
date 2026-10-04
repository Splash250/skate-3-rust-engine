//! Voice uses the admitted dedicated transport. User opt-in and physical PTT
//! remain local gates even when resources configure channels and devices.
use super::Multiplayer;
use bevy::prelude::*;
use skate_voice::{
    AudioEngine, ClientCommand, ClientState, Context, DeviceConfig, Incoming, wire::Frame,
};
use std::collections::{BTreeMap, VecDeque};
#[derive(Resource)]
pub(crate) struct VoiceState {
    enabled: bool,
    engine: Option<AudioEngine>,
    incoming: VecDeque<(u64, Vec<u8>)>,
    client: ClientState,
    generation: u64,
    sequence: u64,
    owners: BTreeMap<String, (u64, bool)>,
    controller: Option<(String, u64)>,
    device_owner: Option<(String, u64)>,
    device_config: DeviceConfig,
    requests: BTreeMap<u64, (String, u64)>,
    list_requests: VecDeque<(String, u64)>,
    events: VecDeque<(String, u64, serde_json::Value)>,
    requested: bool,
    channel: String,
    muted: bool,
    deafened: bool,
    user_muted: bool,
    user_deafened: bool,
    pressed: bool,
    needs_open: bool,
    status: serde_json::Value,
}
impl VoiceState {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            engine: None,
            incoming: VecDeque::new(),
            client: ClientState::default(),
            generation: 1,
            sequence: 0,
            owners: BTreeMap::new(),
            controller: None,
            device_owner: None,
            device_config: DeviceConfig::default(),
            requests: BTreeMap::new(),
            list_requests: VecDeque::new(),
            events: VecDeque::new(),
            requested: true,
            channel: String::new(),
            muted: false,
            deafened: false,
            user_muted: false,
            user_deafened: false,
            pressed: false,
            needs_open: true,
            status: serde_json::json!({"state":if enabled{"waiting_for_connection"}else{"disabled"}}),
        }
    }
    fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        if let Some(engine) = &self.engine {
            engine.controls(false, true, self.generation);
        }
    }
    fn ensure_engine(&mut self) -> Result<(), String> {
        if !self.enabled {
            return Err("Voice is disabled; enable --voice in local game settings".into());
        }
        if self.engine.is_none() {
            self.engine = Some(AudioEngine::start()?);
        }
        Ok(())
    }
    fn live(&self, owner: &str, generation: u64) -> bool {
        self.owners.get(owner) == Some(&(generation, true))
    }
    fn emit(&mut self, owner: String, generation: u64, value: serde_json::Value) {
        if !self.live(&owner, generation) {
            return;
        }
        if self.events.len() >= 32 {
            self.events.pop_front();
        }
        self.events.push_back((owner, generation, value));
    }
}
pub(crate) fn install(app: &mut App, enabled: bool) {
    app.insert_resource(VoiceState::new(enabled))
        .add_systems(PreUpdate, network.after(super::receive))
        .add_systems(Update, input);
}
pub(crate) fn enqueue(state: &mut VoiceState, peer: u64, bytes: Vec<u8>) {
    if !state.enabled || bytes.len() > skate_net::packed::MTU {
        return;
    }
    if state.incoming.len() >= 64 {
        state.incoming.pop_front();
    }
    state.incoming.push_back((peer, bytes));
}
pub(crate) fn snapshot(world: &World) -> serde_json::Value {
    world.get_resource::<VoiceState>().map_or(serde_json::json!({"enabled":false}),|s|serde_json::json!({"enabled":s.enabled,"muted":s.muted||s.user_muted,"deafened":s.deafened||s.user_deafened,"push_to_talk":s.pressed&&s.requested,"channel":s.channel,"device":s.status}))
}
pub(crate) fn take_events(world: &mut World) -> Vec<(String, u64, serde_json::Value)> {
    world
        .get_resource_mut::<VoiceState>()
        .map(|mut s| s.events.drain(..).collect())
        .unwrap_or_default()
}
pub(crate) fn apply(
    world: &mut World,
    owner: &str,
    generation: u64,
    operation: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let command: ClientCommand = serde_json::from_value(operation)
        .map_err(|e| format!("Invalid client voice operation: {e}"))?;
    command.validate()?;
    let mut state = world
        .get_resource_mut::<VoiceState>()
        .ok_or("Voice plugin unavailable")?;
    if !state.enabled {
        return Err("Voice requires local --voice opt-in".into());
    }
    if generation == 0 {
        return Err("Voice requires a live resource generation".into());
    }
    if state
        .owners
        .get(owner)
        .is_some_and(|(old, active)| *old > generation || (*old == generation && !*active))
    {
        return Err("Stale voice resource generation".into());
    }
    if !state.owners.contains_key(owner) && state.owners.len() >= 128 {
        return Err("Voice resource owner capacity reached".into());
    }
    state.owners.insert(owner.into(), (generation, true));
    match command {
        ClientCommand::Configure {
            input_device,
            output_device,
            muted,
            deafened,
        } => {
            if input_device.as_ref().is_some_and(|s| s.len() > 256)
                || output_device.as_ref().is_some_and(|s| s.len() > 256)
            {
                return Err("Voice device identifier exceeds256 bytes".into());
            }
            state.invalidate();
            state.device_config = DeviceConfig {
                input_device,
                output_device,
            };
            state.device_owner = Some((owner.into(), generation));
            state.controller = Some((owner.into(), generation));
            state.muted = muted;
            state.deafened = deafened;
            state.needs_open = true;
            Ok(serde_json::json!({"queued":true}))
        }
        ClientCommand::Transmit { pressed, channel } => {
            let channel = channel.unwrap_or_default();
            if !channel.is_empty()
                && (channel.len() > 129
                    || !channel
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-./".contains(&b))
                    || channel.bytes().filter(|b| *b == b'/').count() != 1)
            {
                return Err("Voice channel must be resource/name".into());
            }
            if state.requested != pressed
                || state.channel != channel
                || state
                    .controller
                    .as_ref()
                    .is_none_or(|(id, old)| id != owner || *old != generation)
            {
                state.invalidate();
            }
            state.requested = pressed;
            state.channel = channel;
            state.controller = Some((owner.into(), generation));
            Ok(serde_json::json!({"requested":pressed,"physical_push_to_talk_required":true}))
        }
        ClientCommand::Devices {} => {
            if state.list_requests.len() >= 16 {
                return Err("Voice device query queue full".into());
            }
            state.ensure_engine()?;
            state.engine.as_ref().unwrap().list_devices()?;
            state.list_requests.push_back((owner.into(), generation));
            Ok(serde_json::json!({"queued":true}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_ptt_commands_keep_current_audio_generation_and_retirement_keeps_user_mute() {
        let mut world = World::new();
        world.insert_resource(VoiceState::new(true));
        let request = serde_json::json!({"kind":"transmit","pressed":true,"channel":"radio/crew"});
        apply(&mut world, "radio", 1, request.clone()).unwrap();
        let generation = world.resource::<VoiceState>().generation;
        for _ in 0..100 {
            apply(&mut world, "radio", 1, request.clone()).unwrap();
        }
        assert_eq!(
            world.resource::<VoiceState>().generation,
            generation,
            "per-frame PTT polling must not continuously discard partially captured audio"
        );
        world.resource_mut::<VoiceState>().user_muted = true;
        retire(&mut world, "radio");
        assert!(world.resource::<VoiceState>().user_muted);
        assert!(world.resource::<VoiceState>().channel.is_empty());
        assert!(apply(&mut world, "radio", 1, request).is_err());
    }
    #[test]
    fn disabled_voice_never_starts_a_worker_for_resource_requests() {
        let mut world = World::new();
        world.insert_resource(VoiceState::new(false));
        assert!(
            apply(
                &mut world,
                "radio",
                1,
                serde_json::json!({"kind":"devices"})
            )
            .is_err()
        );
        assert!(world.resource::<VoiceState>().engine.is_none());
    }
}
pub(crate) fn retire(world: &mut World, owner: &str) {
    let Some(mut state) = world.get_resource_mut::<VoiceState>() else {
        return;
    };
    if let Some((_, active)) = state.owners.get_mut(owner) {
        *active = false;
    }
    state.requests.retain(|_, (id, _)| id != owner);
    state.list_requests.retain(|(id, _)| id != owner);
    state.events.retain(|(id, _, _)| id != owner);
    if state.controller.as_ref().is_some_and(|(id, _)| id == owner) {
        state.controller = None;
        state.channel.clear();
        state.requested = true;
        state.muted = false;
        state.deafened = false;
        state.invalidate();
    }
    if state
        .device_owner
        .as_ref()
        .is_some_and(|(id, _)| id == owner)
    {
        state.device_owner = None;
        state.device_config = DeviceConfig::default();
        state.needs_open = true;
        state.invalidate();
        if let Some(engine) = &state.engine {
            engine.pause();
        }
    }
}
fn input(
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    menu: Option<Res<crate::graphics_menu::Menu>>,
    mods: Option<Res<crate::modding::Mods>>,
    mut state: ResMut<VoiceState>,
) {
    if !state.enabled {
        return;
    }
    let focused = windows.iter().any(|w| w.focused)
        && !menu.is_some_and(|m| m.open)
        && !crate::modding::browser_focused(mods.as_deref());
    let pressed = focused && keys.pressed(KeyCode::KeyV);
    let modifiers = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let mute = focused && modifiers && keys.just_pressed(KeyCode::KeyM);
    let deafen = focused && modifiers && keys.just_pressed(KeyCode::KeyD);
    if state.pressed != pressed || mute || deafen {
        state.pressed = pressed;
        if mute {
            state.user_muted = !state.user_muted;
        }
        if deafen {
            state.user_deafened = !state.user_deafened;
        }
        state.invalidate();
    }
}
fn network(mut state: ResMut<VoiceState>, mut net: ResMut<Multiplayer>) {
    if !state.enabled {
        return;
    }
    let context = net
        .lobby
        .as_ref()
        .filter(|l| l.is_dedicated() && l.connected())
        .and_then(|l| {
            Some(Context {
                session: l.session,
                host_peer: l.host_peer(),
                actor: l.local,
                epoch: l.movement_epoch(),
                instance: l.actors.get(&l.local)?.instance,
            })
        });
    let previous = state.client.context();
    if state.client.synchronize(context) {
        state.invalidate();
        state.incoming.clear();
        if previous.is_none() {
            state.needs_open = true;
        }
        if context.is_none() {
            if let Some(engine) = &state.engine {
                engine.pause();
            }
            state.status = serde_json::json!({"state":"waiting_for_connection"});
        }
    }
    if context.is_some() && state.needs_open {
        let config = state.device_config.clone();
        let result = state
            .ensure_engine()
            .and_then(|()| state.engine.as_ref().unwrap().configure(config));
        state.needs_open = false;
        match result {
            Ok(request) => {
                if let Some(owner) = state.device_owner.clone() {
                    if state.requests.len() >= 32 {
                        if let Some(id) = state.requests.keys().next().copied() {
                            state.requests.remove(&id);
                        }
                    }
                    state.requests.insert(request, owner);
                }
                state.status = serde_json::json!({"state":"opening"});
            }
            Err(error) => state.status = serde_json::json!({"state":"error","message":error}),
        }
    }
    for _ in 0..64 {
        let Some((peer, bytes)) = state.incoming.pop_front() else {
            break;
        };
        if let Some(incoming) = state.client.accept(peer, &bytes) {
            match incoming {
                Incoming::Policy(_) => state.invalidate(),
                Incoming::Playback { packet, changed } => {
                    if changed {
                        state.invalidate();
                    }
                    if !state.deafened && !state.user_deafened {
                        if let Some(engine) = &state.engine {
                            let _ = engine.playback(state.generation, packet);
                        }
                    }
                }
            }
        }
    }
    let transmit = context.is_some()
        && state.pressed
        && state.requested
        && !state.muted
        && !state.user_muted
        && state.client.revision() > 0;
    let deafened = context.is_none() || state.deafened || state.user_deafened;
    let Some(engine) = &state.engine else { return };
    engine.controls(transmit, deafened, state.generation);
    let encoded = engine.poll_encoded();
    let events = engine.poll_events();
    for event in events {
        let owner = if event.request == 0 {
            state.list_requests.pop_front()
        } else {
            state.requests.get(&event.request).cloned()
        };
        state.status = serde_json::json!({"state":event.kind,"value":event.value});
        if let Some((owner, generation)) = owner {
            state.emit(
                owner,
                generation,
                serde_json::json!({"kind":event.kind,"value":event.value}),
            );
        }
        // Keep the active device's owner after ready so a later driver failure
        // still reaches its generation-scoped callback. Replacement/retirement
        // bounds and removes these tickets.
        if matches!(event.kind.as_str(), "error" | "stopped") {
            state.requests.remove(&event.request);
        }
    }
    let Some(context) = context else { return };
    if !transmit {
        return;
    }
    for encoded in encoded.into_iter().take(4) {
        if encoded.generation != state.generation {
            continue;
        }
        let Some(sequence) = state.sequence.checked_add(1) else {
            state.status = serde_json::json!({"state":"error","message":"Voice sequence exhausted; restart the session"});
            break;
        };
        state.sequence = sequence;
        let frame = Frame {
            session: context.session,
            actor: context.actor,
            epoch: context.epoch,
            revision: state.client.revision(),
            sequence,
            channel: state.channel.clone(),
            data: encoded.data,
        };
        if let Ok(packet) = frame.encode() {
            if let Some(transport) = net.transport.as_mut() {
                let _ = transport.send(context.host_peer, &packet);
            }
        }
    }
}
