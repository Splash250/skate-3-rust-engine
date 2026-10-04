//! Resource-owned, local-only browser UI. The engine exchanges bounded JSON with
//! a companion webview process; web content never receives engine/OS bindings.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{BufRead, Read},
    path::{Path, PathBuf},
};

pub mod process;
pub const MAX_MESSAGE: usize = 16 * 1024;
pub const MAX_INIT: usize = 160 * 1024;
pub const MAX_FILES: usize = 512;
pub const MAX_FILE: usize = 8 * 1024 * 1024;
pub const MAX_ASSETS: usize = 32 * 1024 * 1024;
pub const QUEUE: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub entry: String,
    pub files: Vec<String>,
    #[serde(default = "width")]
    pub width: u32,
    #[serde(default = "height")]
    pub height: u32,
    #[serde(default)]
    pub focus: bool,
}
fn width() -> u32 {
    900
}
fn height() -> u32 {
    640
}
impl Options {
    pub fn validate(&self) -> Result<(), String> {
        if self.files.is_empty()
            || self.files.len() > MAX_FILES
            || !self.files.contains(&self.entry)
            || !(320..=1920).contains(&self.width)
            || !(240..=1080).contains(&self.height)
            || !self.entry.ends_with(".html")
        {
            return Err(
                "browser requires a listed HTML entry, 1..512 files and bounded dimensions".into(),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for path in &self.files {
            skate_resources::validate_path(path).map_err(|e| e.to_string())?;
            mime(path)?;
            if !seen.insert(path) {
                return Err("duplicate browser asset".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Init {
    pub root: PathBuf,
    pub title: String,
    pub options: Options,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Message { value: Value },
    Focus { focused: bool },
    Close,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Heartbeat,
    Ready,
    Message { value: Value },
    Focus { focused: bool },
    Closed,
    Error { message: String },
}

/// Includes the newline in the bound. Never reads an unbounded line into memory.
pub fn read_record(reader: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    let n = reader
        .take((limit + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    if n > limit || bytes.last() != Some(&b'\n') {
        return Err("browser IPC record truncated or over budget".into());
    }
    bytes.pop();
    Ok(Some(bytes))
}
pub fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() > limit {
        return Err("browser IPC record exceeds byte budget".into());
    }
    Ok(bytes)
}
pub fn mime(path: &str) -> Result<&'static str, String> {
    Ok(match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => return Err(format!("unsupported browser asset format: {path}")),
    })
}
/// HTTP response policy is supplied by the trusted protocol handler, never by
/// resource HTML. No remote network, subframes, workers, forms or object plugins.
pub const CSP: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'none'; frame-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

pub struct Assets {
    files: BTreeMap<String, Vec<u8>>,
}
impl Assets {
    pub fn load(init: &Init) -> Result<Self, String> {
        init.options.validate()?;
        if init.title.len() > 128 {
            return Err("browser title too long".into());
        }
        let root = init.root.canonicalize().map_err(|e| e.to_string())?;
        let mut files = BTreeMap::new();
        let mut used = 0usize;
        for path in &init.options.files {
            let mut candidate = root.clone();
            for component in Path::new(path).components() {
                candidate.push(component);
                if std::fs::symlink_metadata(&candidate)
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink()
                {
                    return Err("browser assets may not traverse symlinks".into());
                }
            }
            let metadata = std::fs::metadata(&candidate).map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.len() > MAX_FILE as u64 {
                return Err("browser asset exceeds file budget".into());
            }
            let mut bytes = Vec::new();
            std::fs::File::open(&candidate)
                .map_err(|e| e.to_string())?
                .take(MAX_FILE as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            used = used.saturating_add(bytes.len());
            if bytes.len() > MAX_FILE || used > MAX_ASSETS {
                return Err("browser asset set exceeds byte budget".into());
            }
            files.insert(path.clone(), bytes);
        }
        Ok(Self { files })
    }
    pub fn get(&self, uri: &str) -> Option<(&[u8], &'static str)> {
        let path = uri
            .strip_prefix("skate://ui/")
            .or_else(|| uri.strip_prefix("http://skate.ui/"))?;
        // Percent encoding/query/fragment aliases are not paths in this API.
        if skate_resources::validate_path(path).is_err() {
            return None;
        }
        self.files
            .get(path)
            .map(|b| (b.as_slice(), mime(path).unwrap()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_ipc_rejects_unterminated_and_oversized_records() {
        assert!(read_record(&mut &b"1234\n"[..], 4).is_err());
        assert!(read_record(&mut &b"1234"[..], 4).is_err());
        assert_eq!(
            read_record(&mut &b"123\n"[..], 4).unwrap(),
            Some(b"123".to_vec())
        );
        assert!(
            encode(
                &Input::Message {
                    value: Value::String("x".repeat(MAX_MESSAGE))
                },
                MAX_MESSAGE
            )
            .is_err()
        );
    }
    #[test]
    fn asset_policy_rejects_traversal_unlisted_and_unsupported_content() {
        let dir = std::env::temp_dir().join(format!("skate-browser-assets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<h1>Hello</h1>").unwrap();
        std::fs::write(dir.join("secret.json"), "private").unwrap();
        let mut init = Init {
            root: dir.clone(),
            title: "Test".into(),
            options: Options {
                entry: "index.html".into(),
                files: vec!["index.html".into()],
                width: 900,
                height: 640,
                focus: false,
            },
        };
        let assets = Assets::load(&init).unwrap();
        assert!(assets.get("skate://ui/index.html").is_some());
        for path in [
            "skate://ui/secret.json",
            "skate://ui/../secret.json",
            "skate://ui/%2e%2e/secret.json",
            "https://example.org/index.html",
            "skate://ui/index.html?secret",
        ] {
            assert!(assets.get(path).is_none(), "{path}");
        }
        init.options.files.push("payload.wasm".into());
        assert!(init.options.validate().is_err());
        #[cfg(unix)]
        {
            init.options.files = vec!["index.html".into(), "alias.html".into()];
            std::os::unix::fs::symlink(dir.join("secret.json"), dir.join("alias.html")).unwrap();
            assert!(Assets::load(&init).is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
