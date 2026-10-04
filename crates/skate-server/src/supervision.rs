//! Private local IPC with the external process supervisor; never a network API.
use std::path::PathBuf;
fn owner_pid() -> u32 {
    std::env::var("SKATE_SUPERVISOR_OWNER_PID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(std::process::id)
}
fn directory() -> Result<PathBuf, String> {
    std::env::var_os("SKATE_SUPERVISOR_CONTROL")
        .map(PathBuf::from)
        .ok_or_else(|| {
            "Start this server with tools/server_supervisor.py to use this operation".into()
        })
}
fn write(name: &str, value: &serde_json::Value) -> Result<(), String> {
    let root = directory()?;
    let path = root.join(name);
    let temp = root.join(format!("{name}.{}.tmp", std::process::id()));
    std::fs::write(&temp, serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}
pub fn available() -> bool {
    directory().is_ok_and(|p| p.is_dir())
}
pub fn pending() -> bool {
    directory().is_ok_and(|p| p.join("request.json").exists())
}
pub fn pending_store() -> bool {
    directory()
        .ok()
        .and_then(|p| std::fs::read(p.join("request.json")).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| matches!(v["kind"].as_str(), Some("backup" | "restore")))
}
pub fn cancel_restart() {
    if !pending_store() {
        if let Ok(root) = directory() {
            let _ = std::fs::remove_file(root.join("request.json"));
        }
    }
}
pub fn shutdown_intent() -> Result<(), String> {
    write(
        "request.json",
        &serde_json::json!({"kind":"shutdown","pid":owner_pid()}),
    )
}
pub fn heartbeat() -> Result<(), String> {
    write(
        "heartbeat.json",
        &serde_json::json!({"pid":owner_pid(),"tick":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()}),
    )
}
pub fn request(kind: &str, snapshot: Option<&str>) -> Result<String, String> {
    if snapshot.is_some_and(|s| {
        s.is_empty()
            || s.len() > 64
            || !s
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    }) {
        return Err("Snapshot ID must contain 1..64 ASCII identifier characters".into());
    }
    if pending() {
        return Err("A supervisor operation is already draining; wait for its result".into());
    }
    write(
        "request.json",
        &serde_json::json!({"kind":kind,"snapshot":snapshot,"pid":owner_pid()}),
    )?;
    Ok("Supervisor operation scheduled after stopped-store shutdown; inspect operational history for its result".into())
}
pub fn status() -> serde_json::Value {
    let Ok(root) = directory() else {
        return serde_json::json!({"attached":false});
    };
    let path = root.join("status.json");
    if std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= 64 * 1024) {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(value) = serde_json::from_slice(&bytes) {
                return value;
            }
        }
    }
    serde_json::json!({"attached":true,"state":"starting"})
}
