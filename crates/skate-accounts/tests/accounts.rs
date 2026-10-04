use serde_json::{Value, json};
use skate_accounts::*;
use std::{
    fs,
    net::UdpSocket,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
static NEXT: AtomicUsize = AtomicUsize::new(1);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skate-accounts-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
const PASSWORD: &str = "test-password-12345";
fn fixture() -> (Temp, AccountStore, SessionCredentials, VerifiedSession) {
    let temp = Temp::new();
    let store = AccountStore::open(temp.0.join("accounts.sqlite3")).unwrap();
    store.bootstrap_admin("administrator", PASSWORD).unwrap();
    let bundle = store.login("administrator", PASSWORD).unwrap();
    let session = store.authenticate(&bundle.token).unwrap();
    (temp, store, bundle, session)
}
#[test]
fn persistent_roles_inheritance_revocation_and_audit() {
    let (temp, store, _bundle, admin) = fixture();
    assert!(
        AccountStore::open(temp.0.join("accounts.sqlite3")).is_err(),
        "two authorities must not reset the same session registry"
    );
    let player = store.create_account(&admin, "player", PASSWORD).unwrap();
    assert!(store.login("player", "invalid-password").is_err());
    let credentials = store.login("player", PASSWORD).unwrap();
    let user = store.authenticate(&credentials.token).unwrap();
    assert!(store.accounts(&user).is_err());
    store.create_role(&admin, "viewer", None).unwrap();
    store
        .role_permission(&admin, "viewer", "status.read", true)
        .unwrap();
    store
        .create_role(&admin, "moderator", Some("viewer"))
        .unwrap();
    store
        .assign_role(&admin, &player.id, "moderator", true)
        .unwrap();
    assert!(user.permits("status.read"));
    assert!(
        store
            .role_parent(&admin, "viewer", "moderator", true)
            .is_err()
    );
    store
        .role_permission(&admin, "viewer", "status.read", false)
        .unwrap();
    assert!(!user.permits("status.read"));
    store.whitelist_mode(&admin, true).unwrap();
    assert!(!user.is_active());
    assert!(store.login("player", PASSWORD).is_err());
    store.whitelist(&admin, &player.id, true).unwrap();
    let credentials = store.login("player", PASSWORD).unwrap();
    let user = store.authenticate(&credentials.token).unwrap();
    store
        .ban(&admin, &player.id, true, "test moderation")
        .unwrap();
    assert!(!user.is_active());
    assert!(store.login("player", PASSWORD).is_err());
    store
        .ban(&admin, &player.id, false, "appeal accepted")
        .unwrap();
    let credentials = store.login("player", PASSWORD).unwrap();
    let user = store.authenticate(&credentials.token).unwrap();
    store.revoke(&admin, &player.id).unwrap();
    assert!(!user.is_active());
    let audit = store.audit_log(&admin, 0).unwrap();
    assert!(audit.iter().any(|r| r.action == "role.permission"));
    assert!(audit.iter().any(|r| r.action == "account.ban"));
    assert!(audit.iter().all(|r| !r.detail.contains(PASSWORD)));
    let other = Temp::new();
    let other = AccountStore::open(other.0.join("accounts.sqlite3")).unwrap();
    assert!(
        other.accounts(&admin).is_err(),
        "verified identities are bound to their issuing authority"
    );
    drop(store);
    assert!(!admin.is_active());
    let reopened = AccountStore::open(temp.0.join("accounts.sqlite3")).unwrap();
    let credentials = reopened.login("administrator", PASSWORD).unwrap();
    let admin = reopened.authenticate(&credentials.token).unwrap();
    let accounts = reopened.accounts(&admin).unwrap();
    assert_eq!(
        accounts.iter().find(|a| a.username == "player").unwrap().id,
        player.id
    );
    assert!(
        reopened
            .roles(&admin)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "moderator")
    );
    let conn = rusqlite::Connection::open(temp.0.join("accounts.sqlite3")).unwrap();
    let stored: String = conn
        .query_row(
            "SELECT password_hash FROM accounts WHERE username='player'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stored.starts_with("$argon2id$"));
    assert!(!stored.contains(PASSWORD));
}
#[test]
fn encrypted_datagrams_replay_direction_reorder_tamper_revoke() {
    let (_temp, store, bundle, admin) = fixture();
    let mut client = ClientSession::new(bundle).unwrap();
    let mut server = ServerTransport::new(store.clone()).unwrap();
    assert!(ServerTransport::new(store.clone()).is_err());
    let client_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let server_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    server_socket
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let first = client.encode(b"synthetic gameplay").unwrap();
    assert!(!first.windows(18).any(|w| w == b"synthetic gameplay"));
    client_socket
        .send_to(&first, server_socket.local_addr().unwrap())
        .unwrap();
    let mut bytes = [0; MAX_DATAGRAM];
    let (n, _) = server_socket.recv_from(&mut bytes).unwrap();
    let (session, payload) = server.decode(&bytes[..n]).unwrap();
    assert_eq!(payload, b"synthetic gameplay");
    assert_eq!(session.actor(), client.actor);
    assert!(server.decode(&first).is_err());
    assert!(
        client.decode(&first).is_err(),
        "directional nonce domains prevent packet reflection"
    );
    let second = client.encode(b"two").unwrap();
    let third = client.encode(b"three").unwrap();
    assert_eq!(server.decode(&third).unwrap().1, b"three");
    assert_eq!(server.decode(&second).unwrap().1, b"two");
    let good = client.encode(b"four").unwrap();
    let mut bad = good.clone();
    bad[35] ^= 1;
    assert!(server.decode(&bad).is_err());
    assert_eq!(
        server.decode(&good).unwrap().1,
        b"four",
        "forged packets cannot advance replay window"
    );
    let reply = server.encode(&session, b"authority reply").unwrap();
    assert_eq!(client.decode(&reply).unwrap(), b"authority reply");
    assert!(client.decode(&reply).is_err());
    assert!(client.encode(&vec![0; 1201]).is_err());
    store.revoke(&admin, admin.account_id()).unwrap();
    assert!(
        server
            .decode(&client.encode(b"after revoke").unwrap())
            .is_err()
    );
    assert!(server.encode(&session, b"after revoke").is_err());
}
fn request(
    endpoint: &str,
    certificate: &[u8],
    token: Option<&str>,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .add_root_certificate(reqwest::Certificate::from_pem(certificate).unwrap())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let mut request = if body.is_some() {
            client.post(format!("{endpoint}{path}"))
        } else {
            client.get(format!("{endpoint}{path}"))
        };
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(serde_json::to_vec(&body).unwrap());
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let body = response.bytes().await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    })
}
#[test]
fn actual_tls_login_admin_queue_permissions_and_certificate_validation() {
    let temp = Temp::new();
    let directory = temp.0.join("initialized");
    initialize(&directory, "administrator", PASSWORD).unwrap();
    assert!(initialize(&directory, "administrator", PASSWORD).is_err());
    let store = AccountStore::open(directory.join("accounts.sqlite3")).unwrap();
    let bridge = AdminBridge::new();
    let server = AdminServer::bind(
        store.clone(),
        AdminConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            certificate: directory.join("certificate.pem"),
            key: directory.join("private-key.pem"),
        },
        bridge.clone(),
    )
    .unwrap();
    let endpoint = format!("https://localhost:{}", server.local_addr().port());
    let password_file = directory.join("password");
    fs::write(&password_file, PASSWORD).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&password_file, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let config = ClientCredentials {
        endpoint: endpoint.clone(),
        ca_certificate: directory.join("certificate.pem"),
        username: "administrator".into(),
        password_file,
    };
    let (bundle, _codec) = login_client(&config, Duration::from_secs(5)).unwrap();
    let cert = fs::read(&config.ca_certificate).unwrap();
    let token = &bundle.token;
    assert_eq!(
        request(&endpoint, &cert, None, "/v1/admin/status", None).0,
        401
    );
    bridge
        .set_status(
            json!({"attached":true,"players":[],"resources":[{"id":"example","state":"started"}]}),
        )
        .unwrap();
    assert_eq!(
        request(&endpoint, &cert, Some(token), "/v1/admin/status", None).1["attached"],
        true
    );
    let (status, result) = request(
        &endpoint,
        &cert,
        Some(token),
        "/v1/admin/actions",
        Some(json!({"kind":"resource_restart","resource":"example"})),
    );
    assert_eq!(status, 202);
    let ticket = result["ticket"].as_u64().unwrap();
    assert_eq!(
        request(
            &endpoint,
            &cert,
            Some(token),
            &format!("/v1/admin/actions/{ticket}"),
            None
        )
        .1["state"],
        "queued"
    );
    let command = bridge.try_command().unwrap();
    assert!(command.session.permits(command.action.permission()));
    assert_eq!(command.ticket, ticket);
    bridge
        .complete(
            ticket,
            Err(Error {
                code: "missing".into(),
                message: "example is not installed".into(),
            }),
        )
        .unwrap();
    let result = request(
        &endpoint,
        &cert,
        Some(token),
        &format!("/v1/admin/actions/{ticket}"),
        None,
    )
    .1;
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["code"], "missing");
    // A permanent size failure must complete its ticket, so the host's retry
    // FIFO can advance to the next command instead of treating it as Busy.
    for (message, expected_ok) in [("x".repeat(4097), false), ("Restarted".into(), true)] {
        let (status, queued) = request(
            &endpoint,
            &cert,
            Some(token),
            "/v1/admin/actions",
            Some(json!({"kind":"resource_restart","resource":"example"})),
        );
        assert_eq!(status, 202);
        let ticket = queued["ticket"].as_u64().unwrap();
        assert_eq!(bridge.try_command().unwrap().ticket, ticket);
        bridge.complete(ticket, Ok(message)).unwrap();
        let completed = request(
            &endpoint,
            &cert,
            Some(token),
            &format!("/v1/admin/actions/{ticket}"),
            None,
        ).1;
        assert_eq!(completed["state"], "completed");
        assert_eq!(completed["ok"], expected_ok);
        if !expected_ok {
            assert_eq!(completed["error"]["code"], "limit");
        }
    }
    let (status, player) = request(
        &endpoint,
        &cert,
        Some(token),
        "/v1/admin/accounts",
        Some(json!({"username":"ordinary","password":PASSWORD})),
    );
    assert_eq!(status, 201);
    let ordinary = store.login("ordinary", PASSWORD).unwrap();
    assert_eq!(
        request(
            &endpoint,
            &cert,
            Some(&ordinary.token),
            "/v1/admin/actions",
            Some(json!({"kind":"kick","actor":"1"}))
        )
        .0,
        403
    );
    assert_eq!(
        request(
            &endpoint,
            &cert,
            Some(token),
            "/v1/admin/ban",
            Some(json!({"account":player["id"],"banned":true,"reason":"integration test"}))
        )
        .0,
        200
    );
    assert!(store.authenticate(&ordinary.token).is_err());
    let other = temp.0.join("other");
    initialize(&other, "otheradmin", PASSWORD).unwrap();
    let mut invalid = config.clone();
    invalid.ca_certificate = other.join("certificate.pem");
    assert!(login_client(&invalid, Duration::from_secs(2)).is_err());
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let admin = store.authenticate(token).unwrap();
        if store
            .audit_log(&admin, 0)
            .unwrap()
            .iter()
            .any(|r| r.action == "host.completed")
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(server);
}
