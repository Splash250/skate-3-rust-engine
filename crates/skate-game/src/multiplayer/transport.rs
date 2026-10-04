use skate_net::directory::{self, Command as LobbyCommand, Event, Request, Response};
use std::{
    io,
    net::{SocketAddr, UdpSocket},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// Platform adapters move opaque bounded datagrams; session/actors live above this.
pub(super) trait Transport: Send + Sync {
    fn send(&mut self, peer: u64, data: &[u8]) -> io::Result<()>;
    fn receive(&mut self) -> io::Result<Vec<(u64, Vec<u8>)>>;
    fn status(&self) -> String;
    fn actor_id(&self) -> Option<u64> { None }
    fn command(&mut self, _command: LobbyCommand) -> Result<(), String> {
        Err("Leave local multiplayer before browsing Steam lobbies".into())
    }
    fn events(&mut self) -> Vec<Response> {
        vec![]
    }
    fn loopback(&self) -> bool {
        false
    }
    fn metrics(&self) -> String {
        String::new()
    }
    fn congested(&self) -> bool {
        false
    }
}
pub(super) fn endpoint(addr: SocketAddr) -> io::Result<u64> {
    match addr {
        SocketAddr::V4(a) => Ok(((u32::from(*a.ip()) as u64) << 16) | a.port() as u64),
        _ => Err(io::Error::other("Direct testing currently requires IPv4")),
    }
}
pub(super) struct Direct {
    socket: UdpSocket,
}
impl Direct {
    pub fn new(bind: SocketAddr) -> io::Result<Self> {
        endpoint(bind)?;
        let socket = UdpSocket::bind(bind)?;
        skate_net::socket::configure(&socket)?;
        Ok(Self { socket })
    }
}
impl Transport for Direct {
    fn loopback(&self) -> bool {
        self.socket.local_addr().is_ok_and(|a| a.ip().is_loopback())
    }
    fn send(&mut self, peer: u64, data: &[u8]) -> io::Result<()> {
        let address =
            std::net::SocketAddrV4::new(std::net::Ipv4Addr::from((peer >> 16) as u32), peer as u16);
        self.socket.send_to(data, address).map(|_| ())
    }
    fn receive(&mut self) -> io::Result<Vec<(u64, Vec<u8>)>> {
        let mut output = Vec::new();
        let mut buffer = [0; 1500];
        for _ in 0..4096 {
            match self.socket.recv_from(&mut buffer) {
                Ok((n, from)) if n <= skate_net::packed::MTU => {
                    output.push((endpoint(from)?, buffer[..n].to_vec()))
                }
                Ok(_) => (),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(output)
    }
    fn status(&self) -> String {
        "Direct connection (Steam not required)".into()
    }
}
/// Authenticated dedicated UDP. A fresh TLS login creates each codec exactly
/// once; dropping it retires its nonce state, so reconnects must log in again.
pub(super) struct SecureDirect {
    socket: UdpSocket,
    server: SocketAddr,
    codec: skate_accounts::ClientSession,
}
impl SecureDirect {
    pub fn login(path: &Path, server: SocketAddr) -> Result<Self,String> {
        use std::io::Read;
        let mut bytes=Vec::new();
        std::fs::File::open(path).map_err(|e|format!("Account configuration: {e}"))?.take(16385).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        if bytes.len()>16384 {return Err("Account configuration exceeds 16 KiB".into());}
        let mut config:skate_accounts::ClientCredentials=serde_json::from_slice(&bytes).map_err(|_|"Invalid account configuration")?;
        let parent=path.parent().unwrap_or(Path::new("."));
        if config.ca_certificate.is_relative(){config.ca_certificate=parent.join(config.ca_certificate);}
        if config.password_file.is_relative(){config.password_file=parent.join(config.password_file);}
        let (_,codec)=skate_accounts::login_client(&config,Duration::from_secs(10)).map_err(|e|e.to_string())?;
        Self::new(server,codec).map_err(|e|e.to_string())
    }
    fn new(server:SocketAddr,codec:skate_accounts::ClientSession)->io::Result<Self>{
        endpoint(server)?;
        let socket=UdpSocket::bind("0.0.0.0:0")?;skate_net::socket::configure(&socket)?;
        Ok(Self{socket,server,codec})
    }
}
impl Transport for SecureDirect {
    fn actor_id(&self)->Option<u64>{Some(self.codec.actor)}
    fn loopback(&self)->bool{self.server.ip().is_loopback()}
    fn status(&self)->String{"Authenticated direct connection (TLS login, encrypted UDP)".into()}
    fn send(&mut self,peer:u64,data:&[u8])->io::Result<()>{
        if endpoint(self.server)?!=peer {return Err(io::Error::other("Authenticated datagrams require their admitted server"));}
        let packet=self.codec.encode(data).map_err(|e|io::Error::other(e.to_string()))?;
        self.socket.send_to(&packet,self.server).map(|_|())
    }
    fn receive(&mut self)->io::Result<Vec<(u64,Vec<u8>)>>{
        let mut result=Vec::new();let mut buffer=[0u8;1500];
        for _ in 0..4096 {
            match self.socket.recv_from(&mut buffer){
                Ok((len,from)) if from==self.server=>if let Ok(plain)=self.codec.decode(&buffer[..len]){result.push((endpoint(from)?,plain));},
                Ok(_)=>{},
                Err(e) if matches!(e.kind(),io::ErrorKind::WouldBlock|io::ErrorKind::ConnectionReset|io::ErrorKind::ConnectionRefused)=>break,
                Err(e)=>return Err(e),
            }
        }
        Ok(result)
    }
}

pub(super) struct Steam {
    socket: UdpSocket,
    child: Child,
    peer: Option<SocketAddr>,
    cookie: String,
    status: String,
    last_ping: Instant,
    started: Instant,
    metrics: String,
    congested: bool,
    events: Vec<Response>,
    pending: Option<(Request, Instant)>,
    last_command: Instant,
    request_id: u64,
}

fn relay_command(game_dir: &Path) -> Result<Command, String> {
    if !cfg!(any(
        windows,
        all(
            target_os = "linux",
            target_arch = "x86_64",
            target_env = "gnu"
        )
    )) {
        return Err("Steam relay requires Windows or x86_64 GNU/Linux; solo and direct multiplayer remain available".into());
    }
    let relay_dir = game_dir.join("steam-relay");
    let helper = relay_dir.join(skate_platform::exe::name("skate-steam-relay"));
    let library = if cfg!(windows) {
        "steam_api64.dll"
    } else {
        "libsteam_api.so"
    };
    if !helper.is_file() || !relay_dir.join(library).is_file() {
        return Err(
            "Steam relay files missing; solo and direct multiplayer remain available".into(),
        );
    }
    let mut command = Command::new(helper);
    command.current_dir(&relay_dir);
    // Only the helper links Steam. Keep its SDK search path out of the game
    // process while preserving any loader paths supplied by PLAY.sh or Steam.
    #[cfg(target_os = "linux")]
    {
        let mut paths = vec![relay_dir];
        if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        command.env(
            "LD_LIBRARY_PATH",
            std::env::join_paths(paths)
                .map_err(|e| format!("Invalid Steam relay library path: {e}"))?,
        );
    }
    Ok(command)
}

impl Steam {
    pub fn new(peer: u64, session: u64) -> Result<Self, String> {
        let socket = UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        skate_net::socket::configure(&socket).map_err(|e| e.to_string())?;
        let dir = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .unwrap()
            .to_path_buf();
        let cookie = format!("{:016x}", super::unique());
        let mut command = relay_command(&dir)?;
        command
            .args([
                socket.local_addr().map_err(|e| e.to_string())?.to_string(),
                peer.to_string(),
                session.to_string(),
                cookie.clone(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let child = command
            .spawn()
            .map_err(|e| format!("Could not start Steam relay: {e}"))?;
        Ok(Self {
            socket,
            child,
            peer: None,
            cookie,
            status: "Checking Steam; open Steam and sign in if needed".into(),
            last_ping: Instant::now(),
            started: Instant::now(),
            metrics: String::new(),
            congested: false,
            events: vec![],
            pending: None,
            last_command: Instant::now() - Duration::from_secs(1),
            request_id: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "skate relay {} {}",
                std::process::id(),
                super::super::unique()
            ));
            std::fs::create_dir_all(path.join("steam-relay")).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_steam_files_do_not_prevent_direct_udp() {
        let fixture = Fixture::new();
        assert!(
            relay_command(&fixture.0)
                .unwrap_err()
                .contains("direct multiplayer remain available")
        );
        let mut first = Direct::new("127.0.0.1:0".parse().unwrap()).unwrap();
        let mut second = Direct::new("127.0.0.1:0".parse().unwrap()).unwrap();
        let destination = endpoint(second.socket.local_addr().unwrap()).unwrap();
        first.send(destination, b"direct without Steam").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let received = second.receive().unwrap();
            if !received.is_empty() {
                assert_eq!(
                    received,
                    vec![(
                        endpoint(first.socket.local_addr().unwrap()).unwrap(),
                        b"direct without Steam".to_vec()
                    )]
                );
                break;
            }
            assert!(Instant::now() < deadline, "Direct UDP did not arrive");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    #[test]
    fn native_linux_relay_launches_from_paths_with_spaces_and_finds_its_library() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let relay_dir = fixture.0.join("steam-relay");
        let helper = relay_dir.join("skate-steam-relay");
        std::fs::write(
            &helper,
            b"#!/bin/sh\npwd\nprintf '%s\\n' \"$LD_LIBRARY_PATH\"\nprintf '<%s>\\n' \"$@\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(relay_command(&fixture.0).is_err());
        std::fs::write(relay_dir.join("libsteam_api.so"), b"fixture").unwrap();
        let result = relay_command(&fixture.0)
            .unwrap()
            .args(["127.0.0.1:1234", "42", "480", "cookie"])
            .output()
            .unwrap();
        assert!(result.status.success());
        let output = String::from_utf8(result.stdout).unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(Path::new(lines[0]), relay_dir);
        let mut expected = vec![relay_dir];
        if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
            expected.extend(std::env::split_paths(&existing));
        }
        assert_eq!(
            std::env::split_paths(lines[1]).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            &lines[2..],
            &["<127.0.0.1:1234>", "<42>", "<480>", "<cookie>"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_relay_still_uses_packaged_executable_and_dll() {
        let fixture = Fixture::new();
        let relay_dir = fixture.0.join("steam-relay");
        let helper = relay_dir.join("skate-steam-relay.exe");
        std::fs::write(&helper, b"fixture").unwrap();
        assert!(relay_command(&fixture.0).is_err());
        std::fs::write(relay_dir.join("steam_api64.dll"), b"fixture").unwrap();
        assert_eq!(
            relay_command(&fixture.0).unwrap().get_program(),
            helper.as_os_str()
        );
    }
}
impl Drop for Steam {
    fn drop(&mut self) {
        if let Some(peer) = self.peer {
            let _ = self
                .socket
                .send_to(format!("QUIT {}", self.cookie).as_bytes(), peer);
            for _ in 0..10 {
                if self.child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Transport for Steam {
    fn command(&mut self, command: LobbyCommand) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("Steam request in progress; please wait".into());
        }
        self.request_id += 1;
        self.pending = Some((
            Request {
                id: self.request_id,
                command,
            },
            Instant::now(),
        ));
        self.last_command = Instant::now() - Duration::from_secs(1);
        Ok(())
    }
    fn events(&mut self) -> Vec<Response> {
        std::mem::take(&mut self.events)
    }
    fn send(&mut self, target: u64, data: &[u8]) -> io::Result<()> {
        let peer = self
            .peer
            .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "Relay starting"))?;
        let mut packet = u64::from_str_radix(&self.cookie, 16)
            .unwrap()
            .to_le_bytes()
            .to_vec();
        packet.extend(target.to_le_bytes());
        packet.extend(data);
        self.socket.send_to(&packet, peer).map(|_| ())
    }
    fn receive(&mut self) -> io::Result<Vec<(u64, Vec<u8>)>> {
        let mut packets = vec![];
        let mut buffer = [0; 1500];
        let prefix = format!("SK8RELAY {} ", self.cookie);
        for _ in 0..4096 {
            match self.socket.recv_from(&mut buffer) {
                Ok((n, from)) => {
                    if !from.ip().is_loopback() || self.peer.is_some_and(|p| p != from) {
                        continue;
                    }
                    if let Ok(text) = std::str::from_utf8(&buffer[..n]) {
                        if let Some(status) = text.strip_prefix(&prefix) {
                            self.peer = Some(from);
                            if let Some(json) = status.strip_prefix("EVENT ") {
                                if let Some(response) = directory::response(json) {
                                    if response.request == 0
                                        || self
                                            .pending
                                            .as_ref()
                                            .is_some_and(|(r, _)| r.id == response.request)
                                    {
                                        if response.request != 0 {
                                            self.pending = None;
                                        }
                                        self.events.push(response);
                                    }
                                }
                            } else if let Some(stats) = status.strip_prefix("STATS ") {
                                self.congested = stats.split_whitespace().next() == Some("1");
                                self.metrics = stats.split_once(' ').map_or("", |(_, s)| s).into();
                            } else {
                                self.status = status.into();
                            }
                            continue;
                        }
                    }
                    if self.peer == Some(from)
                        && n >= 16
                        && n <= skate_net::packed::MTU + 16
                        && u64::from_le_bytes(buffer[..8].try_into().unwrap())
                            == u64::from_str_radix(&self.cookie, 16).unwrap()
                    {
                        packets.push((
                            u64::from_le_bytes(buffer[8..16].try_into().unwrap()),
                            buffer[16..n].to_vec(),
                        ));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        if let Some((request, since)) = &self.pending {
            if since.elapsed() > Duration::from_secs(18) {
                self.events.push(Response {
                    request: request.id,
                    event: Event::Error(
                        "Steam request timed out. Open Steam and sign in, then retry.".into(),
                    ),
                });
                self.pending = None;
            } else if self.last_command.elapsed() > Duration::from_millis(500) {
                if let Some(peer) = self.peer {
                    let _ = self.socket.send_to(
                        format!("CMD {} {}", self.cookie, directory::encode(request)).as_bytes(),
                        peer,
                    );
                    self.last_command = Instant::now();
                }
            }
        }
        if self.last_ping.elapsed() > Duration::from_secs(1) {
            if let Some(p) = self.peer {
                let _ = self
                    .socket
                    .send_to(format!("PING {}", self.cookie).as_bytes(), p);
            }
            self.last_ping = Instant::now();
        }
        if self.child.try_wait()?.is_some() && !self.status.starts_with("ERROR") {
            self.status = "Steam relay stopped. Open Steam, sign in, and retry.".into();
        }
        if self.peer.is_none() && self.started.elapsed() > Duration::from_secs(10) {
            self.status = "Steam did not respond. Open Steam, sign in, and retry.".into();
        }
        Ok(packets)
    }
    fn status(&self) -> String {
        self.status.clone()
    }
    fn metrics(&self) -> String {
        self.metrics.clone()
    }
    fn congested(&self) -> bool {
        self.congested
    }
}

#[cfg(test)]
mod authenticated_tests {
    use super::*;
    use skate_accounts::{AccountStore, AdminBridge, AdminConfig, AdminServer, ServerTransport};
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
    #[test]
    fn game_transport_logs_in_and_rejects_forged_endpoints_and_replays() {
        let fixture=Temp(std::env::temp_dir().join(format!("skate-game-auth-{}",std::process::id())));
        std::fs::create_dir(&fixture.0).unwrap();
        let auth=fixture.0.join("auth");
        skate_accounts::initialize(&auth,"player","local-game-test-password").unwrap();
        let store=AccountStore::open(auth.join("accounts.sqlite3")).unwrap();
        let server=AdminServer::bind(store.clone(),AdminConfig{bind:"127.0.0.1:0".parse().unwrap(),certificate:auth.join("certificate.pem"),key:auth.join("private-key.pem")},AdminBridge::new()).unwrap();
        let password=fixture.0.join("password");std::fs::write(&password,"local-game-test-password").unwrap();
        #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(&password,std::fs::Permissions::from_mode(0o600)).unwrap();}
        let config=fixture.0.join("client.json");
        std::fs::write(&config,serde_json::to_vec(&serde_json::json!({"endpoint":format!("https://localhost:{}",server.local_addr().port()),"ca_certificate":"auth/certificate.pem","username":"player","password_file":"password"})).unwrap()).unwrap();
        let socket=UdpSocket::bind("127.0.0.1:0").unwrap();socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let address=socket.local_addr().unwrap();let peer=endpoint(address).unwrap();
        let mut client=SecureDirect::login(&config,address).unwrap();
        let mut authority=ServerTransport::new(store).unwrap();
        assert_eq!(client.actor_id(),Some(client.codec.actor));
        assert!(client.send(peer.wrapping_add(1),b"outside").is_err());
        client.send(peer,b"movement").unwrap();
        let mut bytes=[0;2048];let (len,from)=socket.recv_from(&mut bytes).unwrap();
        let (verified,plain)=authority.decode(&bytes[..len]).unwrap();assert_eq!(plain,b"movement");assert_eq!(verified.actor(),client.actor_id().unwrap());
        let response=authority.encode(&verified,b"accepted").unwrap();
        let stranger=UdpSocket::bind("127.0.0.1:0").unwrap();stranger.send_to(&response,from).unwrap();
        socket.send_to(b"plaintext-spoof",from).unwrap();
        assert!(client.receive().unwrap().is_empty());
        socket.send_to(&response,from).unwrap();socket.send_to(&response,from).unwrap();
        let until=Instant::now()+Duration::from_secs(1);let mut received=Vec::new();
        while received.is_empty(){received.extend(client.receive().unwrap());assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
        assert_eq!(received,vec![(peer,b"accepted".to_vec())]);assert!(client.receive().unwrap().is_empty());
        let oversized=fixture.0.join("oversized.json");std::fs::write(&oversized,vec![b' ';16_385]).unwrap();
        assert!(SecureDirect::login(&oversized,address).is_err());
    }
}
