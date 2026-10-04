//! Linux controller transport. Gilrs updates device state, while conversion
//! here reconstructs the XInput ranges consumed by the game.
use super::{CapabilityCache, DeviceError, DevicePacket, disconnected};
use gilrs::{Axis, Button, GamepadId, Gilrs};
use skate_core::input::xbox::XboxState;
use std::cell::RefCell;

const GAMECONTROLLERDB: &str = include_str!("../../gamecontrollerdb.txt");

fn stick(value: f32) -> i16 {
    let value = value.clamp(-1.0, 1.0);
    let scale = if value < 0.0 { 32768.0 } else { 32767.0 };
    (value * scale).round() as i16
}

fn trigger(button: Option<f32>, axis: Option<f32>) -> u8 {
    let value = match button {
        Some(value) => value.clamp(0.0, 1.0),
        None => axis.map_or(0.0, |value| (value.clamp(-1.0, 1.0) + 1.0) * 0.5),
    };
    (value * 255.0).round() as u8
}

fn button_mask(button: Button) -> Option<u16> {
    Some(match button {
        Button::DPadUp => 0x0001,
        Button::DPadDown => 0x0002,
        Button::DPadLeft => 0x0004,
        Button::DPadRight => 0x0008,
        Button::Start => 0x0010,
        Button::Select => 0x0020,
        Button::LeftThumb => 0x0040,
        Button::RightThumb => 0x0080,
        Button::LeftTrigger => 0x0100,
        Button::RightTrigger => 0x0200,
        Button::Mode => 0x0400,
        Button::South => 0x1000,
        Button::East => 0x2000,
        Button::West => 0x4000,
        Button::North => 0x8000,
        _ => return None,
    })
}

fn dpad_bits(x: f32, y: f32) -> u16 {
    let mut bits = 0;
    if y > 0.5 {
        bits |= 0x0001;
    }
    if y < -0.5 {
        bits |= 0x0002;
    }
    if x < -0.5 {
        bits |= 0x0004;
    }
    if x > 0.5 {
        bits |= 0x0008;
    }
    bits
}

struct Slots<T: Copy + Eq> {
    values: [Option<T>; 4],
}

impl<T: Copy + Eq> Default for Slots<T> {
    fn default() -> Self {
        Self { values: [None; 4] }
    }
}

impl<T: Copy + Eq> Slots<T> {
    fn reconcile(&mut self, connected: &[T]) {
        for slot in &mut self.values {
            if slot.is_some_and(|id| !connected.contains(&id)) {
                *slot = None;
            }
        }
        for &id in connected {
            if self.values.contains(&Some(id)) {
                continue;
            }
            let Some(slot) = self.values.iter_mut().find(|slot| slot.is_none()) else {
                break;
            };
            *slot = Some(id);
        }
    }
}

struct Backend {
    gilrs: Gilrs,
    slots: Slots<GamepadId>,
    packet: u32,
}

impl Backend {
    fn new() -> Result<Self, DeviceError> {
        let gilrs = gilrs::GilrsBuilder::new()
            .with_default_filters(false)
            .with_force_feedback(false)
            .add_env_mappings(true)
            .add_included_mappings(false)
            .add_mappings(GAMECONTROLLERDB)
            .build()
            .map_err(|_| DeviceError::State(1))?;
        Ok(Self {
            gilrs,
            slots: Slots::default(),
            packet: 0,
        })
    }

    fn refresh_slots(&mut self) {
        while self.gilrs.next_event().is_some() {}
        let mut connected: Vec<_> = self
            .gilrs
            .gamepads()
            .filter(|(_, gamepad)| gamepad.is_connected())
            .map(|(id, _)| id)
            .collect();
        connected.sort_by_key(|id| usize::from(*id));
        self.slots.reconcile(&connected);
    }

    fn sample(&mut self, index: usize) -> Option<XboxState> {
        self.refresh_slots();
        let id = self.slots.values[index]?;
        let gamepad = self.gilrs.gamepad(id);
        if !gamepad.is_connected() {
            return None;
        }

        let axis = |axis| gamepad.axis_data(axis).map_or(0.0, |data| data.value());
        let button = |button| gamepad.button_data(button).map(|data| data.value());
        let mut buttons = 0;
        for logical in [
            Button::DPadUp,
            Button::DPadDown,
            Button::DPadLeft,
            Button::DPadRight,
            Button::Start,
            Button::Select,
            Button::LeftThumb,
            Button::RightThumb,
            Button::LeftTrigger,
            Button::RightTrigger,
            Button::Mode,
            Button::South,
            Button::East,
            Button::West,
            Button::North,
        ] {
            if gamepad.is_pressed(logical) {
                buttons |= button_mask(logical).unwrap_or(0);
            }
        }
        buttons |= dpad_bits(axis(Axis::DPadX), axis(Axis::DPadY));

        Some(XboxState {
            buttons,
            triggers: [
                trigger(
                    button(Button::LeftTrigger2),
                    gamepad.axis_data(Axis::LeftZ).map(|v| v.value()),
                ),
                trigger(
                    button(Button::RightTrigger2),
                    gamepad.axis_data(Axis::RightZ).map(|v| v.value()),
                ),
            ],
            left: [stick(axis(Axis::LeftStickX)), stick(axis(Axis::LeftStickY))],
            right: [
                stick(axis(Axis::RightStickX)),
                stick(axis(Axis::RightStickY)),
            ],
        })
    }
}

thread_local! {
    static BACKEND: RefCell<Option<Backend>> = const { RefCell::new(None) };
}

pub(super) fn poll(index: usize, cache: &mut CapabilityCache) -> Result<DevicePacket, DeviceError> {
    BACKEND.with(|cell| {
        let mut backend = cell.borrow_mut();
        if backend.is_none() {
            *backend = Some(Backend::new()?);
        }
        let backend = backend.as_mut().expect("backend was initialized");
        let Some(state) = backend.sample(index) else {
            return disconnected(cache);
        };
        let subtype = cache.get(|| Ok(1))?;
        backend.packet = backend.packet.wrapping_add(1);
        Ok(DevicePacket {
            number: backend.packet,
            state,
            subtype,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gilrs::Button;

    #[test]
    fn stick_conversion_preserves_asymmetric_xinput_endpoints() {
        for (input, expected) in [
            (-2.0, -32768),
            (-1.0, -32768),
            (-0.5, -16384),
            (0.0, 0),
            (0.5, 16384),
            (1.0, 32767),
            (2.0, 32767),
        ] {
            assert_eq!(stick(input), expected, "input {input}");
        }
    }

    #[test]
    fn trigger_conversion_prefers_button_value_and_falls_back_to_axis() {
        assert_eq!(trigger(Some(0.0), Some(1.0)), 0);
        assert_eq!(trigger(Some(0.5), Some(-1.0)), 128);
        assert_eq!(trigger(Some(1.0), None), 255);
        assert_eq!(trigger(None, Some(-1.0)), 0);
        assert_eq!(trigger(None, Some(0.0)), 128);
        assert_eq!(trigger(None, Some(1.0)), 255);
        assert_eq!(trigger(None, None), 0);
    }

    #[test]
    fn logical_buttons_use_xusb_bits() {
        for (button, expected) in [
            (Button::DPadUp, 0x0001),
            (Button::DPadDown, 0x0002),
            (Button::DPadLeft, 0x0004),
            (Button::DPadRight, 0x0008),
            (Button::Start, 0x0010),
            (Button::Select, 0x0020),
            (Button::LeftThumb, 0x0040),
            (Button::RightThumb, 0x0080),
            (Button::LeftTrigger, 0x0100),
            (Button::RightTrigger, 0x0200),
            (Button::Mode, 0x0400),
            (Button::South, 0x1000),
            (Button::East, 0x2000),
            (Button::West, 0x4000),
            (Button::North, 0x8000),
        ] {
            assert_eq!(button_mask(button), Some(expected), "button {button:?}");
        }
        assert_eq!(button_mask(Button::Unknown), None);
    }

    #[test]
    fn evdev_hat_axes_supply_dpad_bits() {
        assert_eq!(dpad_bits(0.0, 0.0), 0);
        assert_eq!(dpad_bits(0.0, 0.51), 0x0001);
        assert_eq!(dpad_bits(0.0, -0.51), 0x0002);
        assert_eq!(dpad_bits(-0.51, 0.0), 0x0004);
        assert_eq!(dpad_bits(0.51, 0.0), 0x0008);
        assert_eq!(dpad_bits(-1.0, 1.0), 0x0005);
        assert_eq!(dpad_bits(0.5, -0.5), 0);
    }

    #[test]
    fn slots_keep_connected_devices_and_fill_the_lowest_gap() {
        let mut slots = Slots::default();
        slots.reconcile(&[10, 20]);
        assert_eq!(slots.values, [Some(10), Some(20), None, None]);

        slots.reconcile(&[20]);
        assert_eq!(slots.values, [None, Some(20), None, None]);

        slots.reconcile(&[20, 30]);
        assert_eq!(slots.values, [Some(30), Some(20), None, None]);
    }

    #[test]
    fn slots_ignore_devices_beyond_the_game_limit() {
        let mut slots = Slots::default();
        slots.reconcile(&[1, 2, 3, 4, 5]);
        assert_eq!(slots.values, [Some(1), Some(2), Some(3), Some(4)]);
    }
}
