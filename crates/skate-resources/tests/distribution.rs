use skate_resources::{Cache, HttpServer, Limits, build_set, download_set};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "skate-resource-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fixture(t: &Temp, body: &str) {
    let p = t.0.join("resources/challenge");
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join("resource.json"),r#"{"format":1,"api":1,"id":"challenge","version":"1.0.0","language":"lua","client_scripts":["client.lua"],"server_scripts":["private.lua"],"files":["ui.txt"],"capabilities":["engine.ui"]}"#).unwrap();
    fs::write(p.join("client.lua"), body).unwrap();
    fs::write(p.join("ui.txt"), "shared UI").unwrap();
    fs::write(p.join("private.lua"), "private server rules").unwrap();
}
fn selection() -> BTreeMap<String, u64> {
    BTreeMap::from([("challenge".into(), 1)])
}
fn server(t: &Temp) -> HttpServer {
    HttpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        build_set(&t.0.join("resources"), &selection()).unwrap(),
    )
    .unwrap()
}
#[test]
fn cold_warm_restart_other_server_and_changed_files_reuse_verified_bytes() {
    let t = Temp::new();
    fixture(&t, "return {}");
    let published = build_set(&t.0.join("resources"), &selection()).unwrap();
    let revision = published.set.revision.clone();
    assert!(
        published.set.resources[0]
            .manifest
            .server_scripts
            .is_empty()
    );
    assert!(!published.set.resources[0].files.contains_key("private.lua"));
    let one = HttpServer::bind("127.0.0.1:0".parse().unwrap(), published.clone()).unwrap();
    let two = HttpServer::bind("127.0.0.1:0".parse().unwrap(), published).unwrap();
    let cache_root = t.0.join("cache");
    let cancel = AtomicBool::new(false);
    {
        let cache = Cache::open(&cache_root, Limits::default()).unwrap();
        let first =
            download_set(one.local_addr(), &revision, &cache, "server-one", &cancel).unwrap();
        assert_eq!(first.downloaded_bytes, 18);
        assert_eq!(
            fs::read_to_string(first.roots["challenge"].join("client.lua")).unwrap(),
            "return {}"
        );
        let second =
            download_set(one.local_addr(), &revision, &cache, "server-one", &cancel).unwrap();
        assert_eq!(second.downloaded_bytes, 0);
    }
    let cache = Cache::open(&cache_root, Limits::default()).unwrap();
    let other = download_set(two.local_addr(), &revision, &cache, "server-two", &cancel).unwrap();
    assert_eq!(other.downloaded_bytes, 0);
    fixture(&t, "return {changed=true}");
    let changed = build_set(&t.0.join("resources"), &selection()).unwrap();
    assert_ne!(revision, changed.set.revision);
    let next = changed.set.revision.clone();
    one.replace(changed).unwrap();
    let update = download_set(one.local_addr(), &next, &cache, "server-one", &cancel).unwrap();
    assert_eq!(update.downloaded_bytes, 21);
    assert_eq!(update.reused_bytes, 9);
    assert_eq!(
        fs::read_to_string(other.roots["challenge"].join("client.lua")).unwrap(),
        "return {}"
    );
}
#[test]
fn cache_reuse_survives_a_fresh_client_process() {
    let t = Temp::new();
    fixture(&t, "return {}");
    let set = build_set(&t.0.join("resources"), &selection()).unwrap();
    let revision = set.set.revision.clone();
    let server = HttpServer::bind("127.0.0.1:0".parse().unwrap(), set).unwrap();
    for expected in [18, 0] {
        let output = Command::new(env!("CARGO_BIN_EXE_skate-resource-cache"))
            .arg("fetch")
            .arg(t.0.join("cache"))
            .arg(server.local_addr().to_string())
            .arg(&revision)
            .arg("process-server")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["downloaded_bytes"], expected);
    }
}
#[test]
fn incomplete_corrupt_cancelled_and_oversized_sets_never_publish() {
    let t = Temp::new();
    fixture(&t, "return {}");
    let set = build_set(&t.0.join("resources"), &selection()).unwrap();
    let revision = set.set.revision.clone();
    let server = HttpServer::bind("127.0.0.1:0".parse().unwrap(), set.clone()).unwrap();
    let cache = Cache::open(t.0.join("cache"), Limits::default()).unwrap();
    assert!(
        download_set(
            server.local_addr(),
            &revision,
            &cache,
            "one",
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert!(
        download_set(
            server.local_addr(),
            &"0".repeat(64),
            &cache,
            "one",
            &AtomicBool::new(false)
        )
        .is_err()
    );
    let tiny = Cache::open(
        t.0.join("tiny"),
        Limits {
            max_file_bytes: 4,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(
        download_set(
            server.local_addr(),
            &revision,
            &tiny,
            "one",
            &AtomicBool::new(false)
        )
        .is_err()
    );
    let mut broken = set.clone();
    broken.blobs.values_mut().next().unwrap().push(7);
    assert!(HttpServer::bind("127.0.0.1:0".parse().unwrap(), broken).is_err());
    let fetched = download_set(
        server.local_addr(),
        &revision,
        &cache,
        "one",
        &AtomicBool::new(false),
    )
    .unwrap();
    let digest = &set.set.resources[0].files["client.lua"].digest;
    fs::write(cache.blob_path(digest).unwrap(), "corrupt").unwrap();
    let repaired = download_set(
        server.local_addr(),
        &revision,
        &cache,
        "one",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(repaired.downloaded_bytes, 9);
    fs::write(fetched.roots["challenge"].join("client.lua"), "tampered").unwrap();
    let repaired = download_set(
        server.local_addr(),
        &revision,
        &cache,
        "one",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(repaired.roots["challenge"].join("client.lua")).unwrap(),
        "return {}"
    );
}
#[test]
fn distribution_rejects_private_scripts_and_unknown_http_paths() {
    let t = Temp::new();
    fixture(&t, "return {}");
    let server = server(&t);
    for path in [
        "/private.lua",
        "/blobs/../private.lua",
        "/set?x",
        "/blobs/0000000000000000000000000000000000000000000000000000000000000000",
    ] {
        let mut socket = TcpStream::connect(server.local_addr()).unwrap();
        write!(socket, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut reply = String::new();
        socket.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 404"));
        assert!(!reply.contains("private server rules"));
    }
}
#[test]
fn bounded_audit_preserves_source_and_grant_isolation() {
    let t = Temp::new();
    fixture(&t, "return {}");
    let set = build_set(&t.0.join("resources"), &selection()).unwrap();
    let revision = set.set.revision.clone();
    let server = HttpServer::bind("127.0.0.1:0".parse().unwrap(), set).unwrap();
    let cache = Cache::open(
        t.0.join("cache"),
        Limits {
            max_history: 3,
            ..Limits::default()
        },
    )
    .unwrap();
    let report = download_set(
        server.local_addr(),
        &revision,
        &cache,
        "one",
        &AtomicBool::new(false),
    )
    .unwrap();
    cache
        .record_activation(
            &report.set,
            "one",
            &BTreeMap::from([("challenge".into(), vec!["engine.ui".into()])]),
            None,
        )
        .unwrap();
    cache
        .record_activation(
            &report.set,
            "two",
            &BTreeMap::new(),
            Some("capability denied"),
        )
        .unwrap();
    let entries = cache.inventory().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[1].source, "one");
    assert_eq!(entries[1].grants, vec!["engine.ui"]);
    assert_eq!(entries[2].source, "two");
    assert!(entries[2].grants.is_empty());
    assert_eq!(
        entries[2].activation_error.as_deref(),
        Some("capability denied")
    );
    cache.prune(0).unwrap();
    assert!(cache.inventory().unwrap().len() <= 3);
}
#[cfg(unix)]
#[test]
fn symlinked_content_and_resource_roots_are_rejected() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    fixture(&t, "return {}");
    let client = t.0.join("resources/challenge/client.lua");
    fs::remove_file(&client).unwrap();
    symlink("private.lua", &client).unwrap();
    assert!(build_set(&t.0.join("resources"), &selection()).is_err());
}
#[test]
fn truncated_http_body_cannot_enter_the_cache() {
    use std::net::TcpListener;
    let t = Temp::new();
    let cache = Cache::open(t.0.join("cache"), Limits::default()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = s.read(&mut request);
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{}")
            .unwrap();
    });
    assert!(
        download_set(
            addr,
            &"a".repeat(64),
            &cache,
            "one",
            &AtomicBool::new(false)
        )
        .is_err()
    );
    handle.join().unwrap();
    assert!(cache.inventory().unwrap().is_empty());
}

#[test]
fn mid_transfer_failure_keeps_verified_blobs_and_recovers_without_publishing_a_partial_set() {
    use std::net::TcpListener;
    let t = Temp::new();
    fixture(&t, "return {}");
    let published = build_set(&t.0.join("resources"), &selection()).unwrap();
    let revision = published.set.revision.clone();
    let cache = Cache::open(t.0.join("cache"), Limits::default()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let malicious = published.clone();
    let thread = std::thread::spawn(move || {
        for i in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let body = if i == 0 {
                serde_json::to_vec(&malicious.set).unwrap()
            } else {
                let request = String::from_utf8(request).unwrap();
                let path = request.lines().next().unwrap().split(' ').nth(1).unwrap();
                malicious.blobs[path.strip_prefix("/blobs/").unwrap()].clone()
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream
                .write_all(if i == 2 { &body[..3] } else { &body })
                .unwrap();
        }
    });
    assert!(
        download_set(
            addr,
            &revision,
            &cache,
            "interrupted",
            &AtomicBool::new(false)
        )
        .is_err()
    );
    thread.join().unwrap();
    assert_eq!(fs::read_dir(cache.root().join("sets")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(cache.root().join("blobs")).unwrap().count(), 1);
    let history = cache.inventory().unwrap();
    assert!(!history.last().unwrap().verified);
    assert!(
        history
            .last()
            .unwrap()
            .verification_error
            .as_ref()
            .unwrap()
            .contains("truncated")
    );
    let honest = HttpServer::bind("127.0.0.1:0".parse().unwrap(), published).unwrap();
    let recovered = download_set(
        honest.local_addr(),
        &revision,
        &cache,
        "interrupted",
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(recovered.downloaded_bytes, 9);
    assert_eq!(recovered.reused_bytes, 9);
}
#[test]
fn whole_download_deadline_stops_a_stalled_endpoint() {
    use std::{
        net::TcpListener,
        time::{Duration, Instant},
    };
    let t = Temp::new();
    let cache = Cache::open(t.0.join("cache"), Limits::default()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let _ = socket.read(&mut request);
        std::thread::sleep(Duration::from_millis(400));
    });
    let start = Instant::now();
    let error = skate_resources::download_set_with_timeout(
        addr,
        &"a".repeat(64),
        &cache,
        "slow",
        &AtomicBool::new(false),
        Duration::from_millis(50),
    )
    .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(1));
    thread.join().unwrap();
    assert_eq!(fs::read_dir(cache.root().join("sets")).unwrap().count(), 0);
}
#[test]
fn active_sets_are_pinned_and_failed_activation_does_not_replace_them() {
    let t = Temp::new();
    fixture(&t, "return {}");
    let original = build_set(&t.0.join("resources"), &selection()).unwrap();
    let server = HttpServer::bind("127.0.0.1:0".parse().unwrap(), original.clone()).unwrap();
    let cache = Cache::open(t.0.join("cache"), Limits::default()).unwrap();
    let original = download_set(
        server.local_addr(),
        &original.set.revision,
        &cache,
        "one",
        &AtomicBool::new(false),
    )
    .unwrap();
    cache
        .record_activation(&original.set, "one", &BTreeMap::new(), None)
        .unwrap();
    fixture(&t, "error('startup')");
    let replacement = build_set(&t.0.join("resources"), &selection()).unwrap();
    let next = replacement.set.revision.clone();
    server.replace(replacement).unwrap();
    let failed = download_set(
        server.local_addr(),
        &next,
        &cache,
        "one",
        &AtomicBool::new(false),
    )
    .unwrap();
    cache
        .record_activation(&failed.set, "one", &BTreeMap::new(), Some("startup failed"))
        .unwrap();
    cache.prune(0).unwrap();
    assert!(original.roots["challenge"].join("client.lua").is_file());
    assert!(!failed.roots["challenge"].exists());
    cache.deactivate("one").unwrap();
    cache.prune(0).unwrap();
    assert_eq!(fs::read_dir(cache.root().join("sets")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(cache.root().join("blobs")).unwrap().count(), 0);
}
#[test]
fn disk_budget_accounts_for_materialized_copies_and_history() {
    let t = Temp::new();
    fixture(&t, &"x".repeat(4096));
    let set = build_set(&t.0.join("resources"), &selection()).unwrap();
    let revision = set.set.revision.clone();
    let server = HttpServer::bind("127.0.0.1:0".parse().unwrap(), set).unwrap();
    let cache = Cache::open(
        t.0.join("cache"),
        Limits {
            max_cache_bytes: 7000,
            ..Limits::default()
        },
    )
    .unwrap();
    assert!(
        download_set(
            server.local_addr(),
            &revision,
            &cache,
            "quota",
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(cache.disk_bytes().unwrap() <= 7000);
    assert_eq!(fs::read_dir(cache.root().join("sets")).unwrap().count(), 0);
}
#[test]
fn manifest_shape_and_selected_dependency_files_are_checked_before_publishing() {
    let t = Temp::new();
    fixture(&t, "return {}");
    let root = t.0.join("resources");
    let manifest = root.join("challenge/resource.json");
    let original = fs::read_to_string(&manifest).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&original).unwrap();
    value["unknown"] = "option".into();
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(build_set(&root, &selection()).is_err());
    let mut value: serde_json::Value = serde_json::from_str(&original).unwrap();
    value["dependencies"] = serde_json::json!({"base":"1.0.0"});
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        build_set(&root, &selection())
            .unwrap_err()
            .to_string()
            .contains("base")
    );
    fs::create_dir(root.join("base")).unwrap();
    fs::write(root.join("base/resource.json"),r#"{"format":1,"api":1,"id":"base","version":"1.0.0","language":"lua","shared_scripts":["shared.lua"]}"#).unwrap();
    fs::write(root.join("base/shared.lua"), "return {base=true}").unwrap();
    let set = build_set(&root, &selection()).unwrap();
    assert_eq!(
        set.set
            .resources
            .iter()
            .map(|r| r.manifest.id.as_str())
            .collect::<Vec<_>>(),
        ["base", "challenge"]
    );
    assert_eq!(set.set.resources[0].generation, 1);
    let mut reversed = set.set.resources.clone();
    reversed.reverse();
    let composed =
        skate_resources::PublishedSet::from_resources(reversed, set.blobs.clone()).unwrap();
    assert_eq!(composed.set, set.set);
    let mut extra = set.blobs.clone();
    extra.insert(
        skate_resources::digest_bytes(b"private"),
        b"private".to_vec(),
    );
    assert!(
        skate_resources::PublishedSet::from_resources(set.set.resources.clone(), extra).is_err()
    );
    let mut forged = set.set.clone();
    forged.resources[0]
        .files
        .get_mut("shared.lua")
        .unwrap()
        .size = u64::MAX;
    assert!(forged.validate(Limits::default()).is_err());
    let mut forged = set.set;
    forged.resources[1].manifest.server_scripts = vec!["secret.lua".into()];
    assert!(forged.validate(Limits::default()).is_err());
}

#[test]
fn golden_identities_are_independent_of_native_root_paths_and_server_script_bytes() {
    // Expected constants were derived from explicit portable JSON with a separate
    // hashing harness; this pins the identity contract on Windows and Linux CI.
    let one = Temp::new();
    let two = Temp::new();
    fixture(&one, "return {}");
    fixture(&two, "return {}");
    fs::write(
        two.0.join("resources/challenge/private.lua"),
        "completely different private data",
    )
    .unwrap();
    let a = build_set(&one.0.join("resources"), &selection()).unwrap();
    let b = build_set(&two.0.join("resources"), &selection()).unwrap();
    assert_eq!(a.set, b.set);
    assert_eq!(
        a.set.resources[0].files["client.lua"].digest,
        "2f244d7526f18c5271a030fcadf220d470fe287bdb85ae5c5ee46813e381581b"
    );
    assert_eq!(
        a.set.resources[0].files["ui.txt"].digest,
        "e3f70b2fb1e7230b70f622221a113899ae8ec494d31b495a489be38c8aa3e741"
    );
    assert_eq!(
        a.set.resources[0].content_digest,
        "c9724ea47a3f8e20b510260ad2d329c58a2672ab0d384a77d56b4adb1d243bb4"
    );
    assert_eq!(
        a.set.revision,
        "67fa8ecf5c14c40c4a16b66bfba75788d99bfaa4df3d079244425ff1b7a2df03"
    );
    let newer = build_set(
        &one.0.join("resources"),
        &BTreeMap::from([("challenge".into(), 2)]),
    )
    .unwrap();
    assert_eq!(
        a.set.resources[0].content_digest,
        newer.set.resources[0].content_digest
    );
    assert_ne!(a.set.revision, newer.set.revision);
}

#[test]
fn corrupt_set_metadata_is_isolated_on_reopen_without_losing_verified_blobs() {
    for active in [false, true] {
        let t = Temp::new();
        fixture(&t, "return {}");
        let published = build_set(&t.0.join("resources"), &selection()).unwrap();
        let revision = published.set.revision.clone();
        let server = HttpServer::bind("127.0.0.1:0".parse().unwrap(), published).unwrap();
        let cache_root = t.0.join("cache");
        let cache = Cache::open(&cache_root, Limits::default()).unwrap();
        let report = download_set(
            server.local_addr(),
            &revision,
            &cache,
            "damaged-source",
            &AtomicBool::new(false),
        )
        .unwrap();
        if active {
            cache
                .record_activation(&report.set, "damaged-source", &BTreeMap::new(), None)
                .unwrap();
        }
        fs::write(
            cache_root.join("sets").join(&revision).join("set.json"),
            "{",
        )
        .unwrap();
        drop(cache);
        let cache = Cache::open(&cache_root, Limits::default())
            .expect("one damaged set must not poison cache management");
        assert!(!cache_root.join("sets").join(&revision).exists());
        assert_eq!(fs::read_dir(cache_root.join("active")).unwrap().count(), 0);
        assert!(!cache.inventory().unwrap().is_empty());
        cache.deactivate("damaged-source").unwrap();
        let recovered = download_set(
            server.local_addr(),
            &revision,
            &cache,
            "damaged-source",
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(recovered.downloaded_bytes, 0);
        assert_eq!(recovered.reused_bytes, 18);
        assert_eq!(
            fs::read_to_string(recovered.roots["challenge"].join("client.lua")).unwrap(),
            "return {}"
        );
        cache.prune(0).unwrap();
    }
}
