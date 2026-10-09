use serde_json::json;
use skate_accounts::{ClientCredentials, ClientSession, admin_request, initialize, login_client};
use skate_net::{
    dedicated,
    lobby::{Info, Session},
};
use skate_server::{Host, Map, Options};
use std::{
    fs,
    net::UdpSocket,
    path::PathBuf,
    time::{Duration, Instant},
};
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Client {
    socket: UdpSocket,
    session: Session,
    crypto: ClientSession,
}
fn client(config: &ClientCredentials, map: u64, spoof: bool) -> (String, Client) {
    let (bundle, crypto) = login_client(config, Duration::from_secs(5)).unwrap();
    let token = bundle.token;
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let session = Session::dedicated_client(
        dedicated::SESSION,
        Info {
            id: if spoof {
                crypto.actor.wrapping_add(1)
            } else {
                crypto.actor
            },
            map,
            rig: 22,
            physics: 33,
            appearance: 44,
        },
        1,
    );
    (
        token,
        Client {
            socket,
            session,
            crypto,
        },
    )
}
fn step(host: &mut Host, client: &mut Client, start: Instant) {
    let now = start.elapsed().as_millis() as u64;
    for packet in client.session.service(now) {
        client
            .socket
            .send_to(
                &client.crypto.encode(&packet.data).unwrap(),
                host.local_addr().unwrap(),
            )
            .unwrap();
    }
    host.step().unwrap();
    let mut bytes = [0; 2048];
    while let Ok((n, from)) = client.socket.recv_from(&mut bytes) {
        assert_eq!(from, host.local_addr().unwrap());
        if let Ok(plain) = client.crypto.decode(&bytes[..n]) {
            client.session.receive(1, &plain, now);
        }
    }
}
#[test]
fn verified_identity_is_required_and_real_admin_kick_revokes_live_udp() {
    let temp =
        Temp(std::env::temp_dir().join(format!("skate-account-host-{}", std::process::id())));
    fs::create_dir(&temp.0).unwrap();
    let auth = temp.0.join("auth");
    initialize(&auth, "administrator", "test-password-12345").unwrap();
    let config_file = temp.0.join("accounts.json");
    fs::write(&config_file,serde_json::to_vec(&json!({"database":"auth/accounts.sqlite3","bind":"127.0.0.1:0","certificate":"auth/certificate.pem","key":"auth/private-key.pem"})).unwrap()).unwrap();
    let mut host = Host::bind(Options {
        bind: "127.0.0.1:0".parse().unwrap(),
        session: dedicated::SESSION,
        max_players: 16,
        map: Map::TestWorld, locations:None,
        resources: None,
        accounts: Some(config_file),
        operations: None,
    })
    .unwrap();
    let password = auth.join("password");
    fs::write(&password, "test-password-12345").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&password, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let config = ClientCredentials {
        endpoint: format!(
            "https://localhost:{}",
            host.account_address().unwrap().port()
        ),
        ca_certificate: auth.join("certificate.pem"),
        username: "administrator".into(),
        password_file: password,
    };
    let start = Instant::now();
    let (_, mut spoof) = client(&config, host.map_fingerprint(), true);
    for _ in 0..20 {
        step(&mut host, &mut spoof, start);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        host.player_count(),
        0,
        "even a valid login cannot select someone else's actor ID"
    );
    let plain = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut unprotected = Session::dedicated_client(
        dedicated::SESSION,
        Info {
            id: 71,
            map: host.map_fingerprint(),
            rig: 22,
            physics: 33,
            appearance: 44,
        },
        1,
    );
    for packet in unprotected.service(0) {
        plain
            .send_to(&packet.data, host.local_addr().unwrap())
            .unwrap();
    }
    host.step().unwrap();
    assert_eq!(
        host.player_count(),
        0,
        "account-required hosts reject plaintext UDP"
    );
    let (token, mut accepted) = client(&config, host.map_fingerprint(), false);
    let deadline = Instant::now() + Duration::from_secs(3);
    while host.player_count() == 0 {
        step(&mut host, &mut accepted, start);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(host.player_count(), 1);
    std::thread::sleep(Duration::from_millis(260));
    step(&mut host, &mut accepted, start);
    let status = admin_request(
        &config,
        &token,
        "/v1/admin/status",
        None,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(
        status["players"][0]["account_id"],
        accepted.crypto.account_id
    );
    let queued = admin_request(
        &config,
        &token,
        "/v1/admin/actions",
        Some(json!({"kind":"resource_restart","resource":"missing"})),
        Duration::from_secs(5),
    )
    .unwrap();
    let ticket = queued["ticket"].as_u64().unwrap();
    assert_eq!(host.player_count(), 1);
    for _ in 0..3 {
        host.step().unwrap();
    }
    let result = admin_request(
        &config,
        &token,
        &format!("/v1/admin/actions/{ticket}"),
        None,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(
        result["ok"], false,
        "resource action must report the actual unconfigured-host error"
    );
    // Keep an independent administrator login, because kicking a gameplay session
    // revokes that session's HTTP bearer and AEAD key together.
    let (admin, _) = login_client(&config, Duration::from_secs(5)).unwrap();
    let queued = admin_request(
        &config,
        &admin.token,
        "/v1/admin/actions",
        Some(json!({"kind":"kick","actor":accepted.crypto.actor.to_string()})),
        Duration::from_secs(5),
    )
    .unwrap();
    let ticket = queued["ticket"].as_u64().unwrap();
    for _ in 0..3 {
        host.step().unwrap();
    }
    assert_eq!(host.player_count(), 0);
    let result = admin_request(
        &config,
        &admin.token,
        &format!("/v1/admin/actions/{ticket}"),
        None,
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(result["ok"], true);
    assert!(
        admin_request(
            &config,
            &token,
            "/v1/admin/status",
            None,
            Duration::from_secs(5)
        )
        .is_err()
    );
    for _ in 0..20 {
        step(&mut host, &mut accepted, start);
    }
    assert_eq!(
        host.player_count(),
        0,
        "kicked AEAD credentials cannot reconnect"
    );
    let audit = admin_request(
        &config,
        &admin.token,
        "/v1/admin/audit",
        None,
        Duration::from_secs(5),
    )
    .unwrap();
    assert!(
        audit
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["action"] == "host.queued")
    );
}

#[test]
fn private_settings_actions_are_authorized_redacted_and_rechecked_over_tls() {
    let temp=Temp(std::env::temp_dir().join(format!("skate-settings-admin-{}",std::process::id())));fs::create_dir(&temp.0).unwrap();
    let auth=temp.0.join("auth");initialize(&auth,"administrator","test-password-12345").unwrap();
    let config_file=temp.0.join("accounts.json");
    fs::write(&config_file,serde_json::to_vec(&json!({"database":"auth/accounts.sqlite3","bind":"127.0.0.1:0","certificate":"auth/certificate.pem","key":"auth/private-key.pem"})).unwrap()).unwrap();
    let resource=temp.0.join("resources/settings");fs::create_dir_all(&resource).unwrap();
    fs::write(resource.join("resource.json"),serde_json::to_vec(&json!({"format":1,"api":1,"id":"settings","version":"1.0.0","language":"lua","server_scripts":["main.lua"],"settings":{"private_note":{"type":"string","default":"synthetic-private-default","max_bytes":128,"visibility":"private"}}})).unwrap()).unwrap();
    fs::write(resource.join("main.lua"),"return {}").unwrap();
    let resources=temp.0.join("server.json");fs::write(&resources,serde_json::to_vec(&json!({"root":"resources","storage":"store","ensure":["settings"]})).unwrap()).unwrap();
    let mut host=Host::bind(Options {bind:"127.0.0.1:0".parse().unwrap(),session:dedicated::SESSION,max_players:2,map:Map::TestWorld, locations:None,resources:Some(resources),accounts:Some(config_file),operations:None}).unwrap();
    let password=auth.join("password");fs::write(&password,"test-password-12345").unwrap();
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;fs::set_permissions(&password,fs::Permissions::from_mode(0o600)).unwrap();}
    let mut config=ClientCredentials {endpoint:format!("https://localhost:{}",host.account_address().unwrap().port()),ca_certificate:auth.join("certificate.pem"),username:"administrator".into(),password_file:password};
    let (administrator,_)=login_client(&config,Duration::from_secs(5)).unwrap();
    let call=|token:&str,path:&str,value:Option<serde_json::Value>|admin_request(&config,token,path,value,Duration::from_secs(5));
    let audit_account=call(&administrator.token,"/v1/admin/accounts",Some(json!({"username":"auditor","password":"test-password-12345"}))).unwrap();
    call(&administrator.token,"/v1/admin/roles",Some(json!({"role":"viewer"}))).unwrap();
    for permission in ["status.read","audit.read"] {call(&administrator.token,"/v1/admin/role-permission",Some(json!({"role":"viewer","permission":permission,"grant":true}))).unwrap();}
    call(&administrator.token,"/v1/admin/account-role",Some(json!({"account":audit_account["id"],"role":"viewer","grant":true}))).unwrap();
    let ticket=call(&administrator.token,"/v1/admin/actions",Some(json!({"kind":"settings_read","resource":"settings"}))).unwrap()["ticket"].as_u64().unwrap();
    for _ in 0..3{host.step().unwrap();}
    let result=call(&administrator.token,&format!("/v1/admin/actions/{ticket}"),None).unwrap();
    assert!(result["value"].as_str().unwrap().contains("synthetic-private-default"));
    let set=call(&administrator.token,"/v1/admin/actions",Some(json!({"kind":"settings_set","resource":"settings","key":"private_note","value":"synthetic-updated-secret"}))).unwrap()["ticket"].as_u64().unwrap();
    for _ in 0..3{host.step().unwrap();}
    assert_eq!(call(&administrator.token,&format!("/v1/admin/actions/{set}"),None).unwrap()["ok"],true);
    config.username="auditor".into();let (auditor,_)=login_client(&config,Duration::from_secs(5)).unwrap();
    assert!(admin_request(&config,&auditor.token,&format!("/v1/admin/actions/{ticket}"),None,Duration::from_secs(5)).is_err(),"audit.read alone cannot read private action results");
    assert!(admin_request(&config,&auditor.token,"/v1/admin/actions",Some(json!({"kind":"settings_set","resource":"settings","key":"private_note","value":"unauthorized"})),Duration::from_secs(5)).is_err());
    let status=admin_request(&config,&auditor.token,"/v1/admin/status",None,Duration::from_secs(5)).unwrap().to_string();
    let audit=admin_request(&config,&auditor.token,"/v1/admin/audit",None,Duration::from_secs(5)).unwrap().to_string();
    for secret in ["synthetic-private-default","synthetic-updated-secret"] {assert!(!status.contains(secret));assert!(!audit.contains(secret));}
    // Permission is rechecked at execution, not only when a ticket was issued.
    admin_request(&config,&administrator.token,"/v1/admin/role-permission",Some(json!({"role":"viewer","permission":"server.manage","grant":true})),Duration::from_secs(5)).unwrap();
    let denied=admin_request(&config,&auditor.token,"/v1/admin/actions",Some(json!({"kind":"capacity","players":4})),Duration::from_secs(5)).unwrap()["ticket"].as_u64().unwrap();
    admin_request(&config,&administrator.token,"/v1/admin/role-permission",Some(json!({"role":"viewer","permission":"server.manage","grant":false})),Duration::from_secs(5)).unwrap();
    for _ in 0..3{host.step().unwrap();}
    assert_eq!(admin_request(&config,&administrator.token,&format!("/v1/admin/actions/{denied}"),None,Duration::from_secs(5)).unwrap()["ok"],false);
}
