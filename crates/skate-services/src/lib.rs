//! Bounded server-only backend workers. Gameplay uses only try_send/try_recv.
//! The trusted host registers owners; scripts receive tickets, never owner handles.
#![forbid(unsafe_code)]

mod database;
mod http;

pub use database::{DatabaseProvider, SqliteDatabase};

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Limits {
    pub pending: usize,
    pub pending_per_resource: usize,
    pub database_workers: usize,
    pub http_workers: usize,
    pub request_bytes: usize,
    pub response_bytes: usize,
    pub rows: usize,
    pub statements: usize,
    pub database_bytes: u64,
    pub timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            pending: 128,
            pending_per_resource: 16,
            database_workers: 2,
            http_workers: 8,
            request_bytes: 256 * 1024,
            response_bytes: 1024 * 1024,
            rows: 4096,
            statements: 64,
            database_bytes: 256 * 1024 * 1024,
            timeout: Duration::from_secs(15),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Grants {
    pub database: bool,
    /// Exact scheme/host/port origins approved by the administrator. No redirects.
    pub http_origins: BTreeSet<String>,
}

impl Grants {
    /// Validate trusted server configuration before installing resource scripts.
    pub fn validate(&self) -> Result<(), ServiceError> {
        if self.http_origins.len() > 64 {
            return Err(error(ErrorCode::TooLarge, "maximum 64 HTTP origins"));
        }
        for origin in &self.http_origins {
            http::grant_origin(origin)?;
        }
        Ok(())
    }
}

struct OwnerState {
    resource: String,
    generation: u64,
    active: AtomicBool,
    grants: Grants,
}
#[derive(Clone)]
pub struct Owner(Arc<OwnerState>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Statement {
    pub sql: String,
    #[serde(default)]
    pub params: Vec<SqlValue>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    pub version: u32,
    pub statements: Vec<Statement>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpRequest {
    pub url: String,
    #[serde(default = "get_method")]
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Vec<u8>,
}
fn get_method() -> String {
    "GET".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Query { statement: Statement },
    Transaction { statements: Vec<Statement> },
    Migrate { migrations: Vec<Migration> },
    Http { request: HttpRequest },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<SqlValue>>,
    pub changed: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Database {
        results: Vec<QueryResult>,
    },
    Migrated {
        version: u32,
    },
    Http {
        status: u16,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Invalid,
    Denied,
    Stale,
    Busy,
    Cancelled,
    Timeout,
    TooLarge,
    Database,
    Http,
    Shutdown,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceError {
    pub code: ErrorCode,
    pub message: String,
}
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}
impl std::error::Error for ServiceError {}
pub(crate) fn error(code: ErrorCode, message: impl Into<String>) -> ServiceError {
    ServiceError {
        code,
        message: message.into(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Completion {
    pub resource: String,
    pub generation: u64,
    pub id: u64,
    pub result: Result<Response, ServiceError>,
}
struct Job {
    id: u64,
    owner: Owner,
    operation: Operation,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
impl Job {
    fn check(&self) -> Result<(), ServiceError> {
        check(&self.owner, &self.cancelled, self.deadline)
    }
    fn complete(&self, result: Result<Response, ServiceError>) -> Completion {
        Completion {
            resource: self.owner.0.resource.clone(),
            generation: self.owner.0.generation,
            id: self.id,
            result,
        }
    }
}
fn check(owner: &Owner, cancelled: &AtomicBool, deadline: Instant) -> Result<(), ServiceError> {
    if !owner.0.active.load(Ordering::Acquire) {
        return Err(error(ErrorCode::Stale, "resource generation retired"));
    }
    if cancelled.load(Ordering::Acquire) {
        return Err(error(ErrorCode::Cancelled, "operation cancelled"));
    }
    if Instant::now() >= deadline {
        return Err(error(ErrorCode::Timeout, "operation deadline exceeded"));
    }
    Ok(())
}

/// Cancellation/ownership guard supplied to additional trusted provider adapters.
#[derive(Clone)]
pub struct OperationContext {
    owner: Owner,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
impl OperationContext {
    pub fn resource(&self) -> &str {
        &self.owner.0.resource
    }
    pub fn generation(&self) -> u64 {
        self.owner.0.generation
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn check(&self) -> Result<(), ServiceError> {
        check(&self.owner, &self.cancelled, self.deadline)
    }
}

pub struct Services {
    limits: Limits,
    owners: BTreeMap<String, Owner>,
    pending: BTreeMap<u64, (Owner, Arc<AtomicBool>)>,
    next_id: u64,
    database: Option<mpsc::SyncSender<Job>>,
    http: Option<tokio::sync::mpsc::Sender<Job>>,
    completed: mpsc::Receiver<Completion>,
    threads: Vec<JoinHandle<()>>,
}
impl Services {
    pub fn new(root: PathBuf, limits: Limits) -> Result<Self, ServiceError> {
        Self::with_database(Arc::new(SqliteDatabase::new(root)?), limits)
    }
    pub fn with_database(
        provider: Arc<dyn DatabaseProvider>,
        limits: Limits,
    ) -> Result<Self, ServiceError> {
        if limits.pending == 0
            || limits.pending > 4096
            || limits.pending_per_resource == 0
            || limits.pending_per_resource > limits.pending
            || limits.database_workers == 0
            || limits.database_workers > 16
            || limits.http_workers == 0
            || limits.http_workers > 128
            || limits.request_bytes == 0
            || limits.request_bytes > 16 * 1024 * 1024
            || limits.response_bytes == 0
            || limits.response_bytes > 16 * 1024 * 1024
            || limits.rows == 0
            || limits.rows > 1_000_000
            || limits.statements == 0
            || limits.statements > 1024
            || limits.database_bytes < 4096 * 16
            || limits.database_bytes > 1024 * 1024 * 1024 * 64
            || limits.timeout.is_zero()
            || limits.timeout > Duration::from_secs(300)
        {
            return Err(error(ErrorCode::Invalid, "invalid backend service limits"));
        }
        let (tx, completed) = mpsc::sync_channel(limits.pending);
        let (db_tx, db_rx) = mpsc::sync_channel::<Job>(limits.pending);
        let db_rx = Arc::new(Mutex::new(db_rx));
        let mut threads = Vec::new();
        for index in 0..limits.database_workers {
            let (rx, tx, provider, limits) =
                (db_rx.clone(), tx.clone(), provider.clone(), limits.clone());
            threads.push(
                std::thread::Builder::new()
                    .name(format!("resource-sql-{index}"))
                    .spawn(move || {
                        loop {
                            let job = match rx.lock().unwrap().recv() {
                                Ok(job) => job,
                                Err(_) => break,
                            };
                            let result = job.check().and_then(|_| {
                                provider.execute(
                                    OperationContext {
                                        owner: job.owner.clone(),
                                        cancelled: job.cancelled.clone(),
                                        deadline: job.deadline,
                                    },
                                    &job.operation,
                                    &limits,
                                )
                            });
                            let _ = tx.try_send(job.complete(result));
                        }
                    })
                    .map_err(|_| error(ErrorCode::Shutdown, "cannot start database worker"))?,
            );
        }
        let (http_tx, http_rx) = tokio::sync::mpsc::channel(limits.pending);
        let http_limits = limits.clone();
        let client = http::client()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| error(ErrorCode::Shutdown, "cannot start HTTP runtime"))?;
        threads.push(
            std::thread::Builder::new()
                .name("resource-http".into())
                .spawn(move || {
                    runtime.block_on(http::run(http_rx, tx, http_limits, client));
                })
                .map_err(|_| error(ErrorCode::Shutdown, "cannot start HTTP worker"))?,
        );
        Ok(Self {
            limits,
            owners: BTreeMap::new(),
            pending: BTreeMap::new(),
            next_id: 1,
            database: Some(db_tx),
            http: Some(http_tx),
            completed,
            threads,
        })
    }

    /// Trusted server host only. Activation replaces and retires an older generation.
    pub fn activate(
        &mut self,
        resource: &str,
        generation: u64,
        mut grants: Grants,
    ) -> Result<Owner, ServiceError> {
        if resource.is_empty()
            || resource.len() > 96
            || generation == 0
            || !resource
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        {
            return Err(error(ErrorCode::Invalid, "invalid resource or generation"));
        }
        if let Some(old) = self.owners.get(resource) {
            if generation <= old.0.generation {
                return Err(error(ErrorCode::Stale, "generation must increase"));
            }
        }
        grants.validate()?;
        grants.http_origins = grants
            .http_origins
            .iter()
            .map(|s| http::grant_origin(s))
            .collect::<Result<_, _>>()?;
        let owner = Owner(Arc::new(OwnerState {
            resource: resource.into(),
            generation,
            grants,
            active: AtomicBool::new(true),
        }));
        if let Some(old) = self.owners.insert(resource.into(), owner.clone()) {
            self.revoke(&old);
        }
        Ok(owner)
    }
    pub fn revoke(&mut self, owner: &Owner) {
        owner.0.active.store(false, Ordering::Release);
    }
    pub fn submit(
        &mut self,
        owner: &Owner,
        operation: Operation,
        timeout: Duration,
    ) -> Result<u64, ServiceError> {
        if !self
            .owners
            .get(&owner.0.resource)
            .is_some_and(|o| Arc::ptr_eq(&o.0, &owner.0))
            || !owner.0.active.load(Ordering::Acquire)
        {
            return Err(error(
                ErrorCode::Stale,
                "resource generation retired or unknown",
            ));
        }
        match &operation {
            Operation::Http { request } => http::validate(&owner.0.grants, request)?,
            _ if !owner.0.grants.database => {
                return Err(error(ErrorCode::Denied, "resource.database is not granted"));
            }
            _ => {}
        }
        if timeout.is_zero() || timeout > self.limits.timeout {
            return Err(error(
                ErrorCode::Invalid,
                "timeout exceeds host deadline budget",
            ));
        }
        // Bounded serializer avoids a second unbounded allocation for oversized requests.
        let mut size = SizeLimit {
            remaining: self.limits.request_bytes,
        };
        serde_json::to_writer(&mut size, &operation)
            .map_err(|_| error(ErrorCode::TooLarge, "request exceeds byte budget"))?;
        if self.pending.len() >= self.limits.pending
            || self
                .pending
                .values()
                .filter(|(o, _)| o.0.resource == owner.0.resource)
                .count()
                >= self.limits.pending_per_resource
        {
            return Err(error(
                ErrorCode::Busy,
                "backend queue is full; retry after a completion",
            ));
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| error(ErrorCode::Shutdown, "ticket space exhausted"))?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let job = Job {
            id,
            owner: owner.clone(),
            operation,
            cancelled: cancelled.clone(),
            deadline: Instant::now() + timeout,
        };
        match job.operation {
            Operation::Http { .. } => self
                .http
                .as_ref()
                .ok_or_else(|| error(ErrorCode::Shutdown, "HTTP worker stopped"))?
                .try_send(job)
                .map_err(|_| error(ErrorCode::Busy, "HTTP queue unavailable"))?,
            _ => self
                .database
                .as_ref()
                .ok_or_else(|| error(ErrorCode::Shutdown, "database worker stopped"))?
                .try_send(job)
                .map_err(|_| error(ErrorCode::Busy, "database queue unavailable"))?,
        }
        self.pending.insert(id, (owner.clone(), cancelled));
        Ok(id)
    }
    pub fn cancel(&mut self, owner: &Owner, id: u64) -> Result<(), ServiceError> {
        let (actual, cancelled) = self
            .pending
            .get(&id)
            .ok_or_else(|| error(ErrorCode::Invalid, "unknown ticket"))?;
        if !Arc::ptr_eq(&owner.0, &actual.0) {
            return Err(error(ErrorCode::Denied, "ticket belongs to another owner"));
        }
        cancelled.store(true, Ordering::Release);
        Ok(())
    }
    pub fn poll(&mut self) -> Option<Completion> {
        let mut result = self.completed.try_recv().ok()?;
        if let Some((owner, _)) = self.pending.remove(&result.id) {
            if !owner.0.active.load(Ordering::Acquire) {
                result.result = Err(error(ErrorCode::Stale, "resource generation retired"));
            }
        }
        Some(result)
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}
impl Drop for Services {
    fn drop(&mut self) {
        for owner in self.owners.values() {
            owner.0.active.store(false, Ordering::Release);
        }
        self.database.take();
        self.http.take();
        // Shutdown only; never called from a gameplay callback.
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
struct SizeLimit {
    remaining: usize,
}
impl std::io::Write for SizeLimit {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other("size limit"));
        }
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
