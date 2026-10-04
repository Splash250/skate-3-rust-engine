//! Operator-owned settings. This child module shares only the Host lifecycle.
use super::*;
use serde::Serialize;
use skate_resources::{SettingChange, SettingDefinition, SettingVisibility};
use std::io::{Read, Write};

#[derive(Clone, Copy)]
pub enum SettingAudience {
    Server,
    Client,
    Public,
}
#[derive(Clone, Debug, Serialize)]
pub struct SettingStatus {
    pub definition: SettingDefinition,
    pub value: Value,
    pub pending: Option<Value>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SettingsChange {
    pub applied: bool,
    pub restart_required: bool,
    pub notification_error: Option<String>,
}
impl Host {
    pub fn settings_revision(&self) -> u64 {
        self.settings_revision
    }
    /// Configure operator startup defaults before activation. Persisted admin
    /// overrides win. Neither source is part of downloadable content.
    pub fn configure_settings(
        &mut self,
        id: &str,
        values: BTreeMap<String, Value>,
    ) -> Result<(), String> {
        if self.side != Side::Server || self.running(id) {
            return Err("settings configuration requires a stopped server resource".into());
        }
        let manifest = &self.installed.get(id).ok_or("unknown resource")?.manifest;
        skate_resources::validate_setting_values(&manifest.settings, &values)
            .map_err(|e| e.to_string())?;
        self.settings_defaults.insert(id.into(), values);
        self.prepare_settings(id)
    }
    pub(super) fn prepare_settings(&mut self, id: &str) -> Result<(), String> {
        let manifest = &self.installed.get(id).ok_or("unknown resource")?.manifest;
        let mut values: BTreeMap<_, _> = manifest
            .settings
            .iter()
            .map(|(k, d)| (k.clone(), d.default.clone()))
            .collect();
        if self.side == Side::Server {
            values.extend(self.settings_defaults.get(id).cloned().unwrap_or_default());
            let path = self.storage.join("settings").join(format!("{id}.json"));
            match std::fs::symlink_metadata(&path) {
                Ok(meta) => {
                    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 8192 {
                        return Err(format!("{id}: invalid or oversized settings store"));
                    }
                    let mut bytes = Vec::new();
                    std::fs::File::open(&path)
                        .map_err(|e| e.to_string())?
                        .take(8193)
                        .read_to_end(&mut bytes)
                        .map_err(|e| e.to_string())?;
                    if bytes.len() > 8192 {
                        return Err("settings store grew beyond 8 KiB".into());
                    }
                    let saved: BTreeMap<String, Value> = serde_json::from_slice(&bytes)
                        .map_err(|_| format!("{id}: invalid settings store JSON"))?;
                    values.extend(saved);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        } else if let Some(previous) = self.shared.lock().unwrap().settings.get(id) {
            // A trusted initial snapshot may arrive before VM activation.
            values.extend(previous.clone());
        }
        skate_resources::validate_setting_values(&manifest.settings, &values).map_err(|e| {
            format!("{id}: {e}; migrate the stopped settings store before upgrading")
        })?;
        self.settings_desired.insert(id.into(), values.clone());
        self.shared
            .lock()
            .unwrap()
            .settings
            .insert(id.into(), values);
        self.settings_revision = self.settings_revision.saturating_add(1);
        Ok(())
    }
    pub fn settings_snapshot(&self, id: &str) -> Result<BTreeMap<String, SettingStatus>, String> {
        let manifest = &self.installed.get(id).ok_or("unknown resource")?.manifest;
        let shared = self.shared.lock().unwrap();
        Ok(manifest
            .settings
            .iter()
            .map(|(key, definition)| {
                let value = shared
                    .settings
                    .get(id)
                    .and_then(|v| v.get(key))
                    .cloned()
                    .unwrap_or_else(|| definition.default.clone());
                let pending = self
                    .settings_desired
                    .get(id)
                    .and_then(|v| v.get(key))
                    .filter(|v| *v != &value)
                    .cloned();
                (
                    key.clone(),
                    SettingStatus {
                        definition: definition.clone(),
                        value,
                        pending,
                    },
                )
            })
            .collect())
    }
    pub fn settings_values(
        &self,
        id: &str,
        audience: SettingAudience,
    ) -> Result<BTreeMap<String, Value>, String> {
        Ok(self
            .settings_snapshot(id)?
            .into_iter()
            .filter(|(_, s)| match audience {
                SettingAudience::Server => true,
                SettingAudience::Client => s.definition.visibility != SettingVisibility::Private,
                SettingAudience::Public => s.definition.visibility == SettingVisibility::Public,
            })
            .map(|(key, s)| (key, s.value))
            .collect())
    }
    /// Caller must authorize and audit the key/outcome without retaining values.
    /// Persistence commits before notification. Callback failure cannot undo an
    /// already durable operator change; it retires the faulty resource normally.
    pub fn set_setting(
        &mut self,
        id: &str,
        key: &str,
        value: Value,
    ) -> Result<SettingsChange, String> {
        if self.side != Side::Server {
            return Err("only the server operator can update settings".into());
        }
        let definition = self
            .installed
            .get(id)
            .ok_or("unknown resource")?
            .manifest
            .settings
            .get(key)
            .ok_or("unknown setting")?
            .clone();
        definition
            .validate_value(&value)
            .map_err(|e| e.to_string())?;
        let mut desired = self.settings_desired.get(id).cloned().unwrap_or_default();
        desired.insert(key.into(), value.clone());
        skate_resources::validate_setting_values(&self.installed[id].manifest.settings, &desired)
            .map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(&desired).map_err(|e| e.to_string())?;
        persist(&self.storage.join("settings"), id, &bytes)?;
        self.settings_desired.insert(id.into(), desired);
        self.settings_revision = self.settings_revision.saturating_add(1);
        let restart_required = self.running(id) && definition.change == SettingChange::Restart;
        let mut notification_error = None;
        if !restart_required {
            self.shared
                .lock()
                .unwrap()
                .settings
                .entry(id.into())
                .or_default()
                .insert(key.into(), value.clone());
            if self.running(id) {
                if let Err(error) = self.invoke(
                    id,
                    "on_settings",
                    serde_json::json!({"key":key,"value":value}),
                    None,
                ) {
                    self.fail(id, error);
                    notification_error = Some(
                        "settings saved; notification callback failed and resource retired".into(),
                    );
                }
                if let Err(error) = self.pump_events() {
                    self.diagnostic(error);
                    notification_error = Some("settings saved; notification event failed".into());
                }
            }
        }
        Ok(SettingsChange {
            applied: !restart_required,
            restart_required,
            notification_error,
        })
    }
    /// Full snapshot from the authenticated server, never from a script or peer.
    /// The game host may apply it before on_load of this installed generation.
    pub fn apply_settings(
        &mut self,
        id: &str,
        generation: u64,
        values: BTreeMap<String, Value>,
    ) -> Result<(), String> {
        if self.side != Side::Client || self.generation(id) != Some(generation) {
            return Err("invalid settings side or stale generation".into());
        }
        let definitions = &self
            .installed
            .get(id)
            .ok_or("unknown resource")?
            .manifest
            .settings;
        if values.len() != definitions.len()
            || definitions
                .values()
                .any(|d| d.visibility == SettingVisibility::Private)
        {
            return Err("client settings require the complete public projection".into());
        }
        skate_resources::validate_setting_values(definitions, &values)
            .map_err(|e| e.to_string())?;
        let previous = self
            .shared
            .lock()
            .unwrap()
            .settings
            .insert(id.into(), values.clone())
            .unwrap_or_default();
        if self.running(id) {
            for (key, value) in values {
                if previous.get(&key) != Some(&value) {
                    if let Err(error) = self.invoke(
                        id,
                        "on_settings",
                        serde_json::json!({"key":key,"value":value}),
                        None,
                    ) {
                        self.fail(id, error.clone());
                        return Err(error);
                    }
                }
            }
            self.pump_events()?;
        }
        Ok(())
    }
}
fn persist(directory: &std::path::Path, id: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > 8192 {
        return Err("settings values exceed 8 KiB".into());
    }
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    if std::fs::symlink_metadata(directory)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("settings directory cannot be a symlink".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = directory.join(format!(
        ".{id}-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, directory.join(format!("{id}.json")))?;
        #[cfg(unix)]
        std::fs::File::open(directory)?.sync_all()?;
        Ok::<_, std::io::Error>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}
