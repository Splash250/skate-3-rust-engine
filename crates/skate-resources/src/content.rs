use crate::manifest::{checked_file, read_bounded};
use crate::{Error, Manifest, Result, ordered_manifests, validate_id};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDigest {
    pub digest: String,
    pub size: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub manifest: Manifest,
    #[serde(deserialize_with = "crate::manifest::unique_map")]
    pub files: BTreeMap<String, FileDigest>,
    pub generation: u64,
    pub content_digest: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceSet {
    pub revision: String,
    pub resources: Vec<Resource>,
}
#[derive(Debug, Clone)]
pub struct PublishedSet {
    pub set: ResourceSet,
    pub blobs: BTreeMap<String, Vec<u8>>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_set_bytes: u64,
    pub max_resources: usize,
    pub max_files: usize,
    pub max_cache_bytes: u64,
    pub max_history: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 64 * 1024 * 1024,
            max_set_bytes: 256 * 1024 * 1024,
            max_resources: 128,
            max_files: 16384,
            max_cache_bytes: 1024 * 1024 * 1024,
            max_history: 512,
        }
    }
}
impl Limits {
    pub fn protocol_maximum() -> Self {
        Self {
            max_file_bytes: 256 * 1024 * 1024,
            max_set_bytes: 1024 * 1024 * 1024,
            max_resources: 256,
            max_files: 65536,
            max_cache_bytes: 64 * 1024 * 1024 * 1024,
            max_history: 10000,
        }
    }
    pub fn validate(self) -> Result<()> {
        let hard = Self::protocol_maximum();
        if self.max_file_bytes == 0
            || self.max_file_bytes > hard.max_file_bytes
            || self.max_set_bytes == 0
            || self.max_set_bytes > hard.max_set_bytes
            || self.max_resources == 0
            || self.max_resources > hard.max_resources
            || self.max_files == 0
            || self.max_files > hard.max_files
            || self.max_cache_bytes == 0
            || self.max_cache_bytes > hard.max_cache_bytes
            || self.max_history == 0
            || self.max_history > hard.max_history
        {
            return Err(Error("content/cache budget outside protocol bounds".into()));
        }
        Ok(())
    }
}
pub const MAX_SET_JSON_BYTES: usize = 2 * 1024 * 1024;
/// Full lowercase BLAKE3 identities, never path strings or truncated hashes.
pub fn digest_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
pub fn validate_digest(digest: &str) -> Result<()> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(Error("invalid full BLAKE3 digest".into()));
    }
    Ok(())
}
fn resource_digest(manifest: &Manifest, files: &BTreeMap<String, FileDigest>) -> Result<String> {
    Ok(digest_bytes(&serde_json::to_vec(&(
        "skate-resource-v1",
        manifest,
        files,
    ))?))
}
fn revision(resources: &[Resource]) -> Result<String> {
    Ok(digest_bytes(&serde_json::to_vec(&(
        "skate-resource-set-v1",
        resources,
    ))?))
}
impl ResourceSet {
    pub fn validate(&self, limits: Limits) -> Result<()> {
        limits.validate()?;
        validate_digest(&self.revision)?;
        if self.resources.len() > limits.max_resources {
            return Err(Error("resource count exceeds limit".into()));
        }
        if self.resources.iter().filter(|r| r.manifest.world.is_some()).count() > 1 {
            return Err(Error("resource set selects more than one required world".into()));
        }
        let manifests: Vec<_> = self.resources.iter().map(|r| r.manifest.clone()).collect();
        let order = ordered_manifests(&manifests)?;
        if self
            .resources
            .iter()
            .map(|r| r.manifest.id.as_str())
            .ne(order.iter().map(String::as_str))
        {
            return Err(Error(
                "resource set is not in canonical dependency order".into(),
            ));
        }
        let mut total = 0u64;
        let mut count = 0usize;
        for resource in &self.resources {
            if resource.generation == 0 {
                return Err(Error("resource generation must be nonzero".into()));
            }
            if !resource.manifest.server_scripts.is_empty() {
                return Err(Error(
                    "downloadable manifest contains private server scripts".into(),
                ));
            }
            let expected: BTreeSet<_> = resource
                .manifest
                .public_paths()
                .map(String::as_str)
                .collect();
            if resource
                .files
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                != expected
            {
                return Err(Error(format!(
                    "{}: downloadable files do not match manifest",
                    resource.manifest.id
                )));
            }
            for file in resource.files.values() {
                validate_digest(&file.digest)?;
                if file.size > limits.max_file_bytes {
                    return Err(Error("resource file exceeds byte limit".into()));
                }
                total = total
                    .checked_add(file.size)
                    .ok_or_else(|| Error("resource size overflow".into()))?;
                count += 1;
            }
            if resource_digest(&resource.manifest, &resource.files)? != resource.content_digest {
                return Err(Error(format!(
                    "{}: content digest mismatch",
                    resource.manifest.id
                )));
            }
        }
        if total > limits.max_set_bytes || count > limits.max_files {
            return Err(Error("resource set exceeds byte/file limit".into()));
        }
        if revision(&self.resources)? != self.revision {
            return Err(Error("resource set revision mismatch".into()));
        }
        if serde_json::to_vec(self)?.len() > MAX_SET_JSON_BYTES {
            return Err(Error("resource set metadata exceeds byte limit".into()));
        }
        Ok(())
    }
    pub fn total_bytes(&self) -> u64 {
        self.resources
            .iter()
            .flat_map(|r| r.files.values())
            .map(|f| f.size)
            .sum()
    }
}
impl PublishedSet {
    /// Compose already-frozen resource generations without rereading mutable server
    /// directories. The blob map must contain exactly the selected public allowlist.
    pub fn from_resources(
        resources: Vec<Resource>,
        blobs: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self> {
        let order = ordered_manifests(
            &resources
                .iter()
                .map(|r| r.manifest.clone())
                .collect::<Vec<_>>(),
        )?;
        let mut by_id: BTreeMap<_, _> = resources
            .into_iter()
            .map(|r| (r.manifest.id.clone(), r))
            .collect();
        let resources = order
            .into_iter()
            .map(|id| by_id.remove(&id).unwrap())
            .collect::<Vec<_>>();
        let set = ResourceSet {
            revision: revision(&resources)?,
            resources,
        };
        let published = Self { set, blobs };
        published.validate()?;
        Ok(published)
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_with_limits(Limits::protocol_maximum())
    }
    pub fn validate_with_limits(&self, limits: Limits) -> Result<()> {
        self.set.validate(limits)?;
        let mut expected = BTreeMap::new();
        for resource in &self.set.resources {
            for file in resource.files.values() {
                if let Some(size) = expected.insert(file.digest.clone(), file.size) {
                    if size != file.size {
                        return Err(Error("conflicting blob sizes".into()));
                    }
                }
            }
        }
        if self.blobs.len() != expected.len() {
            return Err(Error(
                "published blob allowlist does not match selected set".into(),
            ));
        }
        for (digest, size) in expected {
            let bytes = self
                .blobs
                .get(&digest)
                .ok_or_else(|| Error("missing published blob".into()))?;
            if bytes.len() as u64 != size || digest_bytes(bytes) != digest {
                return Err(Error("published blob content mismatch".into()));
            }
        }
        Ok(())
    }
}
pub fn build_set(root: &Path, selected: &BTreeMap<String, u64>) -> Result<PublishedSet> {
    build_set_with_limits(root, selected, Limits::default())
}
pub fn build_set_with_limits(
    root: &Path,
    selected: &BTreeMap<String, u64>,
    limits: Limits,
) -> Result<PublishedSet> {
    limits.validate()?;
    let mut manifests = BTreeMap::new();
    let mut pending: Vec<_> = selected.keys().cloned().collect();
    for (id, generation) in selected {
        validate_id(id)?;
        if *generation == 0 {
            return Err(Error(format!("{id}: generation must be nonzero")));
        }
    }
    while let Some(id) = pending.pop() {
        if manifests.contains_key(&id) {
            continue;
        }
        if manifests.len() >= limits.max_resources {
            return Err(Error("resource count exceeds limit".into()));
        }
        let manifest =
            Manifest::read(&root.join(&id)).map_err(|e| Error(format!("resource {id}: {e}")))?;
        if manifest.id != id {
            return Err(Error(format!(
                "resource folder {id} does not match manifest id {}",
                manifest.id
            )));
        }
        pending.extend(manifest.dependencies.keys().cloned());
        manifests.insert(id, manifest);
    }
    let order = ordered_manifests(&manifests.values().cloned().collect::<Vec<_>>())?;
    let mut resources = Vec::new();
    let mut blobs = BTreeMap::new();
    let mut total = 0u64;
    let mut count = 0usize;
    for id in order {
        let manifest = &manifests[&id];
        let resource_root = root.join(&id);
        let mut files = BTreeMap::new();
        // Validate private entry paths too, but never read or retain their bytes for distribution.
        for path in &manifest.server_scripts {
            checked_file(&resource_root, path, false)?;
        }
        for path in manifest.public_paths() {
            let bytes = read_bounded(
                &checked_file(&resource_root, path, false)?,
                limits.max_file_bytes,
            )?;
            total = total
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| Error("resource size overflow".into()))?;
            count += 1;
            if total > limits.max_set_bytes || count > limits.max_files {
                return Err(Error("resource set exceeds byte/file limit".into()));
            }
            let digest = digest_bytes(&bytes);
            files.insert(
                path.clone(),
                FileDigest {
                    digest: digest.clone(),
                    size: bytes.len() as u64,
                },
            );
            blobs.entry(digest).or_insert(bytes);
        }
        let projected = manifest.client_projection();
        let content_digest = resource_digest(&projected, &files)?;
        resources.push(Resource {
            manifest: projected,
            files,
            generation: selected.get(&id).copied().unwrap_or(1),
            content_digest,
        });
    }
    let set = ResourceSet {
        revision: revision(&resources)?,
        resources,
    };
    let published = PublishedSet { set, blobs };
    published.validate()?;
    Ok(published)
}
