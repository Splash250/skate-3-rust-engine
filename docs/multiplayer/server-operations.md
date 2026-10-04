# Dedicated-server operations

Run a trusted local server under the external Python supervisor when automatic
recovery, restart, backup or restore is required. The supervisor owns the child
process; a dead gameplay process cannot supervise itself. The existing HTTPS
administration UI supplies authenticated controls and its live permission and
audit model remains the authorization boundary.

Build and configure:

```sh
cargo build --locked -p skate-server -p skate-accounts
python3 tools/server_supervisor.py /private/server/supervisor.json
python3 tools/server_supervisor.py /private/server/supervisor.json --status
```

`supervisor.json` paths resolve relative to that file. The executable is an
operator-selected trusted local binary; arguments are an array, never a shell.
Example (replace the executable with the actual absolute build/package path):

```json
{
  "executable": "/absolute/package/skate-server",
  "arguments": ["--test-world", "--bind", "0.0.0.0:31030", "--resources", "server.json", "--accounts", "accounts.json", "--operations", "operations.json"],
  "working_directory": ".",
  "data_root": "data",
  "control_root": ".supervisor",
  "backup_root": ".backups",
  "pack_root": ".",
  "startup_seconds": 30,
  "hang_seconds": 15,
  "grace_seconds": 10,
  "max_restarts": 3,
  "restart_window_seconds": 300,
  "backoff_seconds": 1,
  "max_backoff_seconds": 30
}
```

Place **all stores covered by recovery** below `data_root`, including the
resource configuration's `storage` directory and the account configuration's
`database`. Certificate/key files and other configuration outside this directory
are not in a snapshot and must be preserved separately. Use disposable paths for
experiments. The data, control and backup locations must be separate. Snapshots,
credentials, private keys, runtime logs and live stores never belong in Git or
redistributable packs.

The supervisor holds OS locks for its control directory and canonical data-root
sibling throughout execution, backoff and recovery. Distinct control directories
cannot supervise the same data root concurrently.
With `pack_root`, it additionally holds `.pack/run.lock`, so pack activation or
rollback fails while a server is managed. An inherited Unix lock also prevents
a replacement supervisor racing a surviving child. A trusted launcher waits at a closed stdin gate until the supervisor has
established containment; server code cannot race ahead and spawn descendants.
Each server has a process group; graceful supervisor stop uses a private stdin command, waits the configured grace, and then kills
remaining descendants. A separate EOF guard cleans the group if the supervisor
dies. Linux additionally establishes a parent-death signal. Windows establishes
a kill-on-close Job Object and refuses to continue when this boundary cannot be
created; native Windows behavior still requires the native acceptance matrix.

The gameplay loop writes a private heartbeat after a completed `Host::step` at
most every 250 ms. HTTP responsiveness does not count as game health. Startup
and heartbeat stalls, and unexpected nonzero exits, consume the rolling recovery
budget; retries use capped exponential backoff and stop in `recovery_exhausted`
when that budget is exhausted. An ordinary zero exit, console `quit`, Ctrl-C or
supervisor SIGTERM stops without restarting. A scheduled shutdown records its
intent at the start of draining; a crash or hang during that interval also stays
stopped. Resume explicitly cancels that intent. Deliberate authenticated restart,
backup and restore requests are separate private control records; these restart
without spending the crash budget. An unexplained exit 75 still has a bounded
restart budget. No public socket controls the supervisor directly.

Status retains up to the last 64 operational events within a 60 KiB serialization
budget and up to 128 snapshot IDs. Byte-limit omissions are reported explicitly.
Combined child output is retained in two bounded 1 MiB log files. The existing
admin status includes supervisor history, admission counts and maintenance state.
Logs may contain resource-authored text; treat the private control directory as
operator data. A failed restore or backup leaves the supervisor stopped with a
bounded diagnostic, instead of repeatedly changing stores or restarting blindly.

## Administrative controls

Open the existing HTTPS admin UI and use **Maintenance, backups, settings and
profiles**. Operations require live account permissions:

| Operation | Permission | Contract |
| --- | --- | --- |
| `maintenance` / `resume` / `capacity` | `server.manage` | Close/reopen admission; schedule shutdown/restart; change capacity without evicting existing connections |
| `backup` / `restore` | `server.backup` | Stop the managed server and perform the named store operation |
| `settings_read` | `settings.read` | Read the resource's typed settings, private values and pending restart values |
| `settings_set` | `settings.write` | Validate, persist and apply or defer a setting update |
| `profile_read` / `profile_export` | `profile.read` | Read resource performance history or export portable Chrome trace JSON |

Choose a running resource and **Load typed settings** to edit controls derived
from its schema. The editor shows type, bounds, visibility, default, active and
pending values, and distinguishes **Apply live** from **Save for restart**.
Private strings use concealed inputs with an explicit reveal control. Signing
out clears loaded values and cancels results belonging to the previous session.
**Refresh performance history** shows resource/generation percentile summaries
and a recent timeline alongside host ticks. It labels inclusive and exclusive
CPU, worker CPU, queue waiting and IPC receive waiting separately. **Export
trace** produces a downloadable Chrome trace JSON file. Omitted rows and bounded
retention are reported in the view. Maintenance and backup controls remain under
**Maintenance, backup and advanced operations** with reviewable JSON fields.

Post to `/v1/admin/actions` with the usual authenticated bearer token:

```json
{"kind":"maintenance","reason":"Scheduled restart","delay_ms":60000,"restart":true}
{"kind":"resume"}
{"kind":"capacity","players":32}
{"kind":"backup","snapshot":"before-upgrade"}
{"kind":"restore","snapshot":"before-upgrade"}
{"kind":"settings_read","resource":"rounds"}
{"kind":"settings_set","resource":"rounds","key":"duration","value":120}
{"kind":"profile_read","resource":null}
{"kind":"profile_export"}
```

Each call returns the existing action ticket. Reading a ticket requires
`status.read`, the original action's permission, and ownership or `audit.read`.
An auditor cannot read private setting results using `audit.read` alone. Settings
values are absent from queued/completed audit records and general host status.
Normal result messages remain bounded to 4 KiB; settings reads allow 64 KiB,
profile summaries 128 KiB and trace export 512 KiB. Oversize results fail with a
terminal explicit limit error, never truncated invalid JSON.

Maintenance stops new admission immediately, rejects waiting joins, and sends
existing players a warning with the remaining deadline every second. Current
resources and sessions continue until the deadline or the last connection
leaves; then normal resource/native-worker/service cleanup runs and the process
exits. Capacity counts include clients still downloading content. Lowering
capacity never ejects current players. Resume clears warnings and reopens
admission, but cannot interrupt an already queued stopped-store operation.
Restart and backup/restore require the external supervisor; unsupported actions
fail explicitly on an independently launched server.

## Snapshot consistency and restore

Implemented contract: **stopped-store copy**. The supervisor waits for the entire
managed process and its descendants to exit before copying the configured data
root. It limits each snapshot to 4096 regular files and 2 GiB, rejects symlinks
and special files, stages the copy, recovers SQLite rollback journals on that
private staged copy, and runs `integrity_check` plus `foreign_key_check` before
publication. The complete manifest pins every relative path, size and SHA-256.
Actual account authority schemas additionally require their supported schema
version; an unrelated service table named `accounts` remains valid.

Restore validates every manifest entry, rejects extra/missing/corrupt files,
stages another private copy and validates SQLite again before moving the current
store. A durable restore journal permits recovery if interrupted between the two
renames. A failed second rename reinstates the original store. Successful restore
retains the previous directory as `data.previous-<nonce>`; its exact path appears
in operational history. Preserve it until the restored application is accepted.
If an interrupted restore is detected on next supervisor launch, the previous
store is reinstated when the active path is missing. The supervisor never guesses
which unrelated filesystem path to remove.

This is not a live SQLite backup or a cross-process transaction snapshot. Other
writers outside the managed process must be stopped by the operator. It is not
power-loss fault injection or a guarantee about storage firmware. Restoring old
data does not reverse application schema changes, executable versions or resource
versions; coordinate the matching [server pack rollback](server-packs.md) while
stopped. Account sessions intentionally expire after process restart, including
a restored account store, while stable IDs, roles and audit persist.

Focused evidence commands:

```sh
python3 -m unittest tools.test_server_supervisor -v
node tools/test_admin_ui.cjs
cargo test --locked -p skate-server --lib operations::tests
cargo test --locked -p skate-server --test accounts --test operations --test udp -- --test-threads=1
```

Tests use synthetic stores and real local processes/TLS/UDP. Existing native
Windows, physical audio, multi-host/WAN and power-loss acceptance gaps remain
separate; see [native acceptance](native-acceptance.md) and
[production validation](production-validation.md).
