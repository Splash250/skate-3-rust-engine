use crate::{wire::Playback, *};
use std::collections::BTreeMap;
pub type Pcm = [f32; FRAME_SAMPLES];

pub struct Encoder {
    encoder: opus::Encoder,
}
impl Encoder {
    pub fn new() -> Result<Self> {
        let mut encoder =
            opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip)
                .map_err(|e| e.to_string())?;
        encoder
            .set_bitrate(opus::Bitrate::Bits(24_000))
            .map_err(|e| e.to_string())?;
        encoder.set_complexity(5).map_err(|e| e.to_string())?;
        encoder.set_dtx(true).map_err(|e| e.to_string())?;
        encoder.set_inband_fec(true).map_err(|e| e.to_string())?;
        encoder
            .set_packet_loss_perc(10)
            .map_err(|e| e.to_string())?;
        Ok(Self { encoder })
    }
    pub fn encode(&mut self, pcm: &Pcm) -> Result<Vec<u8>> {
        if !pcm
            .iter()
            .all(|n| n.is_finite() && (-1.0..=1.0).contains(n))
        {
            return Err("voice PCM must contain finite normalized samples".into());
        }
        let mut out = [0; MAX_ENCODED];
        let n = self
            .encoder
            .encode_float(pcm, &mut out)
            .map_err(|e| e.to_string())?;
        Ok(out[..n].to_vec())
    }
    pub fn reset(&mut self) -> Result<()> {
        self.encoder.reset_state().map_err(|e| e.to_string())
    }
}
struct Speaker {
    decoder: opus::Decoder,
    epoch: u64,
    queue: BTreeMap<u64, (Vec<u8>, f32)>,
    expected: Option<u64>,
    gain: f32,
    missing: u8,
    wait: u8,
    last: u64,
}
pub struct Mixer {
    speakers: BTreeMap<u64, Speaker>,
    clock: u64,
}
impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}
impl Mixer {
    pub fn new() -> Self {
        Self {
            speakers: BTreeMap::new(),
            clock: 0,
        }
    }
    pub fn clear(&mut self) {
        self.speakers.clear();
    }
    pub fn streams(&self) -> usize {
        self.speakers.len()
    }
    pub fn pending(&self) -> usize {
        self.speakers.values().map(|s| s.queue.len()).sum()
    }
    pub fn push(&mut self, packet: Playback) -> Result<()> {
        if packet.data.is_empty()
            || packet.data.len() > MAX_ENCODED
            || !packet.gain.is_finite()
            || !(0.0..=1.).contains(&packet.gain)
            || packet.sequence == 0
        {
            return Err("invalid playback voice frame".into());
        }
        if opus::packet::get_nb_samples(&packet.data, SAMPLE_RATE).map_err(|e| e.to_string())?
            != FRAME_SAMPLES
        {
            return Err("only20ms mono Opus voice frames are supported".into());
        }
        if self
            .speakers
            .get(&packet.actor)
            .is_some_and(|s| s.epoch != packet.sender_epoch)
        {
            self.speakers.remove(&packet.actor);
        }
        if !self.speakers.contains_key(&packet.actor) {
            if self.speakers.len() >= MAX_TALKERS {
                if let Some(oldest) = self
                    .speakers
                    .iter()
                    .min_by_key(|(_, s)| s.last)
                    .map(|(id, _)| *id)
                {
                    self.speakers.remove(&oldest);
                }
            }
            self.speakers.insert(
                packet.actor,
                Speaker {
                    decoder: opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono)
                        .map_err(|e| e.to_string())?,
                    epoch: packet.sender_epoch,
                    queue: BTreeMap::new(),
                    expected: None,
                    gain: packet.gain,
                    missing: 0,
                    wait: 1,
                    last: self.clock,
                },
            );
        }
        let speaker = self.speakers.get_mut(&packet.actor).unwrap();
        if speaker
            .expected
            .is_some_and(|expected| packet.sequence < expected)
            || speaker.queue.contains_key(&packet.sequence)
        {
            return Err("stale or duplicate playback voice frame".into());
        }
        if speaker
            .expected
            .is_some_and(|expected| packet.sequence > expected.saturating_add(10))
        {
            speaker.queue.clear();
            speaker.expected = None;
            speaker.decoder.reset_state().map_err(|e| e.to_string())?;
            speaker.wait = 1;
        }
        while speaker.queue.len() >= 4 {
            speaker.queue.pop_first();
        }
        speaker.last = self.clock;
        speaker
            .queue
            .insert(packet.sequence, (packet.data, packet.gain));
        Ok(())
    }
    pub fn render(&mut self) -> Pcm {
        self.clock = self.clock.saturating_add(1);
        let mut mix = [0.; FRAME_SAMPLES];
        self.speakers
            .retain(|_, s| self.clock.saturating_sub(s.last) < 100);
        for speaker in self.speakers.values_mut() {
            if speaker.wait > 0 {
                speaker.wait -= 1;
                continue;
            }
            let Some(sequence) = speaker
                .expected
                .or_else(|| speaker.queue.first_key_value().map(|(seq, _)| *seq))
            else {
                continue;
            };
            let mut pcm = [0.; FRAME_SAMPLES];
            let decoded = if let Some((data, gain)) = speaker.queue.remove(&sequence) {
                speaker.gain = gain;
                speaker.missing = 0;
                speaker.decoder.decode_float(&data, &mut pcm, false)
            } else if speaker.missing < 3 {
                speaker.missing += 1;
                if let Some((data, _)) = speaker.queue.get(&sequence.saturating_add(1)) {
                    speaker.decoder.decode_float(data, &mut pcm, true)
                } else {
                    speaker.decoder.decode_float(&[], &mut pcm, false)
                }
            } else {
                speaker.expected = None;
                let _ = speaker.decoder.reset_state();
                continue;
            };
            speaker.expected = sequence.checked_add(1);
            if let Ok(n) = decoded {
                if n == FRAME_SAMPLES {
                    for (i, sample) in pcm.iter().enumerate() {
                        if sample.is_finite() {
                            mix[i] += sample * speaker.gain;
                        }
                    }
                }
            }
        }
        for sample in &mut mix {
            *sample = sample.clamp(-1., 1.);
        }
        mix
    }
}

/// Stateful linear conversion used by device callbacks; the Opus boundary
/// always receives exactly960 mono48kHz samples regardless of device rate.
pub struct CaptureResampler {
    input_rate: u32,
    channels: usize,
    phase: f64,
    last: f32,
    initialized: bool,
    frame: Pcm,
    offset: usize,
}
impl CaptureResampler {
    pub fn new(input_rate: u32, channels: usize) -> Result<Self> {
        if !(8_000..=192_000).contains(&input_rate) || !(1..=32).contains(&channels) {
            return Err("unsupported voice device rate or channel count".into());
        }
        Ok(Self {
            input_rate,
            channels,
            phase: 0.,
            last: 0.,
            initialized: false,
            frame: [0.; FRAME_SAMPLES],
            offset: 0,
        })
    }
    pub fn reset(&mut self) {
        self.phase = 0.;
        self.initialized = false;
        self.offset = 0;
    }
    pub fn push<T: Copy>(
        &mut self,
        input: &[T],
        convert: impl Fn(T) -> f32,
        mut frame: impl FnMut(Pcm),
    ) {
        for samples in input.chunks_exact(self.channels) {
            let sample = (samples.iter().map(|s| convert(*s)).sum::<f32>() / self.channels as f32)
                .clamp(-1., 1.);
            let sample = if sample.is_finite() { sample } else { 0. };
            if !self.initialized {
                self.last = sample;
                self.initialized = true;
                continue;
            }
            while self.phase < 1. {
                self.frame[self.offset] = self.last + (sample - self.last) * self.phase as f32;
                self.offset += 1;
                if self.offset == FRAME_SAMPLES {
                    frame(self.frame);
                    self.offset = 0;
                }
                self.phase += self.input_rate as f64 / SAMPLE_RATE as f64;
            }
            self.phase -= 1.;
            self.last = sample;
        }
    }
}
