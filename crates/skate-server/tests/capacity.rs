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
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("skate-capacity-{}", std::process::id()));
        // A single test owns this process-specific fixture.
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
            .write_all(b"\nlocal radio=false; return {on_update=function() if radio then return end; local members={}; for _,p in ipairs(resource.players()) do if p.position then members[#members+1]=p.id end end; if #members==64 then resource.voice.submit({kind='channel',name='radio',members=members});radio=true end end}").unwrap();
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
            voice: Default::default(), last_voice:0, voice_senders:Default::default(), voice_ages:vec![], voice_frames:0,
            entity_ticks: BTreeMap::new(),
            entity_samples: 0,
        }
    }
    fn send(&mut self, address: SocketAddr, index: usize, now: u64, voice_packet:&[u8]) {
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
        if index<8 && self.voice.revision()>0 && self.lobby.movement_epoch()>0 && now.saturating_sub(self.last_voice)>=20 {
            let packet=skate_voice::wire::Frame {session:7,actor:self.lobby.local,epoch:self.lobby.movement_epoch(),revision:self.voice.revision(),sequence:now,channel:"load/radio".into(),data:voice_packet.to_vec()}.encode().unwrap();
            self.tx+=self.socket.send_to(&packet,address).unwrap() as u64;self.tx_packets+=1;self.last_voice=now;
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
            self.tx += self.socket.send_to(&packet.data, address).unwrap() as u64;
        }
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
                    if skate_voice::wire::is_voice(&bytes[..len]) {
                        if let Some(skate_voice::Incoming::Playback{packet,..})=self.voice.accept(1,&bytes[..len]) {
                            self.voice_senders.insert(packet.actor);self.voice_frames+=1;self.voice_ages.push(now.saturating_sub(packet.sequence));
                        }
                    } else {
                        self.lobby.receive(1, &bytes[..len], now);
                        if self.lobby.connected() {self.voice.synchronize(Some(skate_voice::Context {session:7,host_peer:1,actor:self.lobby.local,epoch:self.lobby.movement_epoch(),instance:self.lobby.actors[&self.lobby.local].instance}));}
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("{error}"),
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
                self.resources
                    .emit(
                        "load",
                        1,
                        "echo",
                        json!({"seq":seq,"sent":now,"padding":"x".repeat(1024)}),
                    )
                    .unwrap();
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
    let fixture = Fixture::new();
    let before = rss_kib("VmRSS:");
    let mut host = Host::bind(Options {
        accounts: None,
        bind: "127.0.0.1:0".parse().unwrap(),
        session: 7,
        max_players: PLAYERS,
        map: Map::TestWorld,
        resources: Some(fixture.0.join("server.json")),
    })
    .unwrap();
    let address = host.local_addr().unwrap();
    let mut peers: Vec<_> = (0..PLAYERS).map(|i| Peer::new(100 + i as u64)).collect();
    let tone=std::array::from_fn(|i|(i as f32*std::f32::consts::TAU*440./48000.).sin()*0.3);
    let voice_packet=skate_voice::Encoder::new().unwrap().encode(&tone).unwrap();
    let started = Instant::now();
    let deadline = Duration::from_secs(20);
    loop {
        let now = started.elapsed().as_millis() as u64;
        for (index, peer) in peers.iter_mut().enumerate() {
            peer.send(address, index, now,&voice_packet);
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
                    && p.voice_senders.len()==(if p.lobby.local<108 {7}else{8})
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
    let voice_frames:u64=peers.iter().map(|p|p.voice_frames).sum();
    let voice_age=percentile(peers.iter().flat_map(|p|p.voice_ages.iter().copied()).collect(),95);
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
    println!("VOICE_ROUTER {}",serde_json::to_string(&host.voice_metrics()).unwrap());
    host.shutdown();
}
