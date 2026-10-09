//! A bounded, asset-free workload through real UDP sockets and server Lua.
//! These peers simulate owners; this does not claim authoritative skate physics.
use serde_json::json;
use skate_net::{
    Body, Bone, Pose,
    lobby::{Info, Session},
    packed::{self, BodyState, Packed, PoseState},
    resources::{CLIENT_KEY, Client as Resources, Kind, ServerRecord, server_key},
};
use skate_server::{Host, Map, Options};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    time::{Duration, Instant},
};

const PLAYERS: usize = 64;
const ECHOES: u64 = 4;
struct Fixture(PathBuf);
impl Fixture {
    fn new(players: usize) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "skate-capacity-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("resources/load")).unwrap();
        std::fs::write(
            root.join("resources/load/resource.json"),
            serde_json::to_vec(&json!({
                "format":1,"api":1,"id":"load","version":"1.0.0","language":"lua",
                "server_scripts":["server.lua"],"capabilities":["resource.network","resource.state","resource.entities","resource.voice"]
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("resources/load/server.lua"),
            "resource.state.set('shared',{ready=true}); resource.on_net('echo',function(data,sender) resource.send('echo',data,sender) end)\nfor i=1,2 do resource.entity({op='spawn',key='motion'..i,shape={type='box',half_extents={0.5,0.5,0.5}},body_type='kinematic',position={0,5,i*10},velocity={1,0,0}}) end"
        ).unwrap();
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(root.join("resources/load/server.lua")).unwrap()
            .write_all(format!("\nlocal radio=false; return {{on_update=function() if radio then return end; local members={{}}; for _,p in ipairs(resource.players()) do if p.position then members[#members+1]=p.id end end; if #members=={players} then resource.voice.submit({{kind='channel',name='radio',members=members}});radio=true end end}}").as_bytes()).unwrap();
        std::fs::write(
            root.join("server.json"),
            serde_json::to_vec(&json!({
                "root":"resources","storage":"store","ensure":["load"],
                "grants":{"load":["resource.network","resource.state","resource.entities","resource.voice"]}
            }))
            .unwrap(),
        )
        .unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Peer {
    socket: UdpSocket,
    lobby: Session,
    resources: Resources,
    enqueued: bool,
    echoes: BTreeSet<u64>,
    shared: bool,
    last_body: u64,
    tx: u64,
    rx: u64,
    tx_packets: u64,
    rx_packets: u64,
    discarded: u64,
    observed: BTreeSet<u64>,
    ages: Vec<u64>,
    echo_latency: Vec<u64>,
    voice: skate_voice::ClientState,
    last_voice: u64,
    voice_senders: BTreeSet<u64>,
    voice_ages: Vec<u64>,
    voice_frames: u64,
    entity_ticks: BTreeMap<u64, u64>,
    entity_samples: u64,
    outbound: Option<DelayedLink>,
    inbound: Option<DelayedLink>,
    pending_echoes: BTreeMap<u64, u64>,
}
impl Peer {
    fn new(id: u64) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        skate_net::socket::configure(&socket).unwrap();
        Self {
            socket,
            lobby: Session::dedicated_client(
                7,
                Info {
                    id,
                    map: skate_net::hash(b"skate-test-world-v1"),
                    rig: 2,
                    physics: 3,
                    appearance: 4,
                },
                1,
            ),
            resources: Resources::default(),
            enqueued: false,
            echoes: BTreeSet::new(),
            shared: false,
            last_body: 0,
            tx: 0,
            rx: 0,
            tx_packets: 0,
            rx_packets: 0,
            discarded: 0,
            observed: BTreeSet::new(),
            ages: Vec::new(),
            echo_latency: Vec::new(),
            voice: Default::default(),
            last_voice: 0,
            voice_senders: Default::default(),
            voice_ages: vec![],
            voice_frames: 0,
            entity_ticks: BTreeMap::new(),
            entity_samples: 0,
            outbound: None,
            inbound: None,
            pending_echoes: BTreeMap::new(),
        }
    }
    fn send(&mut self, address: SocketAddr, index: usize, now: u64, voice_packet: &[u8]) {
        if self.resources.ready() && now.saturating_sub(self.last_body) >= 100 {
            let root = Pose {
                p: [
                    (index % 8) as f32 * 20. + now as f32 / 1000.,
                    1.,
                    (index / 8) as f32 * 20.,
                ],
                q: [0., 0., 0., 1.],
            };
            self.lobby.publish(
                packed::BODY,
                Packed::body(&BodyState {
                    root,
                    enabled: (1 << 33) - 1,
                    bodies: vec![
                        Body {
                            pose: root,
                            velocity: [1., 0., 0.],
                            angular: [0.; 3]
                        };
                        33
                    ],
                })
                .unwrap(),
                now,
            );
            self.lobby.publish(
                packed::POSE,
                Packed::pose(&PoseState {
                    root,
                    bones: (0..32).map(|index| Bone { index, pose: root }).collect(),
                })
                .unwrap(),
                now,
            );
            self.last_body = now;
        }
        if self.resources.offer().is_some() {
            self.lobby
                .publish_application(CLIENT_KEY, self.resources.encode().unwrap(), now);
        }
        if index < 8
            && self.voice.revision() > 0
            && self.lobby.movement_epoch() > 0
            && now.saturating_sub(self.last_voice) >= 20
        {
            let packet = skate_voice::wire::Frame {
                session: 7,
                actor: self.lobby.local,
                epoch: self.lobby.movement_epoch(),
                revision: self.voice.revision(),
                sequence: now,
                channel: "load/radio".into(),
                data: voice_packet.to_vec(),
            }
            .encode()
            .unwrap();
            self.send_packet(address, now, packet);
            self.tx_packets += 1;
            self.last_voice = now;
        }
        for packet in self.lobby.service(now) {
            assert!(packet.data.len() <= packed::MTU);
            self.tx_packets += 1;
            // Drop real outbound movement datagrams deterministically. Reliable
            // resource transfers must progress while snapshot loss is present.
            if self.tx_packets % 53 == 0
                && packed::envelope(&packet.data)
                    .is_some_and(|(_, _, kind, _)| kind == packed::BODY)
            {
                self.discarded += 1;
                continue;
            }
            self.send_packet(address, now, packet.data);
        }
        if let Some(link) = &mut self.outbound {
            for packet in link.drain(now) {
                self.tx += self.socket.send_to(&packet, address).unwrap() as u64;
            }
        }
    }
    fn send_packet(&mut self, address: SocketAddr, now: u64, packet: Vec<u8>) {
        if let Some(link) = &mut self.outbound {
            link.enqueue(now, packet);
        } else {
            self.tx += self.socket.send_to(&packet, address).unwrap() as u64;
        }
    }
    fn accept_packet(&mut self, packet: &[u8], now: u64) {
        if skate_voice::wire::is_voice(packet) {
            if let Some(skate_voice::Incoming::Playback { packet, .. }) =
                self.voice.accept(1, packet)
            {
                self.voice_senders.insert(packet.actor);
                self.voice_frames += 1;
                self.voice_ages.push(now.saturating_sub(packet.sequence));
            }
        } else {
            self.lobby.receive(1, packet, now);
            if self.lobby.connected() {
                self.voice.synchronize(Some(skate_voice::Context {
                    session: 7,
                    host_peer: 1,
                    actor: self.lobby.local,
                    epoch: self.lobby.movement_epoch(),
                    instance: self.lobby.actors[&self.lobby.local].instance,
                }));
            }
        }
    }
    fn enqueue_echo(&mut self, seq: u64, now: u64) {
        self.resources
            .emit(
                "load",
                1,
                "echo",
                json!({"seq":seq,"sent":now,"padding":"x".repeat(1024)}),
            )
            .unwrap();
        assert!(self.pending_echoes.insert(seq, now).is_none());
    }
    fn receive(&mut self, address: SocketAddr, now: u64) {
        let mut bytes = [0u8; 1500];
        loop {
            match self.socket.recv_from(&mut bytes) {
                Ok((len, from)) => {
                    assert_eq!(from, address);
                    assert!(len <= packed::MTU);
                    self.rx += len as u64;
                    self.rx_packets += 1;
                    if self.rx_packets % 71 == 0 {
                        self.discarded += 1;
                        continue;
                    }
                    if let Some(link) = &mut self.inbound {
                        link.enqueue(now, bytes[..len].to_vec());
                    } else {
                        self.accept_packet(&bytes[..len], now);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("{error}"),
            }
        }
        if let Some(link) = &mut self.inbound {
            let ready = link.drain(now);
            for packet in ready {
                self.accept_packet(&packet, now);
            }
        }
        if let Some(server) = self.lobby.host_actor() {
            if let Some(record) = self.lobby.actors[&server]
                .application
                .get(&server_key(self.lobby.local))
            {
                let record: ServerRecord = serde_json::from_slice(&record.value).unwrap();
                self.resources.receive(&record).unwrap();
                // Simulated resource owners acknowledge the fixture content set;
                // HTTP download/install behavior has its own integration suite.
                self.resources.set_ready(true);
            }
        }
        if self.resources.ready() && !self.enqueued {
            for seq in 1..=ECHOES {
                self.enqueue_echo(seq, now);
            }
            self.enqueued = true;
        }
        for msg in self.resources.take_incoming() {
            match msg.kind {
                Kind::Event => {
                    assert_eq!(msg.name, "echo");
                    assert_eq!(msg.value["padding"].as_str().unwrap().len(), 1024);
                    let seq = msg.value["seq"].as_u64().unwrap();
                    assert!(self.echoes.insert(seq), "Resource echo applied twice");
                    assert_eq!(
                        self.pending_echoes.remove(&seq),
                        msg.value["sent"].as_u64(),
                        "unsolicited echo"
                    );
                    self.echo_latency
                        .push(now.saturating_sub(msg.value["sent"].as_u64().unwrap()));
                }
                Kind::State => {
                    if msg.name == "shared" {
                        self.shared = msg.value["ready"] == true;
                    }
                }
            }
        }
        for (&id, actor) in &self.lobby.actors {
            if id == self.lobby.local {
                continue;
            }
            if let Some(latest) = actor.body.latest() {
                self.observed.insert(id);
                if latest.received == now {
                    self.ages.push(now.saturating_sub(latest.state.captured));
                }
            }
        }
        for (&id, e) in self.lobby.entities.entities() {
            if self.entity_ticks.insert(id, e.tick) != Some(e.tick) {
                self.entity_samples += 1;
            }
        }
    }
}
fn rss_kib(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|line| line.starts_with(field))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
fn percentile(mut samples: Vec<u64>, percent: usize) -> u64 {
    assert!(!samples.is_empty());
    samples.sort_unstable();
    samples[(samples.len() - 1) * percent / 100]
}
#[test]
fn sixty_four_real_udp_owners_exchange_movement_shared_state_and_large_resource_events() {
    let fixture = Fixture::new(PLAYERS);
    let before = rss_kib("VmRSS:");
    let mut host = Host::bind(Options {
        accounts: None,
        operations: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        session: 7,
        max_players: PLAYERS,
        map: Map::TestWorld, locations:None,
        resources: Some(fixture.0.join("server.json")),
    })
    .unwrap();
    let address = host.local_addr().unwrap();
    let mut peers: Vec<_> = (0..PLAYERS).map(|i| Peer::new(100 + i as u64)).collect();
    let tone =
        std::array::from_fn(|i| (i as f32 * std::f32::consts::TAU * 440. / 48000.).sin() * 0.3);
    let voice_packet = skate_voice::Encoder::new().unwrap().encode(&tone).unwrap();
    let started = Instant::now();
    let deadline = Duration::from_secs(20);
    loop {
        let now = started.elapsed().as_millis() as u64;
        for (index, peer) in peers.iter_mut().enumerate() {
            peer.send(address, index, now, &voice_packet);
        }
        host.step().unwrap();
        let now = started.elapsed().as_millis() as u64;
        for peer in &mut peers {
            peer.receive(address, now);
        }
        if started.elapsed() >= Duration::from_secs(5)
            && peers.iter().all(|p| {
                p.echoes.len() == ECHOES as usize
                    && p.shared
                    && p.voice_senders.len() == (if p.lobby.local < 108 { 7 } else { 8 })
                    && p.observed.len() == PLAYERS - 1
                    && p.lobby.entities.entities().len() == 2
                    && p.lobby
                        .entities
                        .entities()
                        .values()
                        .all(|e| e.position[0] > 1.)
            })
        {
            break;
        }
        assert!(
            started.elapsed() < deadline,
            "load timeout: admitted={}, echo counts={:?}, visible counts={:?}",
            host.player_count(),
            peers.iter().map(|p| p.echoes.len()).collect::<Vec<_>>(),
            peers.iter().map(|p| p.observed.len()).collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(host.player_count(), PLAYERS);
    let ages: Vec<_> = peers.iter().flat_map(|p| p.ages.iter().copied()).collect();
    let echoes: Vec<_> = peers
        .iter()
        .flat_map(|p| p.echo_latency.iter().copied())
        .collect();
    let now = started.elapsed().as_millis() as u64;
    let observer_ages: Vec<_> = peers
        .iter()
        .flat_map(|p| {
            p.lobby
                .actors
                .iter()
                .filter(|(id, _)| **id != p.lobby.local)
                .filter_map(|(_, a)| {
                    a.body
                        .latest()
                        .map(|r| now.saturating_sub(r.state.captured))
                })
        })
        .collect();
    let observer_p95 = percentile(observer_ages, 95);
    assert!(
        observer_p95 < 2500,
        "Observer p95 staleness {observer_p95} ms exceeded capacity target"
    );
    let p95 = percentile(ages.clone(), 95);
    assert!(
        p95 < 1500,
        "Movement p95 age {p95} ms exceeded bounded load target"
    );
    assert!(peers.iter().all(|p| p.lobby.stats.send_errors == 0));
    let tx: u64 = peers.iter().map(|p| p.tx).sum();
    let rx: u64 = peers.iter().map(|p| p.rx).sum();
    let dropped: u64 = peers.iter().map(|p| p.discarded).sum();
    let voice_frames: u64 = peers.iter().map(|p| p.voice_frames).sum();
    let voice_age = percentile(
        peers
            .iter()
            .flat_map(|p| p.voice_ages.iter().copied())
            .collect(),
        95,
    );
    let entity_samples: u64 = peers.iter().map(|p| p.entity_samples).sum();
    assert!(dropped > 0);
    println!(
        "CAPACITY players={PLAYERS} voice_talkers=8 voice_frames={voice_frames} voice_age_p95_ms={voice_age} opus_frame_bytes={} shared_moving_objects=2 entity_samples={entity_samples} duration_ms={} client_tx_bytes={tx} client_rx_bytes={rx} deliberate_packet_drops={dropped} resource_echoes={} payload_bytes={} movement_samples={} movement_age_p95_ms={p95} observer_age_p95_ms={observer_p95} movement_age_max_ms={} resource_echo_p95_ms={} rss_before_kib={before:?} rss_after_kib={:?} process_peak_rss_kib={:?}",
        voice_packet.len(),
        started.elapsed().as_millis(),
        echoes.len(),
        echoes.len() * 1024,
        ages.len(),
        ages.iter().max().unwrap(),
        percentile(echoes, 95),
        rss_kib("VmRSS:"),
        rss_kib("VmHWM:")
    );
    println!(
        "VOICE_ROUTER {}",
        serde_json::to_string(&host.voice_metrics()).unwrap()
    );
    host.shutdown();
}

/// Seeded one-way 25–75 ms delivery, one percent independent loss. Due-time
/// ordering deliberately lets younger datagrams overtake older ones. The
/// harness itself refuses to hide a server backlog in an unbounded queue.
struct DelayedLink {
    random: u64,
    sequence: u64,
    delivered_sequence: u64,
    queued: BTreeMap<(u64, u64), Vec<u8>>,
    bytes: usize,
    peak_packets: usize,
    peak_bytes: usize,
    drops: u64,
    reordered: u64,
}
impl DelayedLink {
    fn new(seed: u64) -> Self {
        Self {
            random: seed,
            sequence: 0,
            delivered_sequence: 0,
            queued: BTreeMap::new(),
            bytes: 0,
            peak_packets: 0,
            peak_bytes: 0,
            drops: 0,
            reordered: 0,
        }
    }
    fn enqueue(&mut self, now: u64, packet: Vec<u8>) {
        self.random = self
            .random
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.sequence += 1;
        if (self.random >> 32) % 100 == 0 {
            self.drops += 1;
            return;
        }
        let due = now + 25 + ((self.random >> 40) % 51);
        self.bytes += packet.len();
        self.queued.insert((due, self.sequence), packet);
        self.peak_packets = self.peak_packets.max(self.queued.len());
        self.peak_bytes = self.peak_bytes.max(self.bytes);
        assert!(
            self.queued.len() <= 4096 && self.bytes <= 2 * 1024 * 1024,
            "impairment queue exceeded fixed harness bound"
        );
    }
    fn drain(&mut self, now: u64) -> Vec<Vec<u8>> {
        let mut ready = Vec::new();
        while self
            .queued
            .first_key_value()
            .is_some_and(|(key, _)| key.0 <= now)
        {
            let ((_, sequence), packet) = self.queued.pop_first().unwrap();
            self.reordered += u64::from(sequence < self.delivered_sequence);
            self.delivered_sequence = self.delivered_sequence.max(sequence);
            self.bytes -= packet.len();
            ready.push(packet);
        }
        ready
    }
}

/// Exact 1 ms buckets (overflow kept separately) keep measurements constant in
/// memory even when a soak receives millions of frames.
struct Histogram {
    buckets: Vec<u64>,
    count: u64,
    max: u64,
}
impl Histogram {
    fn new() -> Self {
        Self {
            buckets: vec![0; 10_001],
            count: 0,
            max: 0,
        }
    }
    fn record(&mut self, value: u64) {
        self.count += 1;
        self.max = self.max.max(value);
        let index = (value as usize).min(self.buckets.len() - 1);
        self.buckets[index] += 1;
    }
    fn percentile(&self, percent: u64) -> u64 {
        assert!(self.count > 0);
        let target = (self.count - 1) * percent / 100;
        let mut cumulative = 0;
        for (index, count) in self.buckets.iter().enumerate() {
            cumulative += count;
            if cumulative > target {
                return index as u64;
            }
        }
        unreachable!()
    }
}

#[test]
fn delayed_jittered_reordered_combined_traffic_keeps_every_peer_progressing() {
    impaired_workload(8, 8);
}

#[test]
#[ignore = "bounded 64-client soak; SKATE_SOAK_SECONDS=30..300 (default 120), run alone"]
fn sustained_sixty_four_client_combined_traffic_soak() {
    let seconds = std::env::var("SKATE_SOAK_SECONDS")
        .map(|s| {
            s.parse::<u64>()
                .expect("SKATE_SOAK_SECONDS must be an integer")
        })
        .unwrap_or(120);
    assert!(
        (30..=300).contains(&seconds),
        "SKATE_SOAK_SECONDS must be 30..300"
    );
    impaired_workload(64, seconds);
}

fn impaired_workload(players: usize, seconds: u64) {
    let fixture = Fixture::new(players);
    let mut host = Host::bind(Options {
        accounts: None,
        operations: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        session: 7,
        max_players: players,
        map: Map::TestWorld, locations:None,
        resources: Some(fixture.0.join("server.json")),
    })
    .unwrap();
    let address = host.local_addr().unwrap();
    let mut peers: Vec<_> = (0..players)
        .map(|index| {
            let mut peer = Peer::new(100 + index as u64);
            peer.enqueued = true; // This scenario drives ongoing requests itself.
            peer.outbound = Some(DelayedLink::new(1000 + index as u64));
            peer.inbound = Some(DelayedLink::new(2000 + index as u64));
            peer
        })
        .collect();
    let tone =
        std::array::from_fn(|i| (i as f32 * std::f32::consts::TAU * 440. / 48000.).sin() * 0.3);
    let voice_packet = skate_voice::Encoder::new().unwrap().encode(&tone).unwrap();
    let mut movement = Histogram::new();
    let mut voice = Histogram::new();
    let mut echoes = Histogram::new();
    let mut sent = vec![0u64; players];
    let mut last_sent = vec![0u64; players];
    let mut next_sample = 5000;
    let mut rss = Vec::new();
    let mut warm_rss = Vec::new();
    let history_capacity = players * (players - 1) * 64;
    let mut history_warm = false;
    let started = Instant::now();
    let active_ms = seconds * 1000;
    loop {
        let now = started.elapsed().as_millis() as u64;
        for (index, peer) in peers.iter_mut().enumerate() {
            if now < active_ms
                && peer.resources.ready()
                && peer.pending_echoes.len() < 2
                && now.saturating_sub(last_sent[index]) >= 1000
            {
                sent[index] += 1;
                peer.enqueue_echo(sent[index], now);
                last_sent[index] = now;
            }
            assert!(peer.pending_echoes.len() <= 2);
            if let Some(oldest) = peer.pending_echoes.values().min() {
                assert!(
                    now.saturating_sub(*oldest) < 15_000,
                    "peer {index} resource request starved for 15 seconds"
                );
            }
            peer.send(address, index, now, &voice_packet);
        }
        host.step().unwrap();
        let now = started.elapsed().as_millis() as u64;
        for peer in &mut peers {
            peer.receive(address, now);
            for age in peer.ages.drain(..) {
                movement.record(age);
            }
            for age in peer.voice_ages.drain(..) {
                voice.record(age);
            }
            for age in peer.echo_latency.drain(..) {
                echoes.record(age);
            }
        }
        if now >= next_sample {
            let (history_entries, pose_history_entries) = peers
                .iter()
                .flat_map(|p| {
                    p.lobby.actors.iter().filter(move |(id, _)| {
                        **id != p.lobby.local && (100..100 + players as u64).contains(*id)
                    })
                })
                .fold((0usize, 0usize), |(body, pose), (_, actor)| {
                    assert!(
                        actor.body.history.len() <= 64 && actor.pose.history.len() <= 64,
                        "replication history exceeded its fixed bound"
                    );
                    (
                        body + actor.body.history.len(),
                        pose + actor.pose.history.len(),
                    )
                });
            history_warm |= history_entries * 100 >= history_capacity * 95;
            if let Some(value) = rss_kib("VmRSS:") {
                rss.push(value);
                if history_warm {
                    warm_rss.push(value);
                }
            }
            let links = peers
                .iter()
                .flat_map(|p| [p.outbound.as_ref().unwrap(), p.inbound.as_ref().unwrap()]);
            let queued_bytes: usize = links.map(|l| l.bytes).sum();
            println!(
                "SOAK_SAMPLE elapsed_ms={now} rss_kib={:?} delayed_bytes={queued_bytes} body_history_entries={history_entries} body_history_capacity={history_capacity} pose_history_entries={pose_history_entries} history_warm={history_warm} outstanding_echoes={} completed_echoes={} voice_frames={}",
                rss.last(),
                peers.iter().map(|p| p.pending_echoes.len()).sum::<usize>(),
                echoes.count,
                voice.count
            );
            let metrics: serde_json::Value =
                serde_json::from_str(&host.resource_command("metrics load").unwrap()).unwrap();
            assert_eq!(
                metrics["running"], true,
                "load resource retired under traffic"
            );
            assert_eq!(
                metrics["errors"], 0,
                "resource callback failed under traffic"
            );
            println!(
                "SOAK_RUNTIME elapsed_ms={now} lua_heap_bytes={} queued_outputs={} queued_output_accounted_bytes={}",
                metrics["lua_heap_bytes"],
                metrics["queued_outputs"],
                metrics["queued_output_accounted_bytes"]
            );
            next_sample += 5000;
        }
        if now >= active_ms && peers.iter().all(|p| p.pending_echoes.is_empty()) {
            break;
        }
        assert!(now < active_ms + 20_000, "combined workload did not drain");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(host.player_count(), players);
    for (index, peer) in peers.iter().enumerate() {
        assert!(
            sent[index] >= 2,
            "peer {index} never sustained resource traffic"
        );
        assert_eq!(
            peer.echoes.len(),
            sent[index] as usize,
            "peer {index} lost a reliable resource event"
        );
        assert!(peer.shared);
        assert_eq!(
            peer.observed.len(),
            players - 1,
            "peer {index} did not observe every actor"
        );
        assert_eq!(
            peer.voice_senders.len(),
            if index < 8 { 7 } else { 8 },
            "peer {index} missed an eligible talker"
        );
        assert_eq!(peer.lobby.entities.entities().len(), 2);
        assert!(
            peer.lobby
                .entities
                .entities()
                .values()
                .all(|e| e.position[0] > 1.)
        );
        assert_eq!(peer.lobby.stats.send_errors, 0);
    }
    assert!(
        *sent.iter().max().unwrap() <= sent.iter().min().unwrap() * 2,
        "per-peer resource completion ratio exceeded 2:1: {sent:?}"
    );
    let now = started.elapsed().as_millis() as u64;
    let observer_ages: Vec<_> = peers
        .iter()
        .flat_map(|p| {
            p.lobby
                .actors
                .iter()
                .filter(move |(id, _)| **id != p.lobby.local)
                .filter_map(|(_, a)| {
                    a.body
                        .latest()
                        .map(|r| now.saturating_sub(r.state.captured))
                })
        })
        .collect();
    let observer_max = *observer_ages.iter().max().unwrap();
    let observer_p95 = percentile(observer_ages, 95);
    assert!(
        observer_p95 < 2500 && observer_max < 10_000,
        "observer snapshots became stale: p95={observer_p95} max={observer_max}"
    );
    let movement_p95 = movement.percentile(95);
    let voice_p95 = voice.percentile(95);
    let echo_p95 = echoes.percentile(95);
    assert!(movement_p95 < 2500, "movement p95 {movement_p95} ms");
    assert!(voice_p95 < 1000, "voice p95 {voice_p95} ms");
    assert!(echo_p95 < 10_000, "resource p95 {echo_p95} ms");
    // Priority BODY histories fill slowly at 64 peers; start the fixed RSS gate
    // at 95% occupancy. POSE uses remaining bandwidth and is measured separately,
    // so optional streams need not fill to establish BODY workload steady state.
    // A full default soak must include three post-warmup observations on Linux.
    if seconds >= 120 && !rss.is_empty() {
        assert!(
            warm_rss.len() >= 3,
            "soak did not reach a measurable steady-state window"
        );
    }
    if let Some(first) = warm_rss.first() {
        assert!(
            warm_rss
                .iter()
                .all(|value| value.saturating_sub(*first) < 64 * 1024),
            "RSS grew by at least 64 MiB after warmup: {warm_rss:?}"
        );
    }
    let links: Vec<_> = peers
        .iter()
        .flat_map(|p| [p.outbound.as_ref().unwrap(), p.inbound.as_ref().unwrap()])
        .collect();
    let drops: u64 = links.iter().map(|l| l.drops).sum();
    let reordered: u64 = links.iter().map(|l| l.reordered).sum();
    assert!(
        drops > 0 && reordered > 0,
        "impairment profile was not exercised"
    );
    println!(
        "SOAK players={players} active_seconds={seconds} duration_ms={} one_way_delay_min_ms=25 one_way_delay_max_ms=75 seeded_loss_percent=1 delayed_drops={drops} sparse_drops={} reordered_datagrams={reordered} observer_p95_ms={observer_p95} observer_max_ms={observer_max} peak_link_packets={} peak_link_bytes={} resource_echoes={} min_peer_echoes={} max_peer_echoes={} resource_p95_ms={echo_p95} resource_max_ms={} movement_samples={} movement_p95_ms={movement_p95} voice_frames={} voice_p95_ms={voice_p95} client_tx_bytes={} client_rx_bytes={} rss_samples_kib={rss:?} warm_rss_samples_kib={warm_rss:?}",
        started.elapsed().as_millis(),
        peers.iter().map(|p| p.discarded).sum::<u64>(),
        links.iter().map(|l| l.peak_packets).max().unwrap(),
        links.iter().map(|l| l.peak_bytes).max().unwrap(),
        echoes.count,
        sent.iter().min().unwrap(),
        sent.iter().max().unwrap(),
        echoes.max,
        movement.count,
        voice.count,
        peers.iter().map(|p| p.tx).sum::<u64>(),
        peers.iter().map(|p| p.rx).sum::<u64>()
    );
    println!(
        "VOICE_ROUTER {}",
        serde_json::to_string(&host.voice_metrics()).unwrap()
    );
    host.shutdown();
}
