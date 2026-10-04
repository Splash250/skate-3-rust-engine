use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[test]
fn executable_help_and_configuration_errors_have_useful_exit_statuses() {
    let help = Command::new(env!("CARGO_BIN_EXE_skate-server"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("--max-players")
    );
    let missing_map = Command::new(env!("CARGO_BIN_EXE_skate-server"))
        .output()
        .unwrap();
    assert_eq!(missing_map.status.code(), Some(2));
    assert!(
        String::from_utf8(missing_map.stderr)
            .unwrap()
            .contains("Select --map")
    );
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn executable_starts_headlessly_on_an_ephemeral_udp_port() {
    let mut server = Running(
        Command::new(env!("CARGO_BIN_EXE_skate-server"))
            .args(["--test-world", "--bind", "127.0.0.1:0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let stdout = server.0.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let result = reader.read_line(&mut line);
        let _ = send.send((result, line));
        // Keep the pipe open until teardown so status logging cannot trigger a
        // broken-pipe panic in the child while this test checks its liveness.
        for line in reader.lines() {
            if line.is_err() {
                break;
            }
        }
    });
    let (result, line) = receive
        .recv_timeout(Duration::from_secs(5))
        .expect("Server did not announce startup");
    result.unwrap();
    assert!(
        line.contains("Dedicated server listening on 127.0.0.1:"),
        "{line}"
    );
    assert!(line.contains("players=0/16"), "{line}");
    assert!(!line.contains("127.0.0.1:0 "), "{line}");
    assert!(server.0.try_wait().unwrap().is_none());
    drop(server);
    reader.join().unwrap();
}
