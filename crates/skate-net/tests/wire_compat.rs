//! Shared Windows/Linux wire contract. Expected bytes are hand-authored from
//! the v5 layout, never produced by the encoder under test. A matching encoder
//! and decoder changing together must still fail these interoperability tests.
use skate_net::{
    Body, Bone, Pose,
    dedicated::{
        self, ClientEffects, Effect, EffectAck, EffectBatch, EffectKind, Gameplay, PlayerMode,
    },
    lobby::{self, Info, Session},
    packed::{self, BodyState, Packed, PoseState},
};

const SESSION: u64 = 0x0102_0304_0506_0708;
const ACTOR: u64 = 0x1112_1314_1516_1718;
const SEQUENCE: u32 = 0x2122_2324;
const CAPTURED: u64 = 0x3132_3334_3536_3738;
const MAP: u64 = 0x2122_2324_2526_2728;
const INFO: Info = Info {
    id: 2,
    map: MAP,
    rig: 0x3132_3334_3536_3738,
    physics: 0x4142_4344_4546_4748,
    appearance: 0x5152_5354_5556_5758,
};

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect()
}

fn hello() -> Vec<u8> {
    hex("53 4b 38 4e 45 54 30 35 08 07 06 05 04 03 02 01
         02 00 00 00 00 00 00 00 13 00 00 00 00
         02 00 00 00 00 00 00 00 28 27 26 25 24 23 22 21
         38 37 36 35 34 33 32 31 48 47 46 45 44 43 42 41
         58 57 56 55 54 53 52 51")
}

fn roster() -> Vec<u8> {
    hex("53 4b 38 4e 45 54 30 35 08 07 06 05 04 03 02 01
         63 00 00 00 00 00 00 00 14 01 00 00 00
         68 67 66 65 64 63 62 61 00 01 02
         63 00 00 00 00 00 00 00 28 27 26 25 24 23 22 21
         38 37 36 35 34 33 32 31 48 47 46 45 44 43 42 41
         00 00 00 00 00 00 00 00
         00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
         02 00 00 00 00 00 00 00 28 27 26 25 24 23 22 21
         38 37 36 35 34 33 32 31 48 47 46 45 44 43 42 41
         58 57 56 55 54 53 52 51
         68 67 66 65 64 63 62 61 00 00 00 00 00 00 00 00")
}

#[test]
fn dedicated_admission_uses_fixed_width_little_endian_identity_and_fingerprints() {
    let mut client = Session::dedicated_client(SESSION, INFO, 1);
    let outgoing = client.service(0);
    let generated = outgoing
        .iter()
        .find(|p| packed::envelope(&p.data).unwrap().2 == lobby::DEDICATED_HELLO)
        .unwrap();
    assert_eq!(generated.data, hello());

    let mut server = dedicated::Server::new(dedicated::Config {
        session: SESSION,
        server_id: 99,
        map: MAP,
        max_players: 2,
    })
    .unwrap();
    server.receive(1, &hello(), 0);
    assert_eq!(server.player_count(), 1);
    let output = server.service(1);
    let generated = output
        .iter()
        .find(|p| packed::envelope(&p.data).unwrap().2 == lobby::DEDICATED_ROSTER)
        .unwrap();
    // Admission and movement epoch fields are generated at runtime; all other
    // bytes are an independently authored contract shared by Windows and Linux.
    let mut actual = generated.data.clone();
    assert_eq!(actual.len(), 152);
    assert_ne!(&actual[29..37], &[0; 8]);
    assert_eq!(&actual[29..37], &actual[136..144]);
    actual[29..37].copy_from_slice(&roster()[29..37]);
    actual[136..144].copy_from_slice(&roster()[136..144]);
    assert_eq!(actual, roster());

    client.receive(1, &roster(), 1);
    assert!(client.connected());
    assert_eq!(client.host_actor(), Some(99));
    assert_eq!(client.player_ids(), vec![2]);
    assert_eq!(client.actors[&99].info.rig, INFO.rig);
    assert_eq!(client.actors[&99].info.physics, INFO.physics);
    assert_eq!(client.actors[&2].info, INFO);
}

fn root() -> Pose {
    Pose {
        p: [1., -2., 4.],
        q: [0., 0., 0., 1.],
    }
}

fn body() -> BodyState {
    let mut state = BodyState {
        root: root(),
        // Exercise both upper state flags, the 33rd body and the first body.
        enabled: (1 << 63) | (1 << 62) | (1 << 32) | 1,
        bodies: vec![
            Body {
                pose: Pose {
                    p: [1.125, -2.25, 4.5],
                    ..root()
                },
                velocity: [1., -2., 0.5],
                angular: [0., 4., -8.],
            };
            33
        ],
    };
    state.bodies[32].pose.p = [65., -130., 260.];
    state
}

fn body_fixture() -> Vec<u8> {
    let mut bytes = hex("53 4b 38 4e 45 54 30 35 08 07 06 05 04 03 02 01
         18 17 16 15 14 13 12 11 01 24 23 22 21
         00 00 00 00 38 37 36 35 34 33 32 31
         00 00 80 3f 00 00 00 c0 00 00 80 40 03 08 20 80
         01 00 00 00 01 00 00 c0 21 ff ff ff ff 01 00 00 00");
    // Thirty-two identical narrow rows: millimetre i16 offsets, smallest-three
    // identity quaternion, then six IEEE binary16 rates. The final row is wide.
    let narrow = hex("00 7d 00 06 ff f4 01 03 08 20 80 00 3c 00 c0 00 38 00 00 00 44 00 c8");
    for _ in 0..32 {
        bytes.extend(&narrow);
    }
    bytes.extend(hex("01 00 00 80 42 00 00 00 c3 00 00 80 43 03 08 20 80
         00 3c 00 c0 00 38 00 00 00 44 00 c8"));
    bytes
}

#[test]
fn body_keyframe_preserves_all_33_bodies_wide_offsets_flags_and_half_float_rates() {
    let mut encoded = Packed::body(&body()).unwrap();
    encoded.captured = CAPTURED;
    let fixture = body_fixture();
    assert_eq!(
        packed::delta(SESSION, ACTOR, packed::BODY, SEQUENCE, &encoded, None),
        fixture
    );
    assert_eq!(
        packed::envelope(&fixture),
        Some((SESSION, ACTOR, packed::BODY, SEQUENCE))
    );
    let decoded = packed::apply(&fixture, None).unwrap();
    assert_eq!(decoded, encoded);
    let restored = decoded.unpack_body().unwrap();
    assert_eq!(restored.root.p, [1., -2., 4.]);
    assert_eq!(restored.enabled, 0xc000_0001_0000_0001);
    assert_eq!(restored.bodies.len(), 33);
    assert_eq!(restored.bodies[0].pose.p, [1.125, -2.25, 4.5]);
    assert_eq!(restored.bodies[32].pose.p, [65., -130., 260.]);
    for body in &restored.bodies {
        assert_eq!(body.velocity, [1., -2., 0.5]);
        assert_eq!(body.angular, [0., 4., -8.]);
        assert!(body.pose.q[3] > 0.999);
    }
}

#[test]
fn body_delta_uses_u32_baseline_and_u64_mask_for_the_33rd_body() {
    let baseline = packed::apply(&body_fixture(), None).unwrap();
    let mut state = body();
    state.bodies[32].velocity = [2., -4., 1.];
    let mut encoded = Packed::body(&state).unwrap();
    encoded.captured = CAPTURED + 1;
    let fixture = hex("53 4b 38 4e 45 54 30 35 08 07 06 05 04 03 02 01
         18 17 16 15 14 13 12 11 01 25 23 22 21
         24 23 22 21 39 37 36 35 34 33 32 31
         00 00 80 3f 00 00 00 c0 00 00 80 40 03 08 20 80
         01 00 00 00 01 00 00 c0 21 00 00 00 00 01 00 00 00
         01 00 00 80 42 00 00 00 c3 00 00 80 43 03 08 20 80
         00 40 00 c4 00 3c 00 00 00 44 00 c8");
    assert_eq!(
        packed::delta(
            SESSION,
            ACTOR,
            packed::BODY,
            SEQUENCE + 1,
            &encoded,
            Some((SEQUENCE, &baseline))
        ),
        fixture
    );
    assert_eq!(packed::baseline(&fixture), Some(SEQUENCE));
    assert!(packed::apply(&fixture, None).is_none());
    let restored = packed::apply(&fixture, Some(&baseline)).unwrap();
    assert_eq!(restored, encoded);
    assert_eq!(
        restored.unpack_body().unwrap().bodies[32].velocity,
        [2., -4., 1.]
    );
}

#[test]
fn pose_keyframe_preserves_bone_indices_and_quaternion_sign_convention() {
    let state = PoseState {
        root: root(),
        bones: vec![
            Bone {
                index: 7,
                pose: Pose {
                    p: [0.125, -0.25, 0.5],
                    ..root()
                },
            },
            Bone {
                index: 255,
                pose: Pose {
                    p: [64., -128., 256.],
                    q: [0., 0., 0., -1.],
                },
            },
        ],
    };
    let mut encoded = Packed::pose(&state).unwrap();
    encoded.captured = CAPTURED;
    let fixture = hex("53 4b 38 4e 45 54 30 35 08 07 06 05 04 03 02 01
         18 17 16 15 14 13 12 11 02 24 23 22 21
         00 00 00 00 38 37 36 35 34 33 32 31
         00 00 80 3f 00 00 00 c0 00 00 80 40 03 08 20 80
         00 00 00 00 00 00 00 00 02 03 00 00 00 00 00 00 00
         07 00 7d 00 06 ff f4 01 03 08 20 80
         ff 01 00 00 80 42 00 00 00 c3 00 00 80 43 03 08 20 80");
    assert_eq!(
        packed::delta(SESSION, ACTOR, packed::POSE, SEQUENCE, &encoded, None),
        fixture
    );
    let decoded = packed::apply(&fixture, None).unwrap();
    assert_eq!(decoded, encoded);
    let restored = decoded.unpack_pose().unwrap();
    assert_eq!(restored.bones[0].index, 7);
    assert_eq!(restored.bones[0].pose.p, [0.125, -0.25, 0.5]);
    assert_eq!(restored.bones[1].index, 255);
    assert_eq!(restored.bones[1].pose.p, [64., -128., 256.]);
    assert!(restored.bones[1].pose.q[3] > 0.999);
}

const GAMEPLAY: &[u8] = br#"{"mode":"Offboard","trick_seq":9007199254740993,"trick":"Kickflip","landed_seq":5,"landed_trick":"Ollie","bail_seq":6,"sequence_score":300,"line_score":2147483647}"#;
const EFFECTS: &[u8] = br#"{"epoch":9007199254740993,"effects":[{"id":1,"source":3,"target":2,"delta_velocity":[1.5,-2.0,0.25],"kind":"Shove","position":[4.0,5.0,-6.0]}]}"#;
const EFFECT_ACK: &[u8] = br#"{"epoch":9007199254740993,"through":1}"#;

#[test]
fn reliable_gameplay_packet_keeps_utf8_json_and_64_bit_counters() {
    let state = Gameplay {
        mode: PlayerMode::Offboard,
        trick_seq: 9_007_199_254_740_993,
        trick: "Kickflip".into(),
        landed_seq: 5,
        landed_trick: "Ollie".into(),
        bail_seq: 6,
        sequence_score: 300,
        line_score: 2_147_483_647,
    };
    assert_eq!(serde_json::to_vec(&state).unwrap(), GAMEPLAY);
    assert_eq!(serde_json::from_slice::<Gameplay>(GAMEPLAY).unwrap(), state);
    let mut client = Session::dedicated_client(SESSION, INFO, 1);
    client.receive(1, &roster(), 1);
    assert!(client.publish_application(
        dedicated::GAMEPLAY_KEY,
        serde_json::to_vec(&state).unwrap(),
        1
    ));
    let mut fixture = hex("53 4b 38 4e 45 54 30 35 08 07 06 05 04 03 02 01
         02 00 00 00 00 00 00 00 0a 01 00 00 00 10");
    fixture.extend(b"builtin:gameplay");
    fixture.extend(GAMEPLAY);
    let output = client.service(100);
    assert!(output.iter().any(|p| p.data == fixture));
}

#[test]
fn server_effect_fixture_applies_once_and_acknowledges_exact_64_bit_epoch() {
    let effect = Effect {
        id: 1,
        source: 3,
        target: 2,
        delta_velocity: [1.5, -2., 0.25],
        kind: EffectKind::Shove,
        position: [4., 5., -6.],
    };
    let batch = EffectBatch {
        epoch: 9_007_199_254_740_993,
        effects: vec![effect.clone()],
    };
    assert_eq!(serde_json::to_vec(&batch).unwrap(), EFFECTS);
    let mut fixture = hex("53 4b 38 4e 45 54 30 35 08 07 06 05 04 03 02 01
         63 00 00 00 00 00 00 00 0a 01 00 00 00 11");
    fixture.extend(b"builtin:effects:2");
    fixture.extend(EFFECTS);
    let mut client = Session::dedicated_client(SESSION, INFO, 1);
    client.receive(1, &roster(), 1);
    client.receive(1, &fixture, 2);
    let received = &client.actors[&99].application[&dedicated::effects_key(2)].value;
    let received: EffectBatch = serde_json::from_slice(received).unwrap();
    let mut consumer = ClientEffects::default();
    assert_eq!(consumer.consume(&received), vec![effect]);
    assert!(consumer.consume(&received).is_empty());
    assert_eq!(serde_json::to_vec(&consumer.ack()).unwrap(), EFFECT_ACK);
    assert_eq!(
        serde_json::from_slice::<EffectAck>(EFFECT_ACK).unwrap(),
        consumer.ack()
    );
}

#[test]
fn fingerprints_hash_raw_bytes_with_wrapping_u64_arithmetic() {
    assert_eq!(skate_net::hash(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(skate_net::hash(b"hello"), 0xa430_d846_80aa_bd0b);
}
