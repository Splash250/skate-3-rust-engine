use std::process::Command;

/// Prevent packaged helper windows on Windows; a no-op elsewhere.
pub fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}
