#![cfg(all(feature = "devices", target_os = "linux"))]
use skate_voice::{wire::Playback, *};
use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Uses real CPAL ALSA callbacks backed by synthetic file/null PCMs. It never
/// opens a physical microphone or sends audio outside this process.
#[test]
fn real_alsa_capture_playback_and_device_teardown() {
    let directory = std::env::temp_dir().join(format!("skate-voice-alsa-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("input.raw");
    let mut data = Vec::with_capacity(48000 * 4);
    for n in 0..48000 {
        data.extend_from_slice(
            &((n as f32 * 440. * std::f32::consts::TAU / 48000.).sin() * 0.3).to_le_bytes(),
        );
    }
    fs::write(&source, data).unwrap();
    let configuration = directory.join("alsa.conf");
    fs::write(
        &configuration,
        format!(
            r#"
pcm.null {{ type null }}
pcm.capture {{
 type file
 slave.pcm "null"
 file "/dev/null"
 infile "{}"
 format "raw"
}}
pcm.!default {{
 type asym
 capture.pcm "capture"
 playback.pcm "null"
}}
"#,
            source.display()
        ),
    )
    .unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "device_worker_child", "--ignored", "--nocapture"])
        .env("ALSA_CONFIG_PATH", &configuration)
        .env("SKATE_VOICE_SYNTHETIC_INPUT", &source)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            let _ = fs::remove_dir_all(&directory);
            panic!(
                "synthetic ALSA child timed out: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    fs::remove_dir_all(&directory).unwrap();
    assert!(
        output.status.success(),
        "synthetic ALSA child failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "invoked only with isolated synthetic ALSA configuration by the parent test"]
fn device_worker_child() {
    let source = std::env::var_os("SKATE_VOICE_SYNTHETIC_INPUT").expect("synthetic input required");
    let engine = AudioEngine::start().unwrap();
    let ticket = engine.configure(DeviceConfig::default()).unwrap();
    engine.controls(true, false, 1);
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut ready = false;
    while !ready {
        for event in engine.poll_events() {
            assert_ne!(event.kind, "error", "{}", event.value);
            ready |= event.kind == "ready" && event.request == ticket;
        }
        assert!(Instant::now() < deadline, "device ready timeout");
        thread::sleep(Duration::from_millis(2));
    }
    engine.controls(true, false, 1);
    let mut encoder = Encoder::new().unwrap();
    let mut sequence = 1;
    let mut received = false;
    while !received || engine.stats().audible_samples == 0 {
        received |= !engine.poll_encoded().is_empty();
        let samples = std::array::from_fn(|n| {
            ((n as f32 * 440. * std::f32::consts::TAU / 48000.).sin()) * 0.2
        });
        let data = encoder.encode(&samples).unwrap();
        engine
            .playback(
                1,
                Playback {
                    session: 9,
                    actor: 2,
                    recipient_epoch: 1,
                    sender_epoch: 1,
                    revision: 1,
                    sequence,
                    gain: 1.,
                    data,
                },
            )
            .unwrap();
        sequence += 1;
        assert!(
            Instant::now() < deadline,
            "capture/playback timeout: {:?}",
            engine.stats()
        );
        thread::sleep(Duration::from_millis(20));
    }
    let stats = engine.stats();
    assert!(stats.captured_frames > 0 && stats.output_callbacks > 0 && stats.audible_samples > 0);
    engine.controls(false, true, 2);
    thread::sleep(Duration::from_millis(30));
    let muted = engine.stats();
    thread::sleep(Duration::from_millis(50));
    let muted_later = engine.stats();
    assert_eq!(
        muted.captured_frames, muted_later.captured_frames,
        "mute stops capture processing"
    );
    assert_eq!(
        muted.audible_samples, muted_later.audible_samples,
        "deafen silences output callback"
    );
    engine.pause();
    while !engine
        .poll_events()
        .iter()
        .any(|event| event.kind == "stopped")
    {
        assert!(Instant::now() < deadline, "device teardown timeout");
        thread::sleep(Duration::from_millis(2));
    }
    drop(engine);
    assert!(
        !fs::read_dir("/proc/self/fd")
            .unwrap()
            .flatten()
            .any(|entry| {
                fs::read_link(entry.path()).is_ok_and(|path| path.as_os_str() == source)
            }),
        "capture file descriptor must close on device teardown"
    );
}
