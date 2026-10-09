//! Validated map presentation data; no engine entities or asset paths.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const STATE_KEY: &str = "__map_v1";
pub const MAX_BYTES: usize = 8192;
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapSnapshot {
    #[serde(default)]
    pub settings: MapSettings,
    #[serde(default, deserialize_with = "crate::lua_list::list")]
    pub layers: Vec<MapLayer>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_zoom: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapLayer {
    pub key: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default, deserialize_with = "crate::lua_list::list")]
    pub items: Vec<MapItem>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MapItem {
    Marker {
        key: String,
        position: [f32; 3],
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
        #[serde(default, skip_serializing_if = "MapStyle::is_default")]
        style: MapStyle,
    },
    Label {
        key: String,
        position: [f32; 3],
        text: String,
        #[serde(default, skip_serializing_if = "MapStyle::is_default")]
        style: MapStyle,
    },
    Path {
        key: String,
        #[serde(deserialize_with = "crate::lua_list::list")]
        points: Vec<[f32; 3]>,
        #[serde(default, skip_serializing_if = "MapStyle::is_default")]
        style: MapStyle,
    },
    Region {
        key: String,
        #[serde(deserialize_with = "crate::lua_list::list")]
        points: Vec<[f32; 3]>,
        #[serde(default, skip_serializing_if = "MapStyle::is_default")]
        style: MapStyle,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MapStyle {
    pub color: [f32; 4],
    pub size: f32,
}
impl MapStyle {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
impl Default for MapStyle {
    fn default() -> Self {
        Self {
            color: [1., 0.7, 0.25, 1.],
            size: 4.,
        }
    }
}
impl MapItem {
    pub fn key(&self) -> &str {
        match self {
            Self::Marker { key, .. }
            | Self::Label { key, .. }
            | Self::Path { key, .. }
            | Self::Region { key, .. } => key,
        }
    }
    pub fn style(&self) -> &MapStyle {
        match self {
            Self::Marker { style, .. }
            | Self::Label { style, .. }
            | Self::Path { style, .. }
            | Self::Region { style, .. } => style,
        }
    }
}
fn key_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && !s.chars().any(char::is_control)
}
fn text_ok(s: &str) -> bool {
    s.len() <= 64 && !s.chars().any(char::is_control)
}
fn point_ok(p: &[f32; 3]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= 100_000.)
}
impl MapSnapshot {
    pub fn parse(value: Value) -> Result<Self, String> {
        if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > MAX_BYTES {
            return Err("map state exceeds 8 KiB".into());
        }
        let state: Self =
            serde_json::from_value(value).map_err(|e| format!("invalid map state: {e}"))?;
        validate_snapshot(&state)?;
        Ok(state)
    }
}
pub fn validate_snapshot(state: &MapSnapshot) -> Result<(), String> {
    let fail = || Err("invalid map layer keys, coordinates, style or limits".into());
    if state.layers.len() > 8
        || state
            .settings
            .opacity
            .is_some_and(|v| !v.is_finite() || !(0. ..=1.).contains(&v))
        || state.settings.initial_zoom.is_some_and(|v| v > 2)
        || state.settings.title.as_ref().is_some_and(|v| !text_ok(v))
    {
        return fail();
    }
    let mut layers = BTreeSet::new();
    let mut items = 0usize;
    let mut points = 0usize;
    for layer in &state.layers {
        if !key_ok(&layer.key) || !layers.insert(&layer.key) {
            return fail();
        }
        let mut keys = BTreeSet::new();
        items += layer.items.len();
        if items > 128 {
            return fail();
        }
        for item in &layer.items {
            let style = item.style();
            if !key_ok(item.key())
                || !keys.insert(item.key())
                || !style
                    .color
                    .iter()
                    .all(|v| v.is_finite() && (0. ..=1.).contains(v))
                || !style.size.is_finite()
                || style.size <= 0.
                || style.size > 128.
            {
                return fail();
            }
            match item {
                MapItem::Marker {
                    position, label, ..
                } => {
                    if !point_ok(position) || !text_ok(label) {
                        return fail();
                    }
                }
                MapItem::Label { position, text, .. } => {
                    if !point_ok(position) || !text_ok(text) {
                        return fail();
                    }
                }
                MapItem::Path { points: p, .. } | MapItem::Region { points: p, .. } => {
                    let min = if matches!(item, MapItem::Region { .. }) {
                        3
                    } else {
                        2
                    };
                    points += p.len();
                    if points > 512 || p.len() < min || !p.iter().all(point_ok) {
                        return fail();
                    }
                }
            }
        }
    }
    if serde_json::to_vec(state).map_err(|e| e.to_string())?.len() > MAX_BYTES {
        return Err("map state exceeds 8 KiB".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn sample() -> serde_json::Value {
        json!({"layers":[{"key":"route","items":[
        {"kind":"marker","key":"start","position":[1,2,3],"label":"Start"},
        {"kind":"path","key":"line","points":[[0,0,0],[1,0,1]]}]}]})
    }
    #[test]
    fn map_schema_accepts_vector_layers_and_rejects_invalid_replacements() {
        let v = sample();
        assert!(MapSnapshot::parse(v.clone()).is_ok());
        let mut bad = v.clone();
        bad["layers"][0]["items"][0]["position"] = json!([100001, 0, 0]);
        assert!(MapSnapshot::parse(bad).is_err());
        let mut bad = v.clone();
        bad["layers"][0]["items"][0]["label"] = json!("🙂".repeat(17));
        assert!(MapSnapshot::parse(bad).is_err());
        let mut bad = v.clone();
        bad["layers"][0]["items"][1]["key"] = json!("start");
        assert!(MapSnapshot::parse(bad).is_err());
        let mut bad = v.clone();
        bad["layers"][0]["items"][1]["points"] = json!([[0, 0, 0]]);
        assert!(MapSnapshot::parse(bad).is_err());
        let mut bad = v.clone();
        bad["layers"][0]["items"][0]["image"] = json!("/tmp/a.png");
        assert!(MapSnapshot::parse(bad).is_err());
        let mut bad = v.clone();
        bad["layers"] = json!(
            (0..9)
                .map(|i| json!({"key":i.to_string(),"items":[]}))
                .collect::<Vec<_>>()
        );
        assert!(MapSnapshot::parse(bad).is_err());
        let mut bad = v;
        bad["settings"] = json!({"opacity":1.1});
        assert!(MapSnapshot::parse(bad).is_err());
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn map_schema_enforces_count_and_byte_boundaries() {
        let state = |n| json!({"layers":[{"key":"a","items":(0..n).map(|i|json!({"kind":"marker","key":i.to_string(),"position":[0,0,0]})).collect::<Vec<_>>()}]});
        assert!(MapSnapshot::parse(state(128)).is_ok());
        assert!(MapSnapshot::parse(state(129)).is_err());
        let path = |n| json!({"layers":[{"key":"a","items":[{"kind":"path","key":"p","points":vec![[0,0,0];n]}]}]});
        assert!(MapSnapshot::parse(path(512)).is_ok());
        assert!(MapSnapshot::parse(path(513)).is_err());
        let mut state = MapSnapshot::default();
        state.settings.opacity = Some(f32::NAN);
        assert!(validate_snapshot(&state).is_err());
        let big = json!({"layers":[{"key":"a","items":(0..100).map(|i|json!({"kind":"marker","key":i.to_string(),"position":[0,0,0],"label":"x".repeat(64)})).collect::<Vec<_>>()}]});
        assert!(MapSnapshot::parse(big).is_err());
    }
}
