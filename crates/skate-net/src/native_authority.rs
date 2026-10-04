//! Native input replay contract. This carries commands, never client outcomes.
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const MAX_TICKS: u64 = 3600;
pub const MAX_REQUEST_BYTES: usize = 2048;
pub const MAX_REPLY_BYTES: usize = 32768;
pub const MAX_PACKET_INPUTS: usize = 32;
pub const INPUT_KEY: &str = "builtin:native-input";
pub const AUTHORITY_PREFIX: &str = "builtin:native-authority:";
pub fn state_key(actor: u64) -> String {
    format!("{AUTHORITY_PREFIX}{actor}")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputPacket {
    pub inputs: Vec<Input>,
}
impl InputPacket {
    pub fn valid(&self) -> bool {
        !self.inputs.is_empty()
            && self.inputs.len() <= MAX_PACKET_INPUTS
            && self.inputs.iter().all(Input::valid)
            && self
                .inputs
                .windows(2)
                .all(|p| p[0].epoch == p[1].epoch && p[0].tick + 1 == p[1].tick)
    }
    /// A complete resend window fits the 1KiB application record without
    /// quantizing analog actions: 12 header bytes plus 26 bytes per tick.
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if !self.valid() {
            return Err("Invalid native input window".into());
        }
        let first = &self.inputs[0];
        let mut bytes = Vec::with_capacity(12 + self.inputs.len() * 26);
        bytes.push(VERSION as u8);
        bytes.extend_from_slice(&first.epoch.to_le_bytes());
        bytes.extend_from_slice(&(first.tick as u16).to_le_bytes());
        bytes.push(self.inputs.len() as u8);
        for sample in &self.inputs {
            for slot in ANALOG_SLOTS {
                let value = sample.actions[slot];
                bytes.extend_from_slice(&(if value == 0. { 0. } else { value }).to_le_bytes());
            }
            let mut bits = 0u16;
            for (bit, slot) in DIGITAL_SLOTS.into_iter().enumerate() {
                if sample.actions[slot] == 1. {
                    bits |= 1 << bit;
                }
            }
            bytes.extend_from_slice(&bits.to_le_bytes());
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 12 || bytes[0] != VERSION as u8 {
            return Err("Invalid native input header".into());
        }
        let count = bytes[11] as usize;
        if !(1..=MAX_PACKET_INPUTS).contains(&count) || bytes.len() != 12 + 26 * count {
            return Err("Invalid native input framing".into());
        }
        let epoch = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
        let first_tick = u16::from_le_bytes(bytes[9..11].try_into().unwrap()) as u64;
        let mut inputs = Vec::with_capacity(count);
        for (index, sample) in bytes[12..].chunks_exact(26).enumerate() {
            let mut actions = [0.; 18];
            for (offset, slot) in ANALOG_SLOTS.into_iter().enumerate() {
                let start = offset * 4;
                actions[slot] = f32::from_le_bytes(sample[start..start + 4].try_into().unwrap());
            }
            let bits = u16::from_le_bytes(sample[24..26].try_into().unwrap());
            if bits & 0xf000 != 0 {
                return Err("Reserved native input bits".into());
            }
            for (bit, slot) in DIGITAL_SLOTS.into_iter().enumerate() {
                actions[slot] = ((bits >> bit) & 1) as f32;
            }
            inputs.push(Input {
                epoch,
                tick: first_tick + index as u64,
                actions,
            });
        }
        let packet = Self { inputs };
        if !packet.valid() {
            return Err("Invalid native input values".into());
        }
        Ok(packet)
    }
}
const ANALOG_SLOTS: [usize; 6] = [0, 1, 3, 4, 6, 7];
const DIGITAL_SLOTS: [usize; 12] = [2, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Admission {
    pub version: u32,
    pub epoch: u64,
    pub instance: u64,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub epoch: u64,
    pub tick: u64,
    pub actions: [f32; 18],
}
impl Input {
    pub fn valid(&self) -> bool {
        self.epoch != 0
            && (1..=MAX_TICKS).contains(&self.tick)
            && self.actions.iter().enumerate().all(|(index, value)| {
                value.is_finite()
                    && match index {
                        0 | 1 | 3 | 4 => (-1.0..=1.0).contains(value),
                        6 | 7 => (0.0..=1.0).contains(value),
                        _ => *value == 0.0 || *value == 1.0,
                    }
            })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Score {
    pub trick_seq: u32,
    pub trick: String,
    pub landing_seq: u32,
    pub landed_trick: String,
    pub bail_seq: u32,
    pub sequence: f32,
    /// Attempt-wide sum at the native final reward publication boundary.
    pub awarded: f64,
    pub publications: u64,
    pub line: f32,
    pub completed_lines: f32,
    pub multiplier: f32,
    pub clean: bool,
    pub sketchy: bool,
}

/// Native physical/public scoring state; only the trusted native worker produces
/// this. A client input can never contain a snapshot or an outcome claim.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub admission: Admission,
    pub tick: u64,
    pub root: crate::Pose,
    pub enabled: u64,
    pub bodies: Vec<crate::Body>,
    pub state: u32,
    pub score: Score,
    pub history_digest: String,
}
impl Snapshot {
    pub fn state_digest(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("Native snapshot serialization");
        blake3::hash(&bytes).to_hex().to_string()
    }
    pub fn state(&self, status: Status, world: String, difficulty: String, ticks: u64) -> State {
        State {
            admission: self.admission,
            tick: self.tick,
            history_digest: self.history_digest.clone(),
            state_digest: self.state_digest(),
            score: Some(self.score.clone()),
            status,
            world,
            difficulty,
            ticks,
            reason: None,
            resource: String::new(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Starting,
    Running,
    Completed,
    Cancelled,
    Rejected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub resource: String,
    pub admission: Admission,
    pub tick: u64,
    pub history_digest: String,
    pub state_digest: String,
    pub score: Option<Score>,
    pub status: Status,
    pub world: String,
    pub difficulty: String,
    pub ticks: u64,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Start { admission: Admission },
    Step { input: Input },
    Stop,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Ready { snapshot: Snapshot },
    Advanced { snapshot: Snapshot },
    Rejected { error: String },
}

pub struct InputLog {
    admission: Admission,
    inputs: Vec<Input>,
    digest: blake3::Hasher,
}
impl InputLog {
    pub fn new(admission: Admission) -> Result<Self, String> {
        if admission.version != VERSION || admission.epoch == 0 || admission.generation == 0 {
            return Err("Unsupported native authority version or invalid admission".into());
        }
        let mut digest = blake3::Hasher::new();
        digest.update(b"skate-native-input-v1");
        digest.update(&admission.version.to_le_bytes());
        digest.update(&admission.epoch.to_le_bytes());
        digest.update(&admission.instance.to_le_bytes());
        digest.update(&admission.generation.to_le_bytes());
        Ok(Self {
            admission,
            inputs: Vec::new(),
            digest,
        })
    }
    pub fn admission(&self) -> Admission {
        self.admission
    }
    pub fn append(&mut self, input: Input) -> Result<bool, String> {
        if !input.valid() || input.epoch != self.admission.epoch {
            return Err("Invalid native input or stale movement epoch".into());
        }
        if input.tick <= self.inputs.len() as u64 {
            return if self.inputs[input.tick as usize - 1] == input {
                Ok(false)
            } else {
                Err("Native input history cannot be changed".into())
            };
        }
        if input.tick != self.inputs.len() as u64 + 1 {
            return Err("Native input must be contiguous".into());
        }
        self.digest.update(&input.tick.to_le_bytes());
        for action in input.actions {
            // Canonicalize signed zero: both represent the same gameplay input.
            self.digest
                .update(&if action == 0. { 0u32 } else { action.to_bits() }.to_le_bytes());
        }
        self.inputs.push(input);
        Ok(true)
    }
    /// The host clock permits 60 simulation commands/second and three startup
    /// ticks; client timing and packet frequency never increase this allowance.
    pub fn append_at(&mut self, input: Input, elapsed_ms: u64) -> Result<bool, String> {
        if input.tick > elapsed_ms.saturating_mul(60) / 1000 + 3 {
            return Err("Native input exceeds the server simulation clock".into());
        }
        self.append(input)
    }
    pub fn len(&self) -> usize {
        self.inputs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty()
    }
    pub fn inputs(&self) -> &[Input] {
        &self.inputs
    }
    pub fn digest(&self) -> String {
        self.digest.finalize().to_hex().to_string()
    }
}
