use std::{
    path::Path,
    process::{Child, Command, Stdio},
};

// The private PID namespace and --die-with-parent handle descendant teardown.
pub(super) struct IsolationGuard;
pub(super) struct WorkerProcess {
    child: Child,
    pub(super) stdin: Option<Box<dyn std::io::Write + Send>>,
    pub(super) stdout: Option<Box<dyn std::io::Read + Send>>,
}
impl WorkerProcess {
    pub(super) fn id(&self) -> u32 {
        self.child.id()
    }
    pub(super) fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }
    pub(super) fn wait(&mut self) -> std::io::Result<()> {
        self.child.wait().map(|_| ())
    }
}

pub(super) fn spawn_worker(
    dotnet: &Path,
    worker: &Path,
    memory_bytes: usize,
) -> Result<(WorkerProcess, IsolationGuard), String> {
    let mut command = Command::new("/usr/bin/prlimit");
    command.args([
        "--core=0:0",
        "--nofile=256:256",
        "--fsize=268435456:268435456",
        "--",
        "/usr/bin/bwrap",
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--clearenv",
    ]);
    for (source, target) in [
        (dotnet.join("dotnet"), "/runtime/dotnet"),
        (dotnet.join("host"), "/runtime/host"),
        (dotnet.join("shared"), "/runtime/shared"),
        (worker.to_path_buf(), "/worker"),
    ] {
        command.arg("--ro-bind").arg(source).arg(target);
    }
    // Mount only .NET's native dependencies, never an arbitrary resource path,
    // the user's home, /etc, /run, host /proc, or the full system root.
    #[cfg(target_arch = "x86_64")]
    let (triplet, loader) = ("x86_64-linux-gnu", "ld-linux-x86-64.so.2");
    #[cfg(target_arch = "aarch64")]
    let (triplet, loader) = ("aarch64-linux-gnu", "ld-linux-aarch64.so.1");
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return Err("managed isolation unsupported on this Linux architecture".into());
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        let directory = format!("/usr/lib/{triplet}");
        for name in [
            "libc.so.6",
            "libm.so.6",
            "libdl.so.2",
            "libpthread.so.0",
            "librt.so.1",
            "libgcc_s.so.1",
            "libstdc++.so.6",
            "libz.so.1",
            "libzstd.so.1",
            "libssl.so.3",
            "libcrypto.so.3",
            loader,
        ] {
            let target = Path::new(&directory).join(name);
            let source = [
                target.clone(),
                Path::new("/usr/lib64").join(name),
                Path::new("/usr/lib").join(name),
            ]
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| format!("managed isolation requires native library {name}"))?;
            command
                .arg("--ro-bind")
                .arg(source.canonicalize().map_err(|e| e.to_string())?)
                .arg(target);
        }
        command.args([
            "--symlink",
            &directory,
            "/lib",
            "--symlink",
            &directory,
            "/lib64",
        ]);
    }
    command
        .args([
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--chdir",
            "/worker",
            "--setenv",
            "DOTNET_ROOT",
            "/runtime",
            "--setenv",
            "DOTNET_SYSTEM_GLOBALIZATION_INVARIANT",
            "1",
            "--setenv",
            "DOTNET_PROCESSOR_COUNT",
            "2",
            "--setenv",
            "DOTNET_EnableDiagnostics",
            "0",
            "--setenv",
            "DOTNET_CLI_TELEMETRY_OPTOUT",
            "1",
            "--setenv",
            "DOTNET_GCHeapHardLimit",
        ])
        .arg(format!("{:x}", memory_bytes / 2))
        .args(["/runtime/dotnet", "/worker/Skate.ResourceHost.dll"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|e| format!("managed isolation launch: {e}"))?;
    let stdin = child
        .stdin
        .take()
        .map(|v| Box::new(v) as Box<dyn std::io::Write + Send>);
    let stdout = child
        .stdout
        .take()
        .map(|v| Box::new(v) as Box<dyn std::io::Read + Send>);
    Ok((
        WorkerProcess {
            child,
            stdin,
            stdout,
        },
        IsolationGuard,
    ))
}

pub(super) fn resident_bytes(pid: u32) -> usize {
    fn visit(pid: u32, remaining: &mut usize) -> usize {
        if *remaining == 0 {
            return usize::MAX / 2;
        }
        *remaining -= 1;
        let rss = std::fs::read_to_string(format!("/proc/{pid}/status"))
            .ok()
            .and_then(|s| {
                s.lines().find_map(|line| {
                    line.strip_prefix("VmRSS:")
                        .and_then(|v| v.split_whitespace().next()?.parse::<usize>().ok())
                })
            })
            .unwrap_or(0)
            .saturating_mul(1024);
        let children =
            std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).unwrap_or_default();
        children
            .split_whitespace()
            .filter_map(|v| v.parse::<u32>().ok())
            .fold(rss, |sum, child| {
                sum.saturating_add(visit(child, remaining))
            })
    }
    visit(pid, &mut 16)
}
