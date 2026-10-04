//! Bounded HTTP/1.1 transfer on a separate TCP listener. Only negotiated endpoint
//! addresses are used; redirects, proxies, compression and chunked bodies are rejected.
use crate::{
    Cache, Error, MAX_SET_JSON_BYTES, PublishedSet, ResourceSet, Result, digest_bytes,
    validate_digest,
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_HEADER: usize = 8192;
const MAX_WORKERS: usize = 4;
const IO_TIMEOUT: Duration = Duration::from_millis(250);
const HEADER_TIMEOUT: Duration = Duration::from_secs(3);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);
/// A replaceable immutable allowlist; no request path can reach the server filesystem.
pub struct HttpServer {
    limits: crate::Limits,
    address: SocketAddr,
    published: Arc<RwLock<Arc<PublishedSet>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl HttpServer {
    pub fn bind(addr: SocketAddr, published: PublishedSet) -> Result<Self> {
        Self::bind_with_limits(addr, published, crate::Limits::default())
    }
    pub fn bind_with_limits(
        addr: SocketAddr,
        published: PublishedSet,
        limits: crate::Limits,
    ) -> Result<Self> {
        published.validate_with_limits(limits)?;
        let listener = TcpListener::bind(addr)?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let published = Arc::new(RwLock::new(Arc::new(published)));
        let stop = Arc::new(AtomicBool::new(false));
        let current = published.clone();
        let cancelled = stop.clone();
        let worker = thread::Builder::new()
            .name("resource-http".into())
            .spawn(move || {
                let mut workers: Vec<JoinHandle<()>> = Vec::new();
                while !cancelled.load(Ordering::Acquire) {
                    let mut i = 0;
                    while i < workers.len() {
                        if workers[i].is_finished() {
                            let done = workers.swap_remove(i);
                            let _ = done.join();
                        } else {
                            i += 1;
                        }
                    }
                    match listener.accept() {
                        Ok((socket, _)) => {
                            if workers.len() >= MAX_WORKERS {
                                drop(socket);
                                continue;
                            }
                            let set = current.read().unwrap_or_else(|p| p.into_inner()).clone();
                            let stop = cancelled.clone();
                            if let Ok(worker) = thread::Builder::new()
                                .name("resource-http-client".into())
                                .spawn(move || {
                                    let _ = serve(socket, &set, &stop);
                                })
                            {
                                workers.push(worker);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(_) => break,
                    }
                }
                for worker in workers {
                    let _ = worker.join();
                }
            })?;
        Ok(Self {
            limits,
            address,
            published,
            stop,
            worker: Some(worker),
        })
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }
    pub fn replace(&self, published: PublishedSet) -> Result<()> {
        published.validate_with_limits(self.limits)?;
        *self
            .published
            .write()
            .map_err(|_| Error("HTTP content lock poisoned".into()))? = Arc::new(published);
        Ok(())
    }
}
impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn check_cancel(cancel: &AtomicBool, deadline: Instant) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(Error("resource download cancelled".into()));
    }
    if Instant::now() > deadline {
        return Err(Error("resource HTTP transfer timed out".into()));
    }
    Ok(())
}
fn timed_read(
    stream: &mut TcpStream,
    buf: &mut [u8],
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<usize> {
    loop {
        check_cancel(cancel, deadline)?;
        match stream.read(buf) {
            Ok(n) => return Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }
}
fn timed_write(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<()> {
    while !bytes.is_empty() {
        check_cancel(cancel, deadline)?;
        match stream.write(bytes) {
            Ok(0) => return Err(Error("HTTP socket closed while writing".into())),
            Ok(n) => bytes = &bytes[n..],
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn header(
    stream: &mut TcpStream,
    cancel: &AtomicBool,
    operation_deadline: Instant,
) -> Result<String> {
    let deadline = operation_deadline.min(Instant::now() + HEADER_TIMEOUT);
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= MAX_HEADER {
            return Err(Error("HTTP header exceeds limit".into()));
        }
        if timed_read(stream, &mut byte, cancel, deadline)? == 0 {
            return Err(Error("truncated HTTP header".into()));
        }
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).map_err(|_| Error("non-UTF8 HTTP header".into()))
}
fn configure(stream: &TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    Ok(())
}
fn serve(mut stream: TcpStream, published: &PublishedSet, cancel: &AtomicBool) -> Result<()> {
    configure(&stream)?;
    let request = header(&mut stream, cancel, Instant::now() + HEADER_TIMEOUT)?;
    let first = request.lines().next().unwrap_or("");
    let tokens: Vec<_> = first.split(' ').collect();
    let (status, body) = if tokens.len() == 3 && tokens[0] == "GET" && tokens[2] == "HTTP/1.1" {
        if tokens[1] == "/set" {
            (200, serde_json::to_vec(&published.set)?)
        } else if let Some(digest) = tokens[1].strip_prefix("/blobs/") {
            if validate_digest(digest).is_ok() {
                match published.blobs.get(digest) {
                    Some(bytes) => (200, bytes.clone()),
                    None => (404, Vec::new()),
                }
            } else {
                (404, Vec::new())
            }
        } else {
            (404, Vec::new())
        }
    } else {
        (400, Vec::new())
    };
    let response = format!(
        "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/octet-stream\r\n\r\n",
        if status == 200 { "OK" } else { "Rejected" },
        body.len()
    );
    let deadline = Instant::now() + TRANSFER_TIMEOUT;
    timed_write(&mut stream, response.as_bytes(), cancel, deadline)?;
    timed_write(&mut stream, &body, cancel, deadline)
}
fn get(
    endpoint: SocketAddr,
    path: &str,
    max: u64,
    cancel: &AtomicBool,
    operation_deadline: Instant,
) -> Result<Vec<u8>> {
    check_cancel(cancel, operation_deadline)?;
    let connect_timeout = operation_deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_secs(3));
    let mut stream = TcpStream::connect_timeout(&endpoint, connect_timeout)?;
    configure(&stream)?;
    let deadline = operation_deadline.min(Instant::now() + TRANSFER_TIMEOUT);
    timed_write(
        &mut stream,
        format!("GET {path} HTTP/1.1\r\nHost: {endpoint}\r\nConnection: close\r\n\r\n").as_bytes(),
        cancel,
        deadline,
    )?;
    let response = header(&mut stream, cancel, operation_deadline)?;
    let mut lines = response.split("\r\n");
    let status = lines.next().unwrap_or("");
    if !status.starts_with("HTTP/1.1 200 ") {
        return Err(Error(format!(
            "resource endpoint rejected request: {status}"
        )));
    }
    let mut length = None;
    for line in lines.filter(|l| !l.is_empty()) {
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| Error("malformed HTTP header".into()))?;
        if key.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err(Error("duplicate HTTP length".into()));
            }
            length = Some(
                value
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| Error("invalid HTTP length".into()))?,
            );
        }
        if key.eq_ignore_ascii_case("transfer-encoding")
            || key.eq_ignore_ascii_case("content-encoding")
            || key.eq_ignore_ascii_case("location")
        {
            return Err(Error("HTTP encodings and redirects are unsupported".into()));
        }
    }
    let length = length.ok_or_else(|| Error("missing HTTP content length".into()))?;
    if length > max {
        return Err(Error("HTTP content exceeds declared byte limit".into()));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    let mut buffer = [0u8; 16 * 1024];
    while (bytes.len() as u64) < length {
        let remaining = (length - bytes.len() as u64).min(buffer.len() as u64) as usize;
        let n = timed_read(&mut stream, &mut buffer[..remaining], cancel, deadline)?;
        if n == 0 {
            return Err(Error("truncated HTTP content".into()));
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    check_cancel(cancel, deadline)?;
    Ok(bytes)
}
#[derive(Debug)]
pub struct DownloadReport {
    pub set: ResourceSet,
    pub roots: BTreeMap<String, PathBuf>,
    pub downloaded_bytes: u64,
    pub reused_bytes: u64,
}
/// Fetch only from an exact negotiated socket address, verify every full digest,
/// then materialize the complete set. Activation is a separate caller transaction.
pub fn download_set(
    endpoint: SocketAddr,
    expected_revision: &str,
    cache: &Cache,
    source: &str,
    cancel: &AtomicBool,
) -> Result<DownloadReport> {
    download_set_with_timeout(
        endpoint,
        expected_revision,
        cache,
        source,
        cancel,
        Duration::from_secs(120),
    )
}
/// Like `download_set`, with an explicit whole-operation deadline (at most 120 seconds).
pub fn download_set_with_timeout(
    endpoint: SocketAddr,
    expected_revision: &str,
    cache: &Cache,
    source: &str,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<DownloadReport> {
    if timeout.is_zero() || timeout > Duration::from_secs(120) {
        return Err(Error(
            "download timeout must be between zero and 120 seconds".into(),
        ));
    }
    let deadline = Instant::now() + timeout;
    validate_digest(expected_revision)?;
    let bytes = get(
        endpoint,
        "/set",
        MAX_SET_JSON_BYTES as u64,
        cancel,
        deadline,
    )?;
    let set: ResourceSet = serde_json::from_slice(&bytes)?;
    set.validate(cache.limits)?;
    if set.revision != expected_revision {
        return Err(Error(
            "server resource set differs from negotiated revision".into(),
        ));
    }
    let mut downloaded_bytes = 0u64;
    let mut reused_bytes = 0u64;
    let mut downloaded = BTreeMap::new();
    let result = (|| {
        cache.prepare(&set)?;
        for resource in &set.resources {
            downloaded.insert(resource.manifest.id.clone(), 0);
            for file in resource.files.values() {
                check_cancel(cancel, deadline)?;
                if cache.read_blob(file)?.is_some() {
                    reused_bytes += file.size;
                    continue;
                }
                let bytes = get(
                    endpoint,
                    &format!("/blobs/{}", file.digest),
                    file.size,
                    cancel,
                    deadline,
                )?;
                if bytes.len() as u64 != file.size || digest_bytes(&bytes) != file.digest {
                    return Err(Error(format!(
                        "{}: downloaded content failed digest/size verification",
                        resource.manifest.id
                    )));
                }
                cache.store_blob(file, &bytes)?;
                downloaded_bytes += file.size;
                *downloaded.get_mut(&resource.manifest.id).unwrap() += file.size;
            }
        }
        check_cancel(cancel, deadline)?;
        let roots = cache.materialize(&set, cancel)?;
        check_cancel(cancel, deadline)?;
        cache.record_download(&set, source, &downloaded)?;
        Ok(roots)
    })();
    match result {
        Ok(roots) => Ok(DownloadReport {
            set,
            roots,
            downloaded_bytes,
            reused_bytes,
        }),
        Err(error) => {
            let _ =
                cache.record_verification_failure(&set, source, &downloaded, &error.to_string());
            Err(error)
        }
    }
}
