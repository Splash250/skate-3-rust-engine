use std::process::Command;

const FALLBACK: &str = "OS version unavailable";

fn formatted(success: bool, stdout: &[u8]) -> String {
    if success {
        let value = String::from_utf8_lossy(stdout).trim().to_owned();
        if !value.is_empty() {
            return value;
        }
    }
    FALLBACK.to_owned()
}

pub fn os_version() -> String {
    match Command::new("uname").args(["-s", "-r"]).output() {
        Ok(output) => formatted(output.status.success(), &output.stdout),
        Err(_) => FALLBACK.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_successful_output() {
        assert_eq!(formatted(true, b"Linux 6.12.0\n"), "Linux 6.12.0");
    }

    #[test]
    fn failure_and_empty_output_fall_back() {
        assert_eq!(formatted(false, b"Linux\n"), FALLBACK);
        assert_eq!(formatted(true, b" \n"), FALLBACK);
    }
}
