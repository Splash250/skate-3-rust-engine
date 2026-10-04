use skate_server::{Map, Options, map_fingerprint};
use std::{ffi::OsString, path::PathBuf};

fn parse(args: &[&str]) -> Result<Option<Options>, String> {
    Options::parse(args.iter().map(OsString::from))
}

#[test]
fn test_world_has_an_explicit_identity_and_server_defaults() {
    let options = parse(&["--test-world"]).unwrap().unwrap();
    assert_eq!(options.bind.to_string(), "0.0.0.0:31030");
    assert_eq!(options.session, 48031030);
    assert_eq!(options.max_players, 16);
    assert_eq!(options.map, Map::TestWorld);
    assert_eq!(
        map_fingerprint(&options.map).unwrap(),
        skate_net::hash(b"skate-test-world-v1")
    );
}

#[test]
fn map_paths_and_network_settings_are_preserved() {
    let options = parse(&[
        "--map",
        "My Maps/University.skate",
        "--bind",
        "127.0.0.1:0",
        "--session",
        "42",
        "--max-players",
        "2",
    ])
    .unwrap()
    .unwrap();
    assert_eq!(
        options.map,
        Map::File(PathBuf::from("My Maps/University.skate"))
    );
    assert_eq!(options.bind.port(), 0);
    assert_eq!(options.session, 42);
    assert_eq!(options.max_players, 2);
}

#[test]
fn invalid_or_ambiguous_options_do_not_start_a_server() {
    for args in [
        vec![],
        vec!["--map"],
        vec!["--test-world", "--map", "a.skate"],
        vec!["--test-world", "--session", "0"],
        vec!["--test-world", "--session", "nan"],
        vec!["--test-world", "--max-players", "0"],
        vec!["--test-world", "--max-players", "65"],
        vec!["--test-world", "--bind", "[::1]:31030"],
        vec!["--test-world", "--bind", "255.255.255.255:31030"],
        vec!["--test-world", "--plugin", "x"],
        vec!["--test-world", "--session", "1", "--session", "2"],
    ] {
        assert!(parse(&args).is_err(), "accepted {args:?}");
    }
}

#[test]
fn help_does_not_require_assets_or_startup() {
    assert!(parse(&["--help"]).unwrap().is_none());
}

#[test]
fn map_fingerprint_matches_streamed_bytes_and_reports_missing_files() {
    let path = std::env::temp_dir().join(format!(
        "skate-server-map-{}-fingerprint.skate",
        std::process::id()
    ));
    std::fs::write(&path, b"hello").unwrap();
    let fingerprint = map_fingerprint(&Map::File(path.clone()));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(fingerprint.unwrap(), 0xa430d84680aabd0b);
    assert!(map_fingerprint(&Map::File(path)).is_err());
}

#[test]
fn resource_configuration_is_explicit_and_preserves_native_paths() {
    let result = parse(&["--test-world", "--resources", "Server Files/server.json"]);
    assert!(result.is_ok(), "resource configuration option missing: {result:?}");
}

#[test]
fn accepts_sixty_four_players() {
    assert_eq!(parse(&["--test-world","--max-players","64"]).unwrap().unwrap().max_players,64);
}
