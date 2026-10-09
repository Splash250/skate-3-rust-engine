use serde_json::{Value, json};
use skate_resources::locations::{Catalog, LocationSnapshot};
fn catalog() -> Value {
    json!({"version":1,"map":"downtown","interiors":[{"key":"apartment","model":"apartment.glb","collision":"collision.json","transform":[[1,0,0,0],[0,1,0,0],[0,0,1,0],[4096,100,4096,1]],"spawn":[4096,101,4096],"heading":0,"exit":{"position":[4096,100,4098],"style":style()}}],"locations":[{"key":"entry","label":"Apartment","position":[0,0,0],"return_position":[3,0,0],"return_heading":0,"style":style(),"floors":[{"key":"one","label":"Modern Apartment","interior":"apartment"}]}]})
}
fn style() -> Value {
    json!({"color":[1,0.8,0.15],"opacity":0.35,"radius":1,"height":2})
}
#[test]
fn location_catalog_rejects_escape_nan_duplicate_and_missing_floor() {
    let good = catalog();
    assert!(Catalog::parse(&serde_json::to_vec(&good).unwrap()).is_ok());
    for (pointer, value) in [
        ("/interiors/0/model", json!("../escape.glb")),
        ("/interiors/0/spawn/0", json!(1e100)),
        ("/locations/0/floors/0/interior", json!("absent")),
        ("/locations/0/floors", json!([])),
        ("/version", json!(2)),
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(
            Catalog::parse(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let mut bad = good.clone();
    bad["interiors"]
        .as_array_mut()
        .unwrap()
        .push(good["interiors"][0].clone());
    assert!(Catalog::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
    let mut typed = Catalog::parse(&serde_json::to_vec(&good).unwrap()).unwrap();
    typed.interiors[0].spawn[0] = f32::NAN;
    assert!(typed.validate().is_err());
}
#[test]
fn location_snapshot_bounds_styles_and_counts() {
    let good = json!({"generation":"1","locations":[{"key":"entry","label":"Apartment","enabled":true,"style":style()}]});
    let retained = LocationSnapshot::parse(good.clone()).unwrap();
    let original = retained.clone();
    for (pointer, value) in [
        ("/locations/0/style/color/0", json!(1.1)),
        ("/locations/0/style/height", json!(0)),
        ("/locations/0/style/opacity", json!(-0.1)),
        ("/generation", json!("01")),
        ("/generation", json!("0")),
        ("/generation", json!("18446744073709551616")),
    ] {
        let mut bad = good.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        if let Ok(next) = LocationSnapshot::parse(bad) {
            assert_ne!(next, original);
            panic!("accepted {pointer}");
        }
        assert_eq!(retained, original);
    }
    let mut bad = good;
    bad["locations"] = json!(
        (0..33)
            .map(|i| json!({"key":format!("k{i}"),"label":"x","enabled":true,"style":style()}))
            .collect::<Vec<_>>()
    );
    assert!(LocationSnapshot::parse(bad).is_err());
}
#[test]
fn location_manifest_requires_public_catalog() {
    let v = json!({"format":1,"api":1,"id":"rooms","version":"1","language":"lua","locations":"catalog.json","files":["catalog.json"]});
    let mut m: skate_resources::Manifest = serde_json::from_value(v).unwrap();
    m.validate().unwrap();
    m.files.clear();
    assert!(m.validate().is_err());
}
#[test]
fn location_publication_requires_assets_and_hashes_collision() {
    use std::collections::BTreeMap;
    let root = std::env::temp_dir().join(format!("location-publication-{}", std::process::id()));
    let dir = root.join("rooms");
    std::fs::create_dir_all(&dir).unwrap();
    let mut manifest = json!({"format":1,"api":1,"id":"rooms","version":"1","language":"lua","locations":"catalog.json","files":["catalog.json"]});
    std::fs::write(
        dir.join("resource.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join("catalog.json"),
        serde_json::to_vec(&catalog()).unwrap(),
    )
    .unwrap();
    let selected = BTreeMap::from([("rooms".into(), 1)]);
    assert!(skate_resources::build_set(&root, &selected).is_err());
    manifest["files"] = json!(["catalog.json", "apartment.glb", "collision.json"]);
    std::fs::write(
        dir.join("resource.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("apartment.glb"), b"synthetic-model").unwrap();
    std::fs::write(dir.join("collision.json"), b"synthetic-shell-1").unwrap();
    let old = skate_resources::build_set(&root, &selected).unwrap();
    std::fs::write(dir.join("collision.json"), b"synthetic-shell-2").unwrap();
    let next = skate_resources::build_set(&root, &selected).unwrap();
    assert_ne!(old.set.revision, next.set.revision);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn different_shell_changes_admission_identity_and_absence_keeps_fingerprint() {
    use skate_resources::locations::{catalog_revision, world_fingerprint};
    use std::collections::BTreeMap;
    let catalog = Catalog::parse(&serde_json::to_vec(&catalog()).unwrap()).unwrap();
    let mut files = BTreeMap::from([
        ("apartment.glb".into(), b"model".to_vec()),
        ("collision.json".into(), b"shell".to_vec()),
    ]);
    let a = catalog_revision(&catalog, &files).unwrap();
    files.insert("collision.json".into(), b"different shell".to_vec());
    let b = catalog_revision(&catalog, &files).unwrap();
    assert_ne!(a, b);
    assert_eq!(world_fingerprint(123, None), 123);
    assert_ne!(
        world_fingerprint(123, Some(&a)),
        world_fingerprint(123, Some(&b))
    );
}

#[test]
fn catalog_discovery_is_contained_and_deterministic() {
    use skate_resources::locations::PreparedCatalog;
    let root = std::env::temp_dir().join(format!("location-discovery-{}", std::process::id()));
    let maps = root.join("maps");
    std::fs::create_dir_all(&maps).unwrap();
    let map = maps.join("DownTown.skate");
    assert!(
        PreparedCatalog::discover(Some(&map), None)
            .unwrap()
            .is_none()
    );
    let package = maps.join("locations/downtown");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("catalog.json"),
        serde_json::to_vec(&catalog()).unwrap(),
    )
    .unwrap();
    std::fs::write(package.join("apartment.glb"), b"model").unwrap();
    std::fs::write(package.join("collision.json"), b"shell").unwrap();
    let automatic = PreparedCatalog::discover(Some(&map), None)
        .unwrap()
        .unwrap();
    let explicit = PreparedCatalog::discover(Some(&map), Some(&package))
        .unwrap()
        .unwrap();
    assert_eq!(automatic.revision, explicit.revision);
    assert!(
        PreparedCatalog::discover(Some(&maps.join("University.skate")), Some(&package)).is_err()
    );
    assert!(PreparedCatalog::discover(None, Some(&root.join("missing"))).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mutable_markers_cannot_disable_authority_or_cover_return() {
    let c = Catalog::parse(&serde_json::to_vec(&catalog()).unwrap()).unwrap();
    let mut s = LocationSnapshot {
        generation: "1".into(),
        locations: vec![skate_resources::locations::LocationSetting {
            key: c.locations[0].key.clone(),
            label: "Apartment".into(),
            enabled: false,
            style: c.locations[0].style.clone(),
            interaction: None,
        }],
    };
    assert!(s.validate_catalog(&c, 1).is_ok());
    s.locations[0].style.radius = 10.;
    assert!(s.validate_catalog(&c, 1).is_err());
    s.locations[0].style.radius = 0.2;
    s.generation = "2".into();
    assert!(s.validate_catalog(&c, 1).is_err());
}
