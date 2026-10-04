//! Real external supervisor, HTTPS control, account-store snapshot and recovery.
#![cfg(unix)]
use serde_json::json;
use skate_accounts::{ClientCredentials, admin_request, initialize, login_client};
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Fixture {
    root: PathBuf,
    process: Option<Child>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = &mut self.process {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "operational integration timed out"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn login(config: &ClientCredentials) -> String {
    let mut token = None;
    wait(|| {
        token = login_client(config, Duration::from_secs(1))
            .ok()
            .map(|v| v.0.token);
        token.is_some()
    });
    token.unwrap()
}
#[test]
fn supervisor_restores_account_roles_recovers_crash_and_honors_shutdown() {
    let root = std::env::temp_dir().join(format!("skate-supervised-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let mut fixture = Fixture {
        root: root.clone(),
        process: None,
    };
    fs::create_dir(root.join("data")).unwrap();
    let auth = root.join("data/auth");
    initialize(&auth, "administrator", "test-password-12345").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    fs::write(root.join("accounts.json"),serde_json::to_vec(&json!({"database":"data/auth/accounts.sqlite3","bind":format!("127.0.0.1:{port}"),"certificate":"data/auth/certificate.pem","key":"data/auth/private-key.pem"})).unwrap()).unwrap();
    let password = root.join("password");
    fs::write(&password, "test-password-12345").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&password, fs::Permissions::from_mode(0o600)).unwrap();
    let config = ClientCredentials {
        endpoint: format!("https://localhost:{port}"),
        ca_certificate: auth.join("certificate.pem"),
        username: "administrator".into(),
        password_file: password,
    };
    let supervisor =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/server_supervisor.py");
    fs::write(root.join("supervisor.json"),serde_json::to_vec(&json!({"executable":env!("CARGO_BIN_EXE_skate-server"),"arguments":["--test-world","--bind","127.0.0.1:0","--accounts","accounts.json"],"data_root":"data","startup_seconds":10,"hang_seconds":5,"grace_seconds":1,"backoff_seconds":0.1,"max_restarts":2})).unwrap()).unwrap();
    fixture.process = Some(
        Command::new("python3")
            .arg(supervisor)
            .arg(root.join("supervisor.json"))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut token = login(&config);
    let call = |token: &str, path: &str, value: Option<serde_json::Value>| {
        admin_request(&config, token, path, value, Duration::from_secs(5)).unwrap()
    };
    call(
        &token,
        "/v1/admin/roles",
        Some(json!({"role":"before_backup"})),
    );
    call(
        &token,
        "/v1/admin/actions",
        Some(json!({"kind":"backup","snapshot":"known-good"})),
    );
    wait(|| root.join(".backups/known-good/manifest.json").is_file());
    wait(|| login_client(&config, Duration::from_secs(1)).is_ok_and(|v| v.0.token != token));
    token = login(&config);
    call(
        &token,
        "/v1/admin/roles",
        Some(json!({"role":"after_backup"})),
    );
    call(
        &token,
        "/v1/admin/actions",
        Some(json!({"kind":"restore","snapshot":"known-good"})),
    );
    wait(|| {
        fs::read_dir(&root).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("data.previous-")
        })
    });
    token = login(&config);
    let roles = call(&token, "/v1/admin/roles", None).to_string();
    assert!(roles.contains("before_backup"));
    assert!(!roles.contains("after_backup"));
    let status_path = root.join(".supervisor/status.json");
    let status: serde_json::Value =
        serde_json::from_slice(&fs::read(&status_path).unwrap()).unwrap();
    let pid = status["pid"].as_u64().unwrap();
    assert!(
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    wait(|| {
        fs::read(&status_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some_and(|v| {
                v["state"] == "running" && v["pid"].as_u64().is_some_and(|new| new != pid)
            })
    });
    token = login(&config);
    assert!(
        call(&token, "/v1/admin/roles", None)
            .to_string()
            .contains("before_backup")
    );
    call(
        &token,
        "/v1/admin/actions",
        Some(
            json!({"kind":"maintenance","reason":"Operator shutdown","delay_ms":0,"restart":false}),
        ),
    );
    let mut exit = None;
    wait(|| {
        exit = fixture.process.as_mut().unwrap().try_wait().unwrap();
        exit.is_some()
    });
    assert!(exit.unwrap().success());
    let status: serde_json::Value =
        serde_json::from_slice(&fs::read(status_path).unwrap()).unwrap();
    assert_eq!(status["state"], "operator_stopped");
}
