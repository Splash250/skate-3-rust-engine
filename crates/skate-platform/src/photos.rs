//! Narrow local desktop export. Resource scripts never supply paths or filenames.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// The only export location is the operating system's configured Desktop.
/// `dirs` uses XDG user directories on Linux and Known Folders on Windows.
pub struct PhotoDirectory(PathBuf);

fn unique_stamp() -> u128 {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        + u128::from(SERIAL.fetch_add(1, Ordering::Relaxed))
}

fn ordinary_directory(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reject junctions as well as symlinks in the dedicated output directory.
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    metadata.is_dir() && !metadata.file_type().is_symlink()
}

impl PhotoDirectory {
    pub fn resolve() -> Result<Self, String> {
        let desktop = dirs::desktop_dir().ok_or(
            "The OS has no Desktop folder configured. Configure an XDG Desktop directory on Linux or restore the Windows Desktop Known Folder."
        )?;
        Self::at_desktop(&desktop)
    }

    fn at_desktop(desktop: &Path) -> Result<Self, String> {
        if !desktop.is_absolute() || !desktop.is_dir() {
            return Err(
                "The configured Desktop folder is unavailable. Create or restore it, then retry."
                    .into(),
            );
        }
        let desktop = fs::canonicalize(desktop)
            .map_err(|_| "Cannot access the configured Desktop folder.")?;
        let folder = desktop.join("Skate Photos");
        match fs::create_dir(&folder) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err("Cannot create Skate Photos on the Desktop. Check local folder permissions and free space.".into()),
        }
        if !ordinary_directory(&folder) {
            return Err(
                "Skate Photos must be an ordinary folder, not a file, symlink or junction.".into(),
            );
        }
        Ok(Self(folder))
    }

    /// Internal host-only bytes. Callers receive this path only for local logging;
    /// never send it through resource events or multiplayer messages.
    pub fn write_png(&self, bytes: &[u8], cancelled: &AtomicBool) -> Result<PathBuf, String> {
        if bytes.is_empty() || bytes.len() > 16 * 1024 * 1024 {
            return Err("Encoded photo exceeds the 16 MiB export limit.".into());
        }
        if cancelled.load(Ordering::Acquire) {
            return Err("Photo cancelled.".into());
        }
        if !ordinary_directory(&self.0) {
            return Err("Skate Photos output folder changed; reopen the camera.".into());
        }
        for _ in 0..16 {
            let path = self.0.join(format!(
                "Skate-{}-{}.png",
                unique_stamp(),
                std::process::id()
            ));
            let mut file =
                match OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(_) => return Err(
                        "Cannot save photo. Check Desktop permissions and available disk space."
                            .into(),
                    ),
                };
            let result = file.write_all(bytes).and_then(|_| file.sync_all());
            drop(file);
            if result.is_err() || cancelled.load(Ordering::Acquire) {
                let _ = fs::remove_file(&path);
                return Err(if result.is_err() {
                    "Photo write failed. Check free disk space."
                } else {
                    "Photo cancelled."
                }
                .into());
            }
            return Ok(path);
        }
        Err("Could not choose a unique photo filename; retry the shutter.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn fixture() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "skate-photo-test-{}-{}",
            std::process::id(),
            unique_stamp()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn exports_are_unique_confined_and_never_overwrite() {
        let root = fixture();
        let directory = PhotoDirectory::at_desktop(&root).unwrap();
        let cancel = AtomicBool::new(false);
        let first = directory.write_png(b"first", &cancel).unwrap();
        let second = directory.write_png(b"second", &cancel).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(first.parent(), Some(root.join("Skate Photos").as_path()));
        assert_eq!(first.extension().unwrap(), "png");
        cancel.store(true, Ordering::Release);
        assert!(directory.write_png(b"cancelled", &cancel).is_err());
        assert_eq!(
            std::fs::read_dir(root.join("Skate Photos"))
                .unwrap()
                .count(),
            2
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_desktop_and_non_directory_fail_actionably() {
        let root = fixture();
        assert!(PhotoDirectory::at_desktop(&root.join("absent")).is_err());
        std::fs::write(root.join("Skate Photos"), b"occupied").unwrap();
        assert!(PhotoDirectory::at_desktop(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn redirected_output_directory_is_rejected() {
        let root = fixture();
        let elsewhere = fixture();
        std::os::unix::fs::symlink(&elsewhere, root.join("Skate Photos")).unwrap();
        assert!(PhotoDirectory::at_desktop(&root).is_err());
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(elsewhere).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn replaced_output_directory_is_rechecked_before_write() {
        let root = fixture();
        let elsewhere = fixture();
        let directory = PhotoDirectory::at_desktop(&root).unwrap();
        std::fs::remove_dir(root.join("Skate Photos")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("Skate Photos")).unwrap();
        assert!(
            directory
                .write_png(b"photo", &AtomicBool::new(false))
                .is_err()
        );
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(elsewhere).unwrap();
    }
}
