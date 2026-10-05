//! Versioned resource interface descriptors and bounded ownership registry.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<u16>,
    #[serde(default)]
    pub hold_ms: u16,
}
impl Binding {
    pub fn validate(&self) -> bool {
        matches!(
            self.key.as_str(),
            "F2" | "F3" | "F4" | "F7" | "F8" | "KeyI" | "KeyP" | "KeyO"
        ) && self
            .button
            .is_none_or(|b| matches!(b, 0x20 | 0x40 | 0x80 | 0x8000))
            && self.hold_ms <= 2000
    }
    pub fn conflicts(&self, other: &Self) -> bool {
        self.key == other.key || self.button.is_some_and(|b| other.button == Some(b))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub version: u32,
    pub label: String,
    pub icon: String,
    pub category: String,
    pub destination: String,
    #[serde(default)]
    pub quick: bool,
    #[serde(default)]
    pub phone: bool,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub disabled_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<Binding>,
}
fn label(s: &str, n: usize) -> bool {
    !s.trim().is_empty() && s.len() <= n && !s.chars().any(char::is_control)
}
pub fn full_id(s: &str) -> bool {
    s.split_once('/')
        .is_some_and(|(a, b)| crate::schema::valid_id(a) && crate::schema::valid_id(b))
}
impl Descriptor {
    pub fn validate(&self) -> bool {
        self.version == 1
            && label(&self.label, 96)
            && label(&self.icon, 32)
            && label(&self.category, 32)
            && matches!(
                self.destination.as_str(),
                "phone" | "dashboard" | "native" | "action"
            )
            && self.permissions.len() <= 8
            && self.permissions.iter().all(|p| label(p, 64))
            && self.disabled_reason.len() <= 256
            && !self.disabled_reason.chars().any(char::is_control)
            && self.binding.as_ref().is_none_or(Binding::validate)
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub id: String,
    pub owner: String,
    pub generation: String,
    #[serde(flatten)]
    pub descriptor: Descriptor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_conflict: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Register { key: String, descriptor: Descriptor },
    Remove { key: String },
    List,
    Invoke { id: String, generation: String },
    Bind { id: String, binding: Binding },
}
impl Operation {
    pub fn validate(&self) -> bool {
        match self {
            Self::Register { key, descriptor } => {
                crate::schema::valid_id(key) && descriptor.validate()
            }
            Self::Remove { key } => crate::schema::valid_id(key),
            Self::List => true,
            Self::Invoke { id, generation } => {
                full_id(id)
                    && generation
                        .parse::<u64>()
                        .is_ok_and(|g| g > 0 && g.to_string() == *generation)
            }
            Self::Bind { id, binding } => full_id(id) && binding.validate(),
        }
    }
}
#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<String, Entry>,
}
impl Registry {
    pub fn entries(&self) -> Vec<Entry> {
        self.entries.values().cloned().collect()
    }
    pub fn register(
        &mut self,
        owner: &str,
        generation: u64,
        key: &str,
        descriptor: Descriptor,
    ) -> Result<(), String> {
        if !crate::schema::valid_id(owner)
            || !crate::schema::valid_id(key)
            || generation == 0
            || !descriptor.validate()
        {
            return Err("invalid interface descriptor".into());
        }
        let id = format!("{owner}/{key}");
        if self.entries.contains_key(&id) {
            return Err("interface already registered; remove before replacing".into());
        }
        if self.entries.len() >= 128
            || self.entries.values().filter(|e| e.owner == owner).count() >= 32
        {
            return Err("interface limit: 32 per resource, 128 total".into());
        }
        let entry = Entry {
            id: id.clone(),
            owner: owner.into(),
            generation: generation.to_string(),
            descriptor,
            binding_conflict: None,
        };
        // Keep the complete registry export safely below the default 16 KiB
        // resource message boundary, including consumer envelope and local bindings.
        let mut exported = self.entries();
        exported.push(entry.clone());
        if serde_json::to_vec(&exported)
            .map_err(|e| e.to_string())?
            .len()
            + exported.len() * 128
            > 12 * 1024
        {
            return Err("interface metadata budget: 12 KiB total".into());
        }
        self.entries.insert(id, entry);
        Ok(())
    }
    pub fn get(&self, id: &str, generation: &str) -> Result<Entry, String> {
        self.entries
            .get(id)
            .filter(|e| e.generation == generation)
            .cloned()
            .ok_or_else(|| "interface generation is no longer available".into())
    }
    pub fn remove(&mut self, owner: &str, key: &str) {
        self.entries.remove(&format!("{owner}/{key}"));
    }
    pub fn retire(&mut self, owner: &str, generation: u64) {
        self.entries
            .retain(|_, e| e.owner != owner || e.generation != generation.to_string());
    }
    pub fn retire_owner(&mut self, owner: &str) {
        self.entries.retain(|_, e| e.owner != owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor() -> Descriptor {
        serde_json::from_value(serde_json::json!({"version":1,"label":"Inventory","icon":"bag","category":"Player","destination":"dashboard","quick":true})).unwrap()
    }
    #[test]
    fn registry_owns_identity_rejects_duplicate_and_retires_only_generation() {
        let mut r = Registry::default();
        r.register("inventory", 1, "open", descriptor()).unwrap();
        assert!(r.register("inventory", 1, "open", descriptor()).is_err());
        assert!(r.get("inventory/open", "2").is_err());
        r.retire("inventory", 2);
        assert_eq!(r.entries().len(), 1);
        r.retire("inventory", 1);
        assert!(r.entries().is_empty());
    }
    #[test]
    fn registry_bounds_metadata_and_owned_slots() {
        let mut r = Registry::default();
        for i in 0..32 {
            r.register("app", 1, &format!("a{i}"), descriptor())
                .unwrap();
        }
        assert!(r.register("app", 1, "overflow", descriptor()).is_err());
        let mut d = descriptor();
        d.label = "x".repeat(97);
        assert!(!d.validate());
        d = descriptor();
        d.version = 2;
        assert!(!d.validate());
        d = descriptor();
        d.binding = Some(Binding {
            key: "Escape".into(),
            button: Some(0x10),
            hold_ms: 0,
        });
        assert!(!d.validate());
        assert!(r.register("app", 1, "../escape", descriptor()).is_err());
    }
    #[test]
    fn oversized_registry_is_rejected_before_it_can_break_consumer_exports() {
        let mut r = Registry::default();
        let mut d = descriptor();
        d.disabled_reason = "x".repeat(256);
        d.permissions = vec!["x".repeat(64); 8];
        let mut rejected = false;
        for i in 0..32 {
            if r.register("app", 1, &format!("a{i}"), d.clone()).is_err() {
                rejected = true;
                break;
            }
        }
        assert!(rejected);
        assert!(serde_json::to_vec(&r.entries()).unwrap().len() <= 12 * 1024);
        assert!(!r.entries().is_empty());
    }
    #[test]
    fn bindings_reject_reserved_or_ambiguous_physical_inputs() {
        assert!(
            Binding {
                key: "F2".into(),
                button: Some(0x20),
                hold_ms: 600
            }
            .validate()
        );
        for key in ["F9", "F10"] {
            assert!(
                !Binding {
                    key: key.into(),
                    button: None,
                    hold_ms: 0
                }
                .validate(),
                "profiler shortcuts remain reserved"
            );
        }
        for button in [0, 3, 0x10, 0x400] {
            assert!(
                !Binding {
                    key: "F2".into(),
                    button: Some(button),
                    hold_ms: 0
                }
                .validate()
            );
        }
        assert!(
            !Binding {
                key: "KeyV".into(),
                button: None,
                hold_ms: 0
            }
            .validate()
        );
    }
}
