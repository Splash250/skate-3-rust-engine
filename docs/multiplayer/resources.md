# Dedicated resources: hosting, cache and troubleshooting

The resource host adds server-selected Lua programs and public content to the
direct UDP dedicated server. Gameplay still uses UDP; a separate bounded HTTP
service transfers resources over TCP on the same address and port. Steam is not
required. A future transport adapter can reuse the resource host contract.

## Run the bundled challenge

From the repository root:

```sh
cargo run --locked -p skate-server -- --test-world --resources resources/server.json
./PLAY.sh --connect 127.0.0.1:31030 --test-world
```

On Windows, use the built `skate-server.exe` / `skate3rust.exe` with the same
arguments. Clients need their normal prepared character and animation assets;
the downloaded resource set does not include retail game files. For an owned
map, use matching `--map` bytes on server and client instead of `--test-world`.

Allow both UDP and TCP for the selected `--bind` port. TCP serves only the
selected public resource content. Large resource downloads do not use the
movement packet channel. If binding port `0` for tests, HTTP uses the UDP port
actually selected by the operating system.

The built-in content server uses HTTP. It does not authenticate a publisher or
add TLS, account login or anti-cheat to direct UDP. Full content digests detect
wrong bytes relative to the selected manifest; they are not publisher signatures.
Do not expose private configuration or serve the whole resource directory from
an unrelated static file server.

## Server configuration and lifecycle

`--resources` takes a JSON configuration file. Paths are relative to that file:

```json
{
  "root": ".",
  "storage": "../resource-data",
  "ensure": ["landing-challenge"],
  "grants": {
    "skate-rules": ["resource.exports"],
    "landing-challenge": [
      "resource.events", "resource.network", "resource.state", "resource.storage",
      "resource.commands", "resource.exports", "engine.ui"
    ]
  }
}
```

This is the provided [`resources/server.json`](../../resources/server.json).
Dependency closure is selected automatically. Every resource has an explicit
grant set; requesting a capability in its manifest does not itself grant it.
Server-only scripts and the persistence directory are excluded from the public
download set. Keep secrets out of shared/client scripts and `files` entries.

Use the server's standard-input console:

| Command | Effect |
| --- | --- |
| `resources` | Inspect resource status and diagnostics. |
| `start ID` | Start the resource and its dependencies; an already-running resource remains running. |
| `stop ID` | Stop it and active dependents, retiring their owned state. |
| `restart ID` | Stop/start the resource with new generations, handling active dependents. |
| `ensure ID` | Start if stopped; restart if running. |
| `command NAME [args]` | Invoke a registered resource command from the trusted local console. |
| `quit` | Stop resources and shut down cleanly. |

For the example, `command challenge_reset` starts a fresh round;
`stop landing-challenge` removes its client presentation. There is no remote
administrator command interface. The local console is trusted, so protect access
to the server process normally. A script-registered command's permission name is
not a license for a client to execute it through a similarly named event.

## Client admission, updates and failure

A connecting client learns the required resource revision, resolves dependencies,
reuses verified content and downloads missing blobs. It validates the complete
set before publication and starts shared/client scripts in dependency order.
Gameplay admission waits for required resources to be ready. A server-only
entry point is never a client entry point or downloadable script.

Script-only changes can be reloaded with `restart`. Manifest changes require
`stop` followed by `start`, so the installed manifest is rediscovered and exact
dependency versions are checked. Published bytes for a running generation are
frozen; an unrelated lifecycle command does not silently replace them.

Server persistence is scoped by the canonical configuration-file path. Two
configuration files sharing a storage directory remain isolated. Moving a
configuration changes its namespace; keep its canonical location stable when
persistent state should survive restarts.

Stopping/restarting a resource changes its live generation. Event/state traffic
must match that generation. A change to the server-selected set requires content
negotiation and activation again; incomplete or rejected content must not replace
the verified set. Disconnect unloads resource instances and their engine-owned
state while retaining reusable cached bytes.

Cancellation, interrupted transfer and verification failure leave incomplete
content unavailable for execution. Reconnection retries missing content; verified
unchanged blobs can be reused. Script startup failures produce resource-specific
diagnostics rather than admitting a partly initialized gameplay session.

The reliable channel has bounded small-event and chunked-message lanes. State
updates coalesce per key and share an ordered lane. Default limits are 60 events
per second, 64 pending messages and 16 KiB JSON values; negotiated administrator
budgets have hard ceilings. Queue/rate overload disconnects a slow peer instead
of silently losing required traffic. Small events can overtake chunked events.
See [current transfer and budget contract](platform-extension.md#bounded-transfer-and-configuration).

## Persistent cache and grants

Set `SKATE3_RESOURCE_CACHE` to choose a client cache root. Otherwise the client
uses `settings/resources` below the asset directory's parent. Content identities
are full BLAKE3 digests. Resource names, version labels, filesystem path spelling
and the legacy local-mod `u64` fingerprint are not cache identities.

Identical verified bytes can be reused after reconnects, full client restarts,
resource-version changes and connections to other servers. Changed files receive
new digests; unchanged files retain theirs. Two servers' same-named resources
cannot collide just because their labels match. Cached immutable bytes are shared;
Lua state, persisted data and permission decisions are scoped separately.

The client default grant policy permits requested `resource.*` capabilities and
`engine.map`, `engine.ui`, `engine.audio`, `engine.graphics`, `engine.inspect` and
`engine.voice`. Voice device use still requires the client's `--voice` opt-in. Sensitive
player, physics, camera, input, world and animation grants require explicit
source-specific additions in `grants.json` inside the cache root. For example:

```json
{
  "udp://127.0.0.1:31030/48031030": {
    "my-camera-resource": ["engine.camera"]
  }
}
```

Use the exact source namespace shown by the current connection/inventory,
including endpoint and session number. The example illustrates the file shape;
the source key is not a wildcard and the resource must also request the grant.
Do not grant engine control merely to clear an error; inspect the resource and
its requested capabilities first. A new server source does not inherit another
source's decisions.

The inventory/history is a local audit trail: identity/version, exact digests,
source, requested capabilities and grants, verification, transfer size and
activation outcome. It helps explain what ran and why bytes were reused. It is
not proof against a modified client and must never be accepted by a server as
anti-cheat attestation.

Inspect or manage it with the bundled cache tool:

```sh
cargo run --locked -p skate-resources --bin skate-resource-cache -- inspect /path/to/cache
cargo run --locked -p skate-resources --bin skate-resource-cache -- prune /path/to/cache
cargo run --locked -p skate-resources --bin skate-resource-cache -- forget /path/to/cache 'udp://127.0.0.1:31030/48031030'
```

`inspect` prints disk usage and bounded JSON history. `prune` removes inactive
content toward zero bytes by default; supply a byte target as its last argument
to retain more. Active sets are pinned. `forget` releases one source's pin; it
does not run Lua or change its permission policy. Disconnect/clean shutdown
normally releases the pin. After a crash, use `forget` for an abandoned pin.
`fetch CACHE IP:PORT REVISION SOURCE` verifies/materializes content without
executing scripts; it is useful for diagnosing distribution independently.

The cache stores `blobs/<digest>`, `sets/<revision>/<resource-id>`, source pins
under `active/`, and `inventory.json`. The default bounds are 64 MiB per file,
256 MiB per set, 128 resources, 16384 total files, 1 GiB of cache storage and 512
history entries with an 8 MiB audit cap. Disk accounting includes materialized
copies and staging. An operating-system lock coordinates cache writers. A
malformed manifest without usable resource identity is rejected before an
inventory entry can be recorded; valid-manifest transfer failures are recorded.

HTTP requests use an exact socket endpoint, with no redirects, proxy discovery,
compression or chunked transfer. The service has four workers, bounded headers
and transfer deadlines. A client runs at most one download worker, including a
cancelled request awaiting its bounded socket return.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| UDP reaches the server but content cannot download | The selected TCP port must also be reachable; inspect the content listener's address and error. |
| Resource rejected for an unavailable capability | Verify engine/API version, exact manifest spelling and side; inspect the source's grants. |
| Missing or incompatible dependency | Install the requested resource ID and exact version below the configured root; remove dependency cycles. |
| Entry point or public file rejected | Use explicit canonical lowercase relative paths; do not overlap server and public scripts or list a path twice. |
| Digest/size/truncated-download error | Inspect the content source and cache; retry the transfer after correcting bytes. Do not execute an incomplete download. |
| Resource script fails at load/update | Read the resource-specific diagnostic; check callback arguments, permissions, payload sizes and budgets. |
| A camera, input action or UI remains after stop | This is a cleanup defect; capture resource ID/generation and report it. Script cleanup should not be required to restore ownership. |
| Challenge shows no points | Enroll, land a newly settled trick and allow the one-second scoring interval. The example reads `landed_seq`, not trick announcements or a client-supplied score. |
| A FiveM resource does not load | Port its manifest, runtime/API calls and GTA-specific behavior. See the compatibility matrix. |

See [resource authoring](../../sdk/RESOURCES.md), the
[redistributable examples](../../resources/README.md), and the
[Cfx compatibility/API audit](resource-compatibility.md). For transport setup and
the limits of Hybrid Authority, see [dedicated multiplayer](README.md).

Explicit bulk transfer keys, progress, deadlines and cancellation are documented in [Large resource messages](large-messages.md). Browser inventory and voice examples are described in [Browser interfaces](browser-interfaces.md) and [Voice](voice.md).

## Live map resources

Grant `resource.map` to publish server-owned settings and vector layers through the existing state transport. Client-local layers require `engine.map`. See [the map SDK](../../sdk/MAP.md) and the [Lua server example](../../resources/programmable-map/).
