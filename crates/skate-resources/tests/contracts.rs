use skate_resources::{Manifest, ordered_manifests, validate_path};
fn manifest(id: &str) -> Manifest {
    serde_json::from_value(serde_json::json!({"format":1,"api":1,"id":id,"version":"1.0.0","language":"lua","client_scripts":["client.lua"]})).unwrap()
}
#[test]
fn canonical_paths_are_safe_on_linux_and_windows() {
    for path in ["client.lua", "ui/panel-1.png", "shared/game.lua"] {
        validate_path(path).unwrap();
    }
    for path in [
        "../x",
        "/x",
        "C:/x",
        "c:\\x",
        "a//b",
        "a/./b",
        "a/../b",
        "Client.lua",
        "con.lua",
        "aux",
        "nul.txt",
        "com1.lua",
        "a/lpt9.txt",
        "a.",
        "a ",
        "é.lua",
        ".secret",
        "resource.json",
    ] {
        assert!(validate_path(path).is_err(), "accepted {path}");
    }
}
#[test]
fn manifests_reject_incompatibility_duplicates_and_private_overlap() {
    let good = manifest("challenge");
    good.validate().unwrap();
    let mut m = good.clone();
    m.api = 2;
    assert!(m.validate().is_err());
    let mut m = good.clone();
    m.language = "javascript".into();
    assert!(m.validate().is_err());
    let mut m = good.clone();
    m.files = vec!["client.lua".into()];
    assert!(m.validate().is_err());
    let mut m = good.clone();
    m.server_scripts = vec!["client.lua".into()];
    assert!(m.validate().is_err());
    let mut m = good;
    m.client_scripts = vec!["native.dll".into()];
    assert!(m.validate().is_err());
}
#[test]
fn dependencies_are_ordered_and_diagnose_absent_wrong_version_and_cycle() {
    let base = manifest("base");
    let mut game = manifest("game");
    game.dependencies.insert("base".into(), "1.0.0".into());
    assert_eq!(
        ordered_manifests(&[game.clone(), base.clone()]).unwrap(),
        ["base", "game"]
    );
    assert!(
        ordered_manifests(&[game.clone()])
            .unwrap_err()
            .to_string()
            .contains("base")
    );
    let mut wrong = base.clone();
    wrong.version = "2.0.0".into();
    assert!(ordered_manifests(&[game.clone(), wrong]).is_err());
    let mut cycle = base;
    cycle.dependencies.insert("game".into(), "1.0.0".into());
    assert!(
        ordered_manifests(&[game, cycle])
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
}

#[test]
fn duplicate_dependency_keys_are_not_silently_replaced() {
    let parsed = serde_json::from_str::<Manifest>(
        r#"{"format":1,"api":1,"id":"game","version":"1.0.0","language":"lua","dependencies":{"base":"1.0.0","base":"2.0.0"}}"#,
    );
    assert!(parsed.is_err());
}

#[test]
fn javascript_manifests_require_matching_source_extensions() {
    let mut m=manifest("javascript-example");
    m.language="javascript".into();
    assert!(m.validate().is_err());
    m.client_scripts=vec!["client.js".into()];
    m.server_scripts=vec!["server.js".into()];
    m.validate().unwrap();
    assert_eq!(m.client_projection().server_scripts,Vec::<String>::new());
    m.language="csharp".into();
    assert!(m.validate().is_err());
    m.shared_scripts=vec!["main.cs".into()];
    m.client_scripts.clear();
    m.server_scripts.clear();
    assert!(m.validate().is_ok());
}

#[test]
fn required_world_manifest_names_one_public_skate_package() {
    let value=serde_json::json!({"format":1,"api":1,"id":"park","version":"1","language":"lua",
        "files":["park.skate"],"world":{"map":"park.skate","required":true}});
    let map:Manifest=serde_json::from_value(value.clone()).expect("world manifest must be supported");
    map.validate().unwrap();
    for field in [serde_json::json!({"map":"../park.skate","required":true}),
        serde_json::json!({"map":"missing.skate","required":true}),
        serde_json::json!({"map":"park.glb","required":true}),
        serde_json::json!({"map":"park.skate","required":false})] {
        let mut invalid=value.clone(); invalid["world"]=field;
        let parsed:Manifest=serde_json::from_value(invalid).unwrap();
        assert!(parsed.validate().is_err());
    }
}

#[test]
fn authored_world_lods_are_public_ordered_bounded_and_independent() {
    let value=serde_json::json!({"format":1,"api":1,"id":"park","version":"1","language":"lua",
        "files":["park.skate","far.skate"],"world":{"map":"park.skate","required":true,"lods":[{"map":"far.skate","distance":200}]}});
    let manifest:Manifest=serde_json::from_value(value.clone()).expect("authored LOD contract");
    manifest.validate().unwrap();
    for lods in [serde_json::json!([{"map":"park.skate","distance":200}]),
        serde_json::json!([{"map":"hidden.skate","distance":200}]),
        serde_json::json!([{"map":"far.skate","distance":0}]),
        serde_json::json!([{"map":"far.skate","distance":200},{"map":"far.skate","distance":100}])] {
        let mut invalid=value.clone();invalid["world"]["lods"]=lods;
        assert!(serde_json::from_value::<Manifest>(invalid).unwrap().validate().is_err());
    }
}
