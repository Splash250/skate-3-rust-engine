use bevy::prelude::*;
#[derive(Resource,Clone)]
pub(crate) struct MapViewState {
    pub expanded: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom_level: u8,
    pub zoom_fraction: f32,
    pub center: Option<Vec3>,
    pub top_down: bool,
    pub generation: Option<u64>,
    pub previous_buttons: u16,
    pub visible: bool,
    pub bounds: Option<(Vec3, Vec3)>,
}
impl Default for MapViewState {
    fn default() -> Self {
        Self {
            expanded: false,
            yaw: 0.,
            pitch: 50_f32.to_radians(),
            zoom_level: 0,
            zoom_fraction: 0.,
            center: None,
            top_down: false,
            generation: None,
            previous_buttons: 0,
            visible: false,
            bounds: None,
        }
    }
}
fn toggle_requested(keys: &ButtonInput<KeyCode>, buttons: u16, previous: u16) -> bool {
    let modified = [
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::AltLeft,
        KeyCode::AltRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]
    .iter()
    .any(|key| keys.pressed(*key));
    (keys.just_pressed(KeyCode::KeyM) && !modified) || (buttons & !previous & 4 != 0)
}
impl MapViewState {
    fn advance(
        &mut self,
        generation: u64,
        active: bool,
        keys: &ButtonInput<KeyCode>,
        buttons: u16,
    ) {
        if self.generation != Some(generation) || !active {
            self.expanded = false;
            self.zoom_level = 0;
            self.zoom_fraction = 0.;
            if self.generation != Some(generation) {
                self.bounds = None;
                self.center = None;
                self.top_down = false;
                self.zoom_fraction = 0.;
                self.yaw = 0.;
                self.pitch = 50_f32.to_radians();
            }
        } else if toggle_requested(keys, buttons, self.previous_buttons) {
            self.expanded = !self.expanded;
            if !self.expanded {
                self.zoom_level = 0;
                self.zoom_fraction = 0.;
            }
        }
        if active && self.expanded {
            let command = super::navigation::command_down(keys);
            if (!command
                && [KeyCode::Equal, KeyCode::NumpadAdd]
                    .iter()
                    .any(|k| keys.just_pressed(*k)))
                || buttons & !self.previous_buttons & 1 != 0
            {
                self.zoom_level = self.zoom_level.saturating_add(1).min(12);
            }
            if (!command
                && [KeyCode::Minus, KeyCode::NumpadSubtract]
                    .iter()
                    .any(|k| keys.just_pressed(*k)))
                || buttons & !self.previous_buttons & 2 != 0
            {
                self.zoom_level = self.zoom_level.saturating_sub(1);
            }
        }
        self.visible = active;
        self.previous_buttons = buttons;
        self.generation = Some(generation);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_and_controller_edges_ignore_modifiers_and_holds() {
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::KeyM);
        assert!(toggle_requested(&keys, 0, 0));
        for modifier in [KeyCode::ControlLeft, KeyCode::AltRight, KeyCode::SuperLeft] {
            keys.press(modifier);
            assert!(!toggle_requested(&keys, 0, 0));
            keys.release(modifier);
        }
        keys.clear();
        assert!(toggle_requested(&keys, 4, 0));
        assert!(!toggle_requested(&keys, 4, 4));
        assert!(!toggle_requested(&keys, 0x20, 0));
    }
    #[test]
    fn blocked_input_and_map_changes_collapse_and_require_fresh_press() {
        let keys = ButtonInput::default();
        let mut state = MapViewState::default();
        state.advance(0, true, &keys, 0);
        state.advance(0, true, &keys, 4);
        assert!(state.expanded);
        state.advance(0, false, &keys, 4);
        assert!(!state.expanded && !state.visible);
        state.advance(0, true, &keys, 4);
        assert!(!state.expanded);
        state.advance(0, true, &keys, 0);
        state.advance(0, true, &keys, 4);
        assert!(state.expanded);
        state.advance(1, true, &keys, 4);
        assert!(!state.expanded);
        assert_eq!(state.generation, Some(1));
    }
}

pub(super) fn update(world: &mut World) {
    let focused = world.query::<&Window>().iter(world).any(|w| w.focused);
    let active = focused
        && world
            .get_resource::<super::layers::MapLayerRegistry>()
            .is_none_or(|r| r.settings().enabled.unwrap_or(true))
        && world
            .get_resource::<crate::graphics_menu::Menu>()
            .is_none_or(|m| !m.open)
        && world
            .get_resource::<crate::map_transition::MapTransition>()
            .is_none_or(|m| !m.busy())
        && world
            .get_resource::<crate::replay::Replay>()
            .is_none_or(|r| !r.active)
        && world
            .get_resource::<crate::teleport_menu::Travel>()
            .is_none_or(|m| !m.open && !m.closed_this_frame)
        && world
            .get_resource::<crate::customiser::Customiser>()
            .is_none_or(|m| !m.open)
        && world
            .get_resource::<crate::custom_models::CustomModels>()
            .is_none_or(|m| !m.open)
        && world
            .get_resource::<crate::modding::ModMenu>()
            .is_none_or(|m| !m.open)
        && !crate::modding::browser_focused(world.get_resource::<crate::modding::Mods>())
        && !crate::modding::interactions::blocked(
            world.get_resource::<crate::modding::interactions::Interfaces>(),
        );
    let Some(map) = world.get_resource::<crate::map_transition::CurrentMap>() else {
        return;
    };
    let generation = map.generation;
    let buttons = world
        .get_resource::<crate::input::ControllerFrame>()
        .map_or(0, |f| f.raw_input().buttons);
    let mut state = world.remove_resource::<MapViewState>().unwrap_or_default();
    let was_expanded = state.expanded;
    state.advance(
        generation,
        active,
        world.resource::<ButtonInput<KeyCode>>(),
        buttons,
    );
    if state.expanded && !was_expanded {
        if let Some(initial) = world
            .get_resource::<super::layers::MapLayerRegistry>()
            .and_then(|r| r.settings().initial_zoom)
        {
            state.zoom_level = initial;
            state.zoom_fraction = 0.;
        }
    }
    if active && state.expanded {
        super::navigation::update(world, &mut state);
    } else {
        super::navigation::release(world);
    }
    world.insert_resource(state);
}
