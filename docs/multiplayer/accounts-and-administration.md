# Local accounts and administration

`skate-accounts` supplies an optional local identity authority. Enabling it on a
host makes verified TLS login and authenticated encrypted gameplay datagrams
mandatory. An account ID is a persistent random 128-bit identifier. A gameplay
actor ID is a separate random connection identifier issued at login. A username,
UDP address, `Info.id`, Lua event argument or progression-example console label
never authenticates an account.

## Initialize and run

Build the account tool and dedicated server:

```sh
cargo build --locked -p skate-accounts -p skate-server
```

Create a private password file with 12–1024 UTF-8 bytes. Do not put passwords in
command-line arguments, public configuration, scripts committed to Git or logs.
The initializer reads the password from standard input:

```sh
target/debug/skate-account init ./local-accounts administrator < /private/admin-password
```

`local-accounts` must not already exist. The tool creates `accounts.sqlite3`,
`certificate.pem` and `private-key.pem`. The initial account has the
`administrator` role and the `*` permission. Re-running bootstrap against an
existing account store is rejected. SQLite stores Argon2id PHC password hashes
with independent random salts; it never stores plaintext passwords.

The generated certificate covers `localhost`, `127.0.0.1` and `::1`. For remote
clients, provide a certificate whose subject alternative names cover the actual
server hostname, and distribute its trust certificate privately. The client
verifies both that trust and the endpoint hostname; skipping certificate checks
is not supported. Import the chosen trust certificate into a browser's trust
store to use the local admin UI, or supply a certificate already trusted there.

Create `accounts.json` alongside the private directory:

```json
{
  "database": "local-accounts/accounts.sqlite3",
  "bind": "127.0.0.1:31443",
  "certificate": "local-accounts/certificate.pem",
  "key": "local-accounts/private-key.pem"
}
```

Paths are relative to that configuration file. Start the host:

```sh
target/debug/skate-server --test-world --bind 127.0.0.1:31030 --accounts accounts.json
```

Add the existing `--resources server.json` option to run resources. Without
`--accounts`, existing unauthenticated development/LAN UDP behavior is retained.
An account-enabled host rejects plaintext gameplay and mismatched actor IDs.
The host prints listener addresses, never issued tokens or transport keys.

The client credential configuration is:

```json
{
  "endpoint": "https://localhost:31443",
  "ca_certificate": "/private/local-accounts/certificate.pem",
  "username": "player",
  "password_file": "/private/player-password"
}
```

The game uses `--account-config FILE` alongside `--connect`; credential loading
and TLS login happen before gameplay transport starts. The configured CA is the
only trust authority for login, redirects and proxies are disabled, and login
has a bounded deadline. A password file is limited to 1026 bytes including an
optional trailing newline. Unix private files require no group/other access;
initialization sets directory mode 0700 and private file mode 0600. Windows
initialization sets an explicit current-user ACL and credential reads verify
that only the current user or SYSTEM has an allow rule. These ACL operations
fail closed when built-in PowerShell is unavailable. Native Windows execution
still requires platform validation.

## Administrative UI and permissions

Open the HTTPS address in a trusted browser. The UI signs in with the same local
account authority. Its bearer token stays in page memory; no cookie or persistent
browser storage is used. The UI reads actual player/resource/log status, accounts,
roles and audit records. It can create accounts, change role inheritance and
permissions, assign/revoke roles, ban/unban accounts, change whitelist policy,
revoke sessions, kick players and start/stop/restart resources.

Host actions return a ticket in `queued` state. The gameplay host rechecks the
live verified session and required permission before execution, calls the real
resource lifecycle or player-removal method, and reports the actual result.
A missing resource/configuration is an error, never a successful mock response.
SQL, password verification and TLS I/O run outside the gameplay loop. The host
publishes status at most every 250 ms and drains at most four admin actions per
step. Bridge locks use nonblocking attempts on gameplay calls; a busy completion
is retained without repeating the operation.
Host results exceeding4096 bytes become a bounded terminal error, allowing
subsequent commands to complete instead of retrying an oversized result forever.

| Permission | Authority |
|---|---|
| `status.read` | View live host status and action results |
| `accounts.read` | List account records |
| `accounts.create` | Create a local password account |
| `roles.read` | Read role hierarchy and grants |
| `roles.write` | Create roles, edit inheritance/grants, assign or revoke roles |
| `players.ban` | Ban/unban stable accounts |
| `players.whitelist` | Change account whitelist and whitelist-only admission |
| `players.kick` | Disconnect one active gameplay session |
| `sessions.revoke` | Invalidate all sessions for an account |
| `resources.manage` | Start/stop/restart resources |
| `audit.read` | Read audit records and other users' action results |
| `*` | All administrative permissions |

New accounts have no roles and are not whitelisted. A role inherits all grants
of its parent roles. Cycles are rejected transactionally. Role/permission
revocation updates existing verified handles before the change returns.
Ban, whitelist exclusion and session revocation invalidate live gameplay keys
and HTTP bearer tokens; the host removes affected players on its next step.
Unbanning requires a new login. Kick invalidates the selected connection only.
Do not give `roles.write` to users who must not grant themselves broader access.
Browser sign-out clears local page state; use session revocation when keys must
also be invalidated on the server.

Server Lua `resource.players()` snapshots include `account_id` from the host's
verified registry. To authorize persistence for a client event, match its
canonical sender actor to a player record and use that record's account ID.
Never accept an `account_id` supplied in an event body. Plain development hosts
have no authenticated account identity. The bundled persistent-progression
example remains a local console example; its account labels are explicitly not
login credentials.

## API and bounds

All routes run on HTTPS. `POST /v1/login` accepts `{username,password}` and returns
a one-hour session with `account_id`, decimal-string `actor`, `session_id`,
transport `key`, HTTP `token` and `expires`. Treat the entire response as secret.
The transport key is single-connection state: never persist or reconstruct a
codec from a previous bundle; a fresh login supplies fresh random keys.

Administrative routes require `Authorization: Bearer TOKEN`:

- `GET /v1/admin/status`, `/accounts?after=USERNAME`, `/roles`, `/audit?after=ID`.
- `POST /v1/admin/accounts`: `{username,password}`.
- `POST /v1/admin/roles`: `{role,parent}` (parent is optional).
- `POST /v1/admin/role-parent`: `{role,parent,grant}`.
- `POST /v1/admin/role-permission`: `{role,permission,grant}`.
- `POST /v1/admin/account-role`: `{account,role,grant}`.
- `POST /v1/admin/ban`: `{account,banned,reason}`.
- `POST /v1/admin/whitelist`: `{account,allowed}`; `/whitelist-mode`: `{enabled}`.
- `POST /v1/admin/revoke`: `{account}`.
- `POST /v1/admin/actions`: `{kind,resource}` for `resource_start`, `resource_stop`
  or `resource_restart`; `{kind:"kick",actor:"DECIMAL_ID"}` for kick.
- `GET /v1/admin/actions/TICKET`: queued or completed actual result.

The server uses two blocking TLS workers, a 16-connection waiting queue, an
absolute three-second handshake/request/response deadline, 8 KiB headers,
16 KiB bodies and 1 MiB responses. Login is limited to ten attempts per source
IP per minute with a 1024-IP rate table. It supports 256 live sessions total,
eight per account, 64 queued host actions and 256 retained tickets. Status is
limited to 128 KiB. Password hashing uses Argon2id's default 19 MiB, two passes,
one lane; only the bounded TLS workers perform remote login/hash work.

SQLite has a 256 MiB main-file cap, DELETE rollback journaling, FULL synchronous
commits and foreign keys. There is one OS-locked authority per database path.
When resource backend services are loaded, their64MiB process-wide SQLite heap
ceiling also applies here. Resource query memory pressure may temporarily fail
account/admin SQL; authentication and permission checks fail closed on errors.
The schema supports 10,000 accounts, 128 roles, 256 explicit grants per role and
512 effective grants per active account. Account listing is paginated by username in batches of 200, including a
next-page action in the UI. Audit retains the latest 10,000 rows and reads 100 at a
time; export before rotation when longer retention is required. Completed host
results are copied to the audit off the gameplay thread, so a crash in that
short interval may leave a queued row without its completion row. Role,
moderation and account changes are audited in the same transaction as their
persistent change. Completed audit details have a bounded truncated tail.

Process restart preserves account IDs, roles, bans, whitelist and audit but
intentionally expires every session. There is no password-reset/email service,
external identity provider, cluster sharing or multi-host session replication.
TLS and the AEAD envelope protect transport, not a compromised game client or
host. Permissions are server-side policy; client prediction remains untrusted.

Gameplay payloads use ring ChaCha20-Poly1305 with random 256-bit per-session
keys, distinct nonce domains for each direction, authenticated header fields,
monotonic 64-bit counters and a 128-packet replay window. Original payloads stay
bounded to 1200 bytes; the authenticated envelope adds 48 bytes. Forged tags
cannot advance the replay window. A store permits only one active server
transport; dropping it revokes sessions, preventing key/counter reuse.

## Dependencies and evidence

New security dependencies are pinned: RustCrypto Argon2 0.5.3 (MIT/Apache-2.0),
ring 0.17.14 (ISC/MIT/OpenSSL-derived notices), rustls 0.23.45
(Apache-2.0/ISC/MIT), rcgen 0.14.10 (MIT/Apache-2.0) and httparse 1.10.1
(MIT/Apache-2.0). The selected rustls/rcgen backend is ring, avoiding an additional
AWS-LC toolchain. SQLite, reqwest and Tokio reuse the established service stack.
Keep upstream license notices when packaging binaries. Crypto primitives,
password hashing and certificate verification use these libraries; this project
only frames bounded datagrams and enforces session ownership/replay policy.

Focused checks are `cargo test --locked -p skate-accounts` and
`cargo test --locked -p skate-server --test accounts`. They exercise real local
TLS and UDP sockets, password verification, wrong-CA rejection, role inheritance
and live revocation, persistent account recovery, moderation, queue completion,
actor spoofing rejection and real host kicks. Native Windows execution, external
CA infrastructure and power-loss fault injection are not claimed by local Linux
test results.
