use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Canonical, data-only API-1 manifest. Explicit paths are portable lower-case ASCII.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub api: u32,
    pub id: String,
    pub version: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<World>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locations: Option<String>,
    #[serde(default)]
    pub client_scripts: Vec<String>,
    #[serde(default)]
    pub server_scripts: Vec<String>,
    #[serde(default)]
    pub shared_scripts: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default, deserialize_with = "unique_map")]
    pub dependencies: BTreeMap<String, String>,
    #[serde(default)]
    pub exports: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", deserialize_with = "unique_map")]
    pub settings: BTreeMap<String, crate::SettingDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_features: Vec<String>,
}
/// One required server-selected world package. Geometry and all referenced
/// textures are embedded in the validated SKATE file; resource dependencies
/// retain their normal dependency-order activation semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    pub map: String,
    pub required: bool,
    #[serde(default = "world_decoded_budget")]
    pub max_decoded_bytes: u64,
    #[serde(default,skip_serializing_if="Vec::is_empty")]
    pub lods:Vec<WorldLod>,
}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldLod {pub map:String,pub distance:u32}
fn world_decoded_budget() -> u64 { 256 * 1024 * 1024 }
impl Manifest {
    pub fn read(root: &Path) -> Result<Self> {
        let path = checked_file(root, "resource.json", true)?;
        let bytes = read_bounded(&path, MAX_MANIFEST_BYTES as u64)?;
        let manifest: Self = serde_json::from_slice(&bytes)?;
        manifest.validate()?;
        Ok(manifest)
    }
    pub fn validate(&self) -> Result<()> {
        if self.format != 1 || self.api != 1 || !matches!(self.language.as_str(), "lua" | "javascript" | "csharp") {
            return Err(Error(format!(
                "{}: unsupported format/API/language ({}/{}/{})",
                self.id, self.format, self.api, self.language
            )));
        }
        crate::validate_settings(&self.settings)?;
        if self.requires_features.len() > 32 || self.requires_features.iter().collect::<BTreeSet<_>>().len() != self.requires_features.len() { return Err(Error("too many or duplicate engine feature constraints".into())); }
        for feature in &self.requires_features { if !crate::ENGINE_FEATURES.contains(&feature.as_str()) { return Err(Error(format!("{}: required engine feature unavailable: {feature}", self.id))); } }
        validate_id(&self.id)?;
        validate_version(&self.version)?;
        if self.dependencies.len() > 64 || self.exports.len() > 128 || self.capabilities.len() > 64
        {
            return Err(Error(format!(
                "{}: too many dependencies/exports/capabilities",
                self.id
            )));
        }
        let mut paths = BTreeSet::new();
        for (scripts, list) in [
            (true, &self.client_scripts),
            (true, &self.server_scripts),
            (true, &self.shared_scripts),
            (false, &self.files),
        ] {
            for path in list {
                validate_path(path)?;
                let extension=match self.language.as_str() { "javascript"=>".js", "csharp"=>".cs", _=>".lua" };
                if scripts && !path.ends_with(extension) {
                    return Err(Error(format!(
                        "{}: script must be a {extension} file: {path}",
                        self.id
                    )));
                }
                if !paths.insert(path.clone()) {
                    return Err(Error(format!(
                        "{}: duplicate or public/private overlapping path: {path}",
                        self.id
                    )));
                }
            }
        }
        if paths.len() > 2048 {
            return Err(Error(format!("{}: too many files", self.id)));
        }
        if let Some(world) = &self.world {
            validate_path(&world.map)?;
            if !world.required || !world.map.ends_with(".skate") || !self.files.contains(&world.map)
                || !(1024 * 1024..=512 * 1024 * 1024).contains(&world.max_decoded_bytes) {
                return Err(Error(format!("{}: world requires a public .skate file, required=true, and a 1..512 MiB decoded budget", self.id)));
            }
            let mut distance=0;let mut maps=BTreeSet::from([world.map.as_str()]);
            if world.lods.len()>4 {return Err(Error("A world may declare at most four authored render LODs".into()));}
            for lod in &world.lods {
                validate_path(&lod.map)?;
                if !lod.map.ends_with(".skate") || !self.files.contains(&lod.map) || !maps.insert(&lod.map)
                    || !(10..=10000).contains(&lod.distance) || lod.distance<=distance {
                    return Err(Error("World LODs need distinct public .skate files and increasing distances10..10000m".into()));
                }
                distance=lod.distance;
            }
        }
        if let Some(path) = &self.locations {
            validate_path(path)?;
            if !path.ends_with(".json") || !self.files.contains(path) {
                return Err(Error("locations requires a public JSON catalog".into()));
            }
        }
        // A file may not also be another file's directory (portable materialization).
        for path in &paths {
            for (i, _) in path.match_indices('/') {
                if paths.contains(&path[..i]) {
                    return Err(Error(format!("file/directory conflict: {path}")));
                }
            }
        }
        for (id, version) in &self.dependencies {
            validate_id(id)?;
            validate_version(version)?;
            if id == &self.id {
                return Err(Error(format!("dependency cycle: {id} -> {id}")));
            }
        }
        for (kind, list) in [
            ("export", &self.exports),
            ("capability", &self.capabilities),
        ] {
            let mut seen = BTreeSet::new();
            for name in list {
                if name.is_empty()
                    || name.len() > 96
                    || !name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
                    || !seen.insert(name)
                {
                    return Err(Error(format!("invalid or duplicate {kind}: {name}")));
                }
            }
        }
        Ok(())
    }
    pub fn client_projection(&self) -> Self {
        let mut m = self.clone();
        m.server_scripts.clear();
        m.settings.retain(|_, setting| setting.visibility != crate::SettingVisibility::Private);
        m
    }
    pub fn public_paths(&self) -> impl Iterator<Item = &String> {
        self.shared_scripts
            .iter()
            .chain(&self.client_scripts)
            .chain(&self.files)
    }
}
pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
        || reserved(id)
    {
        return Err(Error(format!("invalid resource id: {id}")));
    }
    Ok(())
}
fn validate_version(version: &str) -> Result<()> {
    if version.is_empty()
        || version.len() > 64
        || !version
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
    {
        return Err(Error(format!("invalid exact resource version: {version}")));
    }
    Ok(())
}
fn reserved(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("");
    matches!(stem, "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}
pub fn validate_path(path: &str) -> Result<()> {
    validate_path_inner(path, false)
}
fn validate_path_inner(path: &str, manifest: bool) -> Result<()> {
    if path.is_empty()
        || path.len() > 240
        || (!manifest && path == "resource.json")
        || !path
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-. /".contains(&c))
    {
        return Err(Error(format!("noncanonical resource path: {path}")));
    }
    for segment in path.split('/') {
        if segment.is_empty()
            || segment.len() > 100
            || segment.starts_with('.')
            || segment.ends_with('.')
            || segment.contains(' ')
            || reserved(segment)
        {
            return Err(Error(format!("unsafe resource path: {path}")));
        }
    }
    Ok(())
}
/// Resolve only real directories and regular files; no symlink traversal, including root.
pub(crate) fn checked_file(root: &Path, path: &str, manifest: bool) -> Result<PathBuf> {
    validate_path_inner(path, manifest)?;
    let root_meta = fs::symlink_metadata(root)?;
    if !root_meta.is_dir() || root_meta.file_type().is_symlink() {
        return Err(Error(format!(
            "resource root is not a real directory: {}",
            root.display()
        )));
    }
    let mut current = root.to_path_buf();
    let parts: Vec<_> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        current.push(part);
        let meta = fs::symlink_metadata(&current)?;
        if meta.file_type().is_symlink()
            || (i + 1 == parts.len() && !meta.is_file())
            || (i + 1 < parts.len() && !meta.is_dir())
        {
            return Err(Error(format!(
                "resource path is not a regular file/directory: {}",
                current.display()
            )));
        }
    }
    Ok(current)
}
pub(crate) fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > max {
        return Err(Error(format!(
            "file exceeds byte limit: {}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(Error("file grew beyond byte limit".into()));
    }
    Ok(bytes)
}
pub fn ordered_manifests(manifests: &[Manifest]) -> Result<Vec<String>> {
    let mut by_id = BTreeMap::new();
    for m in manifests {
        m.validate()?;
        if by_id.insert(m.id.clone(), m).is_some() {
            return Err(Error(format!("duplicate resource: {}", m.id)));
        }
    }
    let mut ordered = Vec::new();
    let mut visiting = Vec::new();
    let mut done = BTreeSet::new();
    fn visit(
        id: &str,
        by_id: &BTreeMap<String, &Manifest>,
        visiting: &mut Vec<String>,
        done: &mut BTreeSet<String>,
        ordered: &mut Vec<String>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        if visiting.iter().any(|v| v == id) {
            return Err(Error(format!(
                "dependency cycle: {} -> {id}",
                visiting.join(" -> ")
            )));
        }
        visiting.push(id.to_string());
        let m = by_id[id];
        for (dependency, version) in &m.dependencies {
            let Some(other) = by_id.get(dependency) else {
                return Err(Error(format!(
                    "{id}: missing dependency {dependency}@{version}"
                )));
            };
            if &other.version != version {
                return Err(Error(format!(
                    "{id}: dependency {dependency} requires {version}, found {}",
                    other.version
                )));
            }
            visit(dependency, by_id, visiting, done, ordered)?;
        }
        visiting.pop();
        done.insert(id.to_string());
        ordered.push(id.to_string());
        Ok(())
    }
    for id in by_id.keys() {
        visit(id, &by_id, &mut visiting, &mut done, &mut ordered)?;
    }
    Ok(ordered)
}

/// JSON duplicate object keys are rejected instead of silently replacing contracts.
pub(crate) fn unique_map<'de, D, T>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct Unique<T>(std::marker::PhantomData<T>);
    impl<'de, T: serde::Deserialize<'de>> serde::de::Visitor<'de> for Unique<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object with unique keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, T>()? {
                if out.insert(key.clone(), value).is_some() {
                    return Err(serde::de::Error::custom(format!("duplicate key: {key}")));
                }
            }
            Ok(out)
        }
    }
    deserializer.deserialize_map(Unique(std::marker::PhantomData))
}
