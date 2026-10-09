use super::*;
use skate_mods::resources::{Host, InstalledResource, Side};

struct FixtureRoot(std::path::PathBuf);
impl FixtureRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skate-voice-retirement-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn voice_result_retirement_rejects_drained_commands_and_dependent_batches() {
    for wrapped in [false, true] {
        let root = FixtureRoot::new();
        let voice = root.0.join("a_voice");
        let dependent = root.0.join("z_dependent");
        std::fs::create_dir_all(&voice).unwrap();
        std::fs::create_dir_all(&dependent).unwrap();
        std::fs::write(
            voice.join("client.lua"),
            r#"
            resource.on('voice_result', function() error('voice completion rejected') end)
            return {}
        "#,
        )
        .unwrap();
        std::fs::write(dependent.join("client.lua"), "return {}").unwrap();
        let package = |id: &str, directory: std::path::PathBuf, dependencies: Value| {
            let manifest: skate_resources::Manifest = serde_json::from_value(json!({
                "format":1,"api":1,"id":id,"version":"1.0.0","language":"lua",
                "client_scripts":["client.lua"],"dependencies":dependencies,
                "capabilities":["resource.events","engine.voice","engine.input"]
            }))
            .unwrap();
            InstalledResource {
                grants: manifest.capabilities.iter().cloned().collect(),
                manifest,
                root: directory,
                generation: 1,
            }
        };
        let mut host = Host::new(Side::Client, root.0.join("storage"), "voice-retirement").unwrap();
        host.install(vec![
            package("a_voice", voice, json!({})),
            package("z_dependent", dependent, json!({"a_voice":"1.0.0"})),
        ])
        .unwrap();
        let mut app = App::new();
        app.insert_resource(crate::config::Config {
            asset_root: root.0.join("assets"),
            verification_capture: None,
            map: None,
            map_path: None, locations: None,
            difficulty: Default::default(),
            check_assets: false,
            validate_maps: false,
            mute: false,
            start_paused: false,
            multiplayer: crate::multiplayer::Options {
                connect: Some("127.0.0.1:31030".parse().unwrap()),
                ..Default::default()
            },
            map_fingerprint: 0,
            teleport: None,
        })
        .init_resource::<Assets<AudioSource>>()
        .add_plugins(ModdingPlugin);
        let mut mods = app.world_mut().remove_resource::<Mods>().unwrap();
        mods.ground_ready = true;
        mods.manager.attach_resources(host).unwrap();
        // Deliberately omit VoiceState: the real adapter returns a bounded
        // unavailable response, invoking the actual Lua completion callback.
        let voice = Command::Voice {
            operation: json!({"kind":"devices"}),
        };
        let voice = if wrapped {
            Command::Request {
                key: "voice".into(),
                command: Box::new(voice),
                token: 1,
            }
        } else {
            voice
        };
        mods.manager.commands.extend([
            ("a_voice".into(), voice),
            (
                "a_voice".into(),
                Command::InputOverride {
                    action: 64,
                    value: Some(1.),
                },
            ),
            (
                "z_dependent".into(),
                Command::InputOverride {
                    action: 65,
                    value: Some(1.),
                },
            ),
        ]);
        apply(app.world_mut(), &mut mods);
        assert!(!mods.manager.resources.as_ref().unwrap().running("a_voice"));
        assert!(
            !mods
                .manager
                .resources
                .as_ref()
                .unwrap()
                .running("z_dependent")
        );
        assert!(
            mods.input_overrides.is_empty(),
            "retired commands applied, wrapped={wrapped}: {:?}",
            mods.input_overrides
        );
        assert!(!mods.manager.packages["a_voice"].running());
        assert!(!mods.manager.packages["z_dependent"].running());
        assert!(
            mods.command_results.is_empty(),
            "retired request published a completion"
        );
    }
}
