use super::*;
use crate::input::platform::{DeviceError, DevicePacket};
use skate_core::input::xbox::XboxState;

impl ControllerInput {
    /// Feed test packets through the production conversion, cache, history and Pad.
    /// Only the operating-system read is supplied by the test.
    pub(crate) fn sample_raw_for_test(&mut self, state: XboxState) {
        self.collect([
            Ok(DevicePacket {
                number: self.publications as u32 + 1,
                state,
                subtype: 1,
                kind: None,
            }),
            Err(DeviceError::Disconnected),
            Err(DeviceError::Disconnected),
            Err(DeviceError::Disconnected),
        ]);
        assert!(self.publish_actions());
    }
}

fn packet(number: u32, buttons: u16, left: [i16; 2]) -> Result<DevicePacket, DeviceError> {
    Ok(DevicePacket {
        number,
        state: XboxState {
            buttons,
            triggers: [0; 2],
            left,
            right: [0; 2],
        },
        subtype: 1,
        kind: None,
    })
}

#[test]
fn raw_packets_reach_stock_actions_without_losing_device_identity() {
    let mut input = ControllerInput::default();
    input.collect([
        packet(1, 0x1000, [32767, 0]),
        packet(2, 0x2000, [-32768, 0]),
        packet(3, 0x4000, [0, 0]),
        packet(4, 0x8000, [0, 0]),
    ]);
    assert!(input.publish_actions());
    assert!((input.mapped_actions[0][0] - 1.0).abs() < 0.000001);
    assert!((input.mapped_actions[1][0] + 1.0).abs() < 0.000001);
    assert_eq!(input.mapped_actions[0][16], 1.0); // GP_AFace
    assert_eq!(input.mapped_actions[1][17], 1.0); // GP_BFace
    assert_eq!(input.mapped_actions[2][14], 1.0); // GP_XFace
    assert_eq!(input.mapped_actions[3][15], 1.0); // GP_YFace
    assert_eq!(input.packet_numbers, [Some(1), Some(2), Some(3), Some(4)]);
}

#[test]
fn unchanged_packet_numbers_do_not_suppress_publication_and_empty_drain_does_not_advance() {
    let mut input = ControllerInput::default();
    for _ in 0..4 {
        input.collect(std::array::from_fn(|_| packet(7, 0x1000, [0; 2])));
        assert!(input.publish_actions());
    }
    assert_eq!(input.publications, 4);
    assert_eq!(input.consumed_batches, 4);
    assert_ne!(input.pads[0].records()[12][1] & 0x01000000, 0);
    let prior = input.pads.clone();
    assert!(!input.publish_actions());
    assert_eq!(input.pads, prior);
}

#[test]
fn disconnect_publishes_zero_count_and_reconnect_reinitializes_pad_records() {
    let mut input = ControllerInput::default();
    input.collect(std::array::from_fn(|_| packet(1, 0x1000, [32767, 0])));
    input.publish_actions();
    input.collect(std::array::from_fn(|_| Err(DeviceError::Disconnected)));
    input.publish_actions();
    assert_eq!(input.mapped_actions, [[0.0; 18]; 4]);
    assert!(input.pads.iter().all(|pad| pad.count() == 0));
    input.collect(std::array::from_fn(|_| packet(1, 0, [0; 2])));
    input.publish_actions();
    assert_eq!(input.mapped_actions, [[0.0; 18]; 4]);
    assert_eq!(input.pads[0].records()[12], [0, 0, 0, 1]);
}

#[test]
fn alternate_guitar_subtype_uses_verified_conversion_mask_and_errors_stay_visible() {
    let mut input = ControllerInput::default();
    let special = DevicePacket {
        number: 1,
        state: XboxState {
            buttons: 0,
            triggers: [255; 2],
            left: [0; 2],
            right: [-32768, 32767],
        },
        subtype: 7,
        kind: None,
    };
    input.collect([
        Ok(special),
        Err(DeviceError::Capabilities(5)),
        Err(DeviceError::Disconnected),
        Err(DeviceError::Disconnected),
    ]);
    input.publish_actions();
    assert_eq!(input.mapped_actions[0][6..8], [0.0; 2]);
    assert_eq!(input.mapped_actions[0][3..5], [0.0; 2]);
    assert_eq!(
        input.status[1],
        ControllerStatus::Unavailable(DeviceError::Capabilities(5))
    );
    assert_eq!(input.mapped_actions[1], [0.0; 18]);
}

#[test]
fn tick_input_is_the_single_fixed_tick_snapshot() {
    let mut input = ControllerInput::default();
    input.collect(std::array::from_fn(|_| packet(1, 0, [19661, 0])));
    assert!(input.publish_actions());

    let first = input.tick_input();
    assert_eq!(first.tick(), 1);
    assert!(first.controller_available());
    let mut first_actions = first.actions();
    let first_turn = first_actions.value(64);

    // A second consumer reads the same published tick without touching the
    // platform cache or remapping the controller.
    let second = input.tick_input();
    let mut second_actions = second.actions();
    assert_eq!(second.tick(), first.tick());
    assert_eq!(second_actions.value(64), first_turn);
}

#[test]
fn unavailable_controllers_publish_zero_gameplay_actions() {
    let mut input = ControllerInput::default();
    input.collect(std::array::from_fn(|_| packet(1, 0x1000, [32767, 0])));
    assert!(input.publish_actions());
    assert!(input.tick_input().controller_available());

    input.collect(std::array::from_fn(|_| Err(DeviceError::Disconnected)));
    assert!(input.publish_actions());
    let snapshot = input.tick_input();
    assert!(!snapshot.controller_available());
    assert_eq!(*snapshot.actions().values(), [0.0; 18]);
}

#[test]
fn controller_identity_is_metadata_and_never_changes_gameplay_actions() {
    use crate::input::controller_kind::{from_xinput, XinputCaps};
    let state = XboxState { buttons: 0x1000 | 0x0100, triggers: [255, 30], left: [20000, -12000], right: [-32768, 9000] };
    let kind = std::sync::Arc::new(from_xinput(
        XinputCaps { subtype: 1, flags: 0, vendor_product: Some((0x045e, 0x0b22)) }, &[]));
    let run = |kind: Option<std::sync::Arc<crate::input::controller_kind::ControllerKind>>| {
        let mut input = ControllerInput::default();
        for number in 1..4 {
            input.collect([
                Ok(DevicePacket { number, state, subtype: 1, kind: kind.clone() }),
                Err(DeviceError::Disconnected),
                Err(DeviceError::Disconnected),
                Err(DeviceError::Disconnected),
            ]);
            assert!(input.publish_actions());
        }
        input
    };
    let with = run(Some(kind.clone()));
    let without = run(None);
    let bits = |input: &ControllerInput| input.mapped_actions.map(|slot| slot.map(f32::to_bits));
    assert_eq!(bits(&with), bits(&without));
    assert_eq!(with.tick_input().actions().values().map(f32::to_bits), without.tick_input().actions().values().map(f32::to_bits));
    assert_eq!(with.kind(0).map(|k| k.name.as_str()), Some("Xbox Elite Series 2 (Bluetooth LE)"));
    assert_eq!(with.active_slot(), Some(0));
    assert!(without.kind(0).is_none());
    // A disconnect clears the identity with the slot.
    let mut input = with;
    input.collect(std::array::from_fn(|_| Err(DeviceError::Disconnected)));
    assert!(input.kind(0).is_none());
#[test]
fn trusted_consumed_buttons_keep_physical_voice_input_without_manual_marker_actions() {
    let mut input = ControllerInput::default();
    let mut baseline = ControllerInput::default();
    for number in 1..=3 {
        input.collect_masked(
            [
                packet(number, 0x100 | 0x1, [16000, 0]),
                Err(DeviceError::Disconnected),
                Err(DeviceError::Disconnected),
                Err(DeviceError::Disconnected),
            ],
            0x100,
        );
        input.publish_actions();
        baseline.collect([
            packet(number, 0x1, [16000, 0]),
            Err(DeviceError::Disconnected),
            Err(DeviceError::Disconnected),
            Err(DeviceError::Disconnected),
        ]);
        baseline.publish_actions();
    }
    assert_eq!(input.raw_input().buttons, 0x101);
    assert_eq!(input.session_marker_actions(), (false, false, false));
    // Stick conditioning applies the native 0.25 deadzone. Consuming LB must
    // leave every other mapped action exactly equal to the same packet without LB.
    assert!(baseline.mapped_actions[0][0] > 0.0);
    assert_eq!(input.mapped_actions, baseline.mapped_actions);
}

#[test]
fn one_physical_frame_serves_interface_before_gameplay_collection() {
    let mut frame = crate::input::ControllerFrame(Some([
        Err(DeviceError::Disconnected),
        packet(4, 0x100, [16000, 0]),
        Err(DeviceError::Disconnected),
        Err(DeviceError::Disconnected),
    ]));
    assert_eq!(frame.raw_input().buttons, 0x100);
    assert_eq!(frame.raw_input().left, [16000.0 / 32768.0, 0.0]);
    let mut input = ControllerInput::default();
    input.collect_masked(frame.0.take().unwrap(), 0x100);
    input.publish_actions();
    assert_eq!(input.raw_input().buttons, 0x100);
    assert_eq!(input.packet_numbers[1], Some(4));
    assert!(input.mapped_actions[1][0] > 0.0);
    assert_eq!(input.session_marker_actions(), (false, false, false));
}
