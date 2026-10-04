use crate::*;
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_BODY: usize = 16 * 1024;
const MAX_RESPONSE: usize = 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminConfig {
    pub bind: SocketAddr,
    pub certificate: PathBuf,
    pub key: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientCredentials {
    pub endpoint: String,
    pub ca_certificate: PathBuf,
    pub username: String,
    pub password_file: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostAction {
    ResourceStart {
        resource: String,
    },
    ResourceStop {
        resource: String,
    },
    ResourceRestart {
        resource: String,
    },
    Kick {
        #[serde(with = "crate::decimal")]
        actor: u64,
    },
}
impl HostAction {
    pub fn permission(&self) -> &'static str {
        match self {
            Self::Kick { .. } => "players.kick",
            _ => "resources.manage",
        }
    }
    fn target(&self) -> String {
        match self {
            Self::ResourceStart { resource }
            | Self::ResourceStop { resource }
            | Self::ResourceRestart { resource } => resource.clone(),
            Self::Kick { actor } => actor.to_string(),
        }
    }
    fn validate(&self) -> Result<()> {
        match self {
            Self::ResourceStart { resource }
            | Self::ResourceStop { resource }
            | Self::ResourceRestart { resource } => crate::store::valid_name(resource),
            Self::Kick { actor } if *actor == 0 => Err(error("invalid", "actor must be nonzero")),
            _ => Ok(()),
        }
    }
}
pub struct HostCommand {
    pub ticket: u64,
    pub session: VerifiedSession,
    pub action: HostAction,
}
struct Ticket {
    session: VerifiedSession,
    action: HostAction,
    result: Option<Result<String>>,
    audited: bool,
}
struct BridgeState {
    commands: VecDeque<HostCommand>,
    tickets: BTreeMap<u64, Ticket>,
    status: serde_json::Value,
}
#[derive(Clone)]
pub struct AdminBridge {
    state: Arc<Mutex<BridgeState>>,
    next: Arc<AtomicU64>,
}
impl Default for AdminBridge {
    fn default() -> Self {
        Self::new()
    }
}
impl AdminBridge {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(BridgeState {
                commands: VecDeque::new(),
                tickets: BTreeMap::new(),
                status: serde_json::json!({"attached":false}),
            })),
            next: Arc::new(AtomicU64::new(1)),
        }
    }
    /// Gameplay-side calls never wait for a database or administrative worker.
    pub fn try_command(&self) -> Option<HostCommand> {
        self.state.try_lock().ok()?.commands.pop_front()
    }
    /// A busy result is safe to retry; do not repeat the actual host action.
    pub fn complete(&self, ticket: u64, mut result: Result<String>) -> Result<()> {
        if result
            .as_ref()
            .map_or_else(|e| e.message.len() + e.code.len(), |s| s.len())
            > 4096
        {
            // Size failure is permanent. Store a bounded terminal failure so the
            // host can advance its completion FIFO; only Busy needs retrying.
            result = Err(error("limit", "host result exceeds 4096 bytes"));
        }
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| error("busy", "administration bridge busy"))?;
        let entry = state
            .tickets
            .get_mut(&ticket)
            .ok_or_else(|| error("missing", "unknown action ticket"))?;
        if entry.result.is_some() {
            return Err(error("invalid", "action ticket already completed"));
        }
        entry.result = Some(result);
        Ok(())
    }
    pub fn set_status(&self, status: serde_json::Value) -> Result<()> {
        if serde_json::to_vec(&status)
            .map_err(|_| error("invalid", "invalid status"))?
            .len()
            > 128 * 1024
        {
            return Err(error("limit", "host status exceeds 128 KiB"));
        }
        self.state
            .try_lock()
            .map_err(|_| error("busy", "administration bridge busy"))?
            .status = status;
        Ok(())
    }
    fn enqueue(
        &self,
        store: &AccountStore,
        session: VerifiedSession,
        action: HostAction,
    ) -> Result<u64> {
        action.validate()?;
        session.require(action.permission())?;
        let mut state = self.state.lock().unwrap();
        if state.commands.len() >= 64 {
            return Err(error("busy", "host action queue full"));
        }
        if state.tickets.len() >= 256 {
            let oldest = state
                .tickets
                .iter()
                .find_map(|(id, t)| t.audited.then_some(*id))
                .ok_or_else(|| error("busy", "host action result capacity reached"))?;
            state.tickets.remove(&oldest);
        }
        let ticket = self.next.fetch_add(1, Ordering::Relaxed);
        store.audit_action(
            &session,
            "host.queued",
            &action.target(),
            &format!(
                "ticket {ticket}: {}",
                serde_json::to_string(&action).unwrap()
            ),
        )?;
        state.tickets.insert(
            ticket,
            Ticket {
                session: session.clone(),
                action: action.clone(),
                result: None,
                audited: false,
            },
        );
        state.commands.push_back(HostCommand {
            ticket,
            session,
            action,
        });
        Ok(ticket)
    }
    fn result(&self, session: &VerifiedSession, ticket: u64) -> Result<serde_json::Value> {
        session.require("status.read")?;
        let state = self.state.lock().unwrap();
        let entry = state
            .tickets
            .get(&ticket)
            .ok_or_else(|| error("missing", "unknown action ticket"))?;
        if entry.session.account_id() != session.account_id() && !session.permits("audit.read") {
            return Err(error("denied", "action belongs to another account"));
        }
        Ok(match &entry.result {
            None => serde_json::json!({"ticket":ticket,"state":"queued"}),
            Some(Ok(value)) => {
                serde_json::json!({"ticket":ticket,"state":"completed","ok":true,"value":value})
            }
            Some(Err(e)) => {
                serde_json::json!({"ticket":ticket,"state":"completed","ok":false,"error":e})
            }
        })
    }
    fn flush_audit(&self, store: &AccountStore) {
        // Snapshot under the bridge lock; SQLite never holds up the gameplay caller.
        let pending = {
            let state = self.state.lock().unwrap();
            state
                .tickets
                .iter()
                .filter(|(_, t)| t.result.is_some() && !t.audited)
                .take(16)
                .map(|(id, t)| {
                    (
                        *id,
                        t.session.clone(),
                        t.action.target(),
                        serde_json::to_string(&t.result).unwrap(),
                    )
                })
                .collect::<Vec<_>>()
        };
        for (id, session, target, detail) in pending {
            let mut detail = detail;
            if detail.len() > 3000 {
                let mut end = 3000;
                while !detail.is_char_boundary(end) {
                    end -= 1;
                }
                detail.truncate(end);
                detail.push_str(" [truncated]");
            }
            if store
                .audit_action(
                    &session,
                    "host.completed",
                    &target,
                    &format!("ticket {id}: {detail}"),
                )
                .is_ok()
            {
                if let Some(t) = self.state.lock().unwrap().tickets.get_mut(&id) {
                    t.audited = true;
                }
            }
        }
    }
}

pub struct AdminServer {
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    workers: Vec<JoinHandle<()>>,
}
impl AdminServer {
    pub fn bind(store: AccountStore, config: AdminConfig, bridge: AdminBridge) -> Result<Self> {
        let certificate = bounded_file(&config.certificate, 64 * 1024, false)?;
        let key = bounded_file(&config.key, 64 * 1024, true)?;
        let certs = CertificateDer::pem_slice_iter(&certificate)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| error("invalid", "invalid TLS certificate PEM"))?;
        let key = PrivateKeyDer::from_pem_slice(&key)
            .map_err(|_| error("invalid", "invalid TLS private key PEM"))?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|_| error("tls", "TLS protocol configuration failed"))?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|_| error("tls", "certificate and key do not match"))?;
        let tls = Arc::new(tls);
        let listener = TcpListener::bind(config.bind)
            .map_err(|_| error("io", "cannot bind TLS administration listener"))?;
        let address = listener
            .local_addr()
            .map_err(|_| error("io", "cannot inspect listener"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| error("io", "cannot configure listener"))?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::sync_channel::<(TcpStream, SocketAddr)>(16);
        let rx = Arc::new(Mutex::new(rx));
        let rate = Arc::new(Mutex::new(BTreeMap::<IpAddr, (Instant, u32)>::new()));
        let mut workers = Vec::new();
        for number in 0..2 {
            let (rx, store, tls, bridge, rate, stop) = (
                rx.clone(),
                store.clone(),
                tls.clone(),
                bridge.clone(),
                rate.clone(),
                shutdown.clone(),
            );
            workers.push(
                thread::Builder::new()
                    .name(format!("account-tls-{number}"))
                    .spawn(move || {
                        while !stop.load(Ordering::Acquire) {
                            let item = rx.lock().unwrap().recv_timeout(Duration::from_millis(50));
                            match item {
                                Ok((stream, peer)) => {
                                    let _ = serve(stream, peer, &tls, &store, &bridge, &rate);
                                }
                                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                                Err(mpsc::RecvTimeoutError::Timeout) => {}
                            }
                        }
                    })
                    .map_err(|_| error("unavailable", "cannot start TLS worker"))?,
            );
        }
        let stop = shutdown.clone();
        let accept = thread::Builder::new()
            .name("account-tls-accept".into())
            .spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok(pair) => {
                            let _ = tx.try_send(pair);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => break,
                    }
                    bridge.flush_audit(&store);
                }
                bridge.flush_audit(&store);
            })
            .map_err(|_| error("unavailable", "cannot start TLS listener"))?;
        Ok(Self {
            address,
            shutdown,
            accept: Some(accept),
            workers,
        })
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }
}
impl Drop for AdminServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        for t in self.workers.drain(..) {
            let _ = t.join();
        }
    }
}
struct Request {
    method: String,
    path: String,
    token: Option<String>,
    body: Vec<u8>,
}
struct DeadlineStream {
    stream: TcpStream,
    deadline: Instant,
}
impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "TLS deadline"))?;
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(bytes)
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "TLS deadline"))?;
        self.stream.set_write_timeout(Some(remaining))?;
        self.stream.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}
fn serve(
    stream: TcpStream,
    peer: SocketAddr,
    tls: &Arc<ServerConfig>,
    store: &AccountStore,
    bridge: &AdminBridge,
    rate: &Mutex<BTreeMap<IpAddr, (Instant, u32)>>,
) -> Result<()> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|_| error("io", "cannot set TLS read deadline"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|_| error("io", "cannot set TLS write deadline"))?;
    let mut stream = StreamOwned::new(
        ServerConnection::new(tls.clone())
            .map_err(|_| error("tls", "TLS connection setup failed"))?,
        DeadlineStream {
            stream,
            deadline: Instant::now() + IO_TIMEOUT,
        },
    );
    let request = read_request(&mut stream)?;
    let denied_actor = request
        .token
        .as_deref()
        .and_then(|token| store.authenticate(token).ok());
    let route_name = request.path.clone();
    let (status, kind, body) = match route(request, peer, store, bridge, rate) {
        Ok(response) => response,
        Err(e) => {
            if e.code == "denied" {
                if let Some(session) = &denied_actor {
                    let _ = store.audit_action(session, "admin.denied", &route_name, &e.message);
                }
            }
            let status = match e.code.as_str() {
                "unauthenticated" => 401,
                "denied" => 403,
                "missing" => 404,
                "busy" => 429,
                "limit" => 413,
                _ => 400,
            };
            (
                status,
                "application/json",
                serde_json::to_vec(&serde_json::json!({"ok":false,"error":e})).unwrap(),
            )
        }
    };
    if body.len() > MAX_RESPONSE {
        return write_response(
            &mut stream,
            413,
            "application/json",
            br#"{"ok":false,"error":{"code":"limit","message":"response too large"}}"#,
        );
    }
    write_response(&mut stream, status, kind, &body)
}
fn read_request(stream: &mut impl Read) -> Result<Request> {
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    let header_end = loop {
        if Instant::now() >= deadline {
            return Err(error("timeout", "HTTP request deadline exceeded"));
        }
        let n = stream
            .read(&mut chunk)
            .map_err(|_| error("io", "cannot read TLS request"))?;
        if n == 0 {
            return Err(error("invalid", "incomplete HTTP request"));
        }
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            if end + 4 > 8192 {
                return Err(error("limit", "HTTP headers too large"));
            }
            break end + 4;
        }
        if bytes.len() > 8192 {
            return Err(error("limit", "HTTP headers too large"));
        }
    };
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut parsed = httparse::Request::new(&mut headers);
    if !parsed
        .parse(&bytes[..header_end])
        .map_err(|_| error("invalid", "malformed HTTP request"))?
        .is_complete()
    {
        return Err(error("invalid", "incomplete HTTP headers"));
    }
    let method = parsed.method.unwrap_or("").to_string();
    let path = parsed.path.unwrap_or("").to_string();
    if path.len() > 256 {
        return Err(error("limit", "HTTP path exceeds 256 bytes"));
    }
    let mut length = None;
    let mut token = None;
    for header in parsed.headers.iter() {
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(error("invalid", "chunked requests are not supported"));
        }
        if header.name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(error("invalid", "duplicate content length"));
            }
            length = Some(
                std::str::from_utf8(header.value)
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|n| *n <= MAX_BODY)
                    .ok_or_else(|| error("limit", "invalid or excessive request body"))?,
            );
        }
        if header.name.eq_ignore_ascii_case("authorization") {
            if token.is_some() {
                return Err(error("invalid", "duplicate authorization"));
            }
            token = Some(
                std::str::from_utf8(header.value)
                    .ok()
                    .and_then(|s| s.strip_prefix("Bearer "))
                    .filter(|s| s.len() == 64)
                    .ok_or_else(|| error("unauthenticated", "invalid authorization header"))?
                    .to_string(),
            );
        }
        // Browser requests must originate from this HTTPS site. No CORS is granted.
        if header.name.eq_ignore_ascii_case("sec-fetch-site")
            && header.value != b"same-origin"
            && header.value != b"none"
        {
            return Err(error("denied", "cross-origin request rejected"));
        }
    }
    let length = length.unwrap_or(0);
    if bytes.len() - header_end > length {
        return Err(error("invalid", "HTTP pipelining is not supported"));
    }
    while bytes.len() < header_end + length {
        if Instant::now() >= deadline {
            return Err(error("timeout", "HTTP request deadline exceeded"));
        }
        let remaining = header_end + length - bytes.len();
        let n = stream
            .read(&mut chunk[..remaining.min(1024)])
            .map_err(|_| error("io", "cannot read TLS body"))?;
        if n == 0 {
            return Err(error("invalid", "incomplete HTTP body"));
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    Ok(Request {
        method,
        path,
        token,
        body: bytes[header_end..].to_vec(),
    })
}
fn write_response(stream: &mut impl Write, status: u16, kind: &str, body: &[u8]) -> Result<()> {
    write!(stream,"HTTP/1.1 {status} Response\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; frame-ancestors 'none'; base-uri 'none'; object-src 'none'\r\n\r\n",body.len()).and_then(|_|stream.write_all(body)).and_then(|_|stream.flush()).map_err(|_|error("io","cannot write TLS response"))
}
fn route(
    request: Request,
    peer: SocketAddr,
    store: &AccountStore,
    bridge: &AdminBridge,
    rate: &Mutex<BTreeMap<IpAddr, (Instant, u32)>>,
) -> Result<(u16, &'static str, Vec<u8>)> {
    if request.method == "GET" {
        let asset = match request.path.as_str() {
            "/" => Some(("text/html; charset=utf-8", include_str!("admin.html"))),
            "/app.js" => Some(("text/javascript; charset=utf-8", include_str!("admin.js"))),
            "/app.css" => Some(("text/css; charset=utf-8", include_str!("admin.css"))),
            _ => None,
        };
        if let Some((kind, text)) = asset {
            return Ok((200, kind, text.as_bytes().to_vec()));
        }
    }
    if request.method == "POST" && request.path == "/v1/login" {
        let mut rate = rate.lock().unwrap();
        rate.retain(|_, (start, _)| start.elapsed() < Duration::from_secs(60));
        if !rate.contains_key(&peer.ip()) && rate.len() >= 1024 {
            return Err(error("busy", "login rate registry full"));
        }
        let entry = rate.entry(peer.ip()).or_insert((Instant::now(), 0));
        if entry.1 >= 10 {
            return Err(error("busy", "login rate limit: retry after one minute"));
        }
        entry.1 += 1;
        drop(rate);
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Login {
            username: String,
            password: String,
        }
        let login: Login = serde_json::from_slice(&request.body)
            .map_err(|_| error("invalid", "invalid login body"))?;
        return json(200, &store.login(&login.username, &login.password)?);
    }
    let session = store.authenticate(
        request
            .token
            .as_deref()
            .ok_or_else(|| error("unauthenticated", "bearer session required"))?,
    )?;
    if request.method == "GET" {
        match request.path.as_str() {
            "/v1/admin/accounts" => return json(200, &store.accounts(&session)?),
            "/v1/admin/roles" => return json(200, &store.roles(&session)?),
            "/v1/admin/status" => {
                session.require("status.read")?;
                return json(200, &bridge.state.lock().unwrap().status);
            }
            _ => {}
        }
        if let Some(after) = request.path.strip_prefix("/v1/admin/accounts?after=") {
            return json(200, &store.accounts_page(&session, after)?);
        }
        if let Some(after) = request.path.strip_prefix("/v1/admin/audit?after=") {
            return json(
                200,
                &store.audit_log(
                    &session,
                    after
                        .parse()
                        .map_err(|_| error("invalid", "invalid audit cursor"))?,
                )?,
            );
        }
        if request.path == "/v1/admin/audit" {
            return json(200, &store.audit_log(&session, 0)?);
        }
        if let Some(ticket) = request.path.strip_prefix("/v1/admin/actions/") {
            return json(
                200,
                &bridge.result(
                    &session,
                    ticket
                        .parse()
                        .map_err(|_| error("invalid", "invalid ticket"))?,
                )?,
            );
        }
    }
    if request.method != "POST" {
        return Err(error("missing", "unknown API route"));
    }
    if request.path == "/v1/admin/actions" {
        let action: HostAction = serde_json::from_slice(&request.body)
            .map_err(|_| error("invalid", "invalid host action"))?;
        let ticket = bridge.enqueue(store, session, action)?;
        return json(202, &serde_json::json!({"ticket":ticket,"state":"queued"}));
    }
    let body: serde_json::Value =
        serde_json::from_slice(&request.body).map_err(|_| error("invalid", "invalid JSON body"))?;
    let text = |field: &str| {
        body.get(field)
            .and_then(|v| v.as_str())
            .ok_or_else(|| error("invalid", &format!("missing string field {field}")))
    };
    let flag = |field: &str| {
        body.get(field)
            .and_then(|v| v.as_bool())
            .ok_or_else(|| error("invalid", &format!("missing boolean field {field}")))
    };
    match request.path.as_str() {
        "/v1/admin/accounts" => {
            return json(
                201,
                &store.create_account(&session, text("username")?, text("password")?)?,
            );
        }
        "/v1/admin/roles" => store.create_role(
            &session,
            text("role")?,
            body.get("parent").and_then(|v| v.as_str()),
        )?,
        "/v1/admin/role-parent" => {
            store.role_parent(&session, text("role")?, text("parent")?, flag("grant")?)?
        }
        "/v1/admin/role-permission" => {
            store.role_permission(&session, text("role")?, text("permission")?, flag("grant")?)?
        }
        "/v1/admin/account-role" => {
            store.assign_role(&session, text("account")?, text("role")?, flag("grant")?)?
        }
        "/v1/admin/ban" => {
            store.ban(&session, text("account")?, flag("banned")?, text("reason")?)?
        }
        "/v1/admin/whitelist" => store.whitelist(&session, text("account")?, flag("allowed")?)?,
        "/v1/admin/whitelist-mode" => store.whitelist_mode(&session, flag("enabled")?)?,
        "/v1/admin/revoke" => store.revoke(&session, text("account")?)?,
        _ => return Err(error("missing", "unknown API route")),
    }
    json(200, &serde_json::json!({"ok":true}))
}
fn json<T: Serialize>(status: u16, value: &T) -> Result<(u16, &'static str, Vec<u8>)> {
    Ok((
        status,
        "application/json",
        serde_json::to_vec(value).map_err(|_| error("invalid", "response serialization failed"))?,
    ))
}

/// A verified HTTPS login before the gameplay loop. Passwords and issued keys are never logged.
pub fn login_client(
    config: &ClientCredentials,
    timeout: Duration,
) -> Result<(SessionCredentials, ClientSession)> {
    if timeout.is_zero() || timeout > Duration::from_secs(30) {
        return Err(error("invalid", "login timeout must be in 1ns..30s"));
    }
    let url = reqwest::Url::parse(&config.endpoint)
        .map_err(|_| error("invalid", "invalid login endpoint"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(error("invalid", "login endpoint must be an HTTPS origin"));
    }
    let cert = bounded_file(&config.ca_certificate, 64 * 1024, false)?;
    let password = bounded_file(&config.password_file, 1026, true)?;
    let password = std::str::from_utf8(&password)
        .map_err(|_| error("invalid", "password file must contain UTF-8"))?
        .trim_end_matches(['\r', '\n']);
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(timeout)
        .add_root_certificate(
            reqwest::Certificate::from_pem(&cert)
                .map_err(|_| error("invalid", "invalid CA certificate"))?,
        )
        .build()
        .map_err(|_| error("tls", "cannot configure HTTPS client"))?;
    let payload =
        serde_json::to_vec(&serde_json::json!({"username":config.username,"password":password}))
            .map_err(|_| error("invalid", "cannot encode credentials"))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| error("unavailable", "cannot start login worker"))?;
    let credentials: SessionCredentials = runtime.block_on(async {
        let mut response = client
            .post(url.join("/v1/login").unwrap())
            .header("content-type", "application/json")
            .body(payload)
            .send()
            .await
            .map_err(|_| {
                error(
                    "tls",
                    "HTTPS login failed (check endpoint, CA and deadline)",
                )
            })?;
        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| error("io", "cannot read HTTPS login response"))?
        {
            if body.len() + chunk.len() > 16 * 1024 {
                return Err(error("limit", "login response too large"));
            }
            body.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(error(
                "unauthenticated",
                "login rejected; check credentials, moderation and rate limits",
            ));
        }
        serde_json::from_slice(&body).map_err(|_| error("invalid", "invalid login response"))
    })?;
    let transport = ClientSession::new(credentials.clone())?;
    Ok((credentials, transport))
}

/// Creates a fresh private directory, local TLS certificate/key and initial administrator.
/// Refuses an existing directory so initialization never overwrites an installation.
pub fn initialize(directory: &Path, username: &str, password: &str) -> Result<()> {
    crate::store::valid_username(username)?;
    crate::store::valid_password(password)?;
    std::fs::create_dir(directory).map_err(|_| {
        error(
            "storage",
            "initialization requires a new directory with an existing parent",
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| error("storage", "cannot protect initialization directory"))?;
    }
    #[cfg(windows)]
    {
        crate::store::windows_acl(directory, false)?;
    }
    let certificate = rcgen::generate_simple_self_signed(vec![
        "localhost".into(),
        "127.0.0.1".into(),
        "::1".into(),
    ])
    .map_err(|_| error("tls", "cannot generate local TLS certificate"))?;
    write_private(
        &directory.join("certificate.pem"),
        certificate.cert.pem().as_bytes(),
    )?;
    write_private(
        &directory.join("private-key.pem"),
        certificate.signing_key.serialize_pem().as_bytes(),
    )?;
    AccountStore::open(directory.join("accounts.sqlite3"))?.bootstrap_admin(username, password)?;
    Ok(())
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| error("storage", "cannot create private account file"))?;
    crate::store::private_file(path)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| error("storage", "cannot persist private account file"))?;
    crate::store::private_file(path)
}
fn bounded_file(path: &Path, limit: usize, private: bool) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| error("storage", "cannot inspect account configuration file"))?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(error(
            "invalid",
            "configuration file must be a bounded regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if private && metadata.permissions().mode() & 0o077 != 0 {
            return Err(error(
                "denied",
                "private credential/key file must not permit group or other access (chmod 600)",
            ));
        }
    }
    #[cfg(windows)]
    {
        if private {
            crate::store::windows_acl(path, true)?;
        }
    }
    let _ = private;
    let mut file = std::fs::File::open(path)
        .map_err(|_| error("storage", "cannot open configuration file"))?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("storage", "cannot read configuration file"))?;
    if bytes.len() > limit {
        return Err(error("limit", "configuration file grew beyond its limit"));
    }
    Ok(bytes)
}

/// Verified TLS administrative client for local tooling and integration tests.
/// The bearer token is sent only to this HTTPS origin, with redirects disabled.
pub fn admin_request(
    config: &ClientCredentials,
    token: &str,
    path: &str,
    body: Option<serde_json::Value>,
    timeout: Duration,
) -> Result<serde_json::Value> {
    if !path.starts_with("/v1/admin/")
        || path.len() > 256
        || path.contains(['\r', '\n'])
        || timeout.is_zero()
        || timeout > Duration::from_secs(30)
    {
        return Err(error("invalid", "invalid administrative request"));
    }
    unhex::<32>(token)?;
    let url = reqwest::Url::parse(&config.endpoint)
        .map_err(|_| error("invalid", "invalid HTTPS origin"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(error(
            "invalid",
            "administrative endpoint must be an HTTPS origin",
        ));
    }
    let certificate = bounded_file(&config.ca_certificate, 64 * 1024, false)?;
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .add_root_certificate(
            reqwest::Certificate::from_pem(&certificate)
                .map_err(|_| error("invalid", "invalid CA certificate"))?,
        )
        .build()
        .map_err(|_| error("tls", "cannot configure administrative client"))?;
    let payload = body
        .map(|b| {
            serde_json::to_vec(&b).map_err(|_| error("invalid", "invalid administrative JSON"))
        })
        .transpose()?;
    if payload.as_ref().is_some_and(|p| p.len() > MAX_BODY) {
        return Err(error("limit", "administrative request exceeds 16 KiB"));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| error("unavailable", "cannot start administrative request"))?;
    runtime.block_on(async {
        let mut request = if payload.is_some() {
            client.post(url.join(path).unwrap())
        } else {
            client.get(url.join(path).unwrap())
        }
        .bearer_auth(token);
        if let Some(body) = payload {
            request = request
                .header("content-type", "application/json")
                .body(body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| error("tls", "administrative TLS request failed"))?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| error("io", "cannot read administrative response"))?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err(error("limit", "administrative response too large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| error("invalid", "invalid administrative response"))?;
        if !status.is_success() {
            return Err(value
                .get("error")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_else(|| error("denied", "administrative request rejected")));
        }
        Ok(value)
    })
}
