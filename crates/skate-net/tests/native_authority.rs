use skate_net::native_authority::{Admission, Input, InputLog, InputPacket, MAX_TICKS, VERSION};

fn admission() -> Admission {
    Admission {
        version: VERSION,
        epoch: 9,
        instance: 2,
        generation: 3,
    }
}
fn input(tick: u64) -> Input {
    Input {
        epoch: 9,
        tick,
        actions: [0.; 18],
    }
}

#[test]
fn exact_retransmission_is_idempotent_and_changed_history_is_rejected() {
    let mut log = InputLog::new(admission()).unwrap();
    assert!(log.append(input(1)).unwrap());
    assert!(!log.append(input(1)).unwrap());
    let mut forged = input(1);
    forged.actions[16] = 1.;
    assert!(log.append(forged).is_err());
    assert_eq!(log.len(), 1);
}

#[test]
fn epochs_order_and_action_domains_are_validated_before_mutation() {
    let mut log = InputLog::new(admission()).unwrap();
    assert!(log.append(input(2)).is_err());
    let mut stale = input(1);
    stale.epoch = 8;
    assert!(log.append(stale).is_err());
    for (slot, value) in [(0, f32::NAN), (0, 1.01), (6, -0.1), (17, 0.5)] {
        let mut bad = input(1);
        bad.actions[slot] = value;
        assert!(log.append(bad).is_err(), "slot {slot}, value {value}");
    }
    assert_eq!(log.len(), 0);
    assert!(log.append(input(1)).unwrap());
}

#[test]
fn authority_input_has_no_score_action_descriptor_or_position_claim() {
    for key in ["score", "descriptor", "position", "dt", "state", "landed"] {
        let mut value = serde_json::to_value(input(1)).unwrap();
        value[key] = serde_json::json!(1);
        assert!(serde_json::from_value::<Input>(value).is_err(), "{key}");
    }
}

#[test]
fn clock_budget_and_replay_storage_are_bounded() {
    let mut log = InputLog::new(admission()).unwrap();
    assert!(log.append_at(input(1), 0).unwrap());
    assert!(log.append_at(input(2), 0).unwrap());
    assert!(log.append_at(input(3), 0).unwrap());
    assert!(log.append_at(input(4), 0).is_err());
    assert!(log.append_at(input(4), 17).unwrap());
    for tick in 5..=MAX_TICKS {
        assert!(log.append(input(tick)).unwrap());
    }
    assert!(log.append(input(MAX_TICKS + 1)).is_err());
    assert_eq!(log.len() as u64, MAX_TICKS);
}

#[test]
fn replay_digest_is_identical_and_binds_epoch_instance_and_generation() {
    let mut first = InputLog::new(admission()).unwrap();
    let mut replay = InputLog::new(admission()).unwrap();
    for tick in 1..=120 {
        let mut sample = input(tick);
        sample.actions[3] = (tick as f32 * 0.1).sin();
        first.append(sample.clone()).unwrap();
        replay.append(sample).unwrap();
    }
    assert_eq!(first.digest(), replay.digest());
    for change in 0..3 {
        let mut id = admission();
        match change {
            0 => id.epoch += 1,
            1 => id.instance += 1,
            _ => id.generation += 1,
        }
        let mut other = InputLog::new(id).unwrap();
        for sample in first.inputs() {
            let mut sample = sample.clone();
            sample.epoch = id.epoch;
            other.append(sample).unwrap();
        }
        assert_ne!(first.digest(), other.digest());
    }
}

#[test]
fn compact_window_carries_32_exact_samples_below_the_application_limit() {
    let mut inputs = Vec::new();
    for tick in 1..=32 {
        let mut sample = input(tick);
        for slot in [0, 1, 3, 4] {
            sample.actions[slot] = ((tick * (slot + 1) as u64) as f32).sin();
        }
        sample.actions[6] = tick as f32 / 32.;
        sample.actions[7] = 0.5;
        for slot in [2, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17] {
            sample.actions[slot] = ((tick + slot as u64) % 2) as f32;
        }
        inputs.push(sample);
    }
    let bytes = InputPacket {
        inputs: inputs.clone(),
    }
    .encode()
    .unwrap();
    assert_eq!(bytes.len(), 844);
    assert!(bytes.len() <= 1024);
    assert_eq!(InputPacket::decode(&bytes).unwrap().inputs, inputs);
}

#[test]
fn compact_input_framing_and_values_fail_closed() {
    let bytes = InputPacket {
        inputs: vec![input(1)],
    }
    .encode()
    .unwrap();
    for length in 0..bytes.len() {
        assert!(InputPacket::decode(&bytes[..length]).is_err());
    }
    let mut invalid = bytes.clone();
    invalid.push(0);
    assert!(InputPacket::decode(&invalid).is_err());
    for (offset, value) in [(0, 2), (11, 0), (11, 33), (37, 0x80)] {
        let mut invalid = bytes.clone();
        invalid[offset] = value;
        assert!(InputPacket::decode(&invalid).is_err(), "offset {offset}");
    }
    let mut invalid = bytes.clone();
    invalid[12..16].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(InputPacket::decode(&invalid).is_err());
    let mut invalid = bytes.clone();
    invalid[28..32].copy_from_slice(&(-0.1_f32).to_le_bytes());
    assert!(InputPacket::decode(&invalid).is_err(), "negative trigger");
    let mut invalid = bytes.clone();
    invalid[9..11].copy_from_slice(&(MAX_TICKS as u16 + 1).to_le_bytes());
    assert!(InputPacket::decode(&invalid).is_err());
    assert!(
        InputPacket {
            inputs: (1..=33).map(input).collect()
        }
        .encode()
        .is_err()
    );
    assert!(
        InputPacket {
            inputs: vec![input(1), input(3)]
        }
        .encode()
        .is_err()
    );
    assert!(InputPacket::decode(br#"{\"inputs\":[],\"score\":1000}"#).is_err());
}
