# Remaining dedicated platform capabilities

Starting branch `cross-platform-dedicated-server`, clean HEAD
`3e9f3ac08b16871785976f2da12b80b47118f2bd`, ancestry from `68236c9` and `69ed377`
verified on 2026-10-05. The user authorizes implementation, reversible local
experiments and logical tested local commits. No push, publication or deployment.

## Design and dependency order

1. **Contracts and baseline.** Read the existing platform/evidence/SDK guides,
   query Graphify then verify source. Keep native-input-v1 solitary static-world
   restrictions, course-v1 raw motion/swept collision policy, private identity,
   generation cleanup and direct/peer compatibility unchanged. Record fresh
   evidence separately from historical results.
2. **Settings and profiling foundations** (`profile_settings` owns
   `skate-resources`, `skate-mods`, SDK declarations and focused documentation).
   Add bounded typed manifest settings and engine-feature requirements. Strip
   private schemas/defaults from downloadable projections. Expose own-resource
   read APIs in Lua/JS/C#, host-only persistent validated updates, pending
   restart values and generation-checked notifications. Add bounded per-host
   labelled span history, percentiles and portable Chrome traces; distinguish
   inclusive wall/CPU, exclusive host CPU, worker measurements and wait time.
   Never capture payloads or setting values in profiling. Test types/bounds,
   projection, denied access, persistence and all available actual runtimes.
3. **Operations and admission** (`operations_admission` owns server except
   `resources.rs`, net, accounts, supervisor tool and focused documentation).
   An external local supervisor owns one child, a process lock, bounded restart
   window/backoff, game-loop heartbeat and deliberate-stop contract. Admin
   actions reuse live permission rechecks/audit. Maintenance closes new
   admission, warns existing players and retires resources predictably before
   exit. Backups explicitly use stopped-store semantics; restore validates and
   retains a recoverable previous store. Add bounded dedicated queue/policy,
   verified reservation identity, cancellation/expiry and discovery metadata;
   queued admission still passes all existing host checks. Add a side-effect-free
   resource configuration validation command for recipes.
4. **Integration surfaces** (parent owns `server/resources.rs` and game UI).
   Wire settings replication before activation and live updates; expose private
   settings only on authorized admin reads. Integrate profiling summaries and
   export with tick timing. Dedicated browser persists saved endpoints,
   favorites/recent history and shows untrusted bounded metadata/content preview,
   compatibility, queue/progress/rejections; Steam peer sessions remain intact.
5. **Reproducible packs and authoring** (`packs_editor` owns new Python tools,
   placement exporter/editor, tests, authored `creator-park`, focused docs).
   Version/digest-pinned recipes have reviewable plans, confined staged installs,
   bounded acquisition, no shell actions, history/recovery/rollback and a stable
   `server.json` path (the host persistence namespace includes this path).
   Data remains outside immutable versions and rollback never reverses schema
   migrations. The local visual editor provides selection/transforms/snapping,
   history, rail points/markers, save/load/export and trusted local playtest.
   Shared placement transforms govern rendering, collision and native rails;
   authoring does not mutate live server authority.
6. **Composable gameplay** (parent owns new gameplay resources, integration
   tests and final recipes). Parties/crews, round/session state, map voting and
   rotation, tournaments, account profiles and leaderboards use exact dependency
   exports, settings and scoped state. Stable account identity never comes from
   client payloads. Competitive persistent results only originate from host
   `competition_result`; tournaments use isolated native static-world attempts
   or the existing course verifier. Cover reconnect/late join/retirement,
   permission failures, instance changes and transaction recovery. Deliver
   free-skate, tournament and community-park recipes.
7. **Combined evidence and independent review.** Serialize Cargo with
   `/tmp/skate-platform-cargo.lock`; coordinate graphical/device/load runs.
   Exercise settings/restarts, full/draining queues, crash/recovery, interrupted
   pack upgrades/rollback, verified results/persistence, editor native two-client
   worlds and profiler overhead/bounds. Review independently for correctness and
   security boundaries, reproduce findings, fix and rerun affected checks.
   Run `graphify update .`, keep output ignored, inspect staged inventory and
   create local tested commits. Update the requirement-to-evidence ledger and
   report exact remaining functionality and prerequisites.

## Integration interfaces

- Settings manifest: `settings` map with `type`, `default`, numeric `min/max`,
  string `max_bytes`, enum `options`, `visibility` private/replicated/public,
  `change` live/restart; `requires_features:["resource.settings.v1"]`.
- Runtime read API: `resource.settings.get/all`; C# `SettingsGet/SettingsAll`.
  Host writes validate and persist before publishing; restart writes remain
  pending. Private values never become resource state/public metadata.
- Admission metadata and supervisor control formats are owned by the operations
  track and consumed by the parent; no client-facing setting can grant priority.
- Recipe installation preserves one canonical config pathname and separates
  immutable resource versions from mutable private data.

## Review focus

- Private schema/default/value leakage through manifests, trace, discovery or
  broad status; audit logs record keys/outcomes, not values.
- Queue fairness and capacity reservations cannot trust actor claims; async
  policy completions must match an unexpired connection incarnation.
- Supervisor deliberate shutdown, child reaping, restart exhaustion, live store
  locks, interrupted restore and account secrets.
- Recipe traversal/symlinks/archive expansion, digest/version drift and interrupted
  commit points; rollback's retained data must not imply reversed migrations.
- Forged score/local cosmetic events, duplicate completed results, disconnected
  verified identities, generation changes and bounded gameplay state.
- Editor geometry transforms and deterministic export; stale colliders/rails
  retire through ordinary world lifecycle.

## Progress

- [x] Clean branch/HEAD/ancestry verified; Graphify query works (existing version warning).
- [x] Dependency contracts and explicit ownership dispatched.
- [x] Settings/profiling runtime, server and client surfaces verified.
- [x] Operations/admission/browser verified.
- [x] Recipe/editor implementation and native acceptance verified.
- [x] Gameplay resources and complete packs verified.
- [x] Combined integration, independent review, graph update and local commit inventory.

Existing unavailable acceptance remains visible: exhaustive stock trick families,
shared dynamic native authority, longer native multi-worker/WAN runs, native
Windows/mixed OS, physical microphone/speakers and private Skyline GLB. Recheck
host/device availability; never replace these with synthetic acceptance.

## Implementation evidence

See [the capabilities ledger](../../multiplayer/platform-capabilities-evidence.md)
for actual commands, measured profiling overhead, independent review fixes,
complete recipe recovery tests and Linux native integration. New work remains
subject to the explicit external and broader native acceptance gaps above.
