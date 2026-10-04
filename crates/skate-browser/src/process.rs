//! Bounded companion IPC and owned process-tree teardown. Child creation may
//! perform local disk work; send/poll never block the game loop on web content.
use crate::{Event, Init, Input, MAX_INIT, MAX_MESSAGE, QUEUE, encode, read_record};
use std::{
    io::{BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub struct Page {
    child: Child,
    input: Option<SyncSender<Vec<u8>>>,
    output: std::sync::Mutex<Receiver<Event>>,
    readers: Vec<JoinHandle<()>>,
    started: Instant,
    ready: bool,
    heartbeat: Instant,
    #[cfg(target_os = "linux")]
    group: LinuxGroup,
    #[cfg(windows)]
    job: Job,
}
impl Page {
    pub fn open(executable: &Path, init: Init) -> Result<Self, String> {
        init.options.validate()?;
        // Readability/format/budgets fail before launching the webview. The child
        // repeats this check and serves immutable loaded bytes thereafter.
        crate::Assets::load(&init)?;
        let init = encode(&init, MAX_INIT)?;
        let mut command = Command::new(executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("browser companion unavailable: {e}"))?;
        // The companion waits for Init before constructing WebKit, so it cannot
        // create renderer descendants before entering the bounded group.
        #[cfg(target_os = "linux")]
        let group = match LinuxGroup::attach(&child) {
            Ok(group) => group,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        #[cfg(windows)]
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut stdin = child.stdin.take().ok_or("browser stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("browser stdout unavailable")?;
        let (input, rx) = mpsc::sync_channel::<Vec<u8>>(QUEUE);
        input
            .try_send(init)
            .map_err(|_| "browser initialization queue failed")?;
        let writer = std::thread::spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if stdin.write_all(&bytes).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        let (tx, output) = mpsc::sync_channel(QUEUE);
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let event = match read_record(&mut reader, MAX_MESSAGE) {
                    Ok(Some(bytes)) => {
                        serde_json::from_slice::<Event>(&bytes).unwrap_or(Event::Error {
                            message: "invalid browser IPC response".into(),
                        })
                    }
                    Ok(None) => {
                        let _ = tx.try_send(Event::Closed);
                        break;
                    }
                    Err(message) => {
                        let _ = tx.try_send(Event::Error { message });
                        break;
                    }
                };
                // A bounded channel limits retained web messages even if the
                // render loop is paused; dropping the pipe forces host shutdown.
                if tx.try_send(event).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            output: std::sync::Mutex::new(output),
            readers: vec![reader, writer],
            started: Instant::now(),
            ready: false,
            heartbeat: Instant::now(),
            #[cfg(target_os = "linux")]
            group,
            #[cfg(windows)]
            job,
        })
    }
    pub fn send(&self, input: &Input) -> Result<(), String> {
        self.input
            .as_ref()
            .ok_or("browser closed")?
            .try_send(encode(input, MAX_MESSAGE)?)
            .map_err(|_| "browser backpressure or closed process".into())
    }
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..QUEUE {
            match self.output.lock().unwrap().try_recv() {
                Ok(event) => {
                    if matches!(event, Event::Ready) {
                        self.ready = true;
                        self.heartbeat = Instant::now();
                    }
                    if matches!(event, Event::Heartbeat) {
                        self.heartbeat = Instant::now();
                    } else {
                        events.push(event);
                    }
                }
                Err(_) => break,
            }
        }
        if !self.ready && self.started.elapsed() > Duration::from_secs(20) {
            events.push(Event::Error {
                message: "browser startup deadline exceeded".into(),
            });
        }
        if self.ready && self.heartbeat.elapsed() > Duration::from_secs(5) {
            events.push(Event::Error {
                message: "browser renderer stopped responding".into(),
            });
        }
        if let Some(status) = self.child.try_wait().ok().flatten() {
            if !status.success() {
                events.push(Event::Error {
                    message: "browser companion exited or exceeded its process/memory limit".into(),
                });
            } else if !events
                .iter()
                .any(|e| matches!(e, Event::Closed | Event::Error { .. }))
            {
                events.push(Event::Closed);
            }
        }
        events
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
}
impl Drop for Page {
    fn drop(&mut self) {
        self.input.take();
        #[cfg(target_os = "linux")]
        self.group.terminate();
        #[cfg(unix)]
        {
            // SAFETY: the child owns its newly created process group. Killing
            // that group retires WebKit helper processes as well as the host.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Pipes close when the owned tree exits, waking both bounded workers.
        for worker in self.readers.drain(..) {
            let _ = worker.join();
        }
    }
}

/// A delegated cgroup bounds the complete WebKit process tree. Per-process
/// address-space limits are unsuitable for JIT runtimes with large sparse maps.
#[cfg(target_os = "linux")]
struct LinuxGroup(std::path::PathBuf);
#[cfg(target_os = "linux")]
impl LinuxGroup {
    fn attach(child: &Child) -> Result<Self, String> {
        let current = std::fs::read_to_string("/proc/self/cgroup").map_err(|e| e.to_string())?;
        let path = current
            .lines()
            .find_map(|line| line.strip_prefix("0::/"))
            .ok_or("browser requires cgroup v2 with a delegated memory and pids controller")?;
        let root = std::path::PathBuf::from("/sys/fs/cgroup");
        let current = root.join(path);
        let mut last = "no writable delegated parent".to_string();
        for parent in current
            .ancestors()
            .take_while(|p| p.starts_with(&root) && *p != root)
        {
            let directory = parent.join(format!(
                "skate-browser-{}-{}",
                std::process::id(),
                child.id()
            ));
            match std::fs::create_dir(&directory) {
                Ok(()) => {}
                Err(error) => {
                    last = error.to_string();
                    continue;
                }
            }
            let group = Self(directory);
            let configure = || -> std::io::Result<()> {
                // No controller is enabled or changed on a caller-owned parent.
                // Missing delegation fails this attempt before adding a process.
                for (name, value) in [
                    ("memory.max", "1073741824"),
                    ("memory.swap.max", "0"),
                    ("memory.oom.group", "1"),
                    ("pids.max", "128"),
                ] {
                    let path = group.0.join(name);
                    if !path.is_file() {
                        return Err(std::io::Error::other(format!("missing {name} controller")));
                    }
                    std::fs::write(path, value)?;
                }
                std::fs::write(group.0.join("cgroup.procs"), child.id().to_string())
            };
            match configure() {
                Ok(()) => return Ok(group),
                Err(error) => last = error.to_string(),
            }
        }
        Err(format!(
            "browser needs a writable delegated cgroup v2 with memory/pids controllers (1 GiB budget): {last}"
        ))
    }
    fn terminate(&self) {
        let _ = std::fs::write(self.0.join("cgroup.kill"), "1");
    }
}
#[cfg(target_os = "linux")]
impl Drop for LinuxGroup {
    fn drop(&mut self) {
        self.terminate();
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for Job {}
#[cfg(windows)]
unsafe impl Sync for Job {}
#[cfg(windows)]
impl Job {
    fn attach(child: &Child) -> Result<Self, String> {
        use std::{
            mem::{size_of, zeroed},
            os::windows::io::AsRawHandle,
        };
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        // SAFETY: valid owned process handle; correctly sized JobObject info.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err("browser job creation failed".into());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_JOB_MEMORY;
            info.JobMemoryLimit = 1024 * 1024 * 1024;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0
            {
                CloseHandle(handle);
                return Err("browser process job assignment failed".into());
            }
            Ok(Self(handle))
        }
    }
    fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
