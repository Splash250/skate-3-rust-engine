//! Actual-process failure and offline backup checks; all data is synthetic.
use skate_services::*;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("skate-recovery-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn statement(sql: &str) -> Statement {
    Statement {
        sql: sql.into(),
        params: vec![],
    }
}
fn owner(service: &mut Services) -> Owner {
    service
        .activate(
            "recovery",
            1,
            Grants {
                database: true,
                ..Grants::default()
            },
        )
        .unwrap()
}
fn execute(service: &mut Services, owner: &Owner, operation: Operation) -> Response {
    let id = service
        .submit(owner, operation, Duration::from_secs(5))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        if let Some(completion) = service.poll() {
            assert_eq!(completion.id, id);
            return completion.result.unwrap();
        }
        assert!(Instant::now() < deadline, "backend completion deadline");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn transaction(sql: &str) -> Operation {
    Operation::Transaction {
        statements: vec![statement(sql)],
    }
}
fn migration() -> Operation {
    Operation::Migrate {
        migrations: vec![Migration {
            version: 1,
            statements: vec![statement(
                "CREATE TABLE progress(account TEXT PRIMARY KEY, coins INTEGER NOT NULL CHECK(coins >= 0))",
            )],
        }],
    }
}
fn balance(service: &mut Services, owner: &Owner) -> i64 {
    let Response::Database { results } = execute(
        service,
        owner,
        Operation::Query {
            statement: statement("SELECT coins FROM progress WHERE account='synthetic-account'"),
        },
    ) else {
        panic!("database response expected")
    };
    let SqlValue::Integer(value) = results[0].rows[0][0] else {
        panic!("integer balance expected")
    };
    value
}
fn database(root: &Path) -> PathBuf {
    root.join(format!("{}.sqlite3", blake3::hash(b"recovery").to_hex()))
}

#[test]
fn killed_writer_preserves_committed_data_and_offline_backup_restores_independently() {
    let temp = Temp::new();
    let live = temp.0.join("live");
    let ready = temp.0.join("committed");
    let mut worker = Worker(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "subprocess_uncommitted_writer",
                "--ignored",
                "--nocapture",
            ])
            .env("SKATE_RECOVERY_ROOT", &live)
            .env("SKATE_RECOVERY_READY", &ready)
            .spawn()
            .unwrap(),
    );
    let db = database(&live);
    let journal = db.with_extension("sqlite3-journal");
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        assert!(
            worker.0.try_wait().unwrap().is_none(),
            "writer exited before forced termination"
        );
        if ready.exists() && std::fs::metadata(&journal).is_ok_and(|m| m.len() > 512) {
            // Prove the transaction is running after its first write, not merely
            // queued. A separate reader still sees the committed balance.
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.busy_timeout(Duration::ZERO).unwrap();
            assert_eq!(
                conn.query_row("SELECT coins FROM progress", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                100
            );
            let error = conn.execute_batch("BEGIN IMMEDIATE").unwrap_err();
            assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "writer never created an active rollback journal"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    worker.0.kill().unwrap();
    let status = worker.0.wait().unwrap();
    assert!(
        !status.success(),
        "child must terminate without a graceful service drop"
    );
    assert!(
        journal.exists(),
        "kill must leave the active journal on disk"
    );

    let mut service = Services::new(live.clone(), Limits::default()).unwrap();
    let resource = owner(&mut service);
    assert_eq!(
        execute(&mut service, &resource, migration()),
        Response::Migrated { version: 1 }
    );
    assert_eq!(
        balance(&mut service, &resource),
        100,
        "uncommitted debit must not survive abrupt process death"
    );
    execute(
        &mut service,
        &resource,
        transaction("UPDATE progress SET coins=coins+7"),
    );
    assert_eq!(balance(&mut service, &resource), 107);
    drop(service);
    let connection = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(
        connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    drop(connection);

    // Follow the documented backup contract: all workers have stopped before
    // copying the entire services directory, including any remaining journals.
    let backup = temp.0.join("backup");
    std::fs::create_dir(&backup).unwrap();
    for entry in std::fs::read_dir(&live).unwrap() {
        let entry = entry.unwrap();
        assert!(entry.file_type().unwrap().is_file());
        std::fs::copy(entry.path(), backup.join(entry.file_name())).unwrap();
    }
    let backup_bytes = std::fs::read(database(&backup)).unwrap();
    let mut source = Services::new(live, Limits::default()).unwrap();
    let source_owner = owner(&mut source);
    execute(
        &mut source,
        &source_owner,
        transaction("UPDATE progress SET coins=900"),
    );
    assert_eq!(balance(&mut source, &source_owner), 900);
    drop(source);
    assert_eq!(std::fs::read(database(&backup)).unwrap(), backup_bytes);

    let restored = temp.0.join("restored");
    std::fs::create_dir(&restored).unwrap();
    for entry in std::fs::read_dir(&backup).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), restored.join(entry.file_name())).unwrap();
    }
    let mut service = Services::new(restored, Limits::default()).unwrap();
    let resource = owner(&mut service);
    assert_eq!(
        execute(&mut service, &resource, migration()),
        Response::Migrated { version: 1 }
    );
    assert_eq!(
        balance(&mut service, &resource),
        107,
        "restored snapshot must exclude later source writes"
    );
    execute(
        &mut service,
        &resource,
        transaction("UPDATE progress SET coins=coins+1"),
    );
    assert_eq!(balance(&mut service, &resource), 108);
    println!(
        "RECOVERY killed_writer=true committed_balance=100 recovered_balance=100 backup_balance=107 source_after_backup=900 restored_balance=107 integrity=ok"
    );
}

#[test]
#[ignore = "only parent recovery test supplies a temporary database and kills this child"]
fn subprocess_uncommitted_writer() {
    let root = PathBuf::from(std::env::var_os("SKATE_RECOVERY_ROOT").unwrap());
    let ready = PathBuf::from(std::env::var_os("SKATE_RECOVERY_READY").unwrap());
    let mut service = Services::new(root, Limits::default()).unwrap();
    let resource = owner(&mut service);
    execute(&mut service, &resource, migration());
    execute(
        &mut service,
        &resource,
        transaction("INSERT INTO progress VALUES('synthetic-account',100)"),
    );
    std::fs::write(ready, "committed").unwrap();
    service.submit(&resource, Operation::Transaction { statements: vec![
        statement("UPDATE progress SET coins=coins-25"),
        statement("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n"),
    ] }, Duration::from_secs(15)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        assert!(
            service.poll().is_none(),
            "transaction ended before the parent kill"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("parent failed to terminate child within deadline");
}
