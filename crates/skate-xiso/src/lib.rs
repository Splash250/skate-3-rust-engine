use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use xdvdfs::blockdev::OffsetWrapper;
use xdvdfs::read::read_volume;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Eq, PartialEq)]
pub struct Options {
    pub image: PathBuf,
    pub destination: PathBuf,
}

fn invalid(message: impl Into<String>) -> Box<dyn std::error::Error> {
    io::Error::new(io::ErrorKind::InvalidInput, message.into()).into()
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Options> {
    let mut image = None;
    let mut destination = None;
    let mut args = args.into_iter();
    while let Some(raw) = args.next() {
        let option = raw
            .to_str()
            .ok_or_else(|| invalid("option names must be Unicode"))?;
        let target = match option {
            "-x" if image.is_none() => &mut image,
            "-d" if destination.is_none() => &mut destination,
            "-x" | "-d" => return Err(invalid(format!("duplicate option: {option}"))),
            value if value.starts_with('-') => {
                return Err(invalid(format!("unknown option: {value}")));
            }
            value => return Err(invalid(format!("unexpected positional argument: {value}"))),
        };
        *target =
            Some(PathBuf::from(args.next().ok_or_else(|| {
                invalid(format!("missing value for {option}"))
            })?));
    }
    Ok(Options {
        image: image.ok_or_else(|| invalid("missing required option: -x"))?,
        destination: destination.ok_or_else(|| invalid("missing required option: -d"))?,
    })
}

pub fn checked_component(name: &str) -> Result<&str> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(invalid(format!("unsafe image path component: {name:?}")));
    }
    Ok(name)
}

/// Returns true when this invocation created the destination directory.
pub fn prepare_destination(destination: &Path) -> Result<bool> {
    match fs::create_dir(destination) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if !destination.is_dir() {
                return Err(invalid("extraction destination is not a directory"));
            }
            if destination.read_dir()?.next().is_some() {
                return Err(invalid("extraction destination is not empty"));
            }
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

fn target_path(destination: &Path, parent: &str, name: &str) -> Result<PathBuf> {
    let mut target = destination.to_path_buf();
    for component in parent.split('/').filter(|part| !part.is_empty()) {
        target.push(checked_component(component)?);
    }
    target.push(checked_component(name)?);
    Ok(target)
}

pub fn extract(image: &Path, destination: &Path) -> Result<()> {
    let file = File::open(image)?;
    let mut device = OffsetWrapper::new(BufReader::new(file))?;
    let volume = read_volume(&mut device)?;
    for (parent, entry) in volume.root_table.file_tree(&mut device)? {
        let name = entry.name_str::<io::Error>()?;
        let target = target_path(destination, &parent, &name)?;
        if entry.node.dirent.is_directory() {
            fs::create_dir(&target)?;
        } else {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            entry.node.dirent.seek_to(&mut device)?;
            let mut data = device.get_mut().take(entry.node.dirent.data.size as u64);
            io::copy(&mut data, &mut output)?;
            output.flush()?;
        }
    }
    Ok(())
}

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let options = parse_args(args)?;
    let owned = prepare_destination(&options.destination)?;
    match extract(&options.image, &options.destination) {
        Ok(()) => Ok(()),
        Err(error) => {
            if owned {
                fs::remove_dir_all(&options.destination)?;
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<Options> {
        parse_args(values.iter().map(OsString::from))
    }

    #[test]
    fn accepts_both_option_orders() {
        let expected = Options {
            image: "disc.iso".into(),
            destination: "disc".into(),
        };
        assert_eq!(parse(&["-x", "disc.iso", "-d", "disc"]).unwrap(), expected);
        assert_eq!(parse(&["-d", "disc", "-x", "disc.iso"]).unwrap(), expected);
    }

    #[test]
    fn rejects_invalid_arguments() {
        for (args, message) in [
            (&["-x", "disc.iso"][..], "missing required option: -d"),
            (&["-d"][..], "missing value for -d"),
            (
                &["-x", "a", "-x", "b", "-d", "out"][..],
                "duplicate option: -x",
            ),
            (&["--extract"][..], "unknown option: --extract"),
            (
                &["disc.iso"][..],
                "unexpected positional argument: disc.iso",
            ),
        ] {
            assert_eq!(parse(args).unwrap_err().to_string(), message);
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_unicode_option_names() {
        use std::os::unix::ffi::OsStringExt;
        let error = parse_args([OsString::from_vec(vec![0xff])]).unwrap_err();
        assert_eq!(error.to_string(), "option names must be Unicode");
    }

    #[test]
    fn validates_image_components() {
        assert_eq!(checked_component("data").unwrap(), "data");
        for name in ["", ".", "..", "a/b", "a\\b"] {
            assert!(checked_component(name).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn destination_must_be_empty() {
        let root = std::env::temp_dir().join(format!("skate-xiso-test-{}", std::process::id()));
        let destination = root.join("disc");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("existing"), b"data").unwrap();
        assert_eq!(
            prepare_destination(&destination).unwrap_err().to_string(),
            "extraction destination is not empty"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_extraction_removes_only_its_new_destination() {
        let root = std::env::temp_dir().join(format!("skate-xiso-cleanup-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let destination = root.join("disc");
        let result = run([
            "-x",
            root.join("missing.iso").to_str().unwrap(),
            "-d",
            destination.to_str().unwrap(),
        ]
        .map(OsString::from));
        assert!(result.is_err());
        assert!(!destination.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
