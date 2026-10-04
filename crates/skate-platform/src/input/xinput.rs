//! Windows controller transport via the XInput C ABI.
use super::{CapabilityCache, DeviceError, DevicePacket};
use skate_core::input::xbox::XboxState;
use std::mem::MaybeUninit;

#[repr(C)]
struct Gamepad {
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    left_x: i16,
    left_y: i16,
    right_x: i16,
    right_y: i16,
}

#[repr(C)]
struct State {
    number: u32,
    gamepad: Gamepad,
}

#[repr(C)]
struct Vibration {
    left: u16,
    right: u16,
}

#[repr(C)]
struct Capabilities {
    device_type: u8,
    subtype: u8,
    flags: u16,
    gamepad: Gamepad,
    vibration: Vibration,
}

const _: () = assert!(size_of::<Gamepad>() == 12);
const _: () = assert!(size_of::<State>() == 16);
const _: () = assert!(size_of::<Capabilities>() == 20);

#[link(name = "xinput")]
unsafe extern "system" {
    fn XInputGetState(index: u32, state: *mut State) -> u32;
    fn XInputGetCapabilities(index: u32, flags: u32, capabilities: *mut Capabilities) -> u32;
}

pub(super) fn poll(index: u32, cache: &mut CapabilityCache) -> Result<DevicePacket, DeviceError> {
    let mut state = MaybeUninit::<State>::uninit();
    // SAFETY: `state` is writable, aligned storage for the exact SDK ABI and
    // is read only after XInput reports success.
    let result = unsafe { XInputGetState(index, state.as_mut_ptr()) };
    if result != 0 {
        cache.invalidate();
    }
    if result == 1167 {
        return Err(DeviceError::Disconnected);
    }
    if result != 0 {
        return Err(DeviceError::State(result));
    }

    let subtype = cache.get(|| {
        let mut capabilities = MaybeUninit::<Capabilities>::uninit();
        // SAFETY: `capabilities` has the SDK ABI and is read only on success.
        let result = unsafe { XInputGetCapabilities(index, 1, capabilities.as_mut_ptr()) };
        if result != 0 {
            return Err(DeviceError::Capabilities(result));
        }
        // SAFETY: the successful call initialized the complete structure.
        Ok(unsafe { capabilities.assume_init() }.subtype)
    })?;
    // SAFETY: the successful state call initialized the complete structure.
    let state = unsafe { state.assume_init() };
    Ok(DevicePacket {
        number: state.number,
        state: XboxState {
            buttons: state.gamepad.buttons,
            triggers: [state.gamepad.left_trigger, state.gamepad.right_trigger],
            left: [state.gamepad.left_x, state.gamepad.left_y],
            right: [state.gamepad.right_x, state.gamepad.right_y],
        },
        subtype,
    })
}
