//! Dedicated resource activation and engine ownership. HTTP work never runs on
//! the simulation thread; only fully verified sets can construct client VMs.
use super::Mods;
use bevy::prelude::*;
use skate_mods::resources::{Host, InstalledResource, Output, Side};
use skate_net::resources::{CLIENT_KEY, Client, Kind, Offer};
use skate_resources::{Cache, DownloadReport, Limits, ResourceSet};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

type Identity = (u64, u64, u64); // local actor, server actor, connection incarnation
type Grants = BTreeMap<String, Vec<String>>;
type Policy = BTreeMap<String, Grants>;

struct Download {
    offer: Offer,
    identity: Identity,
    cancel: Arc<AtomicBool>,
    worker: JoinHandle<Result<DownloadReport, String>>,
}
struct Active {
    set: ResourceSet,
    grants: Grants,
}

#[derive(Resource)]
pub(super) struct ClientResources {
    endpoint: Option<SocketAddr>,
    root: PathBuf,
    source: String,
    identity: Option<Identity>,
    channel: Client,
    pending: Option<Download>,
    active: Option<Active>,
    failure: Option<String>,
}
impl ClientResources {
    pub fn new(config: &crate::config::Config, root: PathBuf) -> Self {
        let endpoint = config.multiplayer.connect;
        Self {
            endpoint,
            root,
            source: endpoint
                .map(|a| source(a, config.multiplayer.session))
                .unwrap_or_default(),
            identity: None,
            channel: Client::default(),
            pending: None,
            active: None,
            failure: None,
        }
    }
    fn cancel(&mut self) {
        if let Some(download) = &self.pending {
            download.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl Drop for ClientResources {
    fn drop(&mut self) {
        self.cancel();
        // No waiting for network I/O from a resource destructor. Normal AppExit
        // uses shutdown to retire engine objects and release the cache pin.
    }
}

pub(super) fn cache_root(asset_root: &Path) -> PathBuf {
    std::env::var_os("SKATE3_RESOURCE_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            asset_root
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("settings/resources")
        })
}
fn source(endpoint: SocketAddr, session: u64) -> String {
    format!("udp://{endpoint}/{session}")
}

fn safe_default(capability: &str) -> bool {
    capability.starts_with("resource.")
        || matches!(
            capability,
            "engine.ui" | "engine.audio" | "engine.graphics" | "engine.inspect"
        )
}
fn grants_for(set: &ResourceSet, source: &str, policy: &Policy) -> Result<Grants, String> {
    let mut granted = BTreeMap::new();
    for resource in &set.resources {
        let additions = policy
            .get(source)
            .and_then(|p| p.get(&resource.manifest.id));
        let mut caps = Vec::new();
        for cap in &resource.manifest.capabilities {
            if !safe_default(cap) && !additions.is_some_and(|a| a.contains(cap)) {
                return Err(format!(
                    "{} requires ungranted {cap}; inspect the resource and source policy in grants.json",
                    resource.manifest.id
                ));
            }
            caps.push(cap.clone());
        }
        granted.insert(resource.manifest.id.clone(), caps);
    }
    Ok(granted)
}
fn read_policy(root: &Path) -> Result<Policy, String> {
    let path = root.join("grants.json");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Policy::new()),
        Err(e) => return Err(format!("resource grants: {e}")),
    };
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err("grants.json must be a regular file of at most 64 KiB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("resource grants: {e}"))
}

fn retire(world: &mut World, mods: &mut Mods, client: &mut ClientResources, unpin: bool) {
    client.cancel();
    client.channel.set_ready(false);
    mods.manager.detach_resources();
    // Cleanup must run before a new instance can reuse the same owner ID.
    super::apply(world, mods);
    // Native collision queries are published once per simulation tick. Retire
    // their copied bodies now, including while paused, before a new host starts.
    if let Some(mut physics) = world.get_resource_mut::<crate::physics::GamePhysics>() {
        physics.set_external_queries(None);
        physics.network_proxies = Default::default();
    }
    for (_, body) in std::mem::take(&mut mods.skater_proxies) {
        mods.world.remove(body);
    }
    mods.last_contacts.clear();
    mods.native_snapshot = None;
    if unpin && client.active.is_some() {
        if let Ok(cache) = Cache::open(&client.root, Limits::default()) {
            if let Err(error) = cache.deactivate(&client.source) {
                warn!("Resource cache unpin: {error}");
            }
        }
        client.active = None;
    }
}

fn fail(world: &mut World, mods: &mut Mods, client: &mut ClientResources, error: String) {
    warn!("Dedicated resources stopped: {error}");
    if let Some(active) = &client.active
        && client.channel.ready()
    {
        if let Ok(cache) = Cache::open(&client.root, Limits::default()) {
            let _ =
                cache.record_activation(&active.set, &client.source, &active.grants, Some(&error));
        }
    }
    retire(world, mods, client, true);
    client.failure = Some(error.clone());
    let mut net = world.resource_mut::<crate::multiplayer::Multiplayer>();
    net.leave();
    net.status = format!("Resource activation failed: {error}");
}

pub(super) fn poll(world: &mut World) {
    world.resource_scope(|world, mut client: Mut<ClientResources>| {
        if client.endpoint.is_none() {
            return;
        }
        world.resource_scope(|world, mut mods: Mut<Mods>| {
            if let Err(error) = poll_inner(world, &mut mods, &mut client) {
                fail(world, &mut mods, &mut client, error);
            }
        });
    });
}

fn poll_inner(
    world: &mut World,
    mods: &mut Mods,
    client: &mut ClientResources,
) -> Result<(), String> {
    let net = world.resource::<crate::multiplayer::Multiplayer>();
    mods.skater_remote = net
        .resource_skater_states()
        .into_iter()
        .map(|(id, body, gameplay)| {
            let velocity = body.bodies.first().map_or([0.; 3], |b| b.velocity);
            let angular = body.bodies.first().map_or([0.; 3], |b| b.angular);
            let forward = Quat::from_array(body.root.q) * Vec3::Z;
            let offboard = gameplay.mode == skate_net::dedicated::PlayerMode::Offboard;
            let bailing = gameplay.mode == skate_net::dedicated::PlayerMode::Ragdoll;
            (
                id,
                super::observation::WireObs {
                    p: body.root.p,
                    r: body.root.q,
                    v: velocity,
                    av: angular,
                    fw: forward.to_array(),
                    h: forward.x.atan2(forward.z),
                    sp: Vec3::from_array(velocity).length(),
                    suspended: body.enabled & (1 << 63) != 0,
                    ob: !offboard,
                    ca: if offboard { 500 } else { 100 },
                    ba: bailing,
                    md: if bailing {
                        "bail"
                    } else if offboard {
                        "offboard"
                    } else {
                        "ground"
                    }
                    .into(),
                    tr: gameplay.trick,
                    ts: gameplay.trick_seq as u32,
                    landed: gameplay.landed_seq as u32,
                    landed_name: gameplay.landed_trick,
                    bail_seq: gameplay.bail_seq as u32,
                    sc: gameplay.sequence_score as f32,
                    ls: gameplay.line_score as f32,
                    ..Default::default()
                },
            )
        })
        .collect();
    let identity = if net.is_dedicated() && net.connected() && net.host_actor() != 0 {
        Some((
            net.mod_identity().1,
            net.host_actor(),
            net.connection_generation(),
        ))
    } else {
        None
    };
    let record = net.resource_record();
    if identity != client.identity {
        retire(world, mods, client, true);
        client.channel = Client::default();
        client.identity = identity;
        // A new admitted connection may retry; an offline failure stays visible.
        if identity.is_some() {
            client.failure = None;
        }
    }
    if identity.is_some() {
        if let Some(record) = record {
            if client.channel.receive(&record)? {
                retire(world, mods, client, false);
                client.failure = None;
                info!(
                    "RESOURCE_OFFER source={} revision={}",
                    client.source, record.offer.revision
                );
            }
        }
    }

    // At most one HTTP worker exists, including a cancelled transfer. A rapid
    // series of offers cannot spawn unbounded detached download threads.
    if client
        .pending
        .as_ref()
        .is_some_and(|p| p.worker.is_finished())
    {
        let pending = client.pending.take().unwrap();
        let current = client.identity == Some(pending.identity)
            && client.channel.offer() == Some(&pending.offer)
            && !pending.cancel.load(Ordering::Relaxed);
        let result = pending
            .worker
            .join()
            .unwrap_or_else(|_| Err("resource download worker failed".to_string()));
        if current {
            let report = result?;
            activate(world, mods, client, report)?;
        }
    }
    if client.identity.is_none() || client.failure.is_some() {
        return Ok(());
    }
    if !client.channel.ready() && client.pending.is_none() {
        if let Some(offer) = client.channel.offer().cloned() {
            let endpoint = SocketAddr::new(client.endpoint.unwrap().ip(), offer.port);
            let root = client.root.clone();
            let source = client.source.clone();
            let revision = offer.revision.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let worker = std::thread::Builder::new()
                .name("resource-download".into())
                .spawn(move || {
                    let cache = Cache::open(root, Limits::default()).map_err(|e| e.to_string())?;
                    let report =
                        skate_resources::download_set(endpoint, &revision, &cache, &source, &flag)
                            .map_err(|e| e.to_string())?;
                    // Validate all downloadable models before any VM starts, so
                    // metadata/physics model helpers share the graphics boundary.
                    if let Err(error) = validate_models(&report, &flag) {
                        let _ = cache.record_activation(
                            &report.set,
                            &source,
                            &Grants::new(),
                            Some(&error),
                        );
                        return Err(error);
                    }
                    Ok(report)
                })
                .map_err(|e| format!("resource download worker: {e}"))?;
            client.pending = Some(Download {
                offer,
                identity: client.identity.unwrap(),
                cancel,
                worker,
            });
        }
    }
    if let Some(host) = mods.manager.resources.as_mut() {
        for message in client.channel.take_incoming() {
            match message.kind {
                Kind::Event => host.receive(
                    0,
                    &message.resource,
                    message.generation,
                    &message.name,
                    message.value,
                )?,
                Kind::State => host.apply_state(
                    &message.resource,
                    message.generation,
                    &message.name,
                    message.value,
                )?,
            }
        }
        mods.manager.sync_resources();
        super::apply(world, mods);
    }
    publish(world, client)
}

fn validate_models(report: &DownloadReport, cancel: &AtomicBool) -> Result<(), String> {
    for resource in &report.set.resources {
        let root = report
            .roots
            .get(&resource.manifest.id)
            .ok_or("missing materialized resource")?;
        for path in resource.files.keys().filter(|p| p.ends_with(".glb")) {
            if cancel.load(Ordering::Relaxed) {
                return Err("resource validation cancelled".into());
            }
            let bytes = skate_mods::read_bounded(root, path, 16 * 1024 * 1024)?;
            super::graphics::validate_resource_glb(&bytes)
                .map_err(|e| format!("{}/{path}: {e}", resource.manifest.id))?;
        }
    }
    Ok(())
}

fn activate(
    world: &mut World,
    mods: &mut Mods,
    client: &mut ClientResources,
    report: DownloadReport,
) -> Result<(), String> {
    let cache = Cache::open(&client.root, Limits::default()).map_err(|e| e.to_string())?;
    let mut granted = Grants::new();
    let result = (|| {
        granted = grants_for(&report.set, &client.source, &read_policy(&client.root)?)?;
        let mut host = Host::new(Side::Client, client.root.join("state"), &client.source)?;
        host.install(
            report
                .set
                .resources
                .iter()
                .map(|r| {
                    Ok(InstalledResource {
                        manifest: r.manifest.clone(),
                        root: report.roots.get(&r.manifest.id).cloned().ok_or_else(|| {
                            format!("missing materialized resource {}", r.manifest.id)
                        })?,
                        generation: r.generation,
                        grants: granted[&r.manifest.id]
                            .iter()
                            .cloned()
                            .collect::<BTreeSet<_>>(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        )?;
        let camera = super::camera_position(world);
        mods.manager.snapshot = Arc::new(super::snapshot_ro(world, mods, camera));
        mods.manager.attach_resources(host)?;
        super::apply(world, mods);
        let host = mods
            .manager
            .resources
            .as_ref()
            .ok_or("resource host disappeared during startup")?;
        if let Some(resource) = report
            .set
            .resources
            .iter()
            .find(|r| !host.running(&r.manifest.id))
        {
            return Err(format!(
                "{} failed to apply startup engine commands",
                resource.manifest.id
            ));
        }
        cache
            .record_activation(&report.set, &client.source, &granted, None)
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = cache.record_activation(&report.set, &client.source, &granted, Some(&error));
        return Err(error);
    }
    info!(
        "RESOURCE_ACTIVATED source={} revision={} downloaded_bytes={} reused_bytes={}",
        client.source, report.set.revision, report.downloaded_bytes, report.reused_bytes
    );
    client.active = Some(Active {
        set: report.set,
        grants: granted,
    });
    client.channel.set_ready(true);
    Ok(())
}

fn publish(world: &mut World, client: &ClientResources) -> Result<(), String> {
    if client.channel.offer().is_none() {
        return Ok(());
    }
    let bytes = client.channel.encode()?;
    let mut net = world.resource_mut::<crate::multiplayer::Multiplayer>();
    if !net.publish_application(CLIENT_KEY, bytes) {
        return Err("resource control record could not be published".into());
    }
    if !client.channel.ready() {
        net.status = "Downloading and verifying required server resources...".into();
    }
    Ok(())
}

pub(super) fn flush(world: &mut World) {
    world.resource_scope(|world, mut client: Mut<ClientResources>| {
        if client.endpoint.is_none() || !client.channel.ready() {
            return;
        }
        world.resource_scope(|world, mut mods: Mut<Mods>| {
            let result = (|| {
                let host = mods
                    .manager
                    .resources
                    .as_mut()
                    .ok_or("required resource host is not running")?;
                if let Some(active) = &client.active {
                    if let Some(resource) = active
                        .set
                        .resources
                        .iter()
                        .find(|r| !host.running(&r.manifest.id))
                    {
                        return Err(format!(
                            "required resource {} stopped",
                            resource.manifest.id
                        ));
                    }
                }
                for output in host.drain_outputs() {
                    match output {
                        Output::Event {
                            resource,
                            generation,
                            name,
                            payload,
                            ..
                        } => client.channel.emit(&resource, generation, &name, payload)?,
                        Output::Log { resource, text } => info!("RESOURCE_LOG {resource}: {text}"),
                        Output::State { .. } => {
                            return Err(
                                "client attempted to publish server-owned resource state".into()
                            );
                        }
                    }
                }
                publish(world, &client)
            })();
            if let Err(error) = result {
                fail(world, &mut mods, &mut client, error);
            }
        });
    });
}

pub(super) fn shutdown(world: &mut World) {
    if world
        .get_resource::<Messages<AppExit>>()
        .is_none_or(|events| events.is_empty())
    {
        return;
    }
    world.resource_scope(|world, mut client: Mut<ClientResources>| {
        if client.endpoint.is_none() {
            return;
        }
        world.resource_scope(|world, mut mods: Mut<Mods>| {
            retire(world, &mut mods, &mut client, true);
        });
        // Send the transport goodbye on clean exit so server-side membership
        // and resource participants are retired immediately, not after timeout.
        if let Some(mut net) = world.get_resource_mut::<crate::multiplayer::Multiplayer>() {
            net.leave();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set(capabilities: &[&str]) -> ResourceSet {
        serde_json::from_value(serde_json::json!({"revision":"unused","resources":[{
            "manifest":{"format":1,"api":1,"id":"demo","version":"1.0.0","language":"lua","capabilities":capabilities},
            "files":{},"generation":1,"content_digest":"unused"
        }]})).unwrap()
    }
    #[test]
    fn dedicated_resource_policy_requires_source_specific_sensitive_grants() {
        let origin = source("127.0.0.1:31030".parse().unwrap(), 7);
        let set = set(&["resource.network", "engine.ui", "engine.camera"]);
        assert!(
            grants_for(&set, &origin, &Policy::new())
                .unwrap_err()
                .contains("engine.camera")
        );
        let policy = BTreeMap::from([(
            origin.clone(),
            BTreeMap::from([("demo".into(), vec!["engine.camera".into()])]),
        )]);
        assert_eq!(grants_for(&set, &origin, &policy).unwrap()["demo"].len(), 3);
        assert!(
            grants_for(
                &set,
                &source("127.0.0.1:31031".parse().unwrap(), 7),
                &policy
            )
            .is_err()
        );
        assert!(
            grants_for(
                &set,
                &source("127.0.0.1:31030".parse().unwrap(), 8),
                &policy
            )
            .is_err()
        );
    }
    #[test]
    fn dedicated_resource_default_policy_never_grants_native_control() {
        for cap in [
            "engine.player",
            "engine.physics",
            "engine.camera",
            "engine.input",
            "engine.world",
            "engine.animation",
        ] {
            assert!(!safe_default(cap));
        }
        assert!(
            grants_for(
                &set(&["resource.state", "engine.ui"]),
                "server",
                &Policy::new()
            )
            .is_ok()
        );
    }
    #[test]
    fn dedicated_resource_engine_apply_and_retire_restore_owned_state() {
        use crate::graph_runtime::{CompiledGraph, LoadedGraph, StockGraphs};
        use skate_data::state_graph::{
            StateGraph,
            binding::{Binding, State},
        };
        let root = std::env::temp_dir().join(format!(
            "skate-resource-engine-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("client.lua"),r#"
            return {on_load=function()
                sdk.ui.canvas("hud",{size={120,60},items={{key="label",text="Resource active"}}})
                sdk.ui.menu("menu",{section="Resource tests",title="Ownership",items={{id="test",label="Test"}}})
                sdk.camera.set({1,2,3},{0,0,0})
                sdk.input.override_action(64,0.5)
                sdk.graphs.set_enabled("action","state",0,false)
                sdk.audio.preload("tone.wav")
                sdk.audio.play("voice",{path="tone.wav",spatial=false,loop=true})
                sdk.physics.spawn("box",{shape={type="box",half_extents={0.5,0.5,0.5}},body_type="dynamic"})
            end}
        "#).unwrap();
        let mut wav = b"RIFF".to_vec();
        wav.extend(40u32.to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(8000u32.to_le_bytes());
        wav.extend(16000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(4u32.to_le_bytes());
        wav.extend([0u8; 4]);
        std::fs::write(root.join("tone.wav"), wav).unwrap();
        let config = crate::config::Config {
            asset_root: root.join("assets"),
            verification_capture: None,
            map: None,
            map_path: None,
            difficulty: Default::default(),
            check_assets: false,
            start_paused: false,
            multiplayer: crate::multiplayer::Options {
                connect: Some("127.0.0.1:31030".parse().unwrap()),
                session: 7,
                ..Default::default()
            },
            map_fingerprint: 0,
            teleport: None,
        };
        let mut app = App::new();
        app.insert_resource(config)
            .init_resource::<Assets<AudioSource>>()
            .add_plugins(super::super::ModdingPlugin);
        // A synthetic stock graph lets this exercise the actual engine gate and
        // restore paths without redistributing or requiring retail fixtures.
        let binding = Binding {
            states: vec![State {
                element: 0,
                name: "fixture".into(),
                parent: None,
                children: vec![],
                behaviors: vec![],
                transitions: vec![],
                expression: None,
                enabled: 1,
                active: 0,
                interruptibility: 0,
                interrupt_ancestor: None,
            }],
            transitions: vec![],
            expressions: vec![],
            operations: vec![],
            root: 0,
        };
        let graph = LoadedGraph {
            source: StateGraph { elements: vec![] },
            runtime: CompiledGraph::from_binding(&binding).unwrap(),
            binding,
        };
        app.insert_resource(StockGraphs {
            action: graph.clone(),
            motion: graph,
        });
        let mut mods = app.world_mut().remove_resource::<Mods>().unwrap();
        let mut client = app
            .world_mut()
            .remove_resource::<ClientResources>()
            .unwrap();
        mods.ground_ready = true; // No commands in this fixture need native ground.
        let mut manifest = set(&[
            "engine.ui",
            "engine.camera",
            "engine.input",
            "engine.animation",
            "engine.audio",
            "engine.physics",
        ])
        .resources
        .remove(0)
        .manifest;
        manifest.client_scripts = vec!["client.lua".into()];
        manifest.files = vec!["tone.wav".into()];
        for generation in [1, 2] {
            let mut host = Host::new(Side::Client, root.join("state"), "fixture").unwrap();
            host.install(vec![InstalledResource {
                grants: manifest.capabilities.iter().cloned().collect(),
                manifest: manifest.clone(),
                root: root.clone(),
                generation,
            }])
            .unwrap();
            mods.manager.attach_resources(host).unwrap();
            super::super::apply(app.world_mut(), &mut mods);
            assert!(mods.manager.resources.as_ref().unwrap().running("demo"));
            assert!(mods.canvases.contains_key(&("demo".into(), "hud".into())));
            assert!(
                mods.custom_menus
                    .contains_key(&("demo".into(), "menu".into()))
            );
            assert_eq!(mods.camera.owner.as_deref(), Some("demo"));
            assert_eq!(mods.input_overrides.get(&64), Some(&("demo".into(), 0.5)));
            assert!(
                !app.world()
                    .resource::<StockGraphs>()
                    .action
                    .runtime
                    .program
                    .topology
                    .states[0]
                    .enabled
            );
            let owned_ui: Vec<_> = app
                .world_mut()
                .query_filtered::<Entity, With<Node>>()
                .iter(app.world())
                .collect();
            assert!(!owned_ui.is_empty());
            let audio: Vec<_> = app
                .world_mut()
                .query_filtered::<Entity, With<AudioPlayer>>()
                .iter(app.world())
                .collect();
            assert_eq!(audio.len(), 1);
            assert_eq!(app.world().resource::<Assets<AudioSource>>().len(), 1);
            let body = mods.bodies[&("demo".into(), "box".into())];
            assert!(mods.world.read(body).is_some());
            // A different owner's state must survive this resource's cleanup.
            mods.input_overrides.insert(65, ("other".into(), 0.25));
            mods.camera.saved_near = Some(0.1);
            retire(app.world_mut(), &mut mods, &mut client, false);
            assert!(mods.manager.resources.is_none());
            assert!(audio.iter().all(|e| app.world().get_entity(*e).is_err()));
            assert!(app.world().resource::<Assets<AudioSource>>().is_empty());
            assert!(mods.bodies.is_empty());
            assert!(mods.world.read(body).is_none());
            assert!(mods.canvases.is_empty());
            assert!(mods.custom_menus.is_empty());
            assert!(owned_ui.iter().all(|e| app.world().get_entity(*e).is_err()));
            assert!(mods.camera.owner.is_none());
            assert!(mods.camera.fixed.is_none());
            assert_eq!(mods.camera.saved_near, Some(0.1)); // Presentation restores the captured lens.
            assert!(!mods.input_overrides.contains_key(&64));
            assert_eq!(mods.input_overrides.get(&65), Some(&("other".into(), 0.25)));
            assert!(
                app.world()
                    .resource::<StockGraphs>()
                    .action
                    .runtime
                    .program
                    .topology
                    .states[0]
                    .enabled
            );
            assert!(mods.graph_gates.is_empty());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
