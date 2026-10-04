use crate::manifest::{checked_file, read_bounded};
use crate::{Error, FileDigest, Limits, ResourceSet, Result, digest_bytes, validate_digest};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_AUDIT_BYTES: u64 = 8 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
/// Shared immutable bytes; activation and audit decisions are scoped by source.
#[derive(Debug)]
pub struct Cache {
    root: PathBuf,
    pub(crate) limits: Limits,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryEntry {
    pub timestamp_ms: u64,
    pub source: String,
    pub set_revision: String,
    pub resource_id: String,
    pub version: String,
    pub content_digest: String,
    pub files: BTreeMap<String, FileDigest>,
    pub requested_capabilities: Vec<String>,
    pub grants: Vec<String>,
    pub verified: bool,
    #[serde(default)]
    pub verification_error: Option<String>,
    pub downloaded_bytes: u64,
    pub activation: Option<bool>,
    pub activation_error: Option<String>,
}
fn real_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(Error(format!(
            "cache directory is not a real directory: {}",
            path.display()
        )));
    }
    Ok(())
}
fn real_file(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && !m.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(Error(format!(
            "cache entry is not a regular file: {}",
            path.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
fn source_key(source: &str) -> Result<String> {
    if source.is_empty() || source.len() > 512 || source.chars().any(char::is_control) {
        return Err(Error("invalid source provenance".into()));
    }
    Ok(digest_bytes(source.as_bytes()))
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(Error("resource download cancelled".into()))
    } else {
        Ok(())
    }
}
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Cache {
    pub fn open(root: impl AsRef<Path>, limits: Limits) -> Result<Self> {
        if limits.max_history == 0
            || limits.max_history > 10000
            || limits.max_file_bytes > 16 * 1024 * 1024
            || limits.max_set_bytes > 128 * 1024 * 1024
            || limits.max_resources > 32
            || limits.max_files > 4096
        {
            return Err(Error(
                "cache limits exceed protocol bounds or history is zero".into(),
            ));
        }
        let root = root.as_ref().to_path_buf();
        real_directory(&root)?;
        for dir in ["blobs", "sets", "active", "tmp"] {
            real_directory(&root.join(dir))?;
        }
        let cache = Self { root, limits };
        let _guard = cache.lock()?;
        // All writers hold the OS lock. After process death it is released, so stale staging is safe to remove.
        for entry in fs::read_dir(cache.root.join("tmp"))? {
            let path = entry?.path();
            if fs::symlink_metadata(&path)?.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        cache.prune_locked(limits.max_cache_bytes, &BTreeSet::new())?;
        if cache.disk_bytes()? > limits.max_cache_bytes {
            return Err(Error(
                "pinned content or inventory exceeds cache budget".into(),
            ));
        }
        Ok(cache)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn lock(&self) -> Result<File> {
        let path = self.root.join("lock");
        real_file(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.lock()?;
        Ok(file)
    }
    pub fn blob_path(&self, digest: &str) -> Result<PathBuf> {
        validate_digest(digest)?;
        Ok(self.root.join("blobs").join(digest))
    }
    pub(crate) fn read_blob(&self, file: &FileDigest) -> Result<Option<Vec<u8>>> {
        let path = self.blob_path(&file.digest)?;
        if !real_file(&path)? {
            return Ok(None);
        }
        // Corrupt/truncated entries are misses and are repaired from verified source bytes.
        let bytes = match read_bounded(&path, self.limits.max_file_bytes) {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        if bytes.len() as u64 != file.size || digest_bytes(&bytes) != file.digest {
            return Ok(None);
        }
        Ok(Some(bytes))
    }
    fn atomic_write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        real_file(path)?;
        if self.disk_bytes()?.saturating_add(bytes.len() as u64) > self.limits.max_cache_bytes {
            return Err(Error("atomic cache write exceeds byte budget".into()));
        }
        let temporary = self.root.join("tmp").join(format!(
            "file-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
    pub(crate) fn store_blob(&self, file: &FileDigest, bytes: &[u8]) -> Result<()> {
        if bytes.len() as u64 != file.size
            || file.size > self.limits.max_file_bytes
            || digest_bytes(bytes) != file.digest
        {
            return Err(Error("downloaded blob verification failed".into()));
        }
        let _guard = self.lock()?;
        if self.read_blob(file)?.is_some() {
            return Ok(());
        }
        let path = self.blob_path(&file.digest)?;
        if self.disk_bytes()?.saturating_add(file.size) > self.limits.max_cache_bytes {
            return Err(Error(
                "cache byte budget exhausted; prune unused content or raise budget".into(),
            ));
        }
        self.atomic_write(&path, bytes)
    }
    pub(crate) fn prepare(&self, set: &ResourceSet) -> Result<()> {
        set.validate(self.limits)?;
        let _guard = self.lock()?;
        let mut keep = BTreeSet::new();
        let mut missing = 0u64;
        for file in set.resources.iter().flat_map(|r| r.files.values()) {
            if keep.insert(file.digest.clone()) && self.read_blob(file)?.is_none() {
                missing = missing.saturating_add(file.size);
            }
        }
        // Reserve a complete copy for atomic set materialization, plus bounded manifests/metadata.
        if self.verified_roots(set).is_err() {
            missing = missing
                .saturating_add(set.total_bytes())
                .saturating_add(serde_json::to_vec(set)?.len() as u64)
                .saturating_add(
                    set.resources
                        .iter()
                        .map(|r| {
                            serde_json::to_vec(&r.manifest)
                                .map(|v| v.len() as u64)
                                .unwrap_or(0)
                        })
                        .sum::<u64>(),
                );
        }
        self.prune_locked(self.limits.max_cache_bytes.saturating_sub(missing), &keep)?;
        if self.disk_bytes()?.saturating_add(missing) > self.limits.max_cache_bytes {
            return Err(Error(
                "resource set cannot fit cache budget while preserving active content".into(),
            ));
        }
        Ok(())
    }
    fn verified_roots(&self, set: &ResourceSet) -> Result<BTreeMap<String, PathBuf>> {
        let directory = self.root.join("sets").join(&set.revision);
        let meta = fs::symlink_metadata(&directory)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(Error("invalid cached set directory".into()));
        }
        let recorded: ResourceSet = serde_json::from_slice(&read_bounded(
            &checked_file(&directory, "set.json", false)?,
            crate::MAX_SET_JSON_BYTES as u64,
        )?)?;
        if &recorded != set {
            return Err(Error("cached set metadata mismatch".into()));
        }
        let mut roots = BTreeMap::new();
        for resource in &set.resources {
            let root = directory.join(&resource.manifest.id);
            let manifest = crate::Manifest::read(&root)?;
            if manifest != resource.manifest {
                return Err(Error("cached resource manifest mismatch".into()));
            }
            for (path, file) in &resource.files {
                let bytes = read_bounded(
                    &checked_file(&root, path, false)?,
                    self.limits.max_file_bytes,
                )?;
                if bytes.len() as u64 != file.size || digest_bytes(&bytes) != file.digest {
                    return Err(Error(
                        "cached materialized bytes failed verification".into(),
                    ));
                }
            }
            roots.insert(resource.manifest.id.clone(), root);
        }
        Ok(roots)
    }
    pub(crate) fn materialize(
        &self,
        set: &ResourceSet,
        cancel: &AtomicBool,
    ) -> Result<BTreeMap<String, PathBuf>> {
        set.validate(self.limits)?;
        let _guard = self.lock()?;
        cancelled(cancel)?;
        if let Ok(roots) = self.verified_roots(set) {
            return Ok(roots);
        }
        let metadata_bytes = serde_json::to_vec(set)?.len() as u64
            + set
                .resources
                .iter()
                .map(|r| {
                    serde_json::to_vec(&r.manifest)
                        .map(|v| v.len() as u64)
                        .unwrap_or(0)
                })
                .sum::<u64>();
        if self
            .disk_bytes()?
            .saturating_add(set.total_bytes())
            .saturating_add(metadata_bytes)
            > self.limits.max_cache_bytes
        {
            return Err(Error(
                "atomic set materialization exceeds cache byte budget".into(),
            ));
        }
        let staging = Staging(self.root.join("tmp").join(format!(
            "set-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        fs::create_dir(&staging.0)?;
        for resource in &set.resources {
            cancelled(cancel)?;
            let directory = staging.0.join(&resource.manifest.id);
            fs::create_dir(&directory)?;
            fs::write(
                directory.join("resource.json"),
                serde_json::to_vec(&resource.manifest)?,
            )?;
            for (path, file) in &resource.files {
                cancelled(cancel)?;
                let bytes = self.read_blob(file)?.ok_or_else(|| {
                    Error("required verified blob missing during materialization".into())
                })?;
                let target = directory.join(path);
                fs::create_dir_all(target.parent().unwrap())?;
                let mut output = File::create(target)?;
                output.write_all(&bytes)?;
                output.sync_all()?;
            }
        }
        fs::write(staging.0.join("set.json"), serde_json::to_vec(set)?)?;
        cancelled(cancel)?;
        if self.disk_bytes()? > self.limits.max_cache_bytes {
            return Err(Error(
                "cache byte budget exceeded by staged resource set".into(),
            ));
        }
        let destination = self.root.join("sets").join(&set.revision);
        // Repair a locally corrupt materialization under the same full identity.
        let backup = staging.0.with_extension("old");
        if destination.exists() {
            fs::rename(&destination, &backup)?;
        }
        if let Err(e) = fs::rename(&staging.0, &destination) {
            if backup.exists() {
                let _ = fs::rename(&backup, &destination);
            }
            return Err(e.into());
        }
        if backup.exists() {
            fs::remove_dir_all(backup)?;
        }
        self.verified_roots(set)
    }
    pub fn inventory(&self) -> Result<Vec<InventoryEntry>> {
        let _guard = self.lock()?;
        self.inventory_locked()
    }
    fn inventory_locked(&self) -> Result<Vec<InventoryEntry>> {
        let path = self.root.join("inventory.json");
        if !real_file(&path)? {
            return Ok(Vec::new());
        }
        let entries: Vec<InventoryEntry> =
            serde_json::from_slice(&read_bounded(&path, MAX_AUDIT_BYTES)?)?;
        if entries.len() > 10000 {
            return Err(Error("cache inventory history exceeds hard limit".into()));
        }
        Ok(entries)
    }
    fn append_inventory(&self, entries: Vec<InventoryEntry>) -> Result<()> {
        let _guard = self.lock()?;
        self.append_inventory_locked(entries)
    }
    fn append_inventory_locked(&self, mut entries: Vec<InventoryEntry>) -> Result<()> {
        let mut history = self.inventory_locked()?;
        history.append(&mut entries);
        if history.len() > self.limits.max_history {
            history.drain(..history.len() - self.limits.max_history);
        }
        let bytes = loop {
            let bytes = serde_json::to_vec_pretty(&history)?;
            if bytes.len() as u64 <= MAX_AUDIT_BYTES {
                break bytes;
            }
            if history.is_empty() {
                return Err(Error("inventory entry exceeds byte limit".into()));
            }
            history.remove(0);
        };
        let path = self.root.join("inventory.json");
        if self.disk_bytes()?.saturating_add(bytes.len() as u64) > self.limits.max_cache_bytes {
            return Err(Error("cache inventory cannot fit byte budget".into()));
        }
        self.atomic_write(&path, &bytes)
    }
    fn entries(
        &self,
        set: &ResourceSet,
        source: &str,
        grants: &BTreeMap<String, Vec<String>>,
        downloaded: &BTreeMap<String, u64>,
        activation: Option<bool>,
        error: Option<&str>,
    ) -> Result<Vec<InventoryEntry>> {
        source_key(source)?;
        set.validate(self.limits)?;
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        if error.is_some_and(|e| e.len() > 2048) {
            return Err(Error("activation error exceeds audit limit".into()));
        }
        for granted in grants.values() {
            if granted.len() > 64 || granted.iter().any(|s| s.len() > 96) {
                return Err(Error("audit grants exceed limit".into()));
            }
        }
        Ok(set
            .resources
            .iter()
            .map(|r| InventoryEntry {
                timestamp_ms,
                source: source.into(),
                set_revision: set.revision.clone(),
                resource_id: r.manifest.id.clone(),
                version: r.manifest.version.clone(),
                content_digest: r.content_digest.clone(),
                files: r.files.clone(),
                requested_capabilities: r.manifest.capabilities.clone(),
                grants: grants.get(&r.manifest.id).cloned().unwrap_or_default(),
                verified: true,
                verification_error: None,
                downloaded_bytes: downloaded.get(&r.manifest.id).copied().unwrap_or(0),
                activation,
                activation_error: error.map(str::to_owned),
            })
            .collect())
    }
    pub(crate) fn record_download(
        &self,
        set: &ResourceSet,
        source: &str,
        downloaded: &BTreeMap<String, u64>,
    ) -> Result<()> {
        self.append_inventory(self.entries(
            set,
            source,
            &BTreeMap::new(),
            downloaded,
            None,
            None,
        )?)
    }
    pub(crate) fn record_verification_failure(
        &self,
        set: &ResourceSet,
        source: &str,
        downloaded: &BTreeMap<String, u64>,
        error: &str,
    ) -> Result<()> {
        let mut entries = self.entries(set, source, &BTreeMap::new(), downloaded, None, None)?;
        let message: String = error.chars().take(1024).collect();
        for entry in &mut entries {
            entry.verified = false;
            entry.verification_error = Some(message.clone());
        }
        self.append_inventory(entries)
    }
    /// Commit successful activation by source only after the caller started every resource.
    /// On failure, preserve the previous active set and persist the failure/grants.
    pub fn record_activation(
        &self,
        set: &ResourceSet,
        source: &str,
        grants: &BTreeMap<String, Vec<String>>,
        error: Option<&str>,
    ) -> Result<()> {
        let key = source_key(source)?;
        let _guard = self.lock()?;
        if error.is_none() {
            self.verified_roots(set)?;
        }
        self.append_inventory_locked(self.entries(
            set,
            source,
            grants,
            &BTreeMap::new(),
            Some(error.is_none()),
            error,
        )?)?;
        if error.is_none() {
            self.atomic_write(&self.root.join("active").join(key), set.revision.as_bytes())?;
        }
        Ok(())
    }
    pub fn deactivate(&self, source: &str) -> Result<()> {
        let key = source_key(source)?;
        let _guard = self.lock()?;
        let path = self.root.join("active").join(key);
        if real_file(&path)? {
            fs::remove_file(path)?;
        }
        Ok(())
    }
    pub fn disk_bytes(&self) -> Result<u64> {
        tree_bytes(&self.root)
    }
    /// Remove inactive materializations and unreferenced blobs oldest first. Active sets are pinned.
    /// Returns bytes removed. A target below pinned/audit bytes cannot be reached.
    pub fn prune(&self, target_bytes: u64) -> Result<u64> {
        let _guard = self.lock()?;
        self.prune_locked(target_bytes, &BTreeSet::new())
    }
    // A damaged materialization must not poison management of the shared cache.
    // Call only under the cache lock, after disk_bytes rejects all symlinks.
    fn discard_invalid_metadata_locked(&self) -> Result<()> {
        let mut valid_sets = BTreeSet::new();
        for entry in fs::read_dir(self.root.join("sets"))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            validate_digest(&name)?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(Error("unsafe cached set directory".into()));
            }
            let usable = (|| -> Result<()> {
                let set: ResourceSet = serde_json::from_slice(&read_bounded(
                    &checked_file(&path, "set.json", false)?,
                    crate::MAX_SET_JSON_BYTES as u64,
                )?)?;
                set.validate(self.limits)?;
                if set.revision != name {
                    return Err(Error("cached set directory revision mismatch".into()));
                }
                Ok(())
            })();
            if usable.is_ok() {
                valid_sets.insert(name);
            } else {
                // Only this validated digest-named real directory is disposable.
                // Independent content-addressed blobs are left available for reuse.
                fs::remove_dir_all(path)?;
            }
        }
        for entry in fs::read_dir(self.root.join("active"))? {
            let path = entry?.path();
            if !real_file(&path)? {
                continue;
            }
            let usable = (|| -> Result<bool> {
                let digest = String::from_utf8(read_bounded(&path, 64)?)
                    .map_err(|_| Error("invalid active set pointer".into()))?;
                validate_digest(&digest)?;
                Ok(valid_sets.contains(&digest))
            })();
            if !matches!(usable, Ok(true)) {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
    fn prune_locked(&self, target: u64, keep: &BTreeSet<String>) -> Result<u64> {
        let before = self.disk_bytes()?;
        self.discard_invalid_metadata_locked()?;
        let mut size = self.disk_bytes()?;
        let mut pinned = BTreeSet::new();
        let mut retained = keep.clone();
        for entry in fs::read_dir(self.root.join("active"))? {
            let path = entry?.path();
            if !real_file(&path)? {
                continue;
            }
            let digest = String::from_utf8(read_bounded(&path, 64)?)
                .map_err(|_| Error("invalid active set pointer".into()))?;
            validate_digest(&digest)?;
            pinned.insert(digest);
        }
        let mut sets = Vec::new();
        for entry in fs::read_dir(self.root.join("sets"))? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            validate_digest(&name)?;
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(Error("unsafe cached set directory".into()));
            }
            if pinned.contains(&name) {
                let set: ResourceSet = serde_json::from_slice(&read_bounded(
                    &checked_file(&path, "set.json", false)?,
                    crate::MAX_SET_JSON_BYTES as u64,
                )?)?;
                set.validate(self.limits)?;
                retained.extend(
                    set.resources
                        .iter()
                        .flat_map(|r| r.files.values())
                        .map(|f| f.digest.clone()),
                );
            } else {
                sets.push((metadata.modified().unwrap_or(UNIX_EPOCH), path));
            }
        }
        sets.sort();
        for (_, path) in sets {
            if size <= target {
                break;
            }
            let bytes = tree_bytes(&path)?;
            fs::remove_dir_all(path)?;
            size = size.saturating_sub(bytes);
        }
        // Blobs referenced by remaining sets must survive, including inactive complete sets retained under budget.
        for entry in fs::read_dir(self.root.join("sets"))? {
            let path = entry?.path().join("set.json");
            let set: ResourceSet =
                serde_json::from_slice(&read_bounded(&path, crate::MAX_SET_JSON_BYTES as u64)?)?;
            retained.extend(
                set.resources
                    .iter()
                    .flat_map(|r| r.files.values())
                    .map(|f| f.digest.clone()),
            );
        }
        let mut blobs = Vec::new();
        for entry in fs::read_dir(self.root.join("blobs"))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            validate_digest(&name)?;
            if !retained.contains(&name) {
                let meta = fs::symlink_metadata(entry.path())?;
                if !meta.is_file() || meta.file_type().is_symlink() {
                    return Err(Error("unsafe cached blob".into()));
                }
                blobs.push((
                    meta.modified().unwrap_or(UNIX_EPOCH),
                    entry.path(),
                    meta.len(),
                ));
            }
        }
        blobs.sort();
        for (_, path, bytes) in blobs {
            if size <= target {
                break;
            }
            fs::remove_file(path)?;
            size = size.saturating_sub(bytes);
        }
        Ok(before.saturating_sub(size))
    }
}
fn tree_bytes(path: &Path) -> Result<u64> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(Error(format!("cache contains symlink: {}", path.display())));
    }
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Err(Error("unsupported cache entry".into()));
    }
    let mut bytes = 0u64;
    for entry in fs::read_dir(path)? {
        bytes = bytes.saturating_add(tree_bytes(&entry?.path())?);
    }
    Ok(bytes)
}
