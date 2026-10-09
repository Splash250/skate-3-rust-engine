//! Bounded, immutable location geometry and separately mutable presentation.
//! Positions are world-space metres; transforms are column-major affine matrices.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const MAX_CATALOG_BYTES: usize = 128 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 8 * 1024;
pub const MAX_LOCATIONS: usize = 32;
pub const MAX_INTERIORS: usize = 16;
pub const MAX_FLOORS: usize = 8;
pub const PUBLICATIONS_PER_SECOND: usize = 5;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerStyle {
    pub color: [f32; 3],
    pub opacity: f32,
    pub radius: f32,
    pub height: f32,
}
impl Default for MarkerStyle {
    fn default() -> Self {
        Self {
            color: [1., 0.8, 0.15],
            opacity: 0.35,
            radius: 1.,
            height: 2.,
        }
    }
}
impl MarkerStyle {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .color
            .iter()
            .chain([&self.opacity])
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || !self.radius.is_finite()
            || !(0.1..=10.).contains(&self.radius)
            || !self.height.is_finite()
            || !(0.1..=20.).contains(&self.height)
        {
            return Err("invalid location marker style".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exit {
    pub position: [f32; 3],
    pub style: MarkerStyle,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interior {
    pub key: String,
    pub model: String,
    pub collision: String,
    pub transform: [[f32; 4]; 4],
    pub spawn: [f32; 3],
    pub heading: f32,
    pub exit: Exit,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Floor {
    pub key: String,
    pub label: String,
    pub interior: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub key: String,
    pub label: String,
    pub position: [f32; 3],
    pub return_position: [f32; 3],
    pub return_heading: f32,
    pub style: MarkerStyle,
    pub floors: Vec<Floor>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u32,
    pub map: String,
    pub interiors: Vec<Interior>,
    pub locations: Vec<Location>,
}
fn key(s: &str) -> Result<(), String> {
    crate::validate_id(s).map_err(|e| e.to_string())
}
fn label(s: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > 64 || s.chars().any(char::is_control) {
        Err("location label must contain 1..64 bytes without controls".into())
    } else {
        Ok(())
    }
}
fn point(p: [f32; 3]) -> Result<(), String> {
    if p.iter().any(|v| !v.is_finite() || v.abs() > 100_000.) {
        Err("invalid location coordinate".into())
    } else {
        Ok(())
    }
}
fn heading(h: f32) -> Result<(), String> {
    if !h.is_finite() {
        Err("nonfinite location heading".into())
    } else {
        Ok(())
    }
}
pub fn generation(s: &str) -> Result<u64, String> {
    let n = s
        .parse::<u64>()
        .map_err(|_| "invalid location generation")?;
    if n == 0 || n.to_string() != s {
        Err("noncanonical location generation".into())
    } else {
        Ok(n)
    }
}
impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err("location catalog exceeds byte limit".into());
        }
        let c: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<(), String> {
        key(&self.map)?;
        if self.version != 1
            || self.interiors.len() > MAX_INTERIORS
            || self.locations.len() > MAX_LOCATIONS
        {
            return Err("unsupported location catalog version/count".into());
        }
        let mut interiors = BTreeSet::new();
        for i in &self.interiors {
            key(&i.key)?;
            if !interiors.insert(&i.key) {
                return Err("duplicate interior key".into());
            }
            crate::validate_path(&i.model).map_err(|e| e.to_string())?;
            crate::validate_path(&i.collision).map_err(|e| e.to_string())?;
            if !i.model.ends_with(".glb") || !i.collision.ends_with(".json") {
                return Err("interior requires GLB model and JSON collision".into());
            }
            let m = i.transform;
            if m.iter().flatten().any(|v| !v.is_finite())
                || m[0][3] != 0.
                || m[1][3] != 0.
                || m[2][3] != 0.
                || m[3][3] != 1.
            {
                return Err("interior transform must be finite affine".into());
            }
            let determinant = m[0][0] as f64
                * (m[1][1] as f64 * m[2][2] as f64 - m[1][2] as f64 * m[2][1] as f64)
                - m[1][0] as f64
                    * (m[0][1] as f64 * m[2][2] as f64 - m[0][2] as f64 * m[2][1] as f64)
                + m[2][0] as f64
                    * (m[0][1] as f64 * m[1][2] as f64 - m[0][2] as f64 * m[1][1] as f64);
            if determinant.abs() < 1e-12 {
                return Err("singular interior transform".into());
            }
            point(i.spawn)?;
            heading(i.heading)?;
            point(i.exit.position)?;
            i.exit.style.validate()?;
        }
        let mut locations = BTreeSet::new();
        for l in &self.locations {
            key(&l.key)?;
            label(&l.label)?;
            point(l.position)?;
            point(l.return_position)?;
            heading(l.return_heading)?;
            l.style.validate()?;
            if !locations.insert(&l.key) || l.floors.is_empty() || l.floors.len() > MAX_FLOORS {
                return Err("duplicate location or invalid floor count".into());
            }
            let mut floors = BTreeSet::new();
            for f in &l.floors {
                key(&f.key)?;
                label(&f.label)?;
                if !floors.insert(&f.key) || !interiors.contains(&f.interior) {
                    return Err("duplicate floor or missing interior".into());
                }
            }
        }
        Ok(())
    }
    pub fn validate_files(&self, files: &[String]) -> Result<(), String> {
        for i in &self.interiors {
            for path in [&i.model, &i.collision] {
                if !files.contains(path) {
                    return Err(format!("location asset is not public: {path}"));
                }
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationSetting {
    pub key: String,
    pub label: String,
    pub enabled: bool,
    pub style: MarkerStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationSnapshot {
    pub generation: String,
    pub locations: Vec<LocationSetting>,
}
impl LocationSnapshot {
    /// Mutable presentation cannot reference another owner or hide its safe return.
    pub fn validate_catalog(&self, catalog: &Catalog, owner_generation: u64) -> Result<(), String> {
        Self::parse(serde_json::to_value(self).map_err(|e| e.to_string())?)?;
        if self.generation != owner_generation.to_string() {
            return Err("Stale location owner generation".into());
        }
        for setting in &self.locations {
            let entry = catalog
                .locations
                .iter()
                .find(|l| l.key == setting.key)
                .ok_or("Unknown owner location")?;
            let distance = (entry.position[0] - entry.return_position[0])
                .hypot(entry.position[2] - entry.return_position[2]);
            if setting.style.radius + 0.75 > distance {
                return Err("Marker radius covers its safe return".into());
            }
        }
        Ok(())
    }

    pub fn parse(value: serde_json::Value) -> Result<Self, String> {
        if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > MAX_SNAPSHOT_BYTES {
            return Err("location snapshot exceeds byte limit".into());
        }
        let s: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        generation(&s.generation)?;
        if s.locations.len() > MAX_LOCATIONS {
            return Err("too many locations".into());
        }
        let mut seen = BTreeSet::new();
        for l in &s.locations {
            key(&l.key)?;
            label(&l.label)?;
            l.style.validate()?;
            if !seen.insert(&l.key) {
                return Err("duplicate location settings".into());
            }
            if let Some(event) = &l.interaction {
                key(event)?;
            }
        }
        Ok(s)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryIntent {
    pub location: String,
    pub floor: String,
    pub generation: String,
    pub request: String,
}
impl EntryIntent {
    pub fn validate(&self) -> Result<(), String> {
        key(&self.location)?;
        key(&self.floor)?;
        generation(&self.generation)?;
        generation(&self.request)?;
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationStatus {
    Exterior,
    Preparing,
    AwaitingApproval,
    Interior,
    Returning,
}

/// Stable identity covers geometry, presentation, spawns and every asset byte.
/// No catalog keeps legacy world identities exactly unchanged.
pub fn catalog_revision(
    catalog: &Catalog,
    files: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<String, String> {
    catalog.validate()?;
    let mut canonical = catalog.clone();
    canonical.interiors.sort_by(|a, b| a.key.cmp(&b.key));
    canonical.locations.sort_by(|a, b| a.key.cmp(&b.key));
    let mut digests = std::collections::BTreeMap::new();
    for i in &canonical.interiors {
        for path in [&i.model, &i.collision] {
            let bytes = files
                .get(path)
                .ok_or_else(|| format!("missing location content: {path}"))?;
            let max = if path == &i.model {
                16 * 1024 * 1024
            } else {
                4 * 1024 * 1024
            };
            if bytes.len() > max {
                return Err(format!("location asset exceeds budget: {path}"));
            }
            digests.insert(path, crate::digest_bytes(bytes));
        }
    }
    Ok(crate::digest_bytes(
        &serde_json::to_vec(&("skate-locations-v1", canonical.clone(), digests))
            .map_err(|e| e.to_string())?,
    ))
}
pub fn world_fingerprint(base: u64, revision: Option<&str>) -> u64 {
    let Some(revision) = revision else {
        return base;
    };
    let mut hash = blake3::Hasher::new();
    hash.update(b"skate-location-world-v1");
    hash.update(&base.to_le_bytes());
    hash.update(revision.as_bytes());
    u64::from_le_bytes(hash.finalize().as_bytes()[..8].try_into().unwrap())
}
#[derive(Debug, Clone)]
pub struct PreparedCatalog {
    pub catalog: Catalog,
    pub files: std::collections::BTreeMap<String, Vec<u8>>,
    pub revision: String,
}
impl PreparedCatalog {
    pub fn read(root: &std::path::Path, path: &str) -> Result<Self, String> {
        let load = |path: &str, max: u64| {
            crate::manifest::checked_file(root, path, false)
                .and_then(|p| crate::manifest::read_bounded(&p, max))
                .map_err(|e| e.to_string())
        };
        let catalog = Catalog::parse(&load(path, MAX_CATALOG_BYTES as u64)?)?;
        let mut files = std::collections::BTreeMap::new();
        for i in &catalog.interiors {
            for (p, max) in [
                (&i.model, 16 * 1024 * 1024),
                (&i.collision, 4 * 1024 * 1024),
            ] {
                if !files.contains_key(p) {
                    files.insert(p.clone(), load(p, max)?);
                }
            }
        }
        let revision = catalog_revision(&catalog, &files)?;
        Ok(Self {
            catalog,
            files,
            revision,
        })
    }
}

impl PreparedCatalog {
    /// A map's optional local package lives beside it in locations/<map-stem>.
    /// An explicitly selected package is required; malformed packages never silently disappear.
    pub fn discover(
        map: Option<&std::path::Path>,
        explicit: Option<&std::path::Path>,
    ) -> Result<Option<Self>, String> {
        let name = map
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("test-world")
            .to_ascii_lowercase();
        key(&name)?;
        let directory = match explicit {
            Some(path) => path.to_path_buf(),
            None => {
                let Some(parent) = map.and_then(|p| p.parent()) else {
                    return Ok(None);
                };
                let path = parent.join("locations").join(&name);
                match path.join("catalog.json").try_exists() {
                    Ok(false) => return Ok(None),
                    Ok(true) => {}
                    Err(e) => return Err(e.to_string()),
                }
                path
            }
        };
        let prepared = Self::read(&directory, "catalog.json")?;
        if !prepared.catalog.map.eq_ignore_ascii_case(&name) {
            return Err("Interior catalog belongs to a different map".into());
        }
        Ok(Some(prepared))
    }

    pub fn from_resource(
        resource: &crate::Resource,
        blobs: &std::collections::BTreeMap<String, Vec<u8>>,
    ) -> Result<Option<Self>, String> {
        let Some(path) = &resource.manifest.locations else {
            return Ok(None);
        };
        let bytes = |p: &str| {
            resource
                .files
                .get(p)
                .and_then(|f| blobs.get(&f.digest))
                .ok_or_else(|| format!("Missing admitted interior asset: {p}"))
        };
        let catalog = Catalog::parse(bytes(path)?)?;
        let mut files = std::collections::BTreeMap::new();
        for interior in &catalog.interiors {
            for p in [&interior.model, &interior.collision] {
                files.insert(p.clone(), bytes(p)?.clone());
            }
        }
        let revision = catalog_revision(&catalog, &files)?;
        Ok(Some(Self {
            catalog,
            files,
            revision,
        }))
    }
}
