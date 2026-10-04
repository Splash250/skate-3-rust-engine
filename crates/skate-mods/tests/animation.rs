use serde_json::json;
use skate_mods::animation::{Bank, Limits};

fn fixture() -> serde_json::Value {
    json!({"version":1,"bones":[{"name":"hips","parent":null},{"name":"head","parent":"hips"}],"clips":[{"name":"nod","duration":1.0,"tracks":[{"bone":"head","keys":[{"time":0.0},{"time":1.0,"translation":[0.0,0.2,0.0],"rotation":[0.0,0.0,1.0,0.0]}]}],"markers":[{"time":0.5,"name":"middle","payload":{"cosmetic":true}}]}]})
}
fn parse(value: serde_json::Value) -> Result<Bank, String> {
    Bank::parse(
        &serde_json::to_vec(&value).unwrap(),
        &["HIPS".into(), "HEAD".into()],
        &[-1, 0],
        &Limits::default(),
    )
}

#[test]
fn animation_import_validates_named_rig_and_samples_shortest_quaternions() {
    let bank = parse(fixture()).unwrap();
    let clip = &bank.clips["nod"];
    let sample = clip.sample(0.5);
    assert_eq!(sample.len(), 1);
    assert_eq!(sample[0].0, 1);
    assert!((sample[0].1.translation[1] - 0.1).abs() < 0.00001);
    assert!((sample[0].1.rotation.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 0.00001);
    assert_eq!(clip.markers_between(0.0, 0.75, false, 16).len(), 1);
    assert_eq!(clip.markers_between(0.75, 1.75, true, 16).len(), 1);
    assert_eq!(clip.markers_between(0.0, 10000.0, true, 4).len(), 4);
}

#[test]
fn animation_import_rejects_bad_hierarchy_keyframes_and_oversize_before_install() {
    let mut value = fixture();
    value["bones"][1]["parent"] = json!(null);
    assert!(parse(value).is_err());
    let mut value = fixture();
    value["clips"][0]["tracks"][0]["bone"] = json!("foreign");
    assert!(parse(value).is_err());
    let mut value = fixture();
    let track = value["clips"][0]["tracks"][0].clone();
    value["clips"][0]["tracks"]
        .as_array_mut()
        .unwrap()
        .push(track);
    assert!(parse(value).is_err());
    let mut value = fixture();
    value["clips"][0]["tracks"][0]["keys"][1]["time"] = json!(0.0);
    assert!(parse(value).is_err());
    let mut value = fixture();
    value["clips"][0]["tracks"][0]["keys"][1]["rotation"] = json!([0, 0, 0, 0]);
    assert!(parse(value).is_err());
    let mut value = fixture();
    value["clips"][0]["tracks"][0]["keys"][1]["translation"] = json!([0, 0, 100]);
    assert!(parse(value).is_err());
    let mut value = fixture();
    value["version"] = json!(2);
    assert!(parse(value).is_err());
    let limits = Limits {
        max_bytes: 32,
        ..Default::default()
    };
    assert!(
        Bank::parse(
            &serde_json::to_vec(&fixture()).unwrap(),
            &["HIPS".into(), "HEAD".into()],
            &[-1, 0],
            &limits
        )
        .is_err()
    );
    assert!(
        Limits {
            max_layers: usize::MAX,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
