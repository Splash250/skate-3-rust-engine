//! Bounded voice framing, server routing and optional audio-device workers.
//! Game transports carry these packets after their normal authentication step.
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};

pub const SAMPLE_RATE: u32 = 48_000;
pub const FRAME_SAMPLES: usize = 960;
pub const MAX_ENCODED: usize = 400;
pub const VOICE: u8 = 60;
pub const CONTROL: u8 = 61;
pub const MAX_TALKERS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerCommand {
    Channel { name: String, members: Vec<String> },
    RemoveChannel { name: String },
    Mute { player: String, muted: bool },
    Proximity { meters: f32 },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientCommand {
    Configure {
        #[serde(default)]
        input_device: Option<String>,
        #[serde(default)]
        output_device: Option<String>,
        muted: bool,
        deafened: bool,
    },
    Transmit {
        pressed: bool,
        #[serde(default)]
        channel: Option<String>,
    },
    Devices {},
}
pub type Result<T> = std::result::Result<T, String>;

mod router;
pub mod wire;
pub use router::{Metrics, Player, Router};
#[cfg(feature = "codec")]
mod codec;
#[cfg(feature = "codec")]
pub use codec::{CaptureResampler, Encoder, Mixer, Pcm};
#[cfg(feature = "devices")]
mod devices;
#[cfg(feature = "devices")]
pub use devices::{AudioEngine, AudioStats, DeviceConfig, DeviceEvent, Encoded};
mod client;
pub use client::{ClientState, Context, Incoming};

impl ServerCommand {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Channel { name, members } => {
                if !valid_name(name) || members.len() > 64 {
                    return Err("radio channel requires a valid name and at most64 members".into());
                }
                for member in members {
                    valid_actor(member)?;
                }
            }
            Self::RemoveChannel { name } => {
                if !valid_name(name) {
                    return Err("invalid radio channel name".into());
                }
            }
            Self::Mute { player, .. } => {
                valid_actor(player)?;
            }
            Self::Proximity { meters } => {
                if !meters.is_finite() || !(1.0..=100.).contains(meters) {
                    return Err("voice proximity requires1..100 meters".into());
                }
            }
        }
        Ok(())
    }
}
impl ClientCommand {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Configure {
                input_device,
                output_device,
                ..
            } => {
                for name in [input_device, output_device].into_iter().flatten() {
                    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                        return Err("invalid voice device identifier (maximum256 bytes)".into());
                    }
                }
            }
            Self::Transmit {
                channel: Some(channel),
                ..
            } if !channel.is_empty() => {
                if !wire::valid_channel(channel) {
                    return Err("voice channel must be resource/name".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn valid_actor(actor: &str) -> Result<u64> {
    actor
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && id.to_string() == actor)
        .ok_or_else(|| "player must be a canonical nonzero decimal actor ID".into())
}
