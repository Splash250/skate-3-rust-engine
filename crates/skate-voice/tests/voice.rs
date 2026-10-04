use skate_voice::{
    wire::{Frame, Playback},
    *,
};
#[cfg(feature = "codec")]
use std::net::UdpSocket;
fn players() -> Vec<Player> {
    vec![
        Player {
            actor: 1,
            peer: 11,
            instance: 0,
            epoch: 1,
            position: [0., 0., 0.],
        },
        Player {
            actor: 2,
            peer: 22,
            instance: 0,
            epoch: 1,
            position: [3., 0., 0.],
        },
        Player {
            actor: 3,
            peer: 33,
            instance: 0,
            epoch: 1,
            position: [80., 0., 0.],
        },
        Player {
            actor: 4,
            peer: 44,
            instance: 1,
            epoch: 1,
            position: [1., 0., 0.],
        },
    ]
}
fn frame(router: &Router, sequence: u64, channel: &str) -> Vec<u8> {
    Frame {
        session: 9,
        actor: 1,
        epoch: 1,
        revision: router.revision(),
        sequence,
        channel: channel.into(),
        data: vec![1, 2, 3],
    }
    .encode()
    .unwrap()
}
#[test]
fn proximity_radio_identity_revision_instance_and_resource_cleanup() {
    let mut router = Router::new(9, 999).unwrap();
    let mut players = players();
    router.sync(&players, 0).unwrap();
    router.drain(64);
    let first = frame(&router, 1, "");
    assert!(
        !router.receive(22, &first, 0),
        "peer cannot claim another actor"
    );
    assert!(router.receive(11, &first, 0));
    assert!(!router.receive(11, &first, 0));
    let forwarded = router.drain(64);
    assert_eq!(forwarded.len(), 1);
    assert_eq!(forwarded[0].0, 22);
    let old = forwarded[0].1.clone();
    assert!(Playback::decode(&old).unwrap().gain < 1.);
    let mut client = ClientState::default();
    client.synchronize(Some(Context {
        session: 9,
        host_peer: 999,
        actor: 2,
        epoch: 1,
        instance: 0,
    }));
    assert!(client.accept(33, &old).is_none());
    assert!(client.accept(999, &old).is_some());
    assert!(client.accept(999, &old).is_none());
    router.activate("radio", 1).unwrap();
    router
        .command(
            "radio",
            1,
            ServerCommand::Channel {
                name: "crew".into(),
                members: vec!["1".into(), "3".into(), "4".into()],
            },
        )
        .unwrap();
    router.sync(&players, 20).unwrap();
    router.drain(64);
    let radio = frame(&router, 2, "radio/crew");
    assert!(router.receive(11, &radio, 20));
    let radio_packets = router.drain(64);
    assert_eq!(radio_packets.len(), 1);
    assert_eq!(
        radio_packets[0].0, 33,
        "radio does not cross instance isolation"
    );
    assert_eq!(Playback::decode(&radio_packets[0].1).unwrap().gain, 1.);
    assert!(
        router
            .command("radio", 0, ServerCommand::Proximity { meters: 50. })
            .is_err()
    );
    assert!(router.receive(11, &frame(&router, 3, "radio/crew"), 40));
    router.revoke("radio", 1);
    assert_eq!(
        router.pending(),
        0,
        "resource teardown clears buffered speech immediately"
    );
    assert!(!router.receive(11, &radio, 60));
    router.activate("radio", 2).unwrap();
    assert!(
        router
            .command("radio", 1, ServerCommand::Proximity { meters: 5. })
            .is_err()
    );
    router.sync(&players, 80).unwrap();
    router.drain(64);
    let stale = frame(&router, 4, "");
    players[1].instance = 2;
    players[1].epoch = 2;
    router.sync(&players, 100).unwrap();
    assert!(
        !router.receive(11, &stale, 100),
        "queued speech from an earlier routing revision is rejected"
    );
    client.synchronize(Some(Context {
        session: 9,
        host_peer: 999,
        actor: 2,
        epoch: 2,
        instance: 2,
    }));
    assert!(
        client.accept(999, &old).is_none(),
        "old instance PCM cannot enter new playback context"
    );
    router
        .command(
            "radio",
            2,
            ServerCommand::Mute {
                player: "1".into(),
                muted: true,
            },
        )
        .unwrap();
    router.sync(&players, 120).unwrap();
    assert!(!router.receive(11, &frame(&router, 5, ""), 120));
}
#[test]
fn bounded_voice_rate_queues_and_fairness_with_sixty_four_players() {
    let mut router = Router::new(9, 999).unwrap();
    let players = (1..=64)
        .map(|id| Player {
            actor: id,
            peer: id + 100,
            instance: 0,
            epoch: 1,
            position: [id as f32 / 20., 0., 0.],
        })
        .collect::<Vec<_>>();
    router.sync(&players, 0).unwrap();
    assert_eq!(router.drain(64).len(), 64);
    let mut accepted = 0;
    for sequence in 1..=100 {
        let packet = Frame {
            session: 9,
            actor: 1,
            epoch: 1,
            revision: router.revision(),
            sequence,
            channel: String::new(),
            data: vec![3; MAX_ENCODED],
        }
        .encode()
        .unwrap();
        accepted += usize::from(router.receive(101, &packet, 0));
    }
    assert_eq!(accepted, 5);
    assert!(router.pending() <= 64 * 5);
    assert!(router.metrics.queue_dropped > 0);
    let mut recipients = std::collections::BTreeSet::new();
    for _ in 0..2 {
        for (peer, packet) in router.drain(32) {
            assert!(packet.len() <= skate_net::packed::MTU);
            recipients.insert(peer);
        }
    }
    assert_eq!(
        recipients.len(),
        63,
        "voice draining must rotate across listeners"
    );
    assert!(
        Frame {
            session: 9,
            actor: 1,
            epoch: 1,
            revision: router.revision(),
            sequence: 101,
            channel: String::new(),
            data: vec![0; MAX_ENCODED + 1]
        }
        .encode()
        .is_err()
    );
}

#[test]
fn eight_simultaneous_speakers_survive_batches_and_share_limited_egress_fairly() {
    use std::collections::BTreeSet;
    let mut router = Router::new(9, 999).unwrap();
    let players = (1..=9)
        .map(|actor| Player {
            actor,
            peer: actor + 100,
            instance: 0,
            epoch: 1,
            position: [0.; 3],
        })
        .collect::<Vec<_>>();
    router.sync(&players, 0).unwrap();
    router.drain(512);
    let mut heard = BTreeSet::new();
    for batch in 1..=20 {
        for actor in 1..=8 {
            let packet = Frame {
                session: 9,
                actor,
                epoch: 1,
                revision: router.revision(),
                sequence: batch,
                channel: String::new(),
                data: vec![1, 2, 3],
            }
            .encode()
            .unwrap();
            assert!(router.receive(actor + 100, &packet, batch * 20));
        }
        // Deliberately slower than production: each recipient receives only
        // about four frames/batch, but every selected speaker must get a turn.
        for (peer, bytes) in router.drain(36) {
            if peer == 109 {
                heard.insert(Playback::decode(&bytes).unwrap().actor);
            }
        }
        assert!(router.pending() <= 9 * 17);
    }
    assert_eq!(
        heard,
        (1..=8).collect(),
        "one speaker FIFO must not overwrite all earlier talkers in every batch"
    );
}
#[cfg(feature = "codec")]
#[test]
fn real_opus_pcm_over_real_udp_and_bounded_mixing() {
    let source = UdpSocket::bind("127.0.0.1:0").unwrap();
    let host = UdpSocket::bind("127.0.0.1:0").unwrap();
    let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
    host.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    receiver
        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let mut router = Router::new(9, 999).unwrap();
    router.sync(&players()[..2], 0).unwrap();
    router.drain(64);
    let mut encoder = Encoder::new().unwrap();
    let mut mixer = Mixer::new();
    let mut energy = 0.;
    let mut total = 0;
    let mut buffer = [0; 1200];
    for index in 0..30u64 {
        let pcm = std::array::from_fn(|i| {
            ((index as usize * FRAME_SAMPLES + i) as f32 * 440. * std::f32::consts::TAU
                / SAMPLE_RATE as f32)
                .sin()
                * 0.25
        });
        let data = encoder.encode(&pcm).unwrap();
        assert!(data.len() <= MAX_ENCODED);
        let encoded = Frame {
            session: 9,
            actor: 1,
            epoch: 1,
            revision: router.revision(),
            sequence: index + 1,
            channel: String::new(),
            data,
        }
        .encode()
        .unwrap();
        source
            .send_to(&encoded, host.local_addr().unwrap())
            .unwrap();
        let (n, _) = host.recv_from(&mut buffer).unwrap();
        assert!(router.receive(11, &buffer[..n], index * 20));
        for (peer, packet) in router.drain(64) {
            if peer == 22 {
                host.send_to(&packet, receiver.local_addr().unwrap())
                    .unwrap();
                let (n, _) = receiver.recv_from(&mut buffer).unwrap();
                mixer.push(Playback::decode(&buffer[..n]).unwrap()).unwrap();
            }
        }
        let decoded = mixer.render();
        energy += decoded.iter().map(|v| v * v).sum::<f32>();
        total += decoded.len();
    }
    assert!(
        energy / total as f32 > 0.005,
        "actual Opus-decoded signal must be audible"
    );
    assert!(mixer.pending() <= MAX_TALKERS * 4);
    mixer.clear();
    assert_eq!(mixer.streams(), 0);
    assert!(mixer.render().iter().all(|v| *v == 0.));
}
#[cfg(feature = "codec")]
#[test]
fn capture_resampling_and_playback_discontinuity_are_deterministic() {
    let mut resampler = CaptureResampler::new(44_100, 2).unwrap();
    let input = (0..44_101)
        .flat_map(|i| {
            let v = (i as f32 * 440. * std::f32::consts::TAU / 44_100.).sin() * 0.2;
            [v, v]
        })
        .collect::<Vec<_>>();
    let mut frames = Vec::new();
    resampler.push(&input, |s| s, |frame| frames.push(frame));
    assert_eq!(frames.len(), 50);
    assert!(
        frames
            .iter()
            .flatten()
            .all(|n| n.is_finite() && n.abs() <= 0.201)
    );
    resampler.reset();
    let mut after = 0;
    resampler.push(&vec![0.; 882 * 2], |s| s, |_| after += 1);
    assert_eq!(after, 0, "reset drops fractional pre-context capture");
    let mut encoder = Encoder::new().unwrap();
    assert!(encoder.encode(&[f32::NAN; FRAME_SAMPLES]).is_err());
    let data = encoder.encode(&[0.2; FRAME_SAMPLES]).unwrap();
    let mut mixer = Mixer::new();
    for sequence in 1..20 {
        mixer
            .push(Playback {
                session: 9,
                actor: 1,
                recipient_epoch: 1,
                sender_epoch: 1,
                revision: 1,
                sequence,
                gain: 1.,
                data: data.clone(),
            })
            .unwrap();
    }
    assert!(mixer.pending() <= 4);
    for _ in 0..110 {
        mixer.render();
    }
    assert_eq!(mixer.streams(), 0);
}
