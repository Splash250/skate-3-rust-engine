# Server backend services

`skate-services` implements bounded asynchronous SQL and HTTP(S) operations.
The dedicated host integrates it with the same resource ID, generation and
manifest/configuration grant intersection as other resource APIs. Client scripts
cannot execute these operations. Credentials belong in private server scripts
or server configuration, never in shared/client scripts or downloadable assets.

## Resource API

```lua
resource.on("service_result", function(payload)
    -- payload.key identifies a pending request in this resource generation.
    if payload.result.ok then
        local value = payload.result.value
        -- database: value.results = { {columns, rows, changed}, ... }
        -- http: value.status, value.headers, value.body (array of bytes)
        -- migrate: value.version
    else
        sdk.log(payload.result.error.code .. ": " .. payload.result.error.message)
    end
end)
resource.services.submit("lookup", {
    kind = "query",
    statement = {
        sql = "SELECT coins FROM accounts WHERE id = ?1",
        params = {{type = "text", value = "local-account"}}
    }
}, 5000)
-- resource.services.cancel("lookup")
```

`resource.events` permits the completion handler; `resource.database` permits
`query`, `transaction` and `migrate`; `resource.http` permits HTTP only for exact
administrator-approved origins. The timeout is wall-clock milliseconds including
queue time. Submission and completion polling never perform network or disk I/O
on gameplay ticks. Request keys cannot overlap while outstanding. Native worker
tickets and owner handles are not exposed to scripts.

SQL parameters and results have explicit typed values: `{type="null"}`, or
`{type="integer"|"real"|"text"|"blob", value=...}`. Integers are signed 64-bit
in Rust/SQLite. Resource authors should keep Lua/JSON numeric values within the
exact range of their consumer; opaque full-width identities remain text. Blobs
and HTTP bodies are arrays of bytes; their JSON encoding costs more than the raw
byte count. SQL must be one statement per `statement` entry. Parameters are bound
by SQLite; never concatenate player input into SQL.

`transaction` takes `statements={...}` and commits every statement atomically.
Constraint failure, cancellation/deadline observed before commit, or oversized
results roll back the whole transaction. `query` accepts only read-only SQL.
Both return ordered `results`, each with `columns`, typed `rows`, and `changed`.
`changed` is the directly changed row count for writes; use SQL `RETURNING` when
a transaction needs a value produced by the write.

`migrate` takes ascending `migrations={{version=1,statements={...}},...}`. Versions
are contiguous and positive. The backend stores each version and a BLAKE3 digest
of its exact statement/parameter serialization in protected metadata. Replaying
an identical migration succeeds; editing an applied migration or skipping a
version fails. Append migrations for upgrades. A failed migration batch rolls
back its schema/data/metadata together. Schema changes can use ordinary SQLite
DDL, including `ALTER TABLE`. ATTACH/DETACH, script PRAGMAs, virtual tables,
extension loading, explicit transaction control, and access to `_skate_` metadata
are denied by SQLite's authorizer, including through triggers/views.

HTTP operations have the form:

```lua
resource.services.submit("health", {
    kind = "http",
    request = {url="http://127.0.0.1:8080/health", method="GET"}
}, 1000)
```

Add `"http_origins":{"my-resource":["http://127.0.0.1:8080"]}` to the server
resource configuration alongside its `resource.http` grant. Origins include the
scheme, hostname and port. Different hostnames/ports require their own grants.
Local/private destinations are allowed only when explicitly listed by the
administrator. An origin grant trusts that destination's DNS resolution; it is
not a public-internet-only SSRF filter. Redirects are returned to the script and
never followed. Environment proxies are disabled. TLS certificate verification
and built-in WebPKI roots remain enabled. URL credentials/fragments, proxy and
routing/framing header overrides are rejected. Supported methods are GET, HEAD,
POST, PUT, PATCH, DELETE and OPTIONS. HTTP status errors such as 404 are ordinary
responses; transport/TLS failures use an error completion. Compressed response
decompression is not enabled.

## Ownership, persistence and failure

The server's existing configuration-path scope separates servers; a full BLAKE3
resource ID hash names each SQLite file within that scope's services directory.
No script selects a filesystem path or database name. Generations share durable
data but never handles/callbacks. Stop/restart/failure/disconnect retires pending
work and suppresses old-generation delivery. Retiring an owner cannot undo a
transaction that already committed or an HTTP request already observed by its
destination. Cancellation has the same boundary: use idempotency keys for remote
side effects and reconcile durable state after ambiguous shutdowns. HTTP futures
are dropped when cancelled, with cancellation checked every 5 ms; SQLite progress
handlers interrupt long-running SQL and check again before committing. A write
that races cancellation at its final commit may already have succeeded.

SQLite uses `synchronous=FULL`, DELETE rollback journals, foreign keys, disabled
trusted schema, in-memory temporary storage, and disabled cache spilling. The
main file's page count is capped; rollback journal growth is bounded by the
original pages changed in one transaction. No WAL reader can pin an indefinitely
growing log. SQLite has a fixed **64 MiB process-wide allocator ceiling**, shared
by all backend connections, including temporary sorts. This intentionally
affects other in-process SQLite adapters using the same library, including the
account/admin store. A resource query can temporarily exhaust that shared budget
and make account/admin SQL fail until memory is released; this is bounded shared
availability, not per-resource memory isolation. Memory or disk
quota exhaustion fails an operation; it does not grant more memory. SQLite
reports lock contention as `busy`; retry the entire failed transaction. There is
no implicit retry of a successfully committed write.

Back up a stopped server's services directory, or use a proper SQLite online
backup procedure. Do not copy only a live database while a rollback journal is
active. Ordinary resource unload never deletes durable data. Removing resource
data is an explicit administrator filesystem action. SQLite transaction recovery
is automatic on open. Fresh tests exercise process restart after committed
writes; abrupt process-kill/power-loss fault injection remains separate from that
evidence.

## Budgets and adapter contract

`Services::new(root, Limits)` validates all limits before starting workers.
Library defaults are 128 outstanding requests total, 16 per resource, two SQL
workers, eight concurrent HTTP tasks, 256 KiB serialized request size, 1 MiB
response budget, 4096 rows per statement, 64 statements per transaction or
migration request, 256 MiB per database, and a 15-second maximum deadline. The
dedicated Lua host uses a smaller response budget and enforces its own serialized
callback limit. HTTP caps both declared and streamed lengths; SQL bounds both
value size and aggregate returned values before copying them. Pending counts
include completed-but-unpolled results. Full queues return `busy` immediately.
No queue grows to accommodate overload. Cancellation/revocation retains bounded
slots until the worker releases them, preventing cancel/resubmit overload.

Additional server-owned adapters implement `DatabaseProvider::execute`, supplied
to `Services::with_database`. They receive an opaque `OperationContext` with
resource/generation/deadline and cancellation checks, typed operations, and the
same result limits. They must preserve ownership, atomicity and bounds. An
adapter is trusted host code, not an uploaded DLL/SO. SQLite is the implemented
provider; no PostgreSQL or cloud provider is claimed.

## Dependencies and validation

- [rusqlite](https://docs.rs/rusqlite/0.40.2/rusqlite/) 0.40.2: MIT, maintained
  SQLite bindings. Bundled SQLite (public domain) avoids a system SQLite version
  prerequisite on Linux/Windows; a C compiler is required during builds. Hooks
  implement the authorizer/progress boundary and limits cap SQL values/programs.
- [reqwest](https://docs.rs/reqwest/0.12.28/reqwest/) 0.12: MIT/Apache-2.0,
  asynchronous HTTP. Default native TLS is disabled; rustls uses verified WebPKI
  roots on Linux and Windows, avoiding an OpenSSL deployment dependency.
- [Tokio](https://docs.rs/tokio/1.53.2/tokio/) 1: MIT, one dedicated current-thread
  network runtime; its task pool is limited independently of the game loop.
  Tokio's bounded channels and Rust's bounded SQL/completion channels enforce
  backpressure. BLAKE3 reuses the repository's pinned dependency.

Run `cargo test --locked -p skate-services`. The fixture-free suite uses actual
temporary SQLite files, a separate child process for restart recovery, concurrent
worker transactions, cancellation/deadlines, migration tampering, cross-resource
denials, parameter injection data, and local TCP HTTP services for limits,
timeouts, cancellation and redirect policy. HTTPS certificate verification is
enabled in the real client, but a live TLS endpoint/native Windows run is not
proved by the local plain-HTTP socket tests. The reusable
[progression resource](../../resources/persistent-progression/README.md) exercises
the resource-to-server path; its account labels are not an authentication system.

Fresh Linux validation on 2026-10-04:

| Command | Observed result |
| --- | --- |
| `cargo test --locked -p skate-services` | Nine main integration tests passed in 0.41 s; the child-process restart helper also executed and passed. It appears as ignored in the parent harness because only the parent test launches it with the required temporary-database path. |
| `cargo test --locked -p skate-server --test backend_example` | One test passed in 0.28 s. It launched real server executables with the unchanged bundled resource, waited for its completion logs, drove awards/purchases/inspection through stdin, restarted the resource, restarted the whole process, and confirmed an insufficient-funds purchase rolled back without another item. |

These timings are test wall times on the available machine, not a throughput or
capacity benchmark. Tests also cover repeated revocation, no connection from a
revoked queued HTTP request, transactional cancellation, migration rollback and
upgrade, lowered storage budgets, statement-batch rejection without recursive
preparation, and exact-origin configuration validation. Native Windows, live
HTTPS certificate success, and abrupt power-loss recovery were not exercised.
