//! Headless dedicated-server options, resource runtime and UDP host.
mod accounts;
pub mod operations;
pub mod supervision;
mod base_world;
pub mod locations;
mod voice;
pub mod entities;
pub mod competition;
pub mod native_authority;
pub mod world;
pub mod resources;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    io::{self, Read},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub const USAGE: &str = "Usage: skate-server (--map MAP.skate | --test-world) [--bind IPv4:PORT] [--max-players 1..64] [--session NUMBER] [--resources server.json] [--locations DIRECTORY] [--accounts accounts.json] [--operations operations.json]\n\
Validate a resource configuration without starting scripts: skate-server --validate-resources server.json\n\
Default bind: 0.0.0.0:31030; players: 16; session: 48031030.\n\
Clients use: skate3rust --connect SERVER:31030 --map MAP.skate\n\
Both sides need the same map. The server hashes the map; it does not load retail assets.";

#[derive(Debug, PartialEq, Eq)]
pub enum Map {
    TestWorld,
    File(PathBuf),
}

#[derive(Debug)]
pub struct Options {
    pub bind: SocketAddr,
    pub session: u64,
    pub max_players: usize,
    pub map: Map,
    pub resources: Option<PathBuf>,
    pub locations: Option<PathBuf>,
    pub accounts: Option<PathBuf>,
    pub operations: Option<PathBuf>,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<Self>, String> {
        let mut args = args.into_iter().peekable();
        let mut bind = "0.0.0.0:31030".parse::<SocketAddr>().unwrap();
        let mut session = 48031030;
        let mut max_players = 16;
        let mut map = None;
        let mut resources = None;
        let mut locations = None;
        let mut accounts = None;
        let mut operations = None;
        let mut seen = BTreeSet::new();
        while let Some(arg) = args.next() {
            let key = arg.to_str().ok_or("Option name is not valid UTF-8")?;
            if matches!(key, "--help" | "-h") {
                if !seen.is_empty() || args.peek().is_some() {
                    return Err("Use --help by itself".into());
                }
                return Ok(None);
            }
            if !seen.insert(key.to_owned()) {
                return Err(format!("Repeated option: {key}"));
            }
            match key {
                "--test-world" => {
                    if map.replace(Map::TestWorld).is_some() {
                        return Err("Choose either --map or --test-world".into());
                    }
                }
                "--operations" => { operations = Some(PathBuf::from(args.next().ok_or("--operations requires a configuration file")?)); }
                "--accounts" => {
                    accounts = Some(PathBuf::from(
                        args.next()
                            .ok_or("--accounts requires a configuration file")?,
                    ));
                }
                "--locations" => {locations=Some(PathBuf::from(args.next().ok_or("--locations requires a directory")?));}
                "--resources" => {
                    resources = Some(PathBuf::from(
                        args.next()
                            .ok_or("--resources requires a configuration file")?,
                    ));
                }
                "--map" => {
                    let path = PathBuf::from(args.next().ok_or("--map requires a file")?);
                    if map.replace(Map::File(path)).is_some() {
                        return Err("Choose either --map or --test-world".into());
                    }
                }
                "--bind" | "--session" | "--max-players" => {
                    let value = args
                        .next()
                        .ok_or_else(|| format!("{key} requires a value"))?;
                    let value = value.to_str().ok_or("Option value is not valid UTF-8")?;
                    match key {
                        "--bind" => {
                            bind = value.parse().map_err(|_| "--bind requires IPv4:PORT")?
                        }
                        "--session" => {
                            session = value.parse().map_err(|_| "Invalid session number")?
                        }
                        _ => max_players = value.parse().map_err(|_| "Invalid player limit")?,
                    }
                }
                _ => return Err(format!("Unknown option: {key}")),
            }
        }
        match bind {
            SocketAddr::V4(address)
                if !address.ip().is_multicast() && !address.ip().is_broadcast() => {}
            _ => return Err("--bind requires a unicast or wildcard IPv4 address".into()),
        }
        if session == 0 {
            return Err("Session must be nonzero".into());
        }
        if !(1..=skate_net::dedicated::MAX_PLAYERS).contains(&max_players) {
            return Err("Player limit must be between 1 and 64".into());
        }
        Ok(Some(Self {
            bind,
            session,
            max_players,
            map: map.ok_or("Select --map MAP.skate or --test-world")?,
            resources,
            locations,
            accounts,
            operations,
        }))
    }
}

/// Matches the game's streaming FNV-1a fingerprint, without decoding private data.
pub fn map_fingerprint(map: &Map) -> Result<u64, String> {
    let Map::File(path) = map else {
        return Ok(skate_net::hash(b"skate-test-world-v1"));
    };
    let mut file =
        std::fs::File::open(path).map_err(|error| format!("Map {}: {error}", path.display()))?;
    let mut hash = 0xcbf29ce484222325u64;
    let mut buffer = [0; 65_536];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Read map {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        for byte in &buffer[..count] {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
    }
    Ok(hash)
}

/// Bounded nonblocking I/O around the deterministic authority state machine.
pub struct Host {
    socket: UdpSocket,
    server: skate_net::dedicated::Server,
    started: Instant,
    map: u64,
    resources: Option<resources::Platform>,
    accounts: Option<accounts::Accounts>,
    operations: operations::Operations,
    voice: skate_voice::Router,
    voice_egress:usize,
    last_resource_tick: Instant,
}

impl Host {
    pub fn bind(options: Options) -> Result<Self, String> {
        let voice_egress=voice::egress_budget()?;
        let operations = operations::Operations::new(options.operations.as_deref().map(operations::Config::load).transpose()?.unwrap_or_default(), options.max_players)?;
        // Validate before opening a listening socket, including direct library use.
        let SocketAddr::V4(bind) = options.bind else {
            return Err("Only IPv4 is supported".into());
        };
        if bind.ip().is_multicast() || bind.ip().is_broadcast() {
            return Err("Bind requires a unicast or wildcard address".into());
        }
        let native=skate_resources::locations::PreparedCatalog::discover(match &options.map {Map::TestWorld=>None,Map::File(path)=>Some(path.as_path())},options.locations.as_deref())?;
        // Dedicated add-on identity is carried by mandatory resource admission.
        let map = map_fingerprint(&options.map)?;
        // An incarnation ID prevents effects from a previous process being reused.
        // This is not an authentication credential or an encryption key.
        let identity = format!(
            "{}:{:?}:{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default(),
            options.bind
        );
        let server_id = skate_net::hash(identity.as_bytes()).max(1);
        let mut server = skate_net::dedicated::Server::new(skate_net::dedicated::Config {
            session: options.session,
            server_id,
            map,
            max_players: options.max_players,
        })?;
        if options.resources.is_some() || native.is_some() {
            server.set_base_world_spawn(base_world::spawn(&options.map)?)?;
        }
        let socket = UdpSocket::bind(options.bind)
            .map_err(|error| format!("Bind {}: {error}", options.bind))?;
        skate_net::socket::configure(&socket).map_err(|error| format!("Configure UDP: {error}"))?;
        let resources=if options.resources.is_some()||native.is_some() {Some(resources::Platform::load_with_locations(options.resources.as_deref(),socket.local_addr().map_err(|e|e.to_string())?,&mut server,native.as_ref())?)}else{None};
        let accounts = options
            .accounts
            .as_ref()
            .map(|path| accounts::Accounts::load(path))
            .transpose()?;
        Ok(Self {
            voice_egress,
            voice: skate_voice::Router::new(options.session, server_id)?,
            accounts,
            operations,
            resources,
            last_resource_tick: Instant::now(),
            socket,
            server,
            started: Instant::now(),
            map,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn player_count(&self) -> usize {
        self.server.player_count()
    }
    pub fn voice_metrics(&self)->skate_voice::Metrics {self.voice.metrics}
    pub fn entity_contact_metrics(&self)->entities::ContactMetrics {
        self.resources.as_ref().map(|platform|platform.entity_contact_metrics()).unwrap_or_default()
    }

    pub fn map_fingerprint(&self) -> u64 {
        self.map
    }

    pub fn account_address(&self) -> Option<SocketAddr> {
        self.accounts.as_ref().map(accounts::Accounts::address)
    }

    pub fn resource_address(&self) -> Option<SocketAddr> {
        self.resources.as_ref().map(resources::Platform::address)
    }
    pub fn resource_command(&mut self, command: &str) -> Result<String, String> {
        self.resources
            .as_mut()
            .ok_or("Resources are not configured")?
            .command(command, &mut self.server)
    }
    pub fn exit_requested(&self) -> Option<bool> { self.operations.exit_requested }
    pub fn shutdown(&mut self) {
        self.accounts.take();
        if let Some(resources) = &mut self.resources {
            resources.shutdown();
            resources.sync_voice(&mut self.voice);
        }
        let _ = self.voice.sync(&[], 0);
    }
    pub fn step(&mut self) -> io::Result<()> {
        let now = self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        if let Some(accounts) = &mut self.accounts {
            accounts.step(&mut self.server, &mut self.resources, &mut self.operations, now);
        }
        if let Some(resources) = &mut self.resources {
            resources.sync_voice(&mut self.voice);
        }
        voice::sync(&mut self.voice, &self.server, now);
        // Large enough to distinguish an oversized datagram on Windows as well
        // as POSIX; only protocol-sized data ever reaches the decoder.
        let mut buffer = [0u8; 65_536];
        let mut admission_packets = Vec::new();
        for _ in 0..512 {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, SocketAddr::V4(from))) if len <= skate_accounts::MAX_DATAGRAM => {
                    let peer = (u64::from(u32::from(*from.ip())) << 16) | u64::from(from.port());
                    if let Some(nonce) = skate_net::discovery::query_nonce(&buffer[..len]) {
                        if self.operations.allow_discovery(peer, now) {
                            let preview = self.resources.as_ref().map(|p|p.discovery_preview()).unwrap_or(serde_json::Value::Null);
                            let info = self.operations.info(&self.server, self.accounts.is_some(), preview);
                            if let Some(reply) = skate_net::discovery::response(nonce, &info) { let _ = self.socket.send_to(&reply, from); }
                        }
                        continue;
                    }
                    if let Some(accounts) = &mut self.accounts {
                        if let Some(packet) = accounts.decode(peer, &buffer[..len]) {
                            if skate_voice::wire::is_voice(&packet) {
                                self.voice.receive(peer, &packet, now);
                            } else {
                                if self.operations.receive(&self.server,peer,&packet,accounts.verified(peer),now,&mut admission_packets) {self.server.receive(peer, &packet, now);}
                            }
                        }
                    } else if len <= skate_net::packed::MTU {
                        if skate_voice::wire::is_voice(&buffer[..len]) {
                            self.voice.receive(peer, &buffer[..len], now);
                        } else {
                            if self.operations.receive(&self.server,peer,&buffer[..len],None,now,&mut admission_packets) {self.server.receive(peer, &buffer[..len], now);}
                        }
                    }
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionRefused
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        admission_packets.extend(self.operations.step(&mut self.server, now));
        let mut packets = self.server.service(now);
        packets.extend(admission_packets);
        if let Some(accounts) = &mut self.accounts {
            accounts.sync_identity(&mut self.resources, &self.server);
        }
        if let Some(resources) = &mut self.resources {
            let dt = self.last_resource_tick.elapsed().as_secs_f64().min(0.25);
            self.last_resource_tick = Instant::now();
            resources.step(dt, &mut self.server);
            resources.sync_voice(&mut self.voice);
        }
        // Teleports, kicks and resource callbacks may have changed policy in
        // this tick. Retire old audio before draining after gameplay packets.
        voice::sync(&mut self.voice, &self.server, now);
        packets.extend(self.voice.drain(self.voice_egress).into_iter().map(|(peer, data)| {
            skate_net::lobby::Outgoing { peer, data }
        }));
        for packet in packets {
            let peer = SocketAddrV4::new(
                Ipv4Addr::from((packet.peer >> 16) as u32),
                packet.peer as u16,
            );
            let encoded;
            let data = if let Some(accounts) = &mut self.accounts {
                let Some(bytes) = accounts.encode(packet.peer, &packet.data) else {
                    continue;
                };
                encoded = bytes;
                &encoded
            } else {
                &packet.data
            };
            let sent = self.socket.send_to(data, peer);
            self.server.record_send(data.len(), sent.is_ok());
            if let Err(error) = sent {
                // Snapshot keyframes and reliable effect queues recover transient
                // datagram loss; a single unreachable client must not stop a host.
                if !matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::NetworkUnreachable
                        | io::ErrorKind::HostUnreachable
                ) {
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}
