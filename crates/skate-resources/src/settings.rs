//! Data-only resource settings contracts. Values belong to the declaring resource.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SETTINGS: usize = 64;
pub const MAX_SETTINGS_BYTES: usize = 32 * 1024;
pub const ENGINE_FEATURES: &[&str] = &[
    "resource.settings.v1",
    "resource.profiling.v1",
    "resource.teleport_leases.v1",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingVisibility {
    #[default]
    Private,
    Replicated,
    Public,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingChange {
    #[default]
    Live,
    Restart,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingType {
    Boolean,
    Integer,
    Number,
    String,
    Enum,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingDefinition {
    #[serde(rename = "type")]
    pub kind: SettingType,
    pub default: Value,
    #[serde(default)]
    pub visibility: SettingVisibility,
    #[serde(default)]
    pub change: SettingChange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Number>,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}
fn default_max_bytes() -> usize {
    256
}
pub fn validate_setting_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 64
        || !key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(Error(
            "setting keys require 1..64 lowercase letters, digits or underscores".into(),
        ));
    }
    Ok(())
}
impl SettingDefinition {
    pub fn validate(&self) -> Result<()> {
        if !(1..=4096).contains(&self.max_bytes) {
            return Err(Error("setting max_bytes must be 1..4096".into()));
        }
        let numeric = matches!(self.kind, SettingType::Number | SettingType::Integer);
        if !numeric && (self.min.is_some() || self.max.is_some()) {
            return Err(Error("numeric bounds require a numeric setting".into()));
        }
        if self.kind != SettingType::Enum && !self.options.is_empty() {
            return Err(Error("options require an enum setting".into()));
        }
        if self.kind == SettingType::Enum
            && (self.options.is_empty()
                || self.options.len() > 64
                || self.options.iter().any(|s| s.len() > self.max_bytes)
                || self.options.iter().collect::<BTreeSet<_>>().len() != self.options.len())
        {
            return Err(Error(
                "enum options require 1..64 distinct bounded strings".into(),
            ));
        }
        // Integers remain exactly representable in every supported runtime.
        for bound in [&self.min, &self.max].into_iter().flatten() {
            let n = bound
                .as_f64()
                .ok_or_else(|| Error("invalid numeric bound".into()))?;
            if !n.is_finite()
                || (self.kind == SettingType::Integer
                    && (n.fract() != 0.0 || n.abs() > 9_007_199_254_740_991.0))
            {
                return Err(Error("invalid numeric bound".into()));
            }
        }
        if let (Some(min), Some(max)) = (&self.min, &self.max) {
            if min.as_f64() > max.as_f64() {
                return Err(Error("setting minimum exceeds maximum".into()));
            }
        }
        self.validate_value(&self.default)
    }
    pub fn validate_value(&self, value: &Value) -> Result<()> {
        let valid = match self.kind {
            SettingType::Boolean => value.is_boolean(),
            SettingType::String => value.as_str().is_some_and(|s| s.len() <= self.max_bytes),
            SettingType::Enum => value
                .as_str()
                .is_some_and(|s| self.options.iter().any(|o| o == s)),
            SettingType::Number | SettingType::Integer => value.as_f64().is_some_and(|n| {
                n.is_finite()
                    && (self.kind != SettingType::Integer
                        || (n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0))
                    && self
                        .min
                        .as_ref()
                        .and_then(Number::as_f64)
                        .is_none_or(|min| n >= min)
                    && self
                        .max
                        .as_ref()
                        .and_then(Number::as_f64)
                        .is_none_or(|max| n <= max)
            }),
        };
        if !valid {
            return Err(Error(
                "setting value violates its declared type or bounds".into(),
            ));
        }
        Ok(())
    }
}
pub fn validate_settings(definitions: &BTreeMap<String, SettingDefinition>) -> Result<()> {
    if definitions.len() > MAX_SETTINGS
        || serde_json::to_vec(definitions)?.len() > MAX_SETTINGS_BYTES
    {
        return Err(Error("settings schema exceeds 64 keys or 32 KiB".into()));
    }
    for (key, definition) in definitions {
        validate_setting_key(key)?;
        definition
            .validate()
            .map_err(|e| Error(format!("setting {key}: {e}")))?;
    }
    let defaults = definitions
        .iter()
        .map(|(key, definition)| (key.clone(), definition.default.clone()))
        .collect();
    validate_setting_values(definitions, &defaults)
}
pub fn validate_setting_values(
    definitions: &BTreeMap<String, SettingDefinition>,
    values: &BTreeMap<String, Value>,
) -> Result<()> {
    if serde_json::to_vec(values)?.len() > 8192 {
        return Err(Error("setting values exceed 8 KiB".into()));
    }
    for (key, value) in values {
        let definition = definitions
            .get(key)
            .ok_or_else(|| Error(format!("unknown setting {key}")))?;
        definition
            .validate_value(value)
            .map_err(|e| Error(format!("setting {key}: {e}")))?;
    }
    Ok(())
}
