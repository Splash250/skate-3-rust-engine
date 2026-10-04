use skate_services::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skate-backend-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn register_owner(service: &mut Services, resource: &str) -> Owner {
    service
        .activate(
            resource,
            1,
            Grants {
                database: true,
                ..Grants::default()
            },
        )
        .unwrap()
}
fn sql(text: &str, params: Vec<SqlValue>) -> Statement {
    Statement {
        sql: text.into(),
        params,
    }
}
fn query(text: &str) -> Operation {
    Operation::Query {
        statement: sql(text, vec![]),
    }
}
fn transaction(statements: Vec<Statement>) -> Operation {
    Operation::Transaction { statements }
}
fn wait(service: &mut Services) -> Completion {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(completion) = service.poll() {
            return completion;
        }
        assert!(Instant::now() < deadline, "service never completed");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn execute(
    service: &mut Services,
    owner: &Owner,
    operation: Operation,
) -> Result<Response, ServiceError> {
    service
        .submit(owner, operation, Duration::from_secs(2))
        .unwrap();
    wait(service).result
}
fn migration() -> Operation {
    Operation::Migrate {
        migrations: vec![Migration {
            version: 1,
            statements: vec![
                sql(
                    "CREATE TABLE progression(account TEXT PRIMARY KEY, coins INTEGER NOT NULL CHECK(coins >= 0))",
                    vec![],
                ),
                sql(
                    "CREATE TABLE inventory(account TEXT REFERENCES progression(account), item TEXT, quantity INTEGER CHECK(quantity > 0), PRIMARY KEY(account,item))",
                    vec![],
                ),
            ],
        }],
    }
}
fn balance(service: &mut Services, owner: &Owner) -> i64 {
    let Response::Database { results } = execute(
        service,
        owner,
        query("SELECT coins FROM progression WHERE account='local-account'"),
    )
    .unwrap() else {
        panic!()
    };
    let SqlValue::Integer(value) = results[0].rows[0][0] else {
        panic!()
    };
    value
}

#[test]
fn progression_transactions_concurrent_updates_and_full_process_restart() {
    let temp = Temp::new();
    let mut service = Services::new(temp.0.clone(), Limits::default()).unwrap();
    let owner = register_owner(&mut service, "progression");
    assert_eq!(
        execute(&mut service, &owner, migration()).unwrap(),
        Response::Migrated { version: 1 }
    );
    execute(
        &mut service,
        &owner,
        transaction(vec![sql(
            "INSERT INTO progression VALUES(?1,?2)",
            vec![
                SqlValue::Text("local-account".into()),
                SqlValue::Integer(100),
            ],
        )]),
    )
    .unwrap();
    execute(
        &mut service,
        &owner,
        transaction(vec![
            sql(
                "UPDATE progression SET coins=coins-25 WHERE account=?1",
                vec![SqlValue::Text("local-account".into())],
            ),
            sql(
                "INSERT INTO inventory VALUES(?1,?2,?3)",
                vec![
                    SqlValue::Text("local-account".into()),
                    SqlValue::Text("deck-blue".into()),
                    SqlValue::Integer(1),
                ],
            ),
        ]),
    )
    .unwrap();
    let failed = execute(
        &mut service,
        &owner,
        transaction(vec![
            sql(
                "UPDATE progression SET coins=coins-25 WHERE account='local-account'",
                vec![],
            ),
            sql(
                "INSERT INTO inventory VALUES('local-account','deck-blue',1)",
                vec![],
            ),
        ]),
    )
    .unwrap_err();
    assert_eq!(failed.code, ErrorCode::Database);
    assert_eq!(
        balance(&mut service, &owner),
        75,
        "failed inventory operation must roll back debit"
    );
    for _ in 0..12 {
        service
            .submit(
                &owner,
                transaction(vec![sql(
                    "UPDATE progression SET coins=coins+1 WHERE account='local-account'",
                    vec![],
                )]),
                Duration::from_secs(2),
            )
            .unwrap();
    }
    let mut completed = 0;
    while completed < 12 {
        match wait(&mut service).result {
            Ok(_) => completed += 1,
            Err(e) if e.code == ErrorCode::Busy => {
                service
                    .submit(
                        &owner,
                        transaction(vec![sql(
                            "UPDATE progression SET coins=coins+1 WHERE account='local-account'",
                            vec![],
                        )]),
                        Duration::from_secs(2),
                    )
                    .unwrap();
            }
            Err(e) => panic!("concurrent transaction: {e}"),
        }
    }
    assert_eq!(balance(&mut service, &owner), 87);
    drop(service);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "subprocess_database_reopen", "--ignored"])
        .env("SKATE_SERVICES_TEST_DB", &temp.0)
        .status()
        .unwrap();
    assert!(status.success());
    let mut service = Services::new(temp.0.clone(), Limits::default()).unwrap();
    let owner = register_owner(&mut service, "progression");
    assert_eq!(balance(&mut service, &owner), 88);
}

#[test]
#[ignore = "child process of progression_transactions_concurrent_updates_and_full_process_restart"]
fn subprocess_database_reopen() {
    let root = PathBuf::from(std::env::var_os("SKATE_SERVICES_TEST_DB").unwrap());
    let mut service = Services::new(root, Limits::default()).unwrap();
    let owner = register_owner(&mut service, "progression");
    assert_eq!(
        execute(&mut service, &owner, migration()).unwrap(),
        Response::Migrated { version: 1 }
    );
    assert_eq!(balance(&mut service, &owner), 87);
    execute(
        &mut service,
        &owner,
        transaction(vec![sql(
            "UPDATE progression SET coins=coins+1 WHERE account='local-account'",
            vec![],
        )]),
    )
    .unwrap();
}

#[test]
fn database_capabilities_migrations_isolation_and_statement_sandbox() {
    let temp = Temp::new();
    let mut service = Services::new(temp.0.clone(), Limits::default()).unwrap();
    let a = register_owner(&mut service, "a");
    let b = register_owner(&mut service, "b");
    let denied = service.activate("denied", 1, Grants::default()).unwrap();
    assert_eq!(
        service
            .submit(&denied, query("SELECT 1"), Duration::from_secs(1))
            .unwrap_err()
            .code,
        ErrorCode::Denied
    );
    execute(&mut service, &a, migration()).unwrap();
    assert!(execute(&mut service, &b, query("SELECT * FROM progression")).is_err());
    for statement in [
        "ATTACH ':memory:' AS other",
        "PRAGMA writable_schema=1",
        "SELECT * FROM _skate_migrations",
        "DROP TABLE _skate_migrations",
        "SELECT load_extension('x')",
        "COMMIT",
        "CREATE VIRTUAL TABLE x USING fts5(content)",
    ] {
        assert!(
            execute(&mut service, &a, transaction(vec![sql(statement, vec![])])).is_err(),
            "allowed {statement}"
        );
    }
    assert!(
        execute(
            &mut service,
            &a,
            transaction(vec![sql(
                "CREATE TABLE extra(x); INSERT INTO extra VALUES(1)",
                vec![]
            )])
        )
        .is_err()
    );
    assert_eq!(
        execute(
            &mut service,
            &a,
            Operation::Migrate {
                migrations: vec![Migration {
                    version: 1,
                    statements: vec![sql("CREATE TABLE changed(x)", vec![])]
                }]
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::Invalid
    );
    assert_eq!(
        execute(
            &mut service,
            &a,
            Operation::Migrate {
                migrations: vec![Migration {
                    version: 3,
                    statements: vec![sql("CREATE TABLE gap(x)", vec![])]
                }]
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::Invalid
    );
    assert!(
        execute(
            &mut service,
            &a,
            query("INSERT INTO progression VALUES('oops',1)")
        )
        .is_err()
    );
    assert_eq!(
        execute(&mut service, &a, query(&"SELECT 1;".repeat(16000)))
            .unwrap_err()
            .code,
        ErrorCode::Invalid
    );
    let literal = "x'); DROP TABLE progression; --";
    execute(
        &mut service,
        &a,
        transaction(vec![sql(
            "INSERT INTO progression VALUES(?1, ?2)",
            vec![SqlValue::Text(literal.into()), SqlValue::Integer(1)],
        )]),
    )
    .unwrap();
    let Response::Database { results } =
        execute(&mut service, &a, query("SELECT account FROM progression")).unwrap()
    else {
        panic!()
    };
    assert_eq!(results[0].rows[0][0], SqlValue::Text(literal.into()));
    let Response::Database { results } = execute(
        &mut service,
        &a,
        transaction(vec![
            sql("UPDATE progression SET coins=coins+1", vec![]),
            sql("SELECT account FROM progression", vec![]),
        ]),
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(results[0].changed, 1);
    assert_eq!(
        results[1].changed, 0,
        "SELECT must not inherit the preceding write count"
    );
}

#[test]
fn database_limits_cancellation_deadline_and_retired_generation() {
    let temp = Temp::new();
    let mut service = Services::new(
        temp.0.clone(),
        Limits {
            pending: 2,
            pending_per_resource: 2,
            response_bytes: 4096,
            ..Limits::default()
        },
    )
    .unwrap();
    let owner = register_owner(&mut service, "progression");
    execute(&mut service, &owner, migration()).unwrap();
    let expensive = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n";
    service
        .submit(&owner, query(expensive), Duration::from_millis(25))
        .unwrap();
    assert_eq!(
        wait(&mut service).result.unwrap_err().code,
        ErrorCode::Timeout
    );
    let ticket = service
        .submit(&owner, query(expensive), Duration::from_secs(2))
        .unwrap();
    std::thread::sleep(Duration::from_millis(15));
    service.cancel(&owner, ticket).unwrap();
    assert_eq!(
        wait(&mut service).result.unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(
        execute(&mut service, &owner, query("SELECT zeroblob(8192)"))
            .unwrap_err()
            .code,
        ErrorCode::TooLarge
    );
    let ticket = service
        .submit(&owner, query(expensive), Duration::from_secs(2))
        .unwrap();
    let next = service
        .activate(
            "progression",
            2,
            Grants {
                database: true,
                ..Grants::default()
            },
        )
        .unwrap();
    assert_eq!(
        service.cancel(&next, ticket).unwrap_err().code,
        ErrorCode::Denied
    );
    assert_eq!(
        wait(&mut service).result.unwrap_err().code,
        ErrorCode::Stale
    );
    assert_eq!(
        service
            .submit(&owner, query("SELECT 1"), Duration::from_secs(1))
            .unwrap_err()
            .code,
        ErrorCode::Stale
    );
    execute(&mut service, &next, query("SELECT 1")).unwrap();
}

fn fixture(response: Vec<u8>, delay: Duration) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = [0; 4096];
        let _ = stream.read(&mut bytes);
        std::thread::sleep(delay);
        let _ = stream.write_all(&response);
    });
    (origin, worker)
}
fn http_request(url: &str) -> Operation {
    Operation::Http {
        request: HttpRequest {
            url: url.into(),
            method: "GET".into(),
            headers: BTreeMap::new(),
            body: vec![],
        },
    }
}

#[test]
fn local_http_results_deadlines_cancellation_bounds_and_origin_grants() {
    let temp = Temp::new();
    let mut service = Services::new(
        temp.0.clone(),
        Limits {
            pending: 2,
            pending_per_resource: 2,
            response_bytes: 1024,
            ..Limits::default()
        },
    )
    .unwrap();
    for (index, response, delay, expected, cancel) in [
        (
            1,
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec(),
            0,
            None,
            false,
        ),
        (
            2,
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
            200,
            Some(ErrorCode::Timeout),
            false,
        ),
        (
            3,
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
            200,
            Some(ErrorCode::Cancelled),
            true,
        ),
        (
            4,
            b"HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n".to_vec(),
            0,
            Some(ErrorCode::TooLarge),
            false,
        ),
        (
            5,
            [
                b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".as_slice(),
                &vec![b'x'; 2048],
            ]
            .concat(),
            0,
            Some(ErrorCode::TooLarge),
            false,
        ),
    ] {
        let (origin, worker) = fixture(response, Duration::from_millis(delay));
        let owner = service
            .activate(
                "http",
                index,
                Grants {
                    database: false,
                    http_origins: BTreeSet::from([origin.clone()]),
                },
            )
            .unwrap();
        assert_eq!(
            service
                .submit(
                    &owner,
                    http_request("http://localhost:9/"),
                    Duration::from_secs(1)
                )
                .unwrap_err()
                .code,
            ErrorCode::Denied
        );
        let ticket = service
            .submit(
                &owner,
                http_request(&origin),
                Duration::from_millis(if index == 2 { 40 } else { 1000 }),
            )
            .unwrap();
        if cancel {
            std::thread::sleep(Duration::from_millis(30));
            service.cancel(&owner, ticket).unwrap();
        }
        let completion = wait(&mut service);
        match expected {
            Some(code) => assert_eq!(completion.result.unwrap_err().code, code),
            None => {
                let Response::Http { status, body, .. } = completion.result.unwrap() else {
                    panic!()
                };
                assert_eq!(status, 200);
                assert_eq!(body, b"hello");
            }
        }
        worker.join().unwrap();
    }
}

#[test]
fn http_backpressure_redirect_and_revocation() {
    let temp = Temp::new();
    let mut service = Services::new(
        temp.0.clone(),
        Limits {
            pending: 1,
            pending_per_resource: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    let (origin, worker) = fixture(
        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
        Duration::ZERO,
    );
    let owner = service
        .activate(
            "http",
            1,
            Grants {
                database: false,
                http_origins: BTreeSet::from([origin.clone()]),
            },
        )
        .unwrap();
    service
        .submit(&owner, http_request(&origin), Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        service
            .submit(&owner, http_request(&origin), Duration::from_secs(1))
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
    assert!(matches!(
        wait(&mut service).result.unwrap(),
        Response::Http { status: 302, .. }
    ));
    worker.join().unwrap();
    let (origin, worker) = fixture(
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec(),
        Duration::from_millis(100),
    );
    let owner = service
        .activate(
            "http",
            2,
            Grants {
                database: false,
                http_origins: BTreeSet::from([origin.clone()]),
            },
        )
        .unwrap();
    service
        .submit(&owner, http_request(&origin), Duration::from_secs(1))
        .unwrap();
    std::thread::sleep(Duration::from_millis(25));
    service.revoke(&owner);
    assert_eq!(
        wait(&mut service).result.unwrap_err().code,
        ErrorCode::Stale
    );
    worker.join().unwrap();
}

#[test]
fn interrupted_transactions_failed_migrations_and_database_size_roll_back() {
    let temp = Temp::new();
    let mut service = Services::new(
        temp.0.clone(),
        Limits {
            database_bytes: 64 * 1024,
            ..Limits::default()
        },
    )
    .unwrap();
    let owner = register_owner(&mut service, "rollback");
    execute(&mut service, &owner, migration()).unwrap();
    execute(
        &mut service,
        &owner,
        transaction(vec![sql(
            "INSERT INTO progression VALUES('local-account',100)",
            vec![],
        )]),
    )
    .unwrap();
    let expensive = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n";
    let ticket = service
        .submit(
            &owner,
            transaction(vec![
                sql("UPDATE progression SET coins=coins-25", vec![]),
                sql(expensive, vec![]),
            ]),
            Duration::from_secs(2),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    service.cancel(&owner, ticket).unwrap();
    assert_eq!(
        wait(&mut service).result.unwrap_err().code,
        ErrorCode::Cancelled
    );
    assert_eq!(balance(&mut service, &owner), 100);
    let bad_migration = Operation::Migrate {
        migrations: vec![Migration {
            version: 2,
            statements: vec![
                sql("CREATE TABLE failed_upgrade(x)", vec![]),
                sql("INSERT INTO missing_table VALUES(1)", vec![]),
            ],
        }],
    };
    assert!(execute(&mut service, &owner, bad_migration).is_err());
    assert!(execute(&mut service, &owner, query("SELECT * FROM failed_upgrade")).is_err());
    assert_eq!(execute(&mut service, &owner, Operation::Migrate { migrations: vec![Migration {version: 2, statements: vec![
        sql("ALTER TABLE progression ADD COLUMN experience INTEGER NOT NULL DEFAULT 0", vec![]),
        sql("CREATE TABLE blobs(value BLOB)", vec![]),
    ]}] }).unwrap(), Response::Migrated {version: 2});
    assert_eq!(
        execute(
            &mut service,
            &owner,
            transaction(vec![
                sql("UPDATE progression SET coins=coins-25", vec![]),
                sql("INSERT INTO blobs VALUES(zeroblob(70000))", vec![]),
            ])
        )
        .unwrap_err()
        .code,
        ErrorCode::TooLarge
    );
    assert_eq!(balance(&mut service, &owner), 100);
    let database = temp
        .0
        .join(format!("{}.sqlite3", blake3::hash(b"rollback").to_hex()));
    assert!(std::fs::metadata(database).unwrap().len() <= 64 * 1024);
}

#[test]
fn repeated_revocation_retires_queued_sql_without_mutation() {
    let temp = Temp::new();
    let mut service = Services::new(
        temp.0.clone(),
        Limits {
            database_workers: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    let owner = register_owner(&mut service, "revoked");
    execute(&mut service, &owner, migration()).unwrap();
    execute(
        &mut service,
        &owner,
        transaction(vec![sql(
            "INSERT INTO progression VALUES('local-account',100)",
            vec![],
        )]),
    )
    .unwrap();
    service.submit(&owner, query("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n"), Duration::from_secs(2)).unwrap();
    service
        .submit(
            &owner,
            transaction(vec![sql("UPDATE progression SET coins=0", vec![])]),
            Duration::from_secs(2),
        )
        .unwrap();
    service.revoke(&owner);
    service.revoke(&owner);
    for _ in 0..2 {
        assert_eq!(
            wait(&mut service).result.unwrap_err().code,
            ErrorCode::Stale
        );
    }
    assert_eq!(service.pending_count(), 0);
    let next = service
        .activate(
            "revoked",
            2,
            Grants {
                database: true,
                ..Grants::default()
            },
        )
        .unwrap();
    assert_eq!(balance(&mut service, &next), 100);
}

#[test]
fn revoked_queued_http_never_connects_to_destination() {
    let temp = Temp::new();
    let mut service = Services::new(
        temp.0.clone(),
        Limits {
            http_workers: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    let first = TcpListener::bind("127.0.0.1:0").unwrap();
    let first_origin = format!("http://{}", first.local_addr().unwrap());
    let forbidden = TcpListener::bind("127.0.0.1:0").unwrap();
    forbidden.set_nonblocking(true).unwrap();
    let forbidden_origin = format!("http://{}", forbidden.local_addr().unwrap());
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = first.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = [0; 4096];
        stream.read(&mut bytes).unwrap();
        started_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
    });
    let first_owner = service
        .activate(
            "first",
            1,
            Grants {
                database: false,
                http_origins: BTreeSet::from([first_origin.clone()]),
            },
        )
        .unwrap();
    let retired = service
        .activate(
            "retired",
            1,
            Grants {
                database: false,
                http_origins: BTreeSet::from([forbidden_origin.clone()]),
            },
        )
        .unwrap();
    service
        .submit(
            &first_owner,
            http_request(&first_origin),
            Duration::from_secs(2),
        )
        .unwrap();
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let retired_ticket = service
        .submit(
            &retired,
            http_request(&forbidden_origin),
            Duration::from_secs(2),
        )
        .unwrap();
    service.revoke(&retired);
    service.revoke(&retired);
    release_tx.send(()).unwrap();
    for _ in 0..2 {
        let completion = wait(&mut service);
        if completion.id == retired_ticket {
            assert_eq!(completion.result.unwrap_err().code, ErrorCode::Stale);
        } else {
            completion.result.unwrap();
        }
    }
    assert_eq!(
        forbidden.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    worker.join().unwrap();
}

#[test]
fn http_grants_validate_before_script_activation() {
    for origin in [
        "http://localhost/private",
        "http://localhost/?token=secret",
        "http://user:pass@localhost/",
        "file:///tmp/data",
        "http://localhost/#fragment",
    ] {
        assert!(
            Grants {
                database: false,
                http_origins: BTreeSet::from([origin.into()])
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        Grants {
            database: true,
            http_origins: BTreeSet::from([
                "https://example.com:443/".into(),
                "http://127.0.0.1:8080".into()
            ])
        }
        .validate()
        .is_ok()
    );
    assert_eq!(
        Grants {
            database: false,
            http_origins: (0..65)
                .map(|port| format!("http://127.0.0.1:{}", 8000 + port))
                .collect()
        }
        .validate()
        .unwrap_err()
        .code,
        ErrorCode::TooLarge
    );
}

#[test]
fn views_and_triggers_cannot_read_or_mutate_protected_migration_metadata() {
    let temp = Temp::new();
    let mut service = Services::new(temp.0.clone(), Limits::default()).unwrap();
    let owner = register_owner(&mut service, "indirect-metadata");
    execute(&mut service, &owner, migration()).unwrap();
    execute(
        &mut service,
        &owner,
        transaction(vec![sql(
            "CREATE VIEW metadata_alias AS SELECT version,digest FROM _skate_migrations",
            vec![],
        )]),
    )
    .unwrap();
    assert_eq!(
        execute(&mut service, &owner, query("SELECT * FROM metadata_alias"))
            .unwrap_err()
            .code,
        ErrorCode::Denied,
        "a view must not launder protected metadata reads"
    );
    execute(
        &mut service,
        &owner,
        transaction(vec![sql(
            "CREATE TRIGGER mutate_metadata AFTER INSERT ON progression BEGIN DELETE FROM _skate_migrations; END",
            vec![],
        )]),
    )
    .unwrap();
    assert_eq!(
        execute(
            &mut service,
            &owner,
            transaction(vec![sql(
                "INSERT INTO progression VALUES('local-account',100)",
                vec![],
            )]),
        )
        .unwrap_err()
        .code,
        ErrorCode::Denied,
        "a trigger must not mutate protected metadata"
    );
    let Response::Database { results } = execute(
        &mut service,
        &owner,
        query("SELECT COUNT(*) FROM progression"),
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(results[0].rows[0][0], SqlValue::Integer(0));
    assert_eq!(
        execute(&mut service, &owner, migration()).unwrap(),
        Response::Migrated { version: 1 },
        "the denied trigger must preserve the original migration digest"
    );
}
