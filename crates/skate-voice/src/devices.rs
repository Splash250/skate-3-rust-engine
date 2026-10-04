use crate::{wire::Playback, *};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceConfig {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
}
#[derive(Clone)]
struct AudioFrame {
    generation: u64,
    samples: Pcm,
}
#[derive(Debug)]
pub struct Encoded {
    pub generation: u64,
    pub data: Vec<u8>,
}
#[derive(Debug, Clone, Serialize)]
pub struct DeviceEvent {
    pub request: u64,
    pub kind: String,
    pub value: serde_json::Value,
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct AudioStats {
    pub captured_frames: u64,
    pub output_callbacks: u64,
    pub audible_samples: u64,
}
enum Command {
    Configure {
        generation: u64,
        config: DeviceConfig,
    },
    List,
    Playback {
        generation: u64,
        packet: Playback,
    },
    Pause {
        generation: u64,
    },
}
struct Flags {
    stop: AtomicBool,
    transmitting: AtomicBool,
    deafened: AtomicBool,
    generation: AtomicU64,
    device_generation: AtomicU64,
    captured_frames: AtomicU64,
    output_callbacks: AtomicU64,
    audible_samples: AtomicU64,
}
pub struct AudioEngine {
    command: mpsc::SyncSender<Command>,
    encoded: Mutex<mpsc::Receiver<Encoded>>,
    events: Arc<Mutex<VecDeque<DeviceEvent>>>,
    flags: Arc<Flags>,
    worker: Option<JoinHandle<()>>,
}
impl AudioEngine {
    /// Starts one worker; device opening, enumeration and Opus never block the caller.
    pub fn start() -> Result<Self> {
        let (command, receiver) = mpsc::sync_channel(128);
        let (encoded, encoded_rx) = mpsc::sync_channel(8);
        let flags = Arc::new(Flags {
            stop: AtomicBool::new(false),
            transmitting: AtomicBool::new(false),
            deafened: AtomicBool::new(false),
            generation: AtomicU64::new(1),
            device_generation: AtomicU64::new(0),
            captured_frames: AtomicU64::new(0),
            output_callbacks: AtomicU64::new(0),
            audible_samples: AtomicU64::new(0),
        });
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let (worker_flags, worker_events) = (flags.clone(), events.clone());
        let worker = thread::Builder::new()
            .name("voice-audio".into())
            .spawn(move || run(receiver, encoded, worker_flags, worker_events))
            .map_err(|e| format!("cannot start voice worker: {e}"))?;
        Ok(Self {
            command,
            encoded: Mutex::new(encoded_rx),
            events,
            flags,
            worker: Some(worker),
        })
    }
    pub fn configure(&self, config: DeviceConfig) -> Result<u64> {
        if config.input_device.as_ref().is_some_and(|s| s.len() > 256)
            || config.output_device.as_ref().is_some_and(|s| s.len() > 256)
        {
            return Err("voice device identifier exceeds256 bytes".into());
        }
        self.flags.transmitting.store(false, Ordering::Release);
        let request = self.flags.device_generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.command
            .try_send(Command::Configure {
                generation: request,
                config,
            })
            .map_err(|_| "voice command queue full".to_string())?;
        Ok(request)
    }
    pub fn list_devices(&self) -> Result<()> {
        self.command
            .try_send(Command::List)
            .map_err(|_| "voice command queue full".into())
    }
    pub fn controls(&self, transmit: bool, deafened: bool, generation: u64) {
        self.flags.transmitting.store(transmit, Ordering::Release);
        self.flags.deafened.store(deafened, Ordering::Release);
        self.flags.generation.store(generation, Ordering::Release);
    }
    pub fn playback(&self, generation: u64, packet: Playback) -> Result<()> {
        self.command
            .try_send(Command::Playback { generation, packet })
            .map_err(|_| "voice playback queue full".into())
    }
    pub fn poll_encoded(&self) -> Vec<Encoded> {
        let Ok(receiver) = self.encoded.try_lock() else {
            return vec![];
        };
        receiver.try_iter().take(8).collect()
    }
    pub fn poll_events(&self) -> Vec<DeviceEvent> {
        self.events
            .try_lock()
            .map(|mut e| e.drain(..).collect())
            .unwrap_or_default()
    }
    pub fn stats(&self) -> AudioStats {
        AudioStats {
            captured_frames: self.flags.captured_frames.load(Ordering::Relaxed),
            output_callbacks: self.flags.output_callbacks.load(Ordering::Relaxed),
            audible_samples: self.flags.audible_samples.load(Ordering::Relaxed),
        }
    }
    pub fn pause(&self) {
        self.flags.transmitting.store(false, Ordering::Release);
        self.flags.deafened.store(true, Ordering::Release);
        let _ = self.command.try_send(Command::Pause {
            generation: self.flags.device_generation.fetch_add(1, Ordering::AcqRel) + 1,
        });
    }
}
impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.flags.stop.store(true, Ordering::Release);
        self.flags.transmitting.store(false, Ordering::Release);
        self.flags.deafened.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // Native driver calls are outside gameplay; do not hang a world teardown.
            let deadline = Instant::now() + Duration::from_millis(100);
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}
fn event(
    events: &Mutex<VecDeque<DeviceEvent>>,
    request: u64,
    kind: &str,
    value: serde_json::Value,
) {
    if let Ok(mut queue) = events.lock() {
        if queue.len() >= 32 {
            queue.pop_front();
        }
        queue.push_back(DeviceEvent {
            request,
            kind: kind.into(),
            value,
        });
    }
}
fn run(
    commands: mpsc::Receiver<Command>,
    encoded: mpsc::SyncSender<Encoded>,
    flags: Arc<Flags>,
    events: Arc<Mutex<VecDeque<DeviceEvent>>>,
) {
    let mut encoder = match Encoder::new() {
        Ok(e) => e,
        Err(e) => {
            event(&events, 0, "error", serde_json::json!({"message":e}));
            return;
        }
    };
    let mut mixer = Mixer::new();
    let mut generation = flags.generation.load(Ordering::Acquire);
    let mut devices: Option<Streams> = None;
    let mut next = Instant::now();
    while !flags.stop.load(Ordering::Acquire) {
        let current = flags.generation.load(Ordering::Acquire);
        if current != generation {
            generation = current;
            mixer.clear();
            let _ = encoder.reset();
        }
        if devices
            .as_ref()
            .is_some_and(|d| d.request != flags.device_generation.load(Ordering::Acquire))
        {
            devices = None;
        }
        for command in commands.try_iter().take(128) {
            match command {
                Command::Configure {
                    generation: owned,
                    config,
                } => {
                    if owned != flags.device_generation.load(Ordering::Acquire) {
                        continue;
                    }
                    devices = None;
                    mixer.clear();
                    let _ = encoder.reset();
                    event(&events, owned, "opening", serde_json::json!({}));
                    match open(config, flags.clone(), owned) {
                        Ok(streams) => {
                            if owned != flags.device_generation.load(Ordering::Acquire) {
                                drop(streams);
                                continue;
                            }
                            event(
                                &events,
                                owned,
                                "ready",
                                serde_json::json!({"input":streams.input_name,"output":streams.output_name}),
                            );
                            devices = Some(streams);
                        }
                        Err(e) => event(&events, owned, "error", serde_json::json!({"message":e})),
                    }
                }
                Command::List => match list_devices() {
                    Ok(value) => event(&events, 0, "devices", value),
                    Err(e) => event(&events, 0, "error", serde_json::json!({"message":e})),
                },
                Command::Playback {
                    generation: owned,
                    packet,
                } => {
                    if owned == generation && !flags.deafened.load(Ordering::Acquire) {
                        let _ = mixer.push(packet);
                    }
                }
                Command::Pause { generation: owned } => {
                    if owned != flags.device_generation.load(Ordering::Acquire) {
                        continue;
                    }
                    devices = None;
                    mixer.clear();
                    let _ = encoder.reset();
                    event(&events, owned, "stopped", serde_json::json!({}));
                }
            }
        }
        if let Some(device) = &devices {
            for frame in device.input.try_iter().take(8) {
                if frame.generation == generation && flags.transmitting.load(Ordering::Acquire) {
                    if let Ok(data) = encoder.encode(&frame.samples) {
                        let _ = encoded.try_send(Encoded { generation, data });
                    }
                }
            }
            for message in device.errors.try_iter().take(4) {
                event(
                    &events,
                    device.request,
                    "error",
                    serde_json::json!({"message":message}),
                );
            }
            if Instant::now() >= next {
                let samples = if flags.deafened.load(Ordering::Acquire) {
                    mixer.clear();
                    [0.; FRAME_SAMPLES]
                } else {
                    mixer.render()
                };
                let _ = device.output.try_send(AudioFrame {
                    generation,
                    samples,
                });
                next += Duration::from_millis(20);
                if Instant::now().saturating_duration_since(next) > Duration::from_millis(40) {
                    next = Instant::now();
                }
            }
        } else {
            next = Instant::now();
        }
        thread::sleep(Duration::from_millis(2));
    }
    drop(devices);
    event(
        &events,
        flags.device_generation.load(Ordering::Acquire),
        "stopped",
        serde_json::json!({}),
    );
}
struct Streams {
    request: u64,
    _input: cpal::Stream,
    _output: cpal::Stream,
    input: mpsc::Receiver<AudioFrame>,
    output: mpsc::SyncSender<AudioFrame>,
    errors: mpsc::Receiver<String>,
    input_name: String,
    output_name: String,
}
fn devices(host: &cpal::Host, input: bool) -> Result<Vec<cpal::Device>> {
    let iterator = if input {
        host.input_devices()
    } else {
        host.output_devices()
    }
    .map_err(|e| e.to_string())?;
    Ok(iterator.take(64).collect())
}
fn label(index: usize, device: &cpal::Device) -> String {
    let mut name = format!(
        "{index}:{}",
        device.name().unwrap_or_else(|_| "unknown device".into())
    );
    if name.len() > 256 {
        let mut end = 256;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    name
}

fn list_devices() -> Result<serde_json::Value> {
    let host = cpal::default_host();
    let names = |input| -> Result<Vec<String>> {
        Ok(devices(&host, input)?
            .iter()
            .enumerate()
            .map(|(i, d)| label(i, d))
            .collect())
    };
    Ok(serde_json::json!({"inputs":names(true)?,"outputs":names(false)?}))
}
fn select(host: &cpal::Host, input: bool, requested: &Option<String>) -> Result<cpal::Device> {
    if let Some(requested) = requested {
        devices(host, input)?
            .into_iter()
            .enumerate()
            .find_map(|(i, d)| (label(i, &d) == *requested).then_some(d))
            .ok_or_else(|| "selected voice device is unavailable; refresh device list".into())
    } else {
        if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        }
        .ok_or_else(|| "no default voice device is available".into())
    }
}
fn config(supported: &cpal::SupportedStreamConfig) -> Result<cpal::StreamConfig> {
    let mut config = supported.config();
    if !(8_000..=192_000).contains(&config.sample_rate.0) || !(1..=32).contains(&config.channels) {
        return Err("voice device rate/channels are unsupported".into());
    }
    config.buffer_size = match supported.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => {
            if *min > 4096 {
                return Err("voice device minimum buffer exceeds4096 frames".into());
            }
            cpal::BufferSize::Fixed(480u32.clamp(*min, (*max).min(4096)))
        }
        cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
    };
    Ok(config)
}
fn open(request: DeviceConfig, flags: Arc<Flags>, ticket: u64) -> Result<Streams> {
    let host = cpal::default_host();
    let input = select(&host, true, &request.input_device)?;
    let output = select(&host, false, &request.output_device)?;
    let input_supported = input.default_input_config().map_err(|e| e.to_string())?;
    let output_supported = output.default_output_config().map_err(|e| e.to_string())?;
    let input_config = config(&input_supported)?;
    let output_config = config(&output_supported)?;
    let (input_tx, input_rx) = mpsc::sync_channel(8);
    let (output_tx, output_rx) = mpsc::sync_channel(4);
    let (error_tx, error_rx) = mpsc::sync_channel(4);
    let input_stream = match input_supported.sample_format() {
        cpal::SampleFormat::F32 => capture::<f32>(
            &input,
            &input_config,
            input_tx,
            flags.clone(),
            error_tx.clone(),
            |s| s,
        ),
        cpal::SampleFormat::I16 => capture::<i16>(
            &input,
            &input_config,
            input_tx,
            flags.clone(),
            error_tx.clone(),
            |s| s as f32 / 32768.,
        ),
        cpal::SampleFormat::U16 => capture::<u16>(
            &input,
            &input_config,
            input_tx,
            flags.clone(),
            error_tx.clone(),
            |s| s as f32 / 32768. - 1.,
        ),
        _ => Err("voice input requires f32/i16/u16 device samples".into()),
    }?;
    let output_stream = match output_supported.sample_format() {
        cpal::SampleFormat::F32 => {
            playback::<f32>(&output, &output_config, output_rx, flags, error_tx, |s| s)
        }
        cpal::SampleFormat::I16 => {
            playback::<i16>(&output, &output_config, output_rx, flags, error_tx, |s| {
                (s * 32767.) as i16
            })
        }
        cpal::SampleFormat::U16 => {
            playback::<u16>(&output, &output_config, output_rx, flags, error_tx, |s| {
                ((s + 1.) * 32767.5) as u16
            })
        }
        _ => Err("voice output requires f32/i16/u16 device samples".into()),
    }?;
    input_stream.play().map_err(|e| e.to_string())?;
    output_stream.play().map_err(|e| e.to_string())?;
    Ok(Streams {
        request: ticket,
        _input: input_stream,
        _output: output_stream,
        input: input_rx,
        output: output_tx,
        errors: error_rx,
        input_name: input.name().unwrap_or_default(),
        output_name: output.name().unwrap_or_default(),
    })
}
fn capture<T: cpal::SizedSample + Copy>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    frames: mpsc::SyncSender<AudioFrame>,
    flags: Arc<Flags>,
    errors: mpsc::SyncSender<String>,
    convert: impl Fn(T) -> f32 + Send + 'static,
) -> Result<cpal::Stream> {
    let mut resampler = CaptureResampler::new(config.sample_rate.0, config.channels as usize)?;
    let mut generation = flags.generation.load(Ordering::Acquire);
    device
        .build_input_stream(
            config,
            move |samples: &[T], _| {
                let current = flags.generation.load(Ordering::Acquire);
                if current != generation {
                    generation = current;
                    resampler.reset();
                }
                if flags.stop.load(Ordering::Acquire) || !flags.transmitting.load(Ordering::Acquire)
                {
                    resampler.reset();
                    return;
                }
                resampler.push(samples, &convert, |samples| {
                    flags.captured_frames.fetch_add(1, Ordering::Relaxed);
                    let _ = frames.try_send(AudioFrame {
                        generation,
                        samples,
                    });
                });
            },
            move |e| {
                let _ = errors.try_send(e.to_string().chars().take(256).collect());
            },
            None,
        )
        .map_err(|e| e.to_string())
}
fn playback<T: cpal::SizedSample + Copy>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    frames: mpsc::Receiver<AudioFrame>,
    flags: Arc<Flags>,
    errors: mpsc::SyncSender<String>,
    convert: impl Fn(f32) -> T + Send + 'static,
) -> Result<cpal::Stream> {
    let channels = config.channels as usize;
    let step = SAMPLE_RATE as f64 / config.sample_rate.0 as f64;
    let mut samples = [0.; FRAME_SAMPLES];
    let mut offset = FRAME_SAMPLES;
    let mut generation = flags.generation.load(Ordering::Acquire);
    let (mut left, mut right, mut phase) = (0., 0., 0.);
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                flags.output_callbacks.fetch_add(1, Ordering::Relaxed);
                let current = flags.generation.load(Ordering::Acquire);
                if current != generation {
                    generation = current;
                    offset = FRAME_SAMPLES;
                    left = 0.;
                    right = 0.;
                    phase = 0.;
                }
                if flags.stop.load(Ordering::Acquire) || flags.deafened.load(Ordering::Acquire) {
                    for sample in output {
                        *sample = convert(0.);
                    }
                    for _ in 0..4 {
                        if frames.try_recv().is_err() {
                            break;
                        }
                    }
                    offset = FRAME_SAMPLES;
                    left = 0.;
                    right = 0.;
                    return;
                }
                let mut audible = 0;
                for frame in output.chunks_mut(channels) {
                    let value = (left + (right - left) * phase as f32).clamp(-1., 1.);
                    if value.abs() > 0.0001 {
                        audible += 1;
                    }
                    for sample in frame {
                        *sample = convert(value);
                    }
                    phase += step;
                    while phase >= 1. {
                        left = right;
                        if offset >= FRAME_SAMPLES {
                            let mut next = None;
                            for _ in 0..4 {
                                let Ok(frame) = frames.try_recv() else { break };
                                if frame.generation == generation {
                                    next = Some(frame.samples);
                                    break;
                                }
                            }
                            if let Some(frame) = next {
                                samples = frame;
                                offset = 0;
                            }
                        }
                        right = if offset < FRAME_SAMPLES {
                            let v = samples[offset];
                            offset += 1;
                            v
                        } else {
                            0.
                        };
                        phase -= 1.;
                    }
                }
                flags.audible_samples.fetch_add(audible, Ordering::Relaxed);
            },
            move |e| {
                let _ = errors.try_send(e.to_string().chars().take(256).collect());
            },
            None,
        )
        .map_err(|e| e.to_string())
}
