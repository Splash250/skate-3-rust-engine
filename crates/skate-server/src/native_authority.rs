//! Resource-owned native skating attempts. The trusted game companion runs the
//! recovered solver and scorer; the network only supplies bounded controller input.
#[cfg(test)]
#[path = "native_authority_tests.rs"]
mod integration_tests;
use crate::competition::OwnedEvent;
use serde::Deserialize;
use serde_json::{Value, json};
use skate_net::{
    dedicated::{Server, TeleportDestination},
    native_authority::{
        Admission, InputLog, MAX_REPLY_BYTES, MAX_TICKS, Reply, Request, Snapshot, State, Status,
        VERSION,
    },
};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub executable: PathBuf,
    pub assets: PathBuf,
    #[serde(default = "default_workers")]
    pub max_workers: usize,
}
fn default_workers() -> usize {
    1
}
impl Config {
    pub fn resolve(&mut self, parent: &Path) -> Result<(), String> {
        self.executable = parent
            .join(&self.executable)
            .canonicalize()
            .map_err(|e| format!("Native authority executable: {e}"))?;
        self.assets = parent
            .join(&self.assets)
            .canonicalize()
            .map_err(|e| format!("Native authority assets: {e}"))?;
        if !self.executable.is_file()
            || !self.assets.is_dir()
            || !(1..=4).contains(&self.max_workers)
        {
            return Err("Native authority requires a trusted executable, owned asset directory and 1..4 workers".into());
        }
        Ok(())
    }
}

#[derive(Clone)]
struct World {
    revision: String,
    bytes: Arc<Vec<u8>>,
    catalogs:Vec<skate_resources::locations::PreparedCatalog>,
    spawn: [f32; 3],
    heading: f32,
}

/// One IO thread per bounded worker. The game loop never waits on native asset
/// loading or simulation. Dropping the owner kills its child, including during
/// startup and a blocked read; the IO thread reaps it and removes its scratch map.
struct Worker {
    commands: SyncSender<Request>,
    replies: Receiver<Result<Reply, String>>,
    child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<AtomicBool>,
}
impl Worker {
    fn spawn(
        config: Config,
        world: World,
        admission: Admission,
        workers: Arc<AtomicUsize>,
    ) -> Self {
        workers.fetch_add(1, Ordering::AcqRel);
        let (commands, receive) = mpsc::sync_channel(4);
        let (send, replies) = mpsc::sync_channel(8);
        let child = Arc::new(Mutex::new(None::<Child>));
        let cancelled = Arc::new(AtomicBool::new(false));
        let process = child.clone();
        let stopped = cancelled.clone();
        std::thread::spawn(move || {
            struct Permit(Arc<AtomicUsize>);
            impl Drop for Permit {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _permit = Permit(workers);
            let result = worker_io(
                config, world, admission, &receive, &send, &process, &stopped,
            );
            if let Err(error) = result {
                let _ = send.try_send(Err(error));
            }
            let child = { process.lock().unwrap_or_else(|p| p.into_inner()).take() };
            if let Some(mut child) = child {
                let _ = child.kill();
                let _ = child.wait();
            }
        });
        Self {
            commands,
            replies,
            child,
            cancelled,
        }
    }
    fn send(&self, request: Request) -> Result<bool, String> {
        match self.commands.try_send(request) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(_)) => Ok(false),
            Err(mpsc::TrySendError::Disconnected(_)) => Err("Native worker input closed".into()),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Some(child) = self
            .child
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            let _ = child.kill();
        }
    }
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn create() -> Result<Self, String> {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skate-native-authority-{}-{nanos}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn read_reply(reader: &mut impl BufRead) -> Result<Reply, String> {
    let mut line = Vec::new();
    loop {
        let bytes = reader.fill_buf().map_err(|e| e.to_string())?;
        if bytes.is_empty() {
            return Err("Native worker closed its response stream".into());
        }
        let n = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |n| n + 1);
        if line.len() + n > MAX_REPLY_BYTES {
            return Err("Native worker response exceeds32KiB".into());
        }
        let complete = bytes[n - 1] == b'\n';
        line.extend_from_slice(&bytes[..n]);
        reader.consume(n);
        if complete {
            return serde_json::from_slice(&line)
                .map_err(|e| format!("Native worker protocol: {e}"));
        }
    }
}
fn worker_io(
    config: Config,
    world: World,
    admission: Admission,
    receive: &Receiver<Request>,
    send: &SyncSender<Result<Reply, String>>,
    process: &Mutex<Option<Child>>,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let scratch = Scratch::create()?;
    let map = scratch.0.join("world.skate");
    std::fs::write(&map, world.bytes.as_slice()).map_err(|e| e.to_string())?;
    if cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    let locations=scratch.0.join("locations");std::fs::create_dir_all(&locations).map_err(|e|e.to_string())?;
    let mut index=Vec::new();
    for (n,p) in world.catalogs.iter().enumerate(){
        let root=locations.join(n.to_string());std::fs::create_dir_all(&root).map_err(|e|e.to_string())?;
        for (path,bytes) in &p.files {let target=root.join(path);if let Some(parent)=target.parent(){std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}std::fs::write(target,bytes).map_err(|e|e.to_string())?;}
        std::fs::write(root.join("catalog.json"),serde_json::to_vec(&p.catalog).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;index.push(n.to_string());
    }
    std::fs::write(locations.join("index.json"),serde_json::to_vec(&index).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let mut child = Command::new(&config.executable)
        .arg("--native-authority")
        .arg("--locations")
        .arg(&locations)
        .arg("--assets")
        .arg(&config.assets)
        .arg("--map")
        .arg(&map)
        .arg("--difficulty")
        .arg("normal")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Native worker launch: {e}"))?;
    let mut input = child.stdin.take().ok_or("Native worker stdin missing")?;
    let mut output = BufReader::new(child.stdout.take().ok_or("Native worker stdout missing")?);
    // Share the kill handle before potentially blocking on any pipe operation.
    *process.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);
    if cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut next = Some(Request::Start { admission });
    loop {
        let request = match next.take() {
            Some(request) => request,
            None => match receive.recv_timeout(Duration::from_millis(100)) {
                Ok(request) => request,
                Err(mpsc::RecvTimeoutError::Timeout) if !cancelled.load(Ordering::Acquire) => {
                    continue;
                }
                Err(_) => return Ok(()),
            },
        };
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        input
            .write_all(&bytes)
            .and_then(|_| input.write_all(b"\n"))
            .and_then(|_| input.flush())
            .map_err(|e| e.to_string())?;
        let reply = read_reply(&mut output)?;
        send.try_send(Ok(reply))
            .map_err(|_| "Native worker response backpressure")?;
    }
}

struct Attempt {
    owner: String,
    generation: u64,
    world: String,
    ticks: u64,
    log: InputLog,
    applied: InputLog,
    sent: usize,
    state: State,
    worker: Worker,
    created: Instant,
    started_ms: Option<u64>,
    last_input: u64,
    last_reply: Instant,
}

#[derive(Default)]
pub struct NativeAuthority {
    config: Option<Config>,
    world: Option<World>,
    owners: BTreeMap<String, u64>,
    attempts: BTreeMap<u64, Attempt>,
    workers: Arc<AtomicUsize>,
}
impl NativeAuthority {
    pub fn new(config: Option<Config>) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }
    pub fn set_world(&mut self, published: &skate_resources::PublishedSet) -> Result<(), String> {
        if self.config.is_none() {
            return Ok(());
        }
        let Some(resource) = published
            .set
            .resources
            .iter()
            .find(|r| r.manifest.world.is_some())
        else {
            self.world = None;
            return Ok(());
        };
        let world = resource.manifest.world.as_ref().unwrap();
        let file = resource
            .files
            .get(&world.map)
            .ok_or("Native authority world file missing")?;
        if self
            .world
            .as_ref()
            .is_some_and(|old| old.revision == if published.set.resources.iter().any(|r|r.manifest.locations.is_some()){published.set.revision.clone()}else{file.digest.clone()})
        {
            return Ok(());
        }
        // Reuse the already validated publication, never reread a mutable source path.
        let terrain =
            crate::world::Terrain::from_published(published)?.ok_or("Native world unavailable")?;
        self.world = Some(World {
            revision: terrain.revision.clone(),
            catalogs:{let mut resources=published.set.resources.iter().collect::<Vec<_>>();resources.sort_by_key(|r| &r.manifest.id);resources.into_iter().filter_map(|r|skate_resources::locations::PreparedCatalog::from_resource(r,&published.blobs).transpose()).collect::<Result<_,_>>()?},
            bytes: Arc::new(
                published
                    .blobs
                    .get(&file.digest)
                    .ok_or("Native world blob missing")?
                    .clone(),
            ),
            spawn: terrain.spawn,
            heading: terrain.heading,
        });
        Ok(())
    }
    pub fn sync_resources(&mut self, owners: BTreeMap<String, u64>) {
        self.owners = owners;
    }
    pub fn active(&self, actor: u64) -> bool {
        self.attempts.contains_key(&actor)
    }
    pub fn stop(&mut self) {
        self.attempts.clear();
    }
    pub fn command(
        &mut self,
        owner: &str,
        generation: u64,
        value: Value,
        server: &mut Server,
        blocked_instances: &std::collections::BTreeSet<u64>,
    ) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Operation {
            NativeStart { player: String, ticks: u64 },
            NativeCancel { player: String },
        }
        if self.owners.get(owner) != Some(&generation) || generation == 0 {
            return Err("Native competition owner is not active".into());
        }
        let operation: Operation = serde_json::from_value(value)
            .map_err(|e| format!("Invalid native competition command: {e}"))?;
        let (player, ticks) = match operation {
            Operation::NativeStart { player, ticks } => (player, Some(ticks)),
            Operation::NativeCancel { player } => (player, None),
        };
        let actor = player
            .parse::<u64>()
            .ok()
            .filter(|n| *n != 0 && n.to_string() == player)
            .ok_or("Invalid native competition player")?;
        if ticks.is_none() {
            let a = self
                .attempts
                .get(&actor)
                .ok_or("No native attempt for player")?;
            if a.owner != owner || a.generation != generation {
                return Err("Native attempt belongs to another resource".into());
            }
            let a = self.attempts.remove(&actor).unwrap();
            let mut state = a.state.clone();
            state.status = Status::Cancelled;
            state.reason = Some("resource_cancelled".into());
            server.publish_native(actor, state)?;
            return Ok(json!({"cancelled":true}));
        }
        let ticks = ticks.unwrap();
        let config = self.config.clone().ok_or(
            "Native authority is not configured; trusted game executable and owned assets required",
        )?;
        if !(1..=MAX_TICKS).contains(&ticks)
            || self.attempts.contains_key(&actor)
            || self.workers.load(Ordering::Acquire) >= config.max_workers
        {
            return Err("Native attempt length/player/worker capacity invalid".into());
        }
        let world = self
            .world
            .clone()
            .ok_or("Native authority requires a verified resource world")?;
        let instance = server
            .instance_of(actor)
            .ok_or("Native actor unavailable")?;
        if !server.resource_ready(actor)
            || blocked_instances.contains(&instance)
            || !solitary(server, actor, instance)
        {
            return Err("Native-v1 requires an admitted solitary instance without shared objects or resource rails".into());
        }
        let epoch = server.teleport_now(
            actor,
            TeleportDestination {
                position: world.spawn,
                heading: world.heading,
                velocity: [0.; 3],
                instance,
            },
        )?;
        let admission = Admission {
            version: VERSION,
            epoch,
            instance,
            generation,
        };
        let log = InputLog::new(admission)?;
        let state = State {
            resource: owner.into(),
            admission,
            tick: 0,
            history_digest: log.digest(),
            state_digest: String::new(),
            score: None,
            status: Status::Starting,
            world: world.revision.clone(),
            difficulty: "normal".into(),
            ticks,
            reason: None,
        };
        server.publish_native(actor, state.clone())?;
        let worker = Worker::spawn(config, world.clone(), admission, self.workers.clone());
        self.attempts.insert(
            actor,
            Attempt {
                owner: owner.into(),
                generation,
                world: world.revision,
                ticks,
                applied: InputLog::new(admission)?,
                sent: 0,
                log,
                state,
                worker,
                created: Instant::now(),
                started_ms: None,
                last_input: server.now_ms(),
                last_reply: Instant::now(),
            },
        );
        Ok(
            json!({"started":true,"player":player,"movement_epoch":epoch.to_string(),"verified_rules":"native-input-v1","ticks":ticks}),
        )
    }
    pub fn step(
        &mut self,
        server: &mut Server,
        blocked_instances: &std::collections::BTreeSet<u64>,
    ) -> Vec<OwnedEvent> {
        let mut events = Vec::new();
        for actor in self.attempts.keys().copied().collect::<Vec<_>>() {
            let mut a = self.attempts.remove(&actor).unwrap();
            let admission = a.log.admission();
            let lifecycle = if self.owners.get(&a.owner) != Some(&a.generation) {
                Some("resource_retired")
            } else if self
                .world
                .as_ref()
                .is_none_or(|world| world.revision != a.world)
            {
                Some("world_changed")
            } else if server.movement_epoch_of(actor) != Some(admission.epoch)
                || server.instance_of(actor) != Some(admission.instance)
            {
                Some("movement_epoch_changed")
            } else if blocked_instances.contains(&admission.instance)
                || !solitary(server, actor, admission.instance)
            {
                Some("instance_changed")
            } else {
                None
            };
            let result = if let Some(reason) = lifecycle {
                Err((Status::Cancelled, reason.to_owned()))
            } else {
                advance(&mut a, actor, server).map_err(|error| (Status::Rejected, error))
            };
            match result {
                Ok(false) => {
                    self.attempts.insert(actor, a);
                }
                Ok(true) => {
                    a.state.status = Status::Completed;
                    let _ = server.publish_native(actor, a.state.clone());
                    events.push(OwnedEvent{resource:a.owner,generation:a.generation,value:json!({"kind":"completed","player":actor.to_string(),"verified_rules":"native-input-v1","ticks":a.state.tick,"score":a.state.score})});
                }
                Err((status, reason)) => {
                    a.state.status = status;
                    a.state.reason = Some(reason.clone());
                    a.state.score = None;
                    if server.movement_epoch_of(actor) == Some(admission.epoch) {
                        let _ = server.publish_native(actor, a.state.clone());
                    } else {
                        server.clear_native(actor);
                    }
                    if self.owners.get(&a.owner) == Some(&a.generation) {
                        events.push(OwnedEvent{resource:a.owner,generation:a.generation,value:json!({"kind":if status==Status::Cancelled{"cancelled"}else{"rejected"},"player":actor.to_string(),"reason":reason,"score":0,"verified_rules":"native-input-v1"})});
                    }
                }
            }
        }
        events
    }
}
fn solitary(server: &Server, actor: u64, instance: u64) -> bool {
    server.native_instance_solitary(actor, instance)
}
fn advance(a: &mut Attempt, actor: u64, server: &mut Server) -> Result<bool, String> {
    let now = server.now_ms();
    // Per-packet timeouts alone permit a participant to retain a scarce native
    // worker by sending one tick just before each timeout. Bound the whole run,
    // including 15s for initial client loading and 5s for delivery/replay grace.
    if a.started_ms.is_some_and(|started| {
        now.saturating_sub(started) > (a.ticks * 1_000).div_ceil(60) + 20_000
    }) {
        return Err("native_attempt_deadline".into());
    }
    for _ in 0..8 {
        let reply = match a.worker.replies.try_recv() {
            Ok(reply) => reply?,
            Err(mpsc::TryRecvError::Empty) => break,
            Err(_) => return Err("native_worker_closed".into()),
        };
        let snapshot = match reply {
            Reply::Ready { snapshot } if a.started_ms.is_none() => {
                a.started_ms = Some(now);
                a.last_input = now;
                snapshot
            }
            Reply::Advanced { snapshot } if a.started_ms.is_some() => snapshot,
            Reply::Rejected { error } => return Err(error.chars().take(128).collect()),
            _ => return Err("native_worker_reply_order".into()),
        };
        validate_snapshot(a, &snapshot)?;
        a.last_reply = Instant::now();
        a.state = snapshot.state(Status::Running, a.world.clone(), "normal".into(), a.ticks);
        a.state.resource = a.owner.clone();
        server.publish_native(actor, a.state.clone())?;
        if a.state.tick == a.ticks {
            return Ok(true);
        }
    }
    let Some(started) = a.started_ms else {
        if a.created.elapsed() > Duration::from_secs(30) {
            return Err("native_worker_startup_timeout".into());
        }
        return Ok(false);
    };
    let input_deadline = if a.log.is_empty() { 15_000 } else { 2_000 };
    if now.saturating_sub(a.last_input) > input_deadline {
        return Err("native_input_timeout".into());
    }
    if a.log.len() > a.applied.len() && a.last_reply.elapsed() > Duration::from_secs(2) {
        return Err("native_worker_step_timeout".into());
    }
    if let Some(packet) = server.native_input(actor) {
        for input in packet.inputs {
            if input.epoch != a.log.admission().epoch {
                continue;
            } // stale retransmission after a trusted reset
            if input.tick > a.ticks {
                return Err("native_input_past_attempt".into());
            }
            let idle = a.log.len() == a.applied.len();
            if a.log.append_at(input, now.saturating_sub(started))? {
                if idle {
                    // A legitimate client load or idle period may follow the
                    // previous reply. Newly pending work gets a fresh deadline.
                    a.last_reply = Instant::now();
                }
                a.last_input = now;
            }
        }
    }
    // Delayed windows may exceed the pipe queue. Refill from the bounded
    // accepted journal as replies progress instead of rejecting backpressure.
    while a.sent < a.log.len() && a.sent.saturating_sub(a.applied.len()) < 4 {
        if !a.worker.send(Request::Step {
            input: a.log.inputs()[a.sent].clone(),
        })? {
            break;
        }
        a.sent += 1;
    }
    Ok(false)
}
fn validate_snapshot(a: &mut Attempt, snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.admission != a.log.admission()
        || !snapshot.root.valid()
        || snapshot.bodies.len() != skate_net::BODY_COUNT
        || snapshot.tick > a.sent as u64
        || snapshot.tick < a.applied.len() as u64
        || (snapshot.tick != 0 && snapshot.tick != a.applied.len() as u64 + 1)
    {
        return Err("native_worker_snapshot_invalid".into());
    }
    if snapshot.tick > 0 {
        a.applied
            .append(a.log.inputs()[snapshot.tick as usize - 1].clone())?;
    }
    if snapshot.history_digest != a.applied.digest() {
        return Err("native_worker_history_mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_framing_rejects_overlong_truncated_and_forged_outcome_fields() {
        assert!(
            read_reply(&mut &vec![b'x'; MAX_REPLY_BYTES + 1][..])
                .unwrap_err()
                .contains("exceeds")
        );
        assert!(read_reply(&mut &b"{\"kind\":\"rejected\",\"error\":\"x\"}"[..]).is_err());
        assert!(
            read_reply(&mut &b"{\"kind\":\"rejected\",\"error\":\"x\",\"score\":99}\n"[..])
                .is_err()
        );
        assert!(matches!(
            read_reply(&mut &b"{\"kind\":\"rejected\",\"error\":\"x\"}\n"[..]).unwrap(),
            Reply::Rejected { .. }
        ));
    }
    #[test]
    fn disabled_authority_never_starts_or_accepts_client_scores() {
        let mut native = NativeAuthority::default();
        native.sync_resources(BTreeMap::from([("rules".into(), 1)]));
        let mut server = Server::new(skate_net::dedicated::Config {
            session: 1,
            server_id: 99,
            map: 1,
            max_players: 4,
        })
        .unwrap();
        let blocked = Default::default();
        let command = json!({"kind":"native_start","player":"1","ticks":60});
        assert!(
            native
                .command("rules", 1, command.clone(), &mut server, &blocked)
                .unwrap_err()
                .contains("not configured")
        );
        assert!(
            native
                .command("rules", 2, command.clone(), &mut server, &blocked)
                .unwrap_err()
                .contains("not active")
        );
        let mut forged = command;
        forged["score"] = json!(100000);
        assert!(
            native
                .command("rules", 1, forged, &mut server, &blocked)
                .unwrap_err()
                .contains("Invalid native")
        );
        assert!(native.attempts.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn retiring_a_blocked_worker_kills_reaps_and_releases_its_slot() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::create().unwrap();
        let executable = scratch.0.join("blocked-worker");
        // Deliberately never replies. This is process/pipe lifecycle evidence,
        // not an alternative physics implementation or scoring fixture.
        std::fs::write(
            &executable,
            b"#!/usr/bin/env python3\nimport sys,time\nsys.stdin.readline()\ntime.sleep(30)\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let workers = Arc::new(AtomicUsize::new(0));
        let worker = Worker::spawn(
            Config {
                executable,
                assets: scratch.0.clone(),
                max_workers: 1,
            },
            World {
                revision: "fixture".into(),
                catalogs:vec![],
                bytes: Arc::new(vec![]),
                spawn: [0.; 3],
                heading: 0.,
            },
            Admission {
                version: VERSION,
                epoch: 1,
                instance: 7,
                generation: 1,
            },
            workers.clone(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Some(child) = worker.child.lock().unwrap().as_ref() {
                break child.id();
            }
            assert!(Instant::now() < deadline, "worker was not spawned");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(workers.load(Ordering::Acquire), 1);
        drop(worker);
        while workers.load(Ordering::Acquire) != 0 {
            assert!(
                Instant::now() < deadline,
                "retired worker retained its slot"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        #[cfg(target_os = "linux")]
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "retired native process was not reaped"
        );
    }
}
