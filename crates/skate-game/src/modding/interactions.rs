//! Generic resource interface ownership and physical input arbitration.
use super::Mods;
use bevy::prelude::*;
use skate_mods::interactions::{Binding, Entry, Operation, Registry};
use std::collections::BTreeMap;

#[derive(Resource, Default)]
pub(crate) struct Interfaces {
    registry: Registry,
    policies: BTreeMap<String, bool>,
    bindings: BTreeMap<String, Binding>,
    held: BTreeMap<String, (f32, bool)>,
    was_focused: bool,
    was_camera: bool,
    was_paused: bool,
    was_window_focused: bool,
    release: bool,
    previous_buttons: u16,
    binding_held: bool,
}
impl Interfaces {
    fn observe_context(&mut self, focused: bool, camera: bool, paused: bool, window: bool) {
        if self.was_focused != focused
            || self.was_camera != camera
            || self.was_paused != paused
            || self.was_window_focused != window
        {
            self.release = true;
            self.held.clear();
        }
        self.was_focused = focused;
        self.was_camera = camera;
        self.was_paused = paused;
        self.was_window_focused = window;
    }
}
pub(crate) fn camera_context(value: Option<&Interfaces>) -> bool {
    value.is_some_and(|state| state.was_camera)
}
pub(super) fn install(app: &mut App) {
    let path = bindings_path(app.world());
    let bindings = std::fs::read(&path)
        .ok()
        .filter(|b| b.len() <= 32768)
        .and_then(|b| serde_json::from_slice::<BTreeMap<String, Binding>>(&b).ok())
        .filter(|m| {
            m.len() <= 128
                && m.iter()
                    .all(|(id, b)| skate_mods::interactions::full_id(id) && b.validate())
        })
        .unwrap_or_default();
    app.insert_resource(Interfaces {
        bindings,
        ..default()
    });
    install_input(app);
}
fn install_input(app: &mut App) {
    app.add_systems(
        PreUpdate,
        input
            .after(crate::customiser::navigation)
            .after(crate::input::sample_controllers)
            .before(super::browser::input)
            .before(crate::graphics_menu::MenuInput),
    );
}
fn bindings_path(world: &World) -> std::path::PathBuf {
    world
        .resource::<crate::config::Config>()
        .asset_root
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("settings/interface-bindings.json")
}
pub(crate) fn blocked(value: Option<&Interfaces>) -> bool {
    value.is_some_and(|v| v.was_focused || v.was_paused || v.release || v.binding_held)
}
pub(crate) fn manual_markers(value: Option<&Interfaces>) -> bool {
    value.is_none_or(|v| v.policies.values().all(|enabled| *enabled))
}
pub(super) fn policy(world: &mut World, owner: &str, enabled: bool) {
    world
        .resource_mut::<Interfaces>()
        .policies
        .insert(owner.into(), enabled);
}
pub(super) fn clear_owner(world: &mut World, owner: &str) {
    if let Some(mut state) = world.get_resource_mut::<Interfaces>() {
        state.registry.retire_owner(owner);
        state.policies.remove(owner);
        state.held.clear();
        state.release = true;
    }
}
pub(super) fn clear(world: &mut World) {
    if let Some(mut state) = world.get_resource_mut::<Interfaces>() {
        state.registry = Registry::default();
        state.policies.clear();
        state.held.clear();
        state.release = true;
    }
}
fn generation(mods: &Mods, owner: &str) -> Result<u64, String> {
    mods.manager
        .resources
        .as_ref()
        .filter(|h| h.running(owner))
        .and_then(|h| h.generation(owner))
        .ok_or_else(|| "interfaces require a running dedicated resource".into())
}
fn related(mods: &Mods, caller: &str, owner: &str) -> bool {
    caller == owner
        || mods.manager.resources.as_ref().is_some_and(|h| {
            let set = h.installed();
            set.get(caller)
                .is_some_and(|r| r.manifest.dependencies.contains_key(owner))
                || set
                    .get(owner)
                    .is_some_and(|r| r.manifest.dependencies.contains_key(caller))
        })
}
fn invoke(mods: &mut Mods, entry: Entry) -> Result<(), String> {
    if generation(mods, &entry.owner)?.to_string() != entry.generation {
        return Err("interface owner has retired".into());
    }
    if !entry.descriptor.disabled_reason.is_empty() {
        return Err(entry.descriptor.disabled_reason);
    }
    let key = entry.id.split_once('/').unwrap().1;
    mods.manager.call(
        &entry.owner,
        "on_event",
        serde_json::json!({"type":"interface","key":key,"generation":entry.generation}),
    );
    Ok(())
}
pub(super) fn apply(
    world: &mut World,
    mods: &mut Mods,
    owner: &str,
    operation: Operation,
) -> Result<(), String> {
    let gen_id = generation(mods, owner)?;
    match operation {
        Operation::Register { key, descriptor } => world
            .resource_mut::<Interfaces>()
            .registry
            .register(owner, gen_id, &key, descriptor),
        Operation::Remove { key } => {
            world
                .resource_mut::<Interfaces>()
                .registry
                .remove(owner, &key);
            Ok(())
        }
        Operation::List => {
            let state = world.resource::<Interfaces>();
            let mut entries = state.registry.entries();
            let mut claimed: Vec<Binding> = Vec::new();
            for entry in &mut entries {
                if let Some(binding) = state.bindings.get(&entry.id) {
                    entry.descriptor.binding = Some(binding.clone());
                }
                if !entry.descriptor.disabled_reason.is_empty() {
                    continue;
                }
                if let Some(binding) = entry.descriptor.binding.as_ref() {
                    if claimed.iter().any(|other| other.conflicts(binding)) {
                        entry.binding_conflict =
                            Some("Shortcut conflict; choose another binding".into());
                    } else {
                        claimed.push(binding.clone());
                    }
                }
            }
            entries.retain(|e| related(mods, owner, &e.owner));
            mods.manager.call(
                owner,
                "on_event",
                serde_json::json!({"type":"interfaces","version":1,"entries":entries}),
            );
            Ok(())
        }
        Operation::Invoke { id, generation } => {
            let entry = world
                .resource::<Interfaces>()
                .registry
                .get(&id, &generation)?;
            if !related(mods, owner, &entry.owner) {
                return Err(
                    "interface invocation needs a declared dependency in either direction".into(),
                );
            }
            invoke(mods, entry)
        }
        Operation::Bind { id, binding } => {
            let state = world.resource::<Interfaces>();
            let entry = state
                .registry
                .entries()
                .into_iter()
                .find(|e| e.id == id)
                .ok_or("interface unavailable")?;
            if !related(mods, owner, &entry.owner) {
                return Err("binding needs a declared dependency".into());
            }
            for other in state.registry.entries() {
                if other.id != id {
                    if state
                        .bindings
                        .get(&other.id)
                        .or(other.descriptor.binding.as_ref())
                        .is_some_and(|b| b.conflicts(&binding))
                    {
                        return Err("binding conflicts with another interface".into());
                    }
                }
            }
            if !state.bindings.contains_key(&id) && state.bindings.len() >= 128 {
                return Err("local binding preference limit reached".into());
            }
            let mut next = state.bindings.clone();
            next.insert(id, binding);
            let bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
            let path = bindings_path(world);
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
            std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
            let mut state = world.resource_mut::<Interfaces>();
            state.bindings = next;
            state.held.clear();
            Ok(())
        }
    }
}
fn key(name: &str) -> Option<KeyCode> {
    Some(match name {
        "F2" => KeyCode::F2,
        "F3" => KeyCode::F3,
        "F4" => KeyCode::F4,
        "F7" => KeyCode::F7,
        "F8" => KeyCode::F8,
        "KeyI" => KeyCode::KeyI,
        "KeyP" => KeyCode::KeyP,
        "KeyO" => KeyCode::KeyO,
        _ => return None,
    })
}
fn neutral(pad: crate::input::RawInput) -> bool {
    pad.buttons == 0
        && pad.triggers.iter().all(|v| *v < 0.05)
        && pad.left.iter().chain(&pad.right).all(|v| v.abs() < 0.18)
}
fn input(world: &mut World) {
    if !world.contains_resource::<Mods>() {
        return;
    }
    let pad = world
        .resource::<crate::input::ControllerFrame>()
        .raw_input();
    let keys = world.resource::<ButtonInput<KeyCode>>().clone();
    let dt = world.resource::<Time<Real>>().delta_secs();
    let motion_dt = dt.min(0.1);
    let window_focused = world.query::<&Window>().iter(world).any(|w| w.focused);
    let pause = world
        .get_resource::<crate::graphics_menu::Menu>()
        .is_some_and(|m| m.open);
    let camera = super::photos::active(world);
    let browser = super::browser::focused(world.resource::<Mods>());
    let focused = camera || browser;
    let (released, pressed) = {
        let mut state = world.resource_mut::<Interfaces>();
        state.observe_context(focused, camera, pause, window_focused);
        state.binding_held = false;
        if state.release && neutral(pad) && keys.get_pressed().next().is_none() {
            state.release = false;
        }
        let pressed = pad.buttons & !state.previous_buttons;
        state.previous_buttons = pad.buttons;
        (state.release, pressed)
    };
    if camera && !pause && window_focused {
        let back = pressed & 0x2000 != 0 || keys.just_pressed(KeyCode::Escape);
        if back {
            super::photos::leave(world);
        } else if !released {
            let shutter = pressed & 0x4000 != 0 || keys.just_pressed(KeyCode::F12);
            let look = [
                pad.right[0] + f32::from(keys.pressed(KeyCode::ArrowRight))
                    - f32::from(keys.pressed(KeyCode::ArrowLeft)),
                pad.right[1] + f32::from(keys.pressed(KeyCode::ArrowUp))
                    - f32::from(keys.pressed(KeyCode::ArrowDown)),
            ];
            let zoom = pad.triggers[1] - pad.triggers[0] + f32::from(keys.pressed(KeyCode::PageUp))
                - f32::from(keys.pressed(KeyCode::PageDown));
            world.resource_scope(|world, mut mods: Mut<Mods>| {
                super::photos::input(
                    world,
                    &mut mods,
                    shutter,
                    [look[0] * motion_dt * 1.5, look[1] * motion_dt * 1.5],
                    zoom * motion_dt,
                )
            });
        }
        world
            .resource_mut::<crate::customiser::Navigation>()
            .pressed = 0;
        super::browser::consume_key_edges(&mut world.resource_mut::<ButtonInput<KeyCode>>());
        return;
    }
    if focused || pause || released || !window_focused {
        world.resource_mut::<Interfaces>().held.clear();
        return;
    }
    // Only installed dedicated resources register bindings; offline replay never competes.
    let mut invoked = None;
    {
        let mut state = world.resource_mut::<Interfaces>();
        let entries = state.registry.entries();
        let mut claimed: Vec<Binding> = Vec::new();
        for entry in entries {
            let Some(binding) = state
                .bindings
                .get(&entry.id)
                .or(entry.descriptor.binding.as_ref())
                .cloned()
            else {
                continue;
            };
            if !entry.descriptor.disabled_reason.is_empty()
                || claimed.iter().any(|b| b.conflicts(&binding))
            {
                continue;
            }
            claimed.push(binding.clone());
            let down = key(&binding.key).is_some_and(|k| keys.pressed(k))
                || binding.button.is_some_and(|b| pad.buttons & b != 0);
            state.binding_held |= down;
            let held = state.held.entry(entry.id.clone()).or_insert((0., false));
            if !down {
                *held = (0., false);
                continue;
            }
            held.0 += dt;
            if !held.1 && held.0 * 1000. >= binding.hold_ms as f32 {
                held.1 = true;
                invoked = Some(entry);
                break;
            }
        }
    }
    if let Some(entry) = invoked {
        world.resource_scope(|_, mut mods: Mut<Mods>| {
            if let Err(e) = invoke(&mut mods, entry) {
                warn!("Interface action: {e}");
            }
        });
        world
            .resource_mut::<crate::input::ControllerInput>()
            .discard_gameplay();
        world
            .resource_mut::<crate::customiser::Navigation>()
            .pressed = 0;
        super::browser::consume_key_edges(&mut world.resource_mut::<ButtonInput<KeyCode>>());
        world.resource_mut::<Interfaces>().release = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_sampling_and_resource_input_do_not_cycle_menu_map_ordering() {
        fn menu() {}
        fn map_transition() {}
        let mut app = App::new();
        app.add_plugins(crate::input::InputPlugin);
        install_input(&mut app);
        app.add_systems(
            PreUpdate,
            crate::customiser::navigation
                .after(bevy::input::InputSystems)
                .before(crate::graphics_menu::MenuInput),
        );
        app.add_systems(
            PreUpdate,
            super::super::browser::input
                .after(crate::customiser::navigation)
                .before(crate::graphics_menu::MenuInput),
        );
        app.add_systems(PreUpdate, menu.in_set(crate::graphics_menu::MenuInput));
        app.add_systems(
            PreUpdate,
            map_transition
                .in_set(crate::map_transition::MapTransitionSet)
                .after(crate::graphics_menu::MenuInput)
                .before(crate::input::poll_controllers),
        );
        let mut schedule = app
            .world_mut()
            .resource_mut::<Schedules>()
            .remove(PreUpdate)
            .unwrap();
        schedule.initialize(app.world_mut()).expect(
            "UI reads current physical input before menus; gameplay collects after map transitions",
        );
    }
    #[test]
    fn resource_policy_union_restores_manual_controls_after_retirement() {
        let mut w = World::new();
        w.init_resource::<Interfaces>();
        assert!(manual_markers(w.get_resource::<Interfaces>()));
        policy(&mut w, "a", false);
        policy(&mut w, "b", true);
        assert!(!manual_markers(w.get_resource::<Interfaces>()));
        clear_owner(&mut w, "b");
        assert!(!manual_markers(w.get_resource::<Interfaces>()));
        clear_owner(&mut w, "a");
        assert!(manual_markers(w.get_resource::<Interfaces>()));
        assert!(blocked(w.get_resource::<Interfaces>()));
    }
    #[test]
    fn camera_and_surface_ownership_transitions_require_physical_release() {
        let mut state = Interfaces::default();
        state.observe_context(true, false, false, true);
        state.release = false;
        state.held.insert("fixture/open".into(), (0.5, false));
        state.observe_context(true, true, false, true);
        assert!(state.release && state.held.is_empty());
        assert!(camera_context(Some(&state)));
        state.release = false;
        state.observe_context(true, false, false, true);
        assert!(state.release);
        assert!(!camera_context(Some(&state)));
    }
    #[test]
    fn frame_that_began_paused_cannot_publish_the_menu_closing_input() {
        let mut state = Interfaces {
            was_paused: true,
            ..default()
        };
        assert!(
            blocked(Some(&state)),
            "MenuInput may already have closed pause this frame"
        );
        state.was_paused = false;
        state.release = true;
        assert!(
            blocked(Some(&state)),
            "the following frame waits for physical release"
        );
        state.release = false;
        assert!(!blocked(Some(&state)));
    }
    #[test]
    fn neutral_gate_requires_all_physical_controls_released() {
        let mut pad = crate::input::RawInput::default();
        assert!(neutral(pad));
        pad.buttons = 0x1000;
        assert!(!neutral(pad));
        pad.buttons = 0;
        pad.triggers[0] = 0.1;
        assert!(!neutral(pad));
        pad.triggers[0] = 0.;
        pad.right[0] = 0.8;
        assert!(!neutral(pad));
    }
}
