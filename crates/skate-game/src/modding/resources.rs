//! Dedicated resource activation and engine ownership. HTTP work never runs on
//! the simulation thread; only fully verified sets can construct client VMs.
use super::Mods;
use bevy::prelude::*;
use skate_mods::resources::{Host, InstalledResource, Output, Side};
use skate_net::resources::{CLIENT_KEY, Client, Kind, Offer};
use skate_resources::{Cache, DownloadReport, ResourceSet};
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
struct Initializing {
    host: Host,
    report: DownloadReport,
    granted: Grants,
    needed: BTreeSet<String>,
    started: std::time::Instant,
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
    local_budgets: skate_net::resources::Budgets,
    asset_limits:super::graphics::asset_limits::Limits,
    configuration_error: Option<String>,
    transfers: skate_net::transfers::Transfers,
    diagnostic_sample: Option<std::time::Instant>,
    pending: Option<Download>,
    mounting: Option<DownloadReport>,
    catalog_revisions: BTreeMap<String, String>,
    active: Option<Active>,
    initializing: Option<Initializing>,
    deferred: Vec<skate_net::resources::Message>,
    failure: Option<String>,
}
impl ClientResources {
    pub fn new(config: &crate::config::Config, root: PathBuf) -> Self {
        let endpoint = config.multiplayer.connect;
        let (local_budgets,configuration_error)=match read_network_budgets(&root) {
            Ok(value)=>(value,None),Err(error)=>(Default::default(),Some(error)),
        };
        let (asset_limits,asset_error)=match super::graphics::asset_limits::Limits::read(&root) {
            Ok(value)=>(value,None),Err(error)=>(Default::default(),Some(error)),
        };
        Self {
            endpoint,
            root,
            source: endpoint
                .map(|a| source(a, config.multiplayer.session))
                .unwrap_or_default(),
            identity: None,
            channel: Client::with_budgets(local_budgets).expect("validated network budgets"),
            local_budgets,
            configuration_error:configuration_error.or(asset_error),
            asset_limits,
            transfers: Default::default(),
            diagnostic_sample: (std::env::var("SKATE_RESOURCE_DIAGNOSTICS").as_deref() == Ok("1"))
                .then(std::time::Instant::now),
            pending: None,
            mounting: None,
            catalog_revisions: BTreeMap::new(),
            active: None,
            initializing: None,
            deferred: Vec::new(),
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
            "engine.map" | "engine.ui" | "engine.audio" | "engine.graphics" | "engine.inspect" | "engine.voice"
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
fn read_network_budgets(root:&Path)->Result<skate_net::resources::Budgets,String> {
    let path=root.join("network-budgets.json");
    let metadata=match std::fs::symlink_metadata(&path) {
        Ok(value)=>value,Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(Default::default()),
        Err(error)=>return Err(format!("Local resource budgets: {error}")),
    };
    if !metadata.is_file() || metadata.len()>16384 {return Err("network-budgets.json must be a regular file of at most16KiB".into());}
    use std::io::Read;
    let mut bytes=Vec::new();
    std::fs::File::open(path).map_err(|e|e.to_string())?.take(16385).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    if bytes.len()>16384 {return Err("network-budgets.json exceeds16KiB".into());}
    let value:serde_json::Value=serde_json::from_slice(&bytes)
        .map_err(|e|format!("Local resource budgets: {e}"))?;
    if !value.is_object() {return Err("network-budgets.json must contain an object".into());}
    let value:skate_net::resources::Budgets=serde_json::from_value(value)
        .map_err(|e|format!("Local resource budgets: {e}"))?;
    value.validate()?;Ok(value)
}

fn retire(world: &mut World, mods: &mut Mods, client: &mut ClientResources, unpin: bool) {
    client.cancel();
    if let Err(error)=super::resource_world::clear(world) {warn!("Resource native rail cleanup: {error}");}
    client.mounting=None;
    client.catalog_revisions.clear();
    client.initializing=None;
    client.deferred.clear();
    crate::map_transition::unmount_resource(world);
    client.channel.set_ready(false);
    client.transfers=Default::default();
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
        if let Ok(cache) = Cache::open(&client.root, client.asset_limits.content) {
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
        if let Ok(cache) = Cache::open(&client.root, client.asset_limits.content) {
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
        let net=world.resource::<crate::multiplayer::Multiplayer>();
        let endpoint=net.dedicated_endpoint;
        let session=net.session_identity().map(|identity|identity.0);
        if endpoint!=client.endpoint {
            world.resource_scope(|world, mut mods: Mut<Mods>| {retire(world,&mut mods,&mut client,true);});
            client.identity=None;client.channel=Client::with_budgets(client.local_budgets).expect("validated budgets");
            client.endpoint=endpoint;client.source=endpoint.map(|e|source(e,session.unwrap_or(48031030))).unwrap_or_default();
        }
        if client.endpoint.is_none() {return;}
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
    if let Some(error)=client.configuration_error.take() {return Err(error);}
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
                    suspended: is_remote_suspended(&gameplay),
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
            net.resource_scope_epoch(),
        ))
    } else {
        None
    };
    let record = net.resource_record();
    let visible_scopes=net.visible_resource_scopes();
    if identity != client.identity {
        retire(world, mods, client, true);
        client.channel = Client::with_budgets(client.local_budgets)?;
        client.identity = identity;
        if let Some(endpoint)=client.endpoint {let session=world.resource::<crate::multiplayer::Multiplayer>().session_identity().map(|i|i.0).unwrap_or(48031030);client.source=source(endpoint,session);}
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
            client.catalog_revisions.clear();
            client.mounting=Some(report);
        }
    }
    if client.identity.is_none() || client.failure.is_some() {
        return Ok(());
    }
    if let Some(report)=&client.mounting {
        let mut ready=if let Some(resource)=report.set.resources.iter().find(|r|r.manifest.world.is_some()) {
            let spec=resource.manifest.world.as_ref().unwrap();
            let root=report.roots.get(&resource.manifest.id).ok_or("Required world root is absent")?;
            let digest=&resource.files.get(&spec.map).ok_or("Required world file is absent")?.digest;
            crate::map_transition::mount_resource(world,
                format!("{}:{}:{}",resource.manifest.id,resource.generation,digest),
                crate::map_library::Entry {label:resource.manifest.id.clone(),path:Some(root.join(&spec.map))},
                spec.max_decoded_bytes as usize,spec.lods.iter().map(|lod|(root.join(&lod.map),lod.distance)).collect(),
                crate::map_render::streaming::Options::read(&client.root)?)?
        } else {crate::map_transition::resource_world_idle(world)?};
        if ready {
            let allowed = report.set.resources.iter().filter(|r| r.manifest.locations.is_some()).map(|r| r.manifest.id.as_str()).collect::<BTreeSet<_>>();
            ready &= crate::locations::retain_admitted(world, &allowed);
            for resource in &report.set.resources {
                if let Some(path)=&resource.manifest.locations {
                    if let Some(revision)=client.catalog_revisions.get(&resource.manifest.id) {
                        if let Some(loaded)=crate::locations::load_status(world,&resource.manifest.id,resource.generation,revision){ready &= loaded;continue;}
                    }
                    let root=report.roots.get(&resource.manifest.id).ok_or("Interior resource root missing")?;
                    let package=skate_resources::locations::PreparedCatalog::read(root,path)?;
                    let revision=package.revision.clone();
                    match crate::locations::load(world,&resource.manifest.id,resource.generation,package) {
                        Ok(value)=>{client.catalog_revisions.insert(resource.manifest.id.clone(),revision);ready &= value;},
                        Err(error) if error=="Interior owner is still retiring"=>ready=false,
                        Err(error)=>return Err(error),
                    }
                }
            }
        }
        if ready {let report=client.mounting.take().unwrap();activate(world,mods,client,report)?;}
    }
    if !client.channel.ready() && client.pending.is_none() && client.mounting.is_none() {
        if let Some(offer) = client.channel.offer().cloned() {
            let endpoint = SocketAddr::new(client.endpoint.unwrap().ip(), offer.port);
            let root = client.root.clone();
            let source = client.source.clone();
            let revision = offer.revision.clone();
            let asset_limits=client.asset_limits;
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let worker = std::thread::Builder::new()
                .name("resource-download".into())
                .spawn(move || {
                    let cache = Cache::open(root, asset_limits.content).map_err(|e| e.to_string())?;
                    let report =
                        skate_resources::download_set(endpoint, &revision, &cache, &source, &flag)
                            .map_err(|e| e.to_string())?;
                    // Validate all downloadable models before any VM starts, so
                    // metadata/physics model helpers share the graphics boundary.
                    if let Err(error) = validate_models(&report, &flag,asset_limits) {
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
    if let Some(mut initialization)=client.initializing.take() {
        for message in client.channel.take_incoming() {
            if message.kind==Kind::State && message.name=="__settings" && message.scope==Default::default() {
                initialization.host.apply_scoped_state(&message.resource,message.generation,&message.name,message.value,serde_json::json!({"kind":"resource"}))?;
                initialization.needed.remove(&message.resource);
            } else {
                client.deferred.push(message);
                let bytes=serde_json::to_vec(&client.deferred).map_err(|e|e.to_string())?.len();
                if client.deferred.len()>256 || bytes>client.local_budgets.queue_bytes {return Err("Initial resource messages exceeded bounded activation queue".into());}
            }
        }
        if initialization.needed.is_empty() {finish_activation(world,mods,client,initialization)?;}
        else if initialization.started.elapsed()>std::time::Duration::from_secs(15) {return Err("Server did not supply required resource settings before activation deadline".into());}
        else {client.initializing=Some(initialization);}
    }
    if let Some(host) = mods.manager.resources.as_mut() {
        let scopes:Vec<_>=visible_scopes.iter().filter_map(|scope|serde_json::to_value(scope).ok()).collect();
        host.retain_scoped_state(&scopes);
        for message in std::mem::take(&mut client.deferred).into_iter().chain(client.channel.take_incoming()) {
            if !visible_scopes.contains(&message.scope) {continue;}
            if message.kind==Kind::State && message.name==skate_net::rails::STATE_KEY {
                let allowed=client.active.as_ref().is_some_and(|active|active.grants.get(&message.resource)
                    .is_some_and(|grants|grants.iter().any(|cap|cap=="resource.world")))
                    && host.running(&message.resource) && host.generation(&message.resource)==Some(message.generation);
                if !allowed {return Err("Native rail state requires a live resource.world grant".into());}
                super::resource_world::apply(world,message.resource.clone(),message.generation,message.value.clone())?;
            }
            if message.kind==Kind::State && message.name==skate_mods::map::STATE_KEY {
                let allowed=message.scope==Default::default() && client.active.as_ref().is_some_and(|active|active.grants.get(&message.resource)
                    .is_some_and(|grants|grants.iter().any(|cap|cap=="resource.map")))
                    && host.running(&message.resource) && host.generation(&message.resource)==Some(message.generation);
                if allowed {
                    if let Err(error)=crate::map_view::set_server(world,&message.resource,message.generation,message.value.clone()) {
                        warn!("Map layer update rejected for {}: {}",message.resource,error);
                        continue;
                    }
                } else { warn!("Map layer update rejected: resource.map grant or generation unavailable"); continue; }
            }
            if message.kind==Kind::Event && message.name=="location_result" {
                crate::locations::approval(world,&message.resource,message.generation,&message.value);
                continue;
            }
            if message.kind==Kind::State && message.name=="__locations_v1" {
                let allowed=message.scope==Default::default() && client.active.as_ref().is_some_and(|a|a.grants.get(&message.resource).is_some_and(|g|g.iter().any(|c|c=="resource.locations"))) && host.running(&message.resource) && host.generation(&message.resource)==Some(message.generation);
                if !allowed {return Err("Location state requires a live resource.locations grant".into());}
                if message.value.is_null(){crate::locations::clear(world,&message.resource);}else{crate::locations::set(world,&message.resource,skate_resources::locations::LocationSnapshot::parse(message.value.clone())?)?;}
            }
            match message.kind {
                Kind::Event => host.receive(
                    0,
                    &message.resource,
                    message.generation,
                    &message.name,
                    message.value,
                )?,
                Kind::State => host.apply_scoped_state(
                    &message.resource,
                    message.generation,
                    &message.name,
                    message.value,
                    serde_json::to_value(message.scope).map_err(|e|e.to_string())?,
                )?,
            }
        }
        mods.manager.sync_resources();
        super::apply(world, mods);
    }
    publish(world, client)
}

fn is_remote_suspended(gameplay: &skate_net::dedicated::Gameplay) -> bool {
    gameplay.suspended
}

fn validate_models(report: &DownloadReport, cancel: &AtomicBool,limits:super::graphics::asset_limits::Limits) -> Result<(), String> {
    let mut decoded=0;
    for resource in &report.set.resources {
        let root = report
            .roots
            .get(&resource.manifest.id)
            .ok_or("missing materialized resource")?;
        if let Some(spec)=&resource.manifest.world {
            let bytes=skate_mods::read_bounded(root,&spec.map,512*1024*1024)?;
            let mut remaining=spec.max_decoded_bytes as usize;
            let map=skate_data::skate_map::SkateMap::parse_budgeted(&bytes,&mut remaining,false)?;
            skate_data::resource_world::validate(&map).map_err(|e|format!("{}/{}: {e}",resource.manifest.id,spec.map))?;
            for lod in &spec.lods {
                let bytes=skate_mods::read_bounded(root,&lod.map,512*1024*1024)?;
                let map=skate_data::skate_map::SkateMap::parse_budgeted(&bytes,&mut remaining,true)?;
                skate_data::resource_world::validate_render(&map).map_err(|e|format!("{}/{}: {e}",resource.manifest.id,lod.map))?;
            }
            limits.charge(&mut decoded,(spec.max_decoded_bytes as usize-remaining) as u64)?;
        }
        for path in resource.files.keys().filter(|p| p.ends_with(".glb")) {
            if cancel.load(Ordering::Relaxed) {
                return Err("resource validation cancelled".into());
            }
            let bytes = skate_mods::read_bounded(root, path, limits.model_file_bytes)?;
            let usage=super::graphics::validate_resource_glb_with_limits(&bytes,limits)
                .map_err(|e| format!("{}/{path}: {e}", resource.manifest.id))?;
            limits.charge(&mut decoded,usage)?;
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
    let initialization=(||->Result<Initializing,String>{
        let granted = grants_for(&report.set, &client.source, &read_policy(&client.root)?)?;
        let budgets=client.channel.budgets();
        let mut limits=skate_mods::resources::RuntimeLimits::default();
        limits.max_resources=budgets.resources;
        limits.max_payload_bytes=budgets.value_bytes;
        limits.max_state_keys=budgets.state_keys;
        let maximum=64*1024*1024/limits.max_payload_bytes;
        limits.max_state_keys=limits.max_state_keys.min(maximum);
        limits.max_queued_events=limits.max_queued_events.min(maximum);
        limits.max_queued_outputs=limits.max_queued_outputs.min(maximum);
        let mut host = Host::new_with_limits(Side::Client, client.root.join("state"), &client.source,limits)?;
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
        let needed=report.set.resources.iter().filter(|r|!r.manifest.settings.is_empty()).map(|r|r.manifest.id.clone()).collect();
        Ok(Initializing{host,report,granted,needed,started:std::time::Instant::now()})
    })()?;
    // Transport acknowledgement permits reliable initial settings; callbacks wait.
    client.channel.set_ready(true);
    if initialization.needed.is_empty() {finish_activation(world,mods,client,initialization)}
    else {client.initializing=Some(initialization);Ok(())}
}
fn finish_activation(world:&mut World,mods:&mut Mods,client:&mut ClientResources,initialization:Initializing)->Result<(),String> {
    let Initializing{host,report,granted,..}=initialization;
    let cache=Cache::open(&client.root,client.asset_limits.content).map_err(|e|e.to_string())?;
    let result=(||->Result<(),String>{
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
        cache.record_activation(&report.set,&client.source,&granted,None).map_err(|e|e.to_string())?;
        Ok(())
    })();
    if let Err(error)=result {let _=cache.record_activation(&report.set,&client.source,&granted,Some(&error));return Err(error);}
    info!("RESOURCE_ACTIVATED source={} revision={} downloaded_bytes={} reused_bytes={}",client.source,report.set.revision,report.downloaded_bytes,report.reused_bytes);
    client.active=Some(Active{set:report.set,grants:granted});Ok(())
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
    if client.initializing.is_some() {net.status="Applying server settings before resource callbacks…".into();}
    else if !client.channel.ready() {
        net.status = if client.mounting.is_some() {"Preparing required world and collision..."} else {"Downloading and verifying required server resources..."}.into();
    }
    Ok(())
}

pub(super) fn flush(world: &mut World) {
    world.resource_scope(|world, mut client: Mut<ClientResources>| {
        if client.endpoint.is_none() || !client.channel.ready() || client.initializing.is_some() {
            return;
        }
        world.resource_scope(|world, mut mods: Mut<Mods>| {
            let result = (|| {
                let host = mods
                    .manager
                    .resources
                    .as_mut()
                    .ok_or("required resource host is not running")?;
                if let Some(sampled) = client.diagnostic_sample.as_mut()
                    && sampled.elapsed() >= std::time::Duration::from_secs(5)
                {
                    for (id, metrics) in host.runtime_metrics() {
                        if let Ok(value) = serde_json::to_string(&metrics) {
                            info!("RESOURCE_RUNTIME_METRICS {id}: {value}");
                        }
                    }
                    *sampled = std::time::Instant::now();
                }
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
                let state=&mut *client;
                let events=state.transfers.poll(std::time::Instant::now(),
                    |id,generation|host.running(id)&&host.generation(id)==Some(generation),
                    |ticket,cancel| {
                        if cancel {state.channel.cancel_ticket(ticket)?;}
                        state.channel.large_progress(ticket)
                    });
                for event in events {let _=host.host_event(&event.resource,event.generation,"transfer_progress",event.value);}
                for output in host.drain_outputs() {
                    match output {
                        Output::Event {
                            resource,
                            generation,
                            name,
                            payload,
                            ..
                        } => client.channel.emit(&resource, generation, &name, payload)?,
                        Output::Transfer {resource,generation,key,name,payload,recipient,timeout_ms} => {
                            let result=(|| {
                                client.transfers.available(&resource,generation,&key,timeout_ms)?;
                                if recipient.is_some() {return Err("Client large transfers target the server only".into());}
                                let ticket=client.channel.start_large(&resource,generation,&name,payload)?;
                                let progress=client.channel.large_progress(ticket)?;
                                client.transfers.insert(&resource,generation,&key,timeout_ms,ticket,progress,std::time::Instant::now())
                            })();
                            let event=result.unwrap_or_else(|error|skate_net::transfers::Transfers::failed(&resource,generation,&key,error));
                            let _=host.host_event(&event.resource,event.generation,"transfer_progress",event.value);
                        }
                        Output::CancelTransfer {resource,generation,key} => {
                            let client=&mut *client;
                            let event=client.transfers.cancel(&resource,generation,&key,|ticket,cancel| {
                                if cancel {client.channel.cancel_ticket(ticket)?;}
                                client.channel.large_progress(ticket)
                            });
                            let _=host.host_event(&event.resource,event.generation,"transfer_progress",event.value);
                        }
                        Output::Log { resource, text } => info!("RESOURCE_LOG {resource}: {text}"),
                        Output::Competition { .. } | Output::World { .. } | Output::Voice { .. } | Output::Entity { .. } | Output::State { .. } | Output::Teleport { .. } | Output::Service { .. } | Output::CancelService { .. } => {
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
    #[test]
    fn ragdoll_does_not_hide_remote_player_but_explicit_suspension_does() {
        let mut gameplay = skate_net::dedicated::Gameplay {
            mode: skate_net::dedicated::PlayerMode::Ragdoll,
            ..Default::default()
        };
        assert!(!is_remote_suspended(&gameplay));
        gameplay.suspended = true;
        assert!(is_remote_suspended(&gameplay));
    }

    #[test]
    fn local_network_budgets_are_bounded_and_reject_unknown_fields() {
        let root = std::env::temp_dir().join(format!("skate-local-network-budgets-{}-{}",
            std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(read_network_budgets(&root).unwrap().value_bytes, 16 * 1024);
        let path = root.join("network-budgets.json");
        std::fs::write(&path, r#"{"value_bytes":262144}"#).unwrap();
        assert_eq!(read_network_budgets(&root).unwrap().value_bytes, 256 * 1024);
        for invalid in [r#"{"value_bytes":262145}"#, r#"{"unbounded":true}"#, "[]"] {
            std::fs::write(&path, invalid).unwrap();
            assert!(read_network_budgets(&root).is_err(), "accepted {invalid}");
        }
        std::fs::write(&path, " ".repeat(16385)).unwrap();
        assert!(read_network_budgets(&root).unwrap_err().contains("16KiB"));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(read_network_budgets(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
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
            map_path: None, locations: None,
            difficulty: Default::default(),
            check_assets: false,
            validate_maps: false,
            mute: false,
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

pub(crate) fn location_event(world:&mut World,owner:&str,generation:u64,name:&str,value:serde_json::Value)->Result<(),String>{
    world.resource_scope(|world,mut client:Mut<ClientResources>|{
        if !client.channel.ready()||client.initializing.is_some(){return Err("Interior resources are not admitted".into());}
        if !client.active.as_ref().is_some_and(|a|a.set.resources.iter().any(|r|r.manifest.id==owner&&r.generation==generation&&r.manifest.locations.is_some())){return Err("Interior catalog is not in the admitted set".into());}
        client.channel.emit(owner,generation,name,value)?;publish(world,&client)
    })
}
pub(crate) fn location_world_revision(world:&World)->Option<String>{
    let active=world.get_resource::<ClientResources>()?.active.as_ref()?;
    active.set.resources.iter().any(|r|r.manifest.locations.is_some()).then(||active.set.revision.clone())
}
