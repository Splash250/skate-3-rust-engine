use super::*;
use rusqlite::{
    Connection, OpenFlags, TransactionBehavior,
    hooks::{AuthAction, Authorization},
    limits::Limit,
    types::{Value, ValueRef},
};
use std::path::Path;

/// A server-owned adapter must preserve atomicity, isolation, cancellation and limits.
/// It is called only on a fixed database worker, never on a gameplay thread.
pub trait DatabaseProvider: Send + Sync {
    fn execute(
        &self,
        context: OperationContext,
        operation: &Operation,
        limits: &Limits,
    ) -> Result<Response, ServiceError>;
}

pub struct SqliteDatabase {
    root: PathBuf,
}
impl SqliteDatabase {
    pub fn new(root: PathBuf) -> Result<Self, ServiceError> {
        std::fs::create_dir_all(&root)
            .map_err(|_| error(ErrorCode::Database, "cannot create database directory"))?;
        // SQLite allocator limit is process-wide, including account/admin SQL.
        // Keep the same ceiling for every service instance; exhaustion fails closed.
        let setup = Connection::open_in_memory().map_err(db_error)?;
        setup
            .pragma_update(None, "hard_heap_limit", 64 * 1024 * 1024)
            .map_err(db_error)?;
        Ok(Self { root })
    }
}
impl DatabaseProvider for SqliteDatabase {
    fn execute(
        &self,
        context: OperationContext,
        operation: &Operation,
        limits: &Limits,
    ) -> Result<Response, ServiceError> {
        execute(&self.root, limits, context, operation)
    }
}
fn execute(
    root: &Path,
    limits: &Limits,
    context: OperationContext,
    operation: &Operation,
) -> Result<Response, ServiceError> {
    let path = root.join(format!(
        "{}.sqlite3",
        blake3::hash(context.resource().as_bytes()).to_hex()
    ));
    let mut conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(db_error)?;
    conn.busy_timeout(Duration::from_millis(25))
        .map_err(db_error)?;
    conn.pragma_update(None, "journal_mode", "DELETE")
        .map_err(db_error)?;
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(db_error)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(db_error)?;
    conn.pragma_update(None, "trusted_schema", "OFF")
        .map_err(db_error)?;
    conn.pragma_update(None, "temp_store", "MEMORY")
        .map_err(db_error)?;
    conn.pragma_update(None, "cache_size", -2048)
        .map_err(db_error)?;
    conn.pragma_update(None, "cache_spill", "OFF")
        .map_err(db_error)?;
    let page_size: i64 = conn
        .pragma_query_value(None, "page_size", |r| r.get(0))
        .map_err(db_error)?;
    conn.pragma_update(
        None,
        "max_page_count",
        limits.database_bytes as i64 / page_size,
    )
    .map_err(db_error)?;
    let pages: i64 = conn
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .map_err(db_error)?;
    if pages.saturating_mul(page_size) > limits.database_bytes as i64 {
        return Err(error(
            ErrorCode::TooLarge,
            "existing database exceeds configured storage budget",
        ));
    }
    conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, limits.response_bytes as i32)
        .map_err(db_error)?;
    conn.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, limits.request_bytes as i32)
        .map_err(db_error)?;
    conn.set_limit(Limit::SQLITE_LIMIT_COLUMN, 128)
        .map_err(db_error)?;
    conn.set_limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 4096)
        .map_err(db_error)?;
    conn.set_limit(Limit::SQLITE_LIMIT_EXPR_DEPTH, 64)
        .map_err(db_error)?;
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)
        .map_err(db_error)?;
    let progress_context = context.clone();
    conn.progress_handler(1000, Some(move || progress_context.check().is_err()))
        .map_err(db_error)?;
    // Metadata setup is never executable by a resource statement.
    conn.execute_batch("CREATE TABLE IF NOT EXISTS _skate_migrations(version INTEGER PRIMARY KEY, digest TEXT NOT NULL)").map_err(db_error)?;
    let internal = Arc::new(AtomicBool::new(false));
    let auth_internal = internal.clone();
    conn.authorizer(Some(move |ctx: rusqlite::hooks::AuthContext<'_>| {
        if auth_internal.load(Ordering::Relaxed) {
            return Authorization::Allow;
        }
        if ctx
            .database_name
            .is_some_and(|db| db != "main" && db != "temp")
        {
            return Authorization::Deny;
        }
        use AuthAction::*;
        match ctx.action {
            Read { table_name, .. }
            | Insert { table_name }
            | Update { table_name, .. }
            | Delete { table_name }
            | CreateTable { table_name }
            | AlterTable { table_name, .. }
            | DropTable { table_name }
            | CreateIndex { table_name, .. }
            | DropIndex { table_name, .. }
            | CreateTrigger { table_name, .. }
            | DropTrigger { table_name, .. } => {
                if table_name.to_ascii_lowercase().starts_with("_skate_") {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }
            Function { function_name } => {
                if ["load_extension", "readfile", "writefile", "fts3_tokenizer"]
                    .iter()
                    .any(|f| function_name.eq_ignore_ascii_case(f))
                {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }
            Select
            | Recursive
            | CreateView { .. }
            | DropView { .. }
            | Reindex { .. }
            | Analyze { .. } => Authorization::Allow,
            // ATTACH/DETACH, PRAGMA, transaction control, virtual tables, and
            // unsupported future authorizer actions are intentionally denied.
            _ => Authorization::Deny,
        }
    }))
    .map_err(db_error)?;
    let result = run(&mut conn, &internal, limits, &context, operation);
    if result.is_err() {
        context.check()?;
    }
    result
}
fn run(
    conn: &mut Connection,
    internal: &AtomicBool,
    limits: &Limits,
    context: &OperationContext,
    operation: &Operation,
) -> Result<Response, ServiceError> {
    match operation {
        Operation::Query { statement } => {
            let mut budget = limits.response_bytes;
            Ok(Response::Database {
                results: vec![statement_result(
                    conn,
                    statement,
                    true,
                    limits,
                    &mut budget,
                )?],
            })
        }
        Operation::Transaction { statements } => {
            validate_count(statements.len(), limits)?;
            internal.store(true, Ordering::Relaxed);
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            internal.store(false, Ordering::Relaxed);
            let result = (|| {
                let mut budget = limits.response_bytes;
                let mut results = Vec::new();
                for statement in statements {
                    context.check()?;
                    results.push(statement_result(
                        &tx,
                        statement,
                        false,
                        limits,
                        &mut budget,
                    )?);
                }
                context.check()?;
                Ok(Response::Database { results })
            })();
            // The authorizer permits only adapter-controlled commit/rollback.
            internal.store(true, Ordering::Relaxed);
            match result {
                Ok(value) => {
                    tx.commit().map_err(db_error)?;
                    Ok(value)
                }
                Err(e) => {
                    drop(tx);
                    Err(e)
                }
            }
        }
        Operation::Migrate { migrations } => {
            validate_count(migrations.iter().map(|m| m.statements.len()).sum(), limits)?;
            if migrations.is_empty()
                || migrations
                    .iter()
                    .any(|m| m.version == 0 || m.statements.is_empty())
                || migrations.windows(2).any(|m| m[0].version >= m[1].version)
            {
                return Err(error(
                    ErrorCode::Invalid,
                    "migration versions must be positive and ascending",
                ));
            }
            internal.store(true, Ordering::Relaxed);
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let mut version: u32 = tx
                .query_row(
                    "SELECT COALESCE(MAX(version),0) FROM _skate_migrations",
                    [],
                    |r| r.get(0),
                )
                .map_err(db_error)?;
            let result = (|| {
                for migration in migrations {
                    context.check()?;
                    let digest = blake3::hash(
                        &serde_json::to_vec(migration)
                            .map_err(|_| error(ErrorCode::Invalid, "invalid migration"))?,
                    )
                    .to_hex()
                    .to_string();
                    internal.store(true, Ordering::Relaxed);
                    if migration.version <= version {
                        let old: String = tx
                            .query_row(
                                "SELECT digest FROM _skate_migrations WHERE version=?1",
                                [migration.version],
                                |r| r.get(0),
                            )
                            .map_err(db_error)?;
                        if old != digest {
                            return Err(error(
                                ErrorCode::Invalid,
                                "applied migration changed; append a new migration",
                            ));
                        }
                        continue;
                    }
                    if migration.version != version + 1 {
                        return Err(error(
                            ErrorCode::Invalid,
                            "migration gap; next version is required",
                        ));
                    }
                    internal.store(false, Ordering::Relaxed);
                    let mut budget = limits.response_bytes;
                    for statement in &migration.statements {
                        statement_result(&tx, statement, false, limits, &mut budget)?;
                    }
                    internal.store(true, Ordering::Relaxed);
                    tx.execute(
                        "INSERT INTO _skate_migrations(version,digest) VALUES(?1,?2)",
                        rusqlite::params![migration.version, digest],
                    )
                    .map_err(db_error)?;
                    version = migration.version;
                }
                context.check()?;
                Ok(Response::Migrated { version })
            })();
            internal.store(true, Ordering::Relaxed);
            match result {
                Ok(value) => {
                    tx.commit().map_err(db_error)?;
                    Ok(value)
                }
                Err(e) => {
                    drop(tx);
                    Err(e)
                }
            }
        }
        _ => Err(error(ErrorCode::Invalid, "not a database operation")),
    }
}
fn validate_count(count: usize, limits: &Limits) -> Result<(), ServiceError> {
    if count == 0 || count > limits.statements {
        Err(error(
            ErrorCode::TooLarge,
            "transaction statement budget exceeded",
        ))
    } else {
        Ok(())
    }
}
fn statement_result(
    conn: &Connection,
    statement: &Statement,
    readonly: bool,
    limits: &Limits,
    remaining: &mut usize,
) -> Result<QueryResult, ServiceError> {
    // Batch's iterator prepares one statement at a time. Connection::prepare in
    // rusqlite 0.40 recursively checks every tail; hostile long batches must not
    // turn that recursion into a worker stack overflow.
    use rusqlite::fallible_iterator::FallibleIterator;
    let mut batch = rusqlite::Batch::new(conn, &statement.sql);
    let mut prepared = batch
        .next()
        .map_err(db_error)?
        .ok_or_else(|| error(ErrorCode::Invalid, "empty SQL statement"))?;
    if batch.next().map_err(db_error)?.is_some() {
        return Err(error(
            ErrorCode::Invalid,
            "one SQL statement per entry is required",
        ));
    }
    let is_readonly = prepared.readonly();
    if readonly && !is_readonly {
        return Err(error(
            ErrorCode::Denied,
            "query API permits read-only SQL; use a transaction",
        ));
    }
    let params: Vec<Value> = statement
        .params
        .iter()
        .map(|p| match p {
            SqlValue::Null => Ok(Value::Null),
            SqlValue::Integer(v) => Ok(Value::Integer(*v)),
            SqlValue::Real(v) if v.is_finite() => Ok(Value::Real(*v)),
            SqlValue::Real(_) => Err(error(ErrorCode::Invalid, "nonfinite SQL parameter")),
            SqlValue::Text(v) => Ok(Value::Text(v.clone())),
            SqlValue::Blob(v) => Ok(Value::Blob(v.clone())),
        })
        .collect::<Result<_, _>>()?;
    let columns: Vec<String> = prepared
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect();
    spend(
        remaining,
        columns.iter().map(|s| s.len() + 16).sum::<usize>() + 32,
    )?;
    if columns.is_empty() {
        let changed = prepared
            .execute(rusqlite::params_from_iter(params))
            .map_err(db_error)? as u64;
        return Ok(QueryResult {
            columns,
            rows: vec![],
            changed,
        });
    }
    let mut cursor = prepared
        .query(rusqlite::params_from_iter(params))
        .map_err(db_error)?;
    let mut rows = Vec::new();
    while let Some(row) = cursor.next().map_err(db_error)? {
        if rows.len() >= limits.rows {
            return Err(error(ErrorCode::TooLarge, "SQL row budget exceeded"));
        }
        let mut values = Vec::with_capacity(columns.len());
        for col in 0..columns.len() {
            let value = row.get_ref(col).map_err(db_error)?;
            let size = match value {
                ValueRef::Text(v) | ValueRef::Blob(v) => v.len().saturating_mul(6),
                _ => 32,
            };
            spend(remaining, size + 32)?;
            values.push(match value {
                ValueRef::Null => SqlValue::Null,
                ValueRef::Integer(v) => SqlValue::Integer(v),
                ValueRef::Real(v) if v.is_finite() => SqlValue::Real(v),
                ValueRef::Real(_) => return Err(error(ErrorCode::Invalid, "nonfinite SQL result")),
                ValueRef::Text(v) => SqlValue::Text(
                    std::str::from_utf8(v)
                        .map_err(|_| error(ErrorCode::Invalid, "non-UTF8 SQL text"))?
                        .into(),
                ),
                ValueRef::Blob(v) => SqlValue::Blob(v.to_vec()),
            });
        }
        rows.push(values);
    }
    Ok(QueryResult {
        columns,
        rows,
        changed: if is_readonly { 0 } else { conn.changes() },
    })
}
fn spend(remaining: &mut usize, bytes: usize) -> Result<(), ServiceError> {
    *remaining = remaining
        .checked_sub(bytes)
        .ok_or_else(|| error(ErrorCode::TooLarge, "SQL result byte budget exceeded"))?;
    Ok(())
}
fn db_error(e: rusqlite::Error) -> ServiceError {
    // Do not return SQL text, parameter values, filesystem paths, or secrets.
    match e.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => error(
            ErrorCode::Busy,
            "database busy; retry the whole transaction",
        ),
        Some(
            rusqlite::ErrorCode::AuthorizationForStatementDenied
            | rusqlite::ErrorCode::PermissionDenied,
        ) => error(
            ErrorCode::Denied,
            "SQL operation is outside the resource database grant",
        ),
        Some(
            rusqlite::ErrorCode::TooBig
            | rusqlite::ErrorCode::DiskFull
            | rusqlite::ErrorCode::OutOfMemory,
        ) => error(ErrorCode::TooLarge, "database size or value limit exceeded"),
        Some(rusqlite::ErrorCode::ConstraintViolation) => error(
            ErrorCode::Database,
            "SQL constraint failed; transaction rolled back",
        ),
        _ => error(
            ErrorCode::Database,
            "SQL operation failed; check schema, parameters and migration version",
        ),
    }
}
