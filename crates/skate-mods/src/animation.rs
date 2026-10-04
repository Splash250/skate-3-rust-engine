//! Version 1 resource presentation. All transforms are cosmetic bone-local
//! deltas; this contract never changes physics, movement or score authority.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_resident_bytes: usize,
    pub max_banks: usize,
    pub max_layers: usize,
    pub max_attachments: usize,
    pub max_keyframes: usize,
    pub max_events_per_frame: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 4 * 1024 * 1024,
            max_resident_bytes: 16 * 1024 * 1024,
            max_banks: 16,
            max_layers: 64,
            max_attachments: 64,
            max_keyframes: 65536,
            max_events_per_frame: 128,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<(), String> {
        for (value, maximum) in [
            (self.max_bytes, 16 * 1024 * 1024),
            (self.max_resident_bytes, 64 * 1024 * 1024),
            (self.max_banks, 64),
            (self.max_layers, 256),
            (self.max_attachments, 256),
            (self.max_keyframes, 262144),
            (self.max_events_per_frame, 1024),
        ] {
            if value == 0 || value > maximum {
                return Err("animation limits exceed their positive hard bounds".into());
            }
        }
        if self.max_bytes > self.max_resident_bytes {
            return Err("animation bank bytes exceed resident budget".into());
        }
        Ok(())
    }
}
fn one() -> f32 {
    1.
}
fn local() -> String {
    "local".into()
}
fn fade() -> f32 {
    0.15
}
fn identity() -> [f32; 4] {
    [0., 0., 0., 1.]
}
fn unit() -> [f32; 3] {
    [1.; 3]
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Load {
        key: String,
        path: String,
    },
    Unload {
        key: String,
    },
    Play {
        key: String,
        bank: String,
        clip: String,
        #[serde(default = "local")]
        target: String,
        #[serde(default = "one")]
        speed: f32,
        #[serde(default)]
        looped: bool,
        #[serde(default = "fade")]
        fade_in: f32,
        #[serde(default = "fade")]
        fade_out: f32,
        #[serde(default = "one")]
        weight: f32,
        #[serde(default)]
        offset: f32,
    },
    Stop {
        key: String,
        #[serde(default = "fade")]
        fade_out: f32,
    },
    Appearance {
        key: String,
        #[serde(default = "local")]
        target: String,
        path: String,
    },
    Attach {
        key: String,
        #[serde(default = "local")]
        target: String,
        bone: String,
        path: String,
        #[serde(default)]
        translation: [f32; 3],
        #[serde(default = "identity")]
        rotation: [f32; 4],
        #[serde(default = "unit")]
        scale: [f32; 3],
    },
    Remove {
        key: String,
    },
}
pub fn valid_target(target: &str) -> bool {
    target == "local"
        || target
            .parse::<u64>()
            .is_ok_and(|n| n > 0 && n.to_string() == target)
}
fn range(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}
fn valid_path(path: &str, extension: &str) -> bool {
    path.len() <= 256
        && path.ends_with(extension)
        && !path
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':' | '#'))
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}
impl Operation {
    pub fn validate(&self) -> bool {
        use crate::scene::{valid_asset, valid_key, valid_node};
        match self {
            Self::Load { key, path } => valid_key(key) && valid_path(path, ".json"),
            Self::Unload { key } | Self::Remove { key } => valid_key(key),
            Self::Stop { key, fade_out } => valid_key(key) && range(*fade_out, 0., 5.),
            Self::Play {
                key,
                bank,
                clip,
                target,
                speed,
                fade_in,
                fade_out,
                weight,
                offset,
                ..
            } => {
                valid_key(key)
                    && valid_key(bank)
                    && valid_key(clip)
                    && valid_target(target)
                    && range(*speed, 0.05, 4.)
                    && range(*fade_in, 0., 5.)
                    && range(*fade_out, 0., 5.)
                    && range(*weight, 0., 1.)
                    && range(*offset, 0., 600.)
            }
            Self::Appearance { key, target, path } => {
                valid_key(key) && valid_target(target) && !path.is_empty() && valid_asset(path)
            }
            Self::Attach {
                key,
                target,
                bone,
                path,
                translation,
                rotation,
                scale,
            } => {
                valid_key(key)
                    && valid_target(target)
                    && valid_node(bone)
                    && !path.is_empty()
                    && valid_asset(path)
                    && Delta {
                        translation: *translation,
                        rotation: *rotation,
                        scale: *scale,
                    }
                    .validate()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Delta {
    #[serde(default)]
    pub translation: [f32; 3],
    #[serde(default = "identity")]
    pub rotation: [f32; 4],
    #[serde(default = "unit")]
    pub scale: [f32; 3],
}
impl Default for Delta {
    fn default() -> Self {
        Self {
            translation: [0.; 3],
            rotation: identity(),
            scale: unit(),
        }
    }
}
impl Delta {
    pub fn validate(&self) -> bool {
        self.translation.iter().all(|v| range(*v, -2., 2.))
            && self.rotation.iter().all(|v| range(*v, -1., 1.))
            && (self.rotation.iter().map(|v| v * v).sum::<f32>() - 1.).abs() <= 0.001
            && self.scale.iter().all(|v| range(*v, 0.25, 4.))
    }
    pub fn blend(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0., 1.);
        let sign = if self
            .rotation
            .iter()
            .zip(other.rotation)
            .map(|(a, b)| a * b)
            .sum::<f32>()
            < 0.
        {
            -1.
        } else {
            1.
        };
        let mut rotation =
            std::array::from_fn(|i| self.rotation[i] * (1. - t) + other.rotation[i] * sign * t);
        let norm = rotation.iter().map(|v| v * v).sum::<f32>().sqrt();
        for value in &mut rotation {
            *value /= norm.max(f32::MIN_POSITIVE);
        }
        Self {
            translation: std::array::from_fn(|i| {
                self.translation[i] * (1. - t) + other.translation[i] * t
            }),
            rotation,
            scale: std::array::from_fn(|i| self.scale[i] * (1. - t) + other.scale[i] * t),
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    time: f32,
    #[serde(default)]
    translation: [f32; 3],
    #[serde(default = "identity")]
    rotation: [f32; 4],
    #[serde(default = "unit")]
    scale: [f32; 3],
}
impl Frame {
    fn delta(&self) -> Delta {
        Delta {
            translation: self.translation,
            rotation: self.rotation,
            scale: self.scale,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub time: f32,
    pub name: String,
    #[serde(default)]
    pub payload: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bone {
    name: String,
    parent: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackFile {
    bone: String,
    keys: Vec<Frame>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipFile {
    name: String,
    duration: f32,
    tracks: Vec<TrackFile>,
    #[serde(default)]
    markers: Vec<Marker>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    bones: Vec<Bone>,
    clips: Vec<ClipFile>,
}
#[derive(Clone, Debug)]
pub struct Clip {
    pub duration: f32,
    tracks: Vec<(usize, Vec<Frame>)>,
    pub markers: Vec<Marker>,
}
impl Clip {
    pub fn sample(&self, time: f32) -> Vec<(usize, Delta)> {
        let time = if time.is_finite() {
            time.clamp(0., self.duration)
        } else {
            0.
        };
        self.tracks
            .iter()
            .map(|(bone, keys)| {
                let next = keys.partition_point(|key| key.time <= time);
                let value = if next == 0 {
                    keys[0].delta()
                } else if next == keys.len() {
                    keys[next - 1].delta()
                } else {
                    let (a, b) = (&keys[next - 1], &keys[next]);
                    a.delta()
                        .blend(b.delta(), (time - a.time) / (b.time - a.time))
                };
                (*bone, value)
            })
            .collect()
    }
    /// Bounded even after a long pause or extreme caller time interval. Markers
    /// use (previous,current], so adjacent updates do not duplicate events.
    pub fn markers_between(
        &self,
        previous: f32,
        current: f32,
        looped: bool,
        limit: usize,
    ) -> Vec<&Marker> {
        if !previous.is_finite() || !current.is_finite() || current < previous {
            return vec![];
        }
        let mut events = Vec::new();
        let limit = limit.min(1024);
        for marker in &self.markers {
            if !looped {
                if marker.time > previous && marker.time <= current {
                    events.push((marker.time, marker));
                }
            } else {
                let first = ((previous - marker.time) / self.duration).floor().max(-1.) + 1.;
                for index in 0..limit {
                    let time = marker.time + (first + index as f32) * self.duration;
                    if time > current {
                        break;
                    }
                    if time > previous {
                        events.push((time, marker));
                    }
                }
            }
        }
        events.sort_by(|a, b| a.0.total_cmp(&b.0));
        events
            .into_iter()
            .take(limit)
            .map(|(_, marker)| marker)
            .collect()
    }
}
#[derive(Clone, Debug)]
pub struct Bank {
    pub clips: BTreeMap<String, Clip>,
    pub bytes: usize,
    pub keyframes: usize,
}
impl Bank {
    pub fn parse(
        bytes: &[u8],
        names: &[String],
        parents: &[i32],
        limits: &Limits,
    ) -> Result<Self, String> {
        limits.validate()?;
        if bytes.len() > limits.max_bytes {
            return Err("animation bank exceeds byte limit".into());
        }
        if names.is_empty() || names.len() > 1024 || names.len() != parents.len() {
            return Err("invalid host animation rig".into());
        }
        let lookup: BTreeMap<_, _> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.to_ascii_lowercase(), i))
            .collect();
        if lookup.len() != names.len() {
            return Err("ambiguous host animation bone names".into());
        }
        let file: File =
            serde_json::from_slice(bytes).map_err(|e| format!("animation JSON: {e}"))?;
        if file.version != 1
            || file.bones.is_empty()
            || file.bones.len() > 256
            || file.clips.is_empty()
            || file.clips.len() > 64
        {
            return Err("unsupported animation version or bone/clip count".into());
        }
        let mut declared = BTreeSet::new();
        for bone in file.bones {
            let name = bone.name.to_ascii_lowercase();
            let index = *lookup
                .get(&name)
                .ok_or_else(|| format!("animation bone {} is not in the rig", bone.name))?;
            if !declared.insert(name) {
                return Err("duplicate animation bone".into());
            }
            let parent = if parents[index] < 0 {
                None
            } else {
                names
                    .get(parents[index] as usize)
                    .map(|n| n.to_ascii_lowercase())
            };
            if parent != bone.parent.map(|p| p.to_ascii_lowercase()) {
                return Err(format!("animation parent mismatch for {}", bone.name));
            }
        }
        let mut bank = Self {
            clips: BTreeMap::new(),
            bytes: bytes.len(),
            keyframes: 0,
        };
        for clip in file.clips {
            if !crate::scene::valid_key(&clip.name)
                || !range(clip.duration, 0.001, 600.)
                || clip.tracks.is_empty()
                || clip.tracks.len() > 256
                || clip.markers.len() > 256
                || bank.clips.contains_key(&clip.name)
            {
                return Err("invalid/duplicate animation clip or track/marker count".into());
            }
            let mut tracks = Vec::new();
            let mut seen = BTreeSet::new();
            for track in clip.tracks {
                let bone = track.bone.to_ascii_lowercase();
                if !declared.contains(&bone) || !seen.insert(bone.clone()) || track.keys.is_empty()
                {
                    return Err("unknown/duplicate animation track or empty keyframes".into());
                }
                bank.keyframes = bank
                    .keyframes
                    .checked_add(track.keys.len())
                    .ok_or("animation keyframe overflow")?;
                if bank.keyframes > limits.max_keyframes {
                    return Err("animation keyframe budget exceeded".into());
                }
                let mut previous = -1.;
                for frame in &track.keys {
                    if !range(frame.time, 0., clip.duration)
                        || frame.time <= previous
                        || !frame.delta().validate()
                    {
                        return Err("animation keyframe has invalid time or local transform".into());
                    }
                    previous = frame.time;
                }
                tracks.push((lookup[&bone], track.keys));
            }
            let mut previous = -1.;
            let mut seen = BTreeSet::new();
            for marker in &clip.markers {
                if !range(marker.time, 0., clip.duration)
                    || marker.time < previous
                    || !crate::scene::valid_key(&marker.name)
                    || !seen.insert((marker.time.to_bits(), marker.name.clone()))
                    || serde_json::to_vec(&marker.payload).map_or(true, |b| b.len() > 1024)
                {
                    return Err("invalid animation marker".into());
                }
                previous = marker.time;
            }
            bank.clips.insert(
                clip.name,
                Clip {
                    duration: clip.duration,
                    tracks,
                    markers: clip.markers,
                },
            );
        }
        Ok(bank)
    }
}
