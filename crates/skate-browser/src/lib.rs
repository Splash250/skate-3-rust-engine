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
pub const MAX_SURFACE_WIDTH: u32 = 1280;
pub const MAX_SURFACE_HEIGHT: u32 = 960;
pub const MAX_FRAME: usize = MAX_SURFACE_WIDTH as usize * MAX_SURFACE_HEIGHT as usize * 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default = "surface_scale")]
    pub scale: f32,
    #[serde(default = "surface_offset")]
    pub offset: [f32; 2],
    #[serde(default = "surface_fps")]
    pub fps: u32,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    #[default]
    BottomRight,
    Center,
}
fn surface_scale() -> f32 {
    1.0
}
fn surface_offset() -> [f32; 2] {
    [24.0, 24.0]
}
fn surface_fps() -> u32 {
    20
}
impl Default for Surface {
    fn default() -> Self {
        Self {
            anchor: Anchor::default(),
            scale: surface_scale(),
            offset: surface_offset(),
            fps: surface_fps(),
        }
    }
}
/// Exact RGBA byte count, checked before any frame allocation or read.
pub fn frame_len(width: u32, height: u32) -> Result<usize, String> {
    if !(320..=MAX_SURFACE_WIDTH).contains(&width) || !(240..=MAX_SURFACE_HEIGHT).contains(&height)
    {
        return Err("browser surface dimensions outside 320..1280 x 240..960".into());
    }
    Ok(width as usize * height as usize * 4)
}
#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SurfaceInput {
    Pointer {
        x: f32,
        y: f32,
        #[serde(default)]
        click: bool,
    },
    Wheel {
        delta: f32,
    },
    Key {
        key: String,
        #[serde(default)]
        shift: bool,
    },
    Text {
        text: String,
    },
    Navigate {
        direction: String,
    },
}
impl SurfaceInput {
    pub fn validate(&self) -> bool {
        match self {
            Self::Pointer { x, y, .. } => {
                x.is_finite()
                    && y.is_finite()
                    && *x >= 0.
                    && *y >= 0.
                    && *x <= MAX_SURFACE_WIDTH as f32
                    && *y <= MAX_SURFACE_HEIGHT as f32
            }
            Self::Wheel { delta } => delta.is_finite() && delta.abs() <= 2000.,
            Self::Key { key, .. } => matches!(
                key.as_str(),
                "Tab"
                    | "SelectAll"
                    | "Enter"
                    | "Escape"
                    | "Backspace"
                    | "Delete"
                    | "ArrowUp"
                    | "ArrowDown"
                    | "ArrowLeft"
                    | "ArrowRight"
                    | "Home"
                    | "End"
                    | " "
            ),
            Self::Text { text } => text.len() <= 256 && !text.chars().any(char::is_control),
            Self::Navigate { direction } => matches!(
                direction.as_str(),
                "up" | "down" | "left" | "right" | "accept" | "back"
            ),
        }
    }
}

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
    #[serde(default)]
    pub surface: Option<Surface>,
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
        if let Some(surface) = &self.surface {
            frame_len(self.width, self.height)?;
            if !surface.scale.is_finite()
                || !(0.5..=2.).contains(&surface.scale)
                || !(1..=30).contains(&surface.fps)
                || surface
                    .offset
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=256.).contains(v))
            {
                return Err("browser surface scale/offset/refresh outside bounds".into());
            }
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
    Frame,
    SurfaceInput { input: SurfaceInput },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    /// Header followed by exactly width*height*4 bytes on the private pipe.
    Frame {
        width: u32,
        height: u32,
        #[serde(skip)]
        rgba: Vec<u8>,
    },
    Heartbeat,
    Ready,
    Message {
        value: Value,
    },
    Focus {
        focused: bool,
    },
    Closed,
    Error {
        message: String,
    },
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
        "ttf" => "font/ttf",
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
    fn frame_bounds_and_input_validation_reject_untrusted_dimensions_and_values() {
        assert_eq!(frame_len(1280, 960).unwrap(), MAX_FRAME);
        for (w, h) in [(0, 0), (u32::MAX, 960), (1280, u32::MAX), (1920, 1080)] {
            assert!(frame_len(w, h).is_err());
        }
        assert!(
            !SurfaceInput::Pointer {
                x: f32::NAN,
                y: 0.,
                click: true
            }
            .validate()
        );
        assert!(
            !SurfaceInput::Text {
                text: "x".repeat(257)
            }
            .validate()
        );
        assert!(
            !SurfaceInput::Text {
                text: "a\nb".into()
            }
            .validate()
        );
        assert!(
            !SurfaceInput::Navigate {
                direction: "evaluate".into()
            }
            .validate()
        );
        assert!(
            !SurfaceInput::Key {
                key: "F12".into(),
                shift: false
            }
            .validate()
        );
        assert!(
            SurfaceInput::Navigate {
                direction: "accept".into()
            }
            .validate()
        );
    }
    #[test]
    fn composited_surface_contract_is_bounded_and_backwards_compatible() {
        let base = serde_json::json!({"entry":"index.html","files":["index.html"],"width":400,"height":800,"focus":true});
        assert!(
            serde_json::from_value::<Options>(base.clone())
                .unwrap()
                .validate()
                .is_ok()
        );
        let mut value = base;
        value["surface"] =
            serde_json::json!({"anchor":"bottom_right","scale":1.0,"offset":[24,24],"fps":20});
        assert!(
            serde_json::from_value::<Options>(value.clone())
                .unwrap()
                .validate()
                .is_ok()
        );
        for (key, invalid) in [
            ("scale", serde_json::json!(0)),
            ("fps", serde_json::json!(61)),
            ("offset", serde_json::json!([-1, 0])),
        ] {
            let mut bad = value.clone();
            bad["surface"][key] = invalid;
            assert!(serde_json::from_value::<Options>(bad).map_or(true, |o| o.validate().is_err()));
        }
        value["width"] = serde_json::json!(1920);
        assert!(
            serde_json::from_value::<Options>(value)
                .unwrap()
                .validate()
                .is_err()
        );
    }
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
                surface: None,
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
