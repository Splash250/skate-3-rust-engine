use crate::*;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use ring::digest::{SHA256, digest};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

const SESSION_SECONDS: u64 = 3600;
const MAX_SESSIONS: usize = 256;
#[derive(Clone, Serialize, Deserialize)]
pub struct SessionCredentials {
    pub account_id: String,
    #[serde(with = "crate::decimal")]
    pub actor: u64,
    pub session_id: String,
    /// Secret transport key; deliver only through verified TLS. Never log.
    pub key: String,
    /// Secret HTTP bearer token; deliver only through verified TLS. Never log.
    pub token: String,
    pub expires: u64,
}
pub(crate) struct SessionState {
    pub issuer: Arc<()>,
    pub account_id: String,
    pub actor: u64,
    pub id: [u8; 16],
    pub key: [u8; 32],
    pub token_tag: [u8; 32],
    pub expires: u64,
    pub active: AtomicBool,
    pub permissions: RwLock<BTreeSet<String>>,
}
#[derive(Clone)]
pub struct VerifiedSession(pub(crate) Arc<SessionState>);
impl VerifiedSession {
    pub fn account_id(&self) -> &str {
        &self.0.account_id
    }
    pub fn actor(&self) -> u64 {
        self.0.actor
    }
    /// Host-side immediate disconnect; the key cannot be used again.
    pub fn revoke_connection(&self) {
        self.0.active.store(false, Ordering::Release);
    }
    pub fn is_active(&self) -> bool {
        self.0.active.load(Ordering::Acquire) && now() < self.0.expires
    }
    pub fn permits(&self, permission: &str) -> bool {
        self.is_active()
            && self
                .0
                .permissions
                .try_read()
                .is_ok_and(|p| p.contains("*") || p.contains(permission))
    }
    pub(crate) fn require(&self, permission: &str) -> Result<()> {
        if self.permits(permission) {
            Ok(())
        } else {
            Err(error(
                "denied",
                "verified session lacks permission or is revoked",
            ))
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    pub id: String,
    pub username: String,
    pub roles: Vec<String>,
    pub banned: bool,
    pub whitelisted: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct AuditRecord {
    pub id: i64,
    pub time: u64,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub detail: String,
}
struct Inner {
    path: PathBuf,
    issuer: Arc<()>,
    _lock: std::fs::File,
    pub(crate) transport_claimed: AtomicBool,
    mutation: Mutex<()>,
    sessions: Mutex<BTreeMap<[u8; 16], VerifiedSession>>,
    dummy_hash: String,
}
impl Drop for Inner {
    fn drop(&mut self) {
        for session in self.sessions.get_mut().unwrap().values() {
            session.0.active.store(false, Ordering::Release);
        }
    }
}
#[derive(Clone)]
pub struct AccountStore(Arc<Inner>);
impl AccountStore {
    /// One live authority per path. Process restart intentionally expires old sessions.
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| error("storage", "cannot create account storage directory"))?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock_path = path.with_extension("authority.lock");
        let lock = options
            .open(&lock_path)
            .map_err(|_| error("storage", "cannot open account authority lock"))?;
        lock.try_lock().map_err(|_| {
            error(
                "busy",
                "another account authority is already using this database",
            )
        })?;
        private_file(&lock_path)?;
        let file = options
            .open(&path)
            .map_err(|_| error("storage", "cannot open account database"))?;
        drop(file);
        private_file(&path)?;
        let conn = connect(&path)?;
        conn.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            INSERT OR IGNORE INTO metadata VALUES('schema','1');
            INSERT OR IGNORE INTO metadata VALUES('whitelist','0');
            CREATE TABLE IF NOT EXISTS accounts(id TEXT PRIMARY KEY,username TEXT UNIQUE NOT NULL,password_hash TEXT NOT NULL,banned INTEGER NOT NULL DEFAULT 0,whitelisted INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS roles(name TEXT PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS role_parents(role TEXT REFERENCES roles(name),parent TEXT REFERENCES roles(name),PRIMARY KEY(role,parent));
            CREATE TABLE IF NOT EXISTS role_permissions(role TEXT REFERENCES roles(name),permission TEXT,PRIMARY KEY(role,permission));
            CREATE TABLE IF NOT EXISTS account_roles(account TEXT REFERENCES accounts(id),role TEXT REFERENCES roles(name),PRIMARY KEY(account,role));
            CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY,token_hash BLOB NOT NULL,account TEXT REFERENCES accounts(id),expires INTEGER NOT NULL,revoked INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS audit(id INTEGER PRIMARY KEY,time INTEGER NOT NULL,actor TEXT NOT NULL,action TEXT NOT NULL,target TEXT NOT NULL,detail TEXT NOT NULL);
            UPDATE sessions SET revoked=1 WHERE revoked=0;
            COMMIT;").map_err(db_error)?;
        let schema: String = conn
            .query_row("SELECT value FROM metadata WHERE key='schema'", [], |r| {
                r.get(0)
            })
            .map_err(db_error)?;
        if schema != "1" {
            return Err(error("storage", "unsupported account schema version"));
        }
        private_file(&path)?;
        Ok(Self(Arc::new(Inner {
            path,
            issuer: Arc::new(()),
            _lock: lock,
            transport_claimed: AtomicBool::new(false),
            mutation: Mutex::new(()),
            sessions: Mutex::new(BTreeMap::new()),
            dummy_hash: hash_password("unmatchable-dummy-password")?,
        })))
    }
    fn require(&self, session: &VerifiedSession, permission: &str) -> Result<()> {
        if !Arc::ptr_eq(&self.0.issuer, &session.0.issuer) {
            return Err(error(
                "denied",
                "session belongs to another account authority",
            ));
        }
        session.require(permission)
    }
    pub(crate) fn claim_transport(&self) -> Result<()> {
        if self.0.transport_claimed.swap(true, Ordering::AcqRel) {
            Err(error(
                "busy",
                "account authority already has an active transport",
            ))
        } else {
            Ok(())
        }
    }
    pub(crate) fn release_transport(&self) {
        for session in self.0.sessions.lock().unwrap().values() {
            session.0.active.store(false, Ordering::Release);
        }
        self.0.transport_claimed.store(false, Ordering::Release);
    }
    pub fn bootstrap_admin(&self, username: &str, password: &str) -> Result<AccountRecord> {
        valid_username(username)?;
        valid_password(password)?;
        let hash = hash_password(password)?;
        let _gate = self.0.mutation.lock().unwrap();
        let mut conn = connect(&self.0.path)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))
            .map_err(db_error)?;
        if count != 0 {
            return Err(error(
                "denied",
                "administrator bootstrap requires an empty account store",
            ));
        }
        let id = hex(&random::<16>()?);
        tx.execute(
            "INSERT INTO accounts(id,username,password_hash,whitelisted) VALUES(?1,?2,?3,1)",
            params![id, username, hash],
        )
        .map_err(db_error)?;
        tx.execute("INSERT INTO roles(name) VALUES('administrator')", [])
            .map_err(db_error)?;
        tx.execute(
            "INSERT INTO role_permissions VALUES('administrator','*')",
            [],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO account_roles VALUES(?1,'administrator')",
            [&id],
        )
        .map_err(db_error)?;
        audit(&tx, "local-bootstrap", "account.bootstrap", &id, username)?;
        tx.commit().map_err(db_error)?;
        record(&conn, &id)
    }
    pub fn login(&self, username: &str, password: &str) -> Result<SessionCredentials> {
        if valid_username(username).is_err() || valid_password(password).is_err() {
            return Err(error("unauthenticated", "invalid credentials"));
        }
        let conn = connect(&self.0.path)?;
        let row: Option<(String, String)> = conn
            .query_row(
                "SELECT id,password_hash FROM accounts WHERE username=?1",
                [username],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        let encoded = row
            .as_ref()
            .map_or(self.0.dummy_hash.as_str(), |r| r.1.as_str());
        let parsed = PasswordHash::new(encoded)
            .map_err(|_| error("storage", "invalid stored password hash"))?;
        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_err()
            || row.is_none()
        {
            let _ = audit(
                &conn,
                "unauthenticated",
                "login.failed",
                username,
                "invalid credentials",
            );
            return Err(error("unauthenticated", "invalid credentials"));
        }
        let id = row.unwrap().0;
        let _gate = self.0.mutation.lock().unwrap();
        let account = record(&conn, &id)?;
        let whitelist: String = conn
            .query_row(
                "SELECT value FROM metadata WHERE key='whitelist'",
                [],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if account.banned || (whitelist == "1" && !account.whitelisted) {
            return Err(error("denied", "account is banned or is not whitelisted"));
        }
        let mut sessions = self.0.sessions.lock().unwrap();
        sessions.retain(|_, s| s.is_active());
        if sessions.len() >= MAX_SESSIONS
            || sessions.values().filter(|s| s.account_id() == id).count() >= 8
        {
            return Err(error(
                "busy",
                "session capacity reached; revoke an old session",
            ));
        }
        let session_id = random::<16>()?;
        let key = random::<32>()?;
        let token = random::<32>()?;
        let token_hash: [u8; 32] = digest(&SHA256, &token).as_ref().try_into().unwrap();
        let actor = u64::from_le_bytes(random::<8>()?).max(1);
        if sessions.values().any(|s| s.actor() == actor) {
            return Err(error(
                "busy",
                "connection identifier collision; retry login",
            ));
        }
        let expires = now() + SESSION_SECONDS;
        let session = VerifiedSession(Arc::new(SessionState {
            issuer: self.0.issuer.clone(),
            account_id: id.clone(),
            actor,
            id: session_id,
            key,
            token_tag: ring::hmac::sign(
                &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &token_hash),
                b"skate-account-bearer-v1",
            )
            .as_ref()
            .try_into()
            .unwrap(),
            expires,
            active: AtomicBool::new(true),
            permissions: RwLock::new(permissions(&conn, &id)?),
        }));
        conn.execute(
            "DELETE FROM sessions WHERE revoked=1 OR expires<=?1",
            [now() as i64],
        )
        .map_err(db_error)?;
        conn.execute(
            "INSERT INTO sessions(id,token_hash,account,expires) VALUES(?1,?2,?3,?4)",
            params![hex(&session_id), token_hash.as_slice(), id, expires as i64],
        )
        .map_err(db_error)?;
        audit(
            &conn,
            &id,
            "login.success",
            &hex(&session_id),
            "local password",
        )?;
        sessions.insert(session_id, session);
        Ok(SessionCredentials {
            account_id: id,
            actor,
            session_id: hex(&session_id),
            key: hex(&key),
            token: hex(&token),
            expires,
        })
    }
    pub fn authenticate(&self, token: &str) -> Result<VerifiedSession> {
        let bytes = unhex::<32>(token).map_err(|_| error("unauthenticated", "invalid session"))?;
        let hash = digest(&SHA256, &bytes);
        let sessions = self
            .0
            .sessions
            .try_lock()
            .map_err(|_| error("busy", "authentication registry busy"))?;
        sessions
            .values()
            .find(|s| {
                s.is_active()
                    && ring::hmac::verify(
                        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, hash.as_ref()),
                        b"skate-account-bearer-v1",
                        &s.0.token_tag,
                    )
                    .is_ok()
            })
            .cloned()
            .ok_or_else(|| error("unauthenticated", "invalid or revoked session"))
    }
    pub(crate) fn by_id(&self, id: &[u8; 16]) -> Result<VerifiedSession> {
        self.0
            .sessions
            .try_lock()
            .map_err(|_| error("busy", "authentication registry busy"))?
            .get(id)
            .filter(|s| s.is_active())
            .cloned()
            .ok_or_else(|| error("unauthenticated", "invalid or revoked session"))
    }
    pub fn accounts(&self, session: &VerifiedSession) -> Result<Vec<AccountRecord>> {
        self.accounts_page(session, "")
    }
    pub fn accounts_page(
        &self,
        session: &VerifiedSession,
        after: &str,
    ) -> Result<Vec<AccountRecord>> {
        self.require(session, "accounts.read")?;
        if !after.is_empty() {
            valid_username(after)?;
        }
        let conn = connect(&self.0.path)?;
        let ids = conn
            .prepare("SELECT id FROM accounts WHERE username>?1 ORDER BY username LIMIT 200")
            .map_err(db_error)?
            .query_map([after], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        ids.iter().map(|id| record(&conn, id)).collect()
    }
    pub fn roles(&self, session: &VerifiedSession) -> Result<serde_json::Value> {
        self.require(session, "roles.read")?;
        let conn = connect(&self.0.path)?;
        let rows = conn
            .prepare("SELECT name FROM roles ORDER BY name LIMIT 128")
            .map_err(db_error)?
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let mut result = Vec::new();
        for role in rows {
            let parents = conn
                .prepare("SELECT parent FROM role_parents WHERE role=?1")
                .map_err(db_error)?
                .query_map([&role], |r| r.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?;
            let grants = conn
                .prepare("SELECT permission FROM role_permissions WHERE role=?1")
                .map_err(db_error)?
                .query_map([&role], |r| r.get::<_, String>(0))
                .map_err(db_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_error)?;
            result.push(serde_json::json!({"name":role,"parents":parents,"permissions":grants}));
        }
        Ok(serde_json::Value::Array(result))
    }
    pub fn create_account(
        &self,
        actor: &VerifiedSession,
        username: &str,
        password: &str,
    ) -> Result<AccountRecord> {
        self.require(actor, "accounts.create")?;
        valid_username(username)?;
        valid_password(password)?;
        let encoded = hash_password(password)?;
        let _gate = self.0.mutation.lock().unwrap();
        self.require(actor, "accounts.create")?;
        let mut conn = connect(&self.0.path)?;
        let tx = conn.transaction().map_err(db_error)?;
        let count: i64 = tx
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))
            .map_err(db_error)?;
        if count >= 10000 {
            return Err(error("limit", "account capacity reached"));
        }
        let id = hex(&random::<16>()?);
        tx.execute(
            "INSERT INTO accounts(id,username,password_hash) VALUES(?1,?2,?3)",
            params![id, username, encoded],
        )
        .map_err(db_error)?;
        audit(&tx, actor.account_id(), "account.create", &id, username)?;
        tx.commit().map_err(db_error)?;
        record(&conn, &id)
    }
    pub fn create_role(
        &self,
        actor: &VerifiedSession,
        role: &str,
        parent: Option<&str>,
    ) -> Result<()> {
        self.require(actor, "roles.write")?;
        valid_name(role)?;
        if let Some(parent) = parent {
            valid_name(parent)?;
        }
        self.mutate(
            actor,
            "roles.write",
            "role.create",
            role,
            parent.unwrap_or(""),
            |conn| {
                let count: i64 = conn
                    .query_row("SELECT count(*) FROM roles", [], |r| r.get(0))
                    .map_err(db_error)?;
                if count >= 128 {
                    return Err(error("limit", "role capacity reached"));
                }
                conn.execute("INSERT INTO roles(name) VALUES(?1)", [role])
                    .map_err(db_error)?;
                if let Some(parent) = parent {
                    conn.execute(
                        "INSERT INTO role_parents VALUES(?1,?2)",
                        params![role, parent],
                    )
                    .map_err(db_error)?;
                }
                Ok(())
            },
        )
    }
    pub fn role_parent(
        &self,
        actor: &VerifiedSession,
        role: &str,
        parent: &str,
        grant: bool,
    ) -> Result<()> {
        valid_name(role)?;
        valid_name(parent)?;
        self.mutate(actor, "roles.write", "role.parent", role, parent, |conn| {
            if grant {
                let mut visited = BTreeSet::new();
                let mut next = vec![parent.to_string()];
                while let Some(value) = next.pop() {
                    if value == role {
                        return Err(error("invalid", "role inheritance cycle"));
                    }
                    if visited.insert(value.clone()) {
                        next.extend(
                            conn.prepare("SELECT parent FROM role_parents WHERE role=?1")
                                .map_err(db_error)?
                                .query_map([value], |r| r.get::<_, String>(0))
                                .map_err(db_error)?
                                .collect::<std::result::Result<Vec<_>, _>>()
                                .map_err(db_error)?,
                        );
                    }
                }
                conn.execute(
                    "INSERT OR IGNORE INTO role_parents VALUES(?1,?2)",
                    params![role, parent],
                )
                .map_err(db_error)?;
            } else {
                conn.execute(
                    "DELETE FROM role_parents WHERE role=?1 AND parent=?2",
                    params![role, parent],
                )
                .map_err(db_error)?;
            }
            Ok(())
        })
    }
    pub fn role_permission(
        &self,
        actor: &VerifiedSession,
        role: &str,
        permission: &str,
        grant: bool,
    ) -> Result<()> {
        valid_name(role)?;
        valid_name(permission)?;
        self.mutate(
            actor,
            "roles.write",
            "role.permission",
            role,
            &format!("{permission}:{grant}"),
            |conn| {
                if grant {
                    let count: i64 = conn
                        .query_row(
                            "SELECT count(*) FROM role_permissions WHERE role=?1",
                            [role],
                            |r| r.get(0),
                        )
                        .map_err(db_error)?;
                    if count >= 256 {
                        return Err(error("limit", "role permission limit"));
                    }
                    conn.execute(
                        "INSERT OR IGNORE INTO role_permissions VALUES(?1,?2)",
                        params![role, permission],
                    )
                    .map_err(db_error)?;
                } else {
                    conn.execute(
                        "DELETE FROM role_permissions WHERE role=?1 AND permission=?2",
                        params![role, permission],
                    )
                    .map_err(db_error)?;
                }
                Ok(())
            },
        )
    }
    pub fn assign_role(
        &self,
        actor: &VerifiedSession,
        account: &str,
        role: &str,
        grant: bool,
    ) -> Result<()> {
        unhex::<16>(account)?;
        valid_name(role)?;
        self.mutate(
            actor,
            "roles.write",
            "account.role",
            account,
            &format!("{role}:{grant}"),
            |conn| {
                if grant {
                    conn.execute(
                        "INSERT OR IGNORE INTO account_roles VALUES(?1,?2)",
                        params![account, role],
                    )
                    .map_err(db_error)?;
                } else {
                    conn.execute(
                        "DELETE FROM account_roles WHERE account=?1 AND role=?2",
                        params![account, role],
                    )
                    .map_err(db_error)?;
                }
                Ok(())
            },
        )
    }
    pub fn ban(
        &self,
        actor: &VerifiedSession,
        account: &str,
        banned: bool,
        reason: &str,
    ) -> Result<()> {
        unhex::<16>(account)?;
        if reason.len() > 512 {
            return Err(error("limit", "moderation reason too long"));
        }
        self.mutate(
            actor,
            "players.ban",
            "account.ban",
            account,
            reason,
            |conn| {
                if conn
                    .execute(
                        "UPDATE accounts SET banned=?1 WHERE id=?2",
                        params![banned, account],
                    )
                    .map_err(db_error)?
                    == 0
                {
                    return Err(error("missing", "unknown account"));
                }
                Ok(())
            },
        )
    }
    pub fn whitelist(&self, actor: &VerifiedSession, account: &str, allowed: bool) -> Result<()> {
        unhex::<16>(account)?;
        self.mutate(
            actor,
            "players.whitelist",
            "account.whitelist",
            account,
            if allowed { "allow" } else { "deny" },
            |conn| {
                if conn
                    .execute(
                        "UPDATE accounts SET whitelisted=?1 WHERE id=?2",
                        params![allowed, account],
                    )
                    .map_err(db_error)?
                    == 0
                {
                    return Err(error("missing", "unknown account"));
                }
                Ok(())
            },
        )
    }
    pub fn whitelist_mode(&self, actor: &VerifiedSession, enabled: bool) -> Result<()> {
        self.mutate(
            actor,
            "players.whitelist",
            "whitelist.mode",
            "server",
            if enabled { "enabled" } else { "disabled" },
            |conn| {
                conn.execute(
                    "UPDATE metadata SET value=?1 WHERE key='whitelist'",
                    [if enabled { "1" } else { "0" }],
                )
                .map_err(db_error)?;
                Ok(())
            },
        )
    }
    pub fn revoke(&self, actor: &VerifiedSession, account: &str) -> Result<()> {
        unhex::<16>(account)?;
        self.require(actor, "sessions.revoke")?;
        let _gate = self.0.mutation.lock().unwrap();
        self.require(actor, "sessions.revoke")?;
        let mut conn = connect(&self.0.path)?;
        let tx = conn.transaction().map_err(db_error)?;
        tx.execute("UPDATE sessions SET revoked=1 WHERE account=?1", [account])
            .map_err(db_error)?;
        audit(
            &tx,
            actor.account_id(),
            "session.revoke",
            account,
            "all sessions",
        )?;
        tx.commit().map_err(db_error)?;
        for session in self
            .0
            .sessions
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.account_id() == account)
        {
            session.0.active.store(false, Ordering::Release);
        }
        Ok(())
    }
    fn mutate(
        &self,
        actor: &VerifiedSession,
        permission: &str,
        action: &str,
        target: &str,
        detail: &str,
        work: impl FnOnce(&Connection) -> Result<()>,
    ) -> Result<()> {
        self.require(actor, permission)?;
        let _gate = self.0.mutation.lock().unwrap();
        self.require(actor, permission)?;
        let mut conn = connect(&self.0.path)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_error)?;
        work(&tx)?;
        audit(&tx, actor.account_id(), action, target, detail)?;
        // Prepare every fallible refresh before commit. Live handles change before this
        // mutation returns; errors roll back both permissions and moderation.
        let whitelist: String = tx
            .query_row(
                "SELECT value FROM metadata WHERE key='whitelist'",
                [],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let sessions = self.0.sessions.lock().unwrap();
        let mut updates = Vec::new();
        for session in sessions.values() {
            let account = record(&tx, session.account_id())?;
            let allowed = !account.banned && (whitelist != "1" || account.whitelisted);
            let grants = permissions(&tx, session.account_id())?;
            if !allowed {
                tx.execute(
                    "UPDATE sessions SET revoked=1 WHERE id=?1",
                    [hex(&session.0.id)],
                )
                .map_err(db_error)?;
            }
            updates.push((session.clone(), allowed, grants));
        }
        tx.commit().map_err(db_error)?;
        for (session, allowed, grants) in updates {
            if !allowed {
                session.0.active.store(false, Ordering::Release);
            }
            *session.0.permissions.write().unwrap() = grants;
        }
        Ok(())
    }
    pub fn audit_log(&self, session: &VerifiedSession, after: i64) -> Result<Vec<AuditRecord>> {
        self.require(session, "audit.read")?;
        connect(&self.0.path)?.prepare("SELECT id,time,actor,action,target,detail FROM audit WHERE id>?1 ORDER BY id LIMIT 100").map_err(db_error)?.query_map([after],|r|Ok(AuditRecord{id:r.get(0)?,time:r.get::<_,i64>(1)? as u64,actor:r.get(2)?,action:r.get(3)?,target:r.get(4)?,detail:r.get(5)?})).map_err(db_error)?.collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)
    }
    pub(crate) fn audit_action(
        &self,
        session: &VerifiedSession,
        action: &str,
        target: &str,
        detail: &str,
    ) -> Result<()> {
        audit(
            &connect(&self.0.path)?,
            session.account_id(),
            action,
            target,
            detail,
        )
    }
}
fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).map_err(db_error)?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(db_error)?;
    conn.execute_batch("PRAGMA foreign_keys=ON;PRAGMA trusted_schema=OFF;PRAGMA synchronous=FULL;PRAGMA journal_mode=DELETE;PRAGMA max_page_count=65536;").map_err(db_error)?;
    Ok(conn)
}
fn record(conn: &Connection, id: &str) -> Result<AccountRecord> {
    let (username, banned, whitelisted) = conn
        .query_row(
            "SELECT username,banned,whitelisted FROM accounts WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(db_error)?;
    let roles = conn
        .prepare("SELECT role FROM account_roles WHERE account=?1 ORDER BY role")
        .map_err(db_error)?
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(db_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(AccountRecord {
        id: id.into(),
        username,
        roles,
        banned,
        whitelisted,
    })
}
fn permissions(conn: &Connection, id: &str) -> Result<BTreeSet<String>> {
    let grants=conn.prepare("WITH RECURSIVE effective(role) AS (SELECT role FROM account_roles WHERE account=?1 UNION SELECT parent FROM role_parents JOIN effective ON role_parents.role=effective.role) SELECT DISTINCT permission FROM role_permissions JOIN effective ON role_permissions.role=effective.role LIMIT 513").map_err(db_error)?.query_map([id],|r|r.get::<_,String>(0)).map_err(db_error)?.collect::<std::result::Result<BTreeSet<_>,_>>().map_err(db_error)?;
    if grants.len() > 512 {
        return Err(error(
            "limit",
            "an account may inherit at most 512 permissions",
        ));
    }
    Ok(grants)
}

fn audit(conn: &Connection, actor: &str, action: &str, target: &str, detail: &str) -> Result<()> {
    if detail.len() > 4096 {
        return Err(error("limit", "audit detail too long"));
    }
    conn.execute(
        "INSERT INTO audit(time,actor,action,target,detail) VALUES(?1,?2,?3,?4,?5)",
        params![now() as i64, actor, action, target, detail],
    )
    .map_err(db_error)?;
    // Retain a bounded local audit window; operators can export records before rotation.
    conn.execute(
        "DELETE FROM audit WHERE id <= (SELECT COALESCE(MAX(id),0)-10000 FROM audit)",
        [],
    )
    .map_err(db_error)?;
    Ok(())
}
fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::encode_b64(&random::<16>()?)
        .map_err(|_| error("unavailable", "cannot encode password salt"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| error("unavailable", "password hashing failed"))
}
pub(crate) fn valid_username(username: &str) -> Result<()> {
    if !(3..=32).contains(&username.len())
        || !username
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        Err(error(
            "invalid",
            "username requires 3..32 lowercase ASCII letters, digits, underscore or hyphen",
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn valid_password(password: &str) -> Result<()> {
    if !(12..=1024).contains(&password.len()) {
        Err(error("invalid", "password requires 12..1024 bytes"))
    } else {
        Ok(())
    }
}
pub(crate) fn valid_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-*".contains(&b))
    {
        Err(error("invalid", "invalid permission or role name"))
    } else {
        Ok(())
    }
}
fn db_error(_: rusqlite::Error) -> Error {
    error(
        "storage",
        "account operation failed; check uniqueness, references and storage availability",
    )
}
pub(crate) fn private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| error("storage", "cannot protect private account file"))?;
    }
    #[cfg(windows)]
    {
        windows_acl(path, false)?;
    }
    Ok(())
}
#[cfg(windows)]
pub(crate) fn windows_acl(path: &Path, verify: bool) -> Result<()> {
    // PowerShell ships with supported Windows installations. Pass the path as an
    // environment value, never as interpolated shell syntax. Fail closed if ACL
    // setup or verification is unavailable.
    let script = if verify {
        r#"$ErrorActionPreference='Stop';$u=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value;$a=Get-Acl -LiteralPath $env:SKATE_ACCOUNT_PRIVATE_PATH;foreach($r in $a.Access){if($r.AccessControlType -eq 'Allow'){$s=$r.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value;if($s -ne $u -and $s -ne 'S-1-5-18'){exit 3}}};exit 0"#
    } else {
        r#"$ErrorActionPreference='Stop';$p=$env:SKATE_ACCOUNT_PRIVATE_PATH;$u=[System.Security.Principal.WindowsIdentity]::GetCurrent().User;if((Get-Item -LiteralPath $p).PSIsContainer){$a=New-Object System.Security.AccessControl.DirectorySecurity;$r=New-Object System.Security.AccessControl.FileSystemAccessRule($u,'FullControl','ContainerInherit,ObjectInherit','None','Allow')}else{$a=New-Object System.Security.AccessControl.FileSecurity;$r=New-Object System.Security.AccessControl.FileSystemAccessRule($u,'FullControl','Allow')};$a.SetOwner($u);$a.SetAccessRuleProtection($true,$false);$a.AddAccessRule($r);Set-Acl -LiteralPath $p -AclObject $a;exit 0"#
    };
    let status = std::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("SKATE_ACCOUNT_PRIVATE_PATH", path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| error("denied", "cannot run Windows credential ACL protection"))?;
    if status.success() {
        Ok(())
    } else {
        Err(error(
            "denied",
            "Windows credential file ACL is not private or could not be protected",
        ))
    }
}
