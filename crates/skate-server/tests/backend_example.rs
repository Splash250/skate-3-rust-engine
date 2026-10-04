//! Exercise the shipped progression script through the real server process and
//! console. No synthetic replacement for its Lua/backend integration is used.
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "skate-progression-example-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let resource = root.join("resources/persistent-progression");
        std::fs::create_dir_all(&resource).unwrap();
        std::fs::write(
            resource.join("resource.json"),
            include_str!("../../../resources/persistent-progression/resource.json"),
        )
        .unwrap();
        std::fs::write(
            resource.join("server.lua"),
            include_str!("../../../resources/persistent-progression/server.lua"),
        )
        .unwrap();
        std::fs::write(root.join("server.json"), serde_json::to_vec(&json!({
            "root":"resources", "storage":"store", "ensure":["persistent-progression"],
            "grants":{"persistent-progression":["resource.database","resource.events","resource.commands"]}
        })).unwrap()).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct ServerProcess {
    child: Child,
    input: ChildStdin,
    lines: mpsc::Receiver<String>,
    reader: Option<JoinHandle<()>>,
}
impl ServerProcess {
    fn start(config: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_skate-server"))
            .args(["--test-world", "--bind", "127.0.0.1:0", "--resources"])
            .arg(config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::sync_channel(256);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut process = Self {
            child,
            input,
            lines,
            reader: Some(reader),
        };
        process.expect("Progression ready");
        process
    }
    fn command(&mut self, command: &str) {
        writeln!(self.input, "{command}").unwrap();
        self.input.flush().unwrap();
    }
    fn expect(&mut self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut observed = Vec::new();
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|e| {
                    panic!("waiting for {expected:?}: {e}; observed: {observed:?}")
                });
            if line.contains(expected) {
                return;
            }
            observed.push(line);
        }
    }
    fn inspect(&mut self, balance: u32, inventory: u32) {
        self.command("command progress_inspect local-account");
        self.expect(&format!("Balance: {balance}"));
        self.expect(&format!("Inventory deck_blue: {inventory}"));
    }
    fn stop(&mut self) {
        self.command("quit");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "server did not shut down");
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}
impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The test writes a bounded command set, producing fewer than 256 lines.
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn bundled_progression_survives_resource_and_process_restart_and_rolls_back() {
    let fixture = Fixture::new();
    let config = fixture.0.join("server.json");
    let mut first = ServerProcess::start(&config);
    first.command("command progress_award local-account 100");
    first.expect("award_1 committed");
    first.command("command progress_buy local-account");
    first.expect("buy_2 committed");
    first.inspect(75, 1);
    first.command("restart persistent-progression");
    first.expect("Progression ready");
    first.inspect(75, 1);
    first.stop();

    let mut second = ServerProcess::start(&config);
    second.inspect(75, 1);
    for number in 2..=4 {
        second.command("command progress_buy local-account");
        second.expect(&format!("buy_{number} committed"));
    }
    second.command("command progress_buy local-account");
    second.expect("SQL constraint failed; transaction rolled back");
    second.inspect(0, 4);
    second.stop();
}
