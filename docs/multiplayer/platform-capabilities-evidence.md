# Dedicated platform capabilities: implementation and evidence

This ledger covers the 2026-10-05 implementation after clean branch
`cross-platform-dedicated-server` at
`3e9f3ac08b16871785976f2da12b80b47118f2bd`. Ancestry from `68236c9` and `69ed377`
was verified before editing. Earlier native/soak evidence remains in
[platform-extension-evidence.md](platform-extension-evidence.md); it is not
counted as fresh evidence here. All commands ran locally on Linux. Nothing was
pushed, published or deployed. Logs, captures, accounts and disposable stores
are outside Git under `/tmp/skate-platform-20261005/` unless stated otherwise.

## Requirement-to-evidence matrix

“Implemented / verified” describes the tested contracts below, not universal
platform or production acceptance. External and broader acceptance remains open.

| Requirement | Status | Implementation and fresh evidence | Limits / continuation |
| --- | --- | --- | --- |
| Resource profiling/history | Implemented / verified | Resource/generation/source-labelled callback, event, export and managed IPC spans; bounded history and percentile tables in authenticated admin; portable Chrome export; Lua, QuickJS and actual C# tests and overhead measurements below | Windowed instrumentation, not native stack sampling. Worker CPU is separate; inclusive spans must not be summed. |
| Typed settings | Implemented / verified | Typed schemas, defaults/bounds, private/replicated/public projections, durable operator changes, live notifications and pending restart values, feature constraints, Lua/JS/C# access, admin schema editor | Secrets belong in private operator configuration, never recipes. Startup waits for replicated settings before client callbacks. |
| Supervision/maintenance | Implemented / verified on Linux | External process owner, bounded crash/hang recovery, game heartbeat, drain warnings, deliberate-stop intent, status/history, authenticated controls; real supervisor/SQLite restart integration and destructive disposable-store tests | Windows containment implemented but requires native Windows acceptance. Snapshot contract is stopped-store copy, not live backup or power-loss proof. |
| Dedicated browser/admission | Implemented / verified on Linux | Saved endpoints/favorites/recent joins, untrusted bounded metadata/content previews, freshness/compatibility checks, actual native keyboard joining, bounded queue/cancel/expiry/capacity/reservations and asynchronous live-account permission policy | Numeric IPv4 saved/direct endpoints; optional endpoint query, no public directory deployed. Anonymous bounds are not Sybil protection. |
| Reproducible packs | Implemented / verified | Three complete pinned recipes; plan/apply/verify/history/recover/rollback, trusted engine validation, bounded acquisition and confinement; actual cold/warm startup, upgrade/interrupted publication/rollback | Rollback changes code/configuration, not data migrations. Directly launched servers must be stopped before installation changes. |
| Visual authoring | Implemented / verified on Linux | Browser selection/transforms/snapping/history, rail/marker editing, deterministic save/export/reload, original Creator Courtyard; actual native Playtest/Stop and two-client authored-world use | Editor owns local drafts and disposable playtests. Dedicated world replacement uses ordinary authoritative admission/cleanup. |
| Reusable gameplay | Implemented / verified | Six replaceable resources: verified-account profiles, crews, rounds, voting/rotation, tournaments, verified leaderboards; real UDP/course-verifier and SQLite recovery/lifecycle tests | Tournament enrollment/in-flight bracket is ephemeral; committed results persist. Native rounds remain solitary/static. World changes end leased private attempts and require reconnect. |
| Combined integration/review | Implemented / verified | Settings/restart, world rotation, actual browser join, profiling export, pack recovery, supervisor recovery, source review and reproduced trust-boundary regressions | Final native run, HUD correction, Graphify update and inspected commit inventory are recorded below. |
| Exhaustive native trick families / shared dynamic authority | Incomplete | Existing bounded native-input-v1 and course-v1 guards preserved | Requires additional native solver/authority work and trick-family acceptance; do not remove guards. |
| Longer native multi-worker / multi-host WAN | Incomplete | Historical 120-second 64-owner impaired UDP soak remains separately documented | Run longer native worker workloads and real independently hosted peers with recorded network conditions. |
| Native Windows / mixed Windows–Linux | Blocked | Only a usable Linux host was available | Supply a native Windows host, owned assets and matching binaries; exercise process Job cleanup, managed runtime, joining and resource worlds. Cross-compilation is insufficient. |
| Physical microphone/speaker | Blocked | `/dev/snd` exists but devices are inaccessible to this session | Provide accessible physical capture/playback devices. Synthetic/encoded voice tests do not satisfy this. |
| Private Skyline integration | Blocked | `sdk/examples/skyline/skyline.glb` absent | Provide that privately owned fixture outside commits; rerun its documented native integration. |
| Workspace formatting | Existing differences | Scoped formatting and diff whitespace checks used | Do not fold a workspace-wide formatting rewrite into this change. |

## Run and use

Build trusted local binaries:

```sh
cargo build --locked -p skate-game -p skate-server -p skate-accounts --bin skate3rust --bin skate-server --bin skate-account
```

Follow
[server-packs.md](server-packs.md) to review and install one of:

- `resources/packs/free-skate.recipe.json`: anonymous local practice, crews,
  rounds, map voting and both redistributable parks; persistent verified profile
  features become available when an account authority is configured.
- `resources/packs/tournament.recipe.json`: account-required tournament/course
  workflow with all six gameplay resources and both parks.
- `resources/packs/community-park.recipe.json`: account-required community
  free-skate with all six resources, profiles and optional verified competition.

Recipes contain exact file digests/lengths, versions and provenance, without
credentials, databases or retail assets. Review the printed `plan`, use `apply`
with `--source-root resources` and the absolute trusted server executable, then
`verify`. Account-required packs use `init-accounts`; keep the password on stdin
and private files outside Git. Start the stable installed `server.json`, never a
configuration inside an immutable version directory.

The game entry point is **Multiplayer → Dedicated servers**: enter endpoint and
optional local account-profile path, Save, Refresh, Join; Cancel join cancels
pending login and queue work. See [dedicated-discovery.md](dedicated-discovery.md).
The existing HTTPS admin now includes **Maintenance, backups, settings and
profiles**. Typed controls disclose live/restart behavior; private reads and
trace actions retain their own live permissions. See
[server-operations.md](server-operations.md),
[resource-settings.md](resource-settings.md) and
[resource-profiling.md](resource-profiling.md).

Run Park Studio with:

```sh
python3 tools/park_editor.py --workspace resources --scene creator-park/placements.json --open
```

Add `--game-executable` and `--assets` with
trusted absolute local paths for native Playtest. See
[visual-authoring.md](visual-authoring.md) and
[gameplay-platform.md](gameplay-platform.md) for controls, exports, settings,
authority and persistence contracts.

## Focused checks and measurements

All Cargo commands sharing `target/` were serialized with
`flock /tmp/skate-platform-cargo.lock`. Initial failures and reproduction logs
were retained; assertions were not weakened to obtain passing results.

| Command / workload | Actual result | Local evidence |
| --- | --- | --- |
| `cargo test --locked -p skate-resources --test contracts --test distribution` | 9 contracts and 16 distribution tests passed, including private schema exclusion and engine-feature validation | `settings-profile-suites.log`, `settings-distribution.log` |
| `cargo test --locked -p skate-mods --test resource_runtime` | 56 passed, 2 explicitly ignored measurement/specialized cases | `js-empty-players-runtime-suite.log` |
| `cargo test --locked -p skate-mods --doc` with the profiling guard compile-fail case | Non-Send thread-affinity regression passed after independently reproduced failure | `profile-thread-red.log`, `profile-thread-green.log` |
| `cargo test --locked -p skate-server --test platform_settings -- --test-threads=1` | 1 real UDP integration passed: settings before callbacks, private omission, live/restart persistence, bounded trace, no dry-run filesystem side effects | `review-platform-settings.log` |
| `cargo test --locked -p skate-server --test gameplay_resources -- --test-threads=1` | 14 passed: actual Lua/JS resources, spoof denial, reconnect, permissions, map/instance lifecycle, real course result, actual SQLite outbox replay/commit-before-ack, resource stop/restart teleport cleanup | `leaderboard-category-full.log` |
| Server teleport lease unit/API tests | 5 lease tests and 1 API boundary test passed; real UDP immediate stop/restart before content readiness also passed | `teleport-leases-final.log`, `teleport-api-final.log`, `readmission-net-final.log` |
| `cargo test --locked -p skate-server --test accounts --test operations --test udp --test voice -- --test-threads=1` | 2 account, 1 real external-supervisor, 4 UDP and 1 voice integration passed; final UDP selection passed 5 including terminal incompatible-physics rejection | `operations-integration-final.log` |
| `cargo test --locked -p skate-server --lib operations::tests` | 6 passed: capacity/queue/policy/reservation/drain and active-world metadata | `operations-unit-final.log` |
| `python3 -m unittest tools.test_server_supervisor -v` | 18 passed after startup, shutdown, restore and cross-restart history/log-bound regressions | `supervisor-final.log` |
| `node tools/test_admin_ui.cjs` | Admin DOM contracts passed: typed/private controls, profile tables/export and logout response invalidation | `admin-ui-final.log` |
| Combined pack/editor/park/supervisor Python selection | Final combined selection passed 45 tests (18 supervisor, 27 park/editor/packs) | `python-final.log` |
| `cargo test --locked -p skate-game --bin skate3rust` | 369 passed, 107 existing ignored native/private/specialized cases; two owned-asset native authority tests separately passed | `game-hud-final.log`, `native-physics-final.log` |

Actual managed runtime prerequisites were available: `.NET` at
`/tmp/skate-csharp-sdk/dotnet/dotnet`, fresh worker at
`/tmp/skate-managed-host-platform-settings`. With `SKATE_DOTNET_ROOT` and
`SKATE_MANAGED_HOST` set to their parent/runtime directories,
`cargo test --locked -p skate-mods --test managed_runtime csharp_ -- --ignored
--nocapture --test-threads=1` passed six actual worker tests. They include
Lua/JS/C# exports/lifecycle/persistence, network/service capability boundaries,
reflection/process/I/O denial, runaway/memory cleanup, typed settings and worker
CPU/IPC measurements. Log: `settings-profile-managed-final.log`.

The alternating disabled/enabled profiling workload used four Lua and four
QuickJS resources, 240,000 callbacks, three paired runs after warmup. Median
dispatch: **138,497 ns disabled / 146,189 ns enabled**, incremental **961 ns per
resource**, ratio **1.056**. Actual C# IPC workload: **62,483 / 66,994 ns**, added
**4,512 ns**, ratio **1.072**. Both retained exactly **4096 spans**. Commands are
in [resource-profiling.md](resource-profiling.md); logs are
`settings-profile-overhead.log` and `settings-profile-managed-final.log`.
These are local optimized-test-profile measurements, not a universal overhead
guarantee. Span retention defaults to 60 seconds/4096 entries and is capped at
300 seconds/8192 entries. CPU categories and wait attribution are explicitly
separate. History read and trace export report size-limit omissions.

## Integrated installations and native evidence

All three recipes were installed cold and warm with the actual engine validator,
started with their complete ensured resource sets, upgraded to changed pinned
configuration, and rolled back while retaining a mutable-data sentinel. The two
account-required packs used actual `skate-account` initialization and private
SQLite authority stores. An injected interruption after configuration publication
was recovered using the real validator and receipt verification. All passed;
`packs-final.log`, disposable installations `packs-final/`, and the exact local
driver `verify-packs.py` retain commands and assertions. Repeatable fault cases
are committed in `tools/test_server_pack.py`.

Browser editing selected/rotated a ramp, undid/redid it, extended a rail, saved
and exported a **21,064-byte** edited park. Actual native Vulkan Playtest loaded
**112 render / 88 collision triangles**, one native spline/four primitives and
reported ground contacts. Stop ended both the game and crash-reporter processes
and removed the temporary export. The base checked-in Creator Courtyard is
**18,796 bytes**, with 100 render / 76 collision triangles and three native rail
primitives. These are original redistributable geometry, not private retail data.
The final editor cleanup evidence is `editor-native-final.log` and
`editor-native-processes.json`: no surviving native/guard PIDs and no temporary
export after Stop. An independently reproduced orphan-descendant regression now
uses the supervisor's gated process containment; output rotates at 1 MiB and
the UI reads a bounded 4 KiB diagnostic tail.

`tools/verify_platform_capabilities.py` drives real X11 keyboard events through
the native dedicated browser while a second client uses direct UDP. It checks
both players in Creator Courtyard, changes live and pending-restart settings,
restarts their resources, votes/rotates to Community Practice Park, confirms
replacement collision/rail counts and both admissions, exports a bounded trace
and checks saved recent-server history. Run with prepared owned assets and an
empty external output directory. Use a dedicated X11 display when the desktop
compositor blocks synthetic input; this run used isolated Xvfb `:194` at
1280×800. This is actual native application/input acceptance, not physical
controller or audio acceptance. Final `native7/results.json` reports `ok:true`, 1536 trace events, both
clients admitted before and after rotation, and the expected 76/3 → 32/1
collision-triangle/native-primitive replacement. Screenshots were inspected.
An earlier screenshot exposed a multiline overlay spacing defect; two ECS
regressions reproduced it, and the final screenshot confirms separated resource
rows after the native layout fix. Functional assertions remained enabled throughout.

Initial combined runs exposed and fixed real defects: empty JavaScript player
arrays, an empty Lua menu array, stale asynchronous login completion, and customiser
navigation reading the previous frame's input before the menu read current
input. The scheduling fix orders navigation after Bevy input collection. A
later run completed all gameplay phases but correctly failed its final check
because intentional world retirement was reported as a resource failure; the
host now consumes those intentional retirement records before publishing.

## Independent review and trust boundaries

Separate agents reviewed one another's source and integration contracts, with
explicit ownership and no concurrent writes to the same files. Validated
findings were reproduced, fixed, then rechecked:

- Supervisor: same-data-root duplicate ownership, process containment startup
  race, descendant cleanup, shutdown intent surviving a crash during drain,
  restore staging cleanup, and history/log retention across restarts.
- Installation: FIFO/device substitution could block local reads; descriptor
  checks plus nonblocking/no-follow opens close the reproduced path. Account
  bootstrap now places private authority stores under the supervised data root.
- Privacy and asynchronous state: private action results require their original
  permission even for auditors; logout discards stale responses; client cancel
  discards late login, roster and queue replies; policy completion binds to the
  original queue ticket; profiling guards cannot move to another thread.
- Gameplay: standings now retain top 10 per ruleset and display five per category,
  after real SQLite and HUD regressions reproduced native results disappearing
  behind course results; newest result-cache entries survive decimal ID rollover; profile
  visits retry after a saturated persistence queue; tournament cancellation and
  generation retirement restore temporary travel through content readmission,
  with bounded timeout and fail-closed cleanup. Competition publication remains
  a host-local event; client scores/cosmetic events cannot earn persistent ranks.

Review does not establish absence of every possible vulnerability. Course-v1
motion/swept collision, BODY.root's bounded 0.20 m terrain-reference policy and
native-input-v1 epochs/history/reconciliation/solitary-static admission remain
their documented contracts. No shared dynamic native authority is implied.

## Final broad checks and load

`cargo test --locked -p skate-net -p skate-resources -p skate-services
-p skate-accounts -p skate-server -- --test-threads=1` passed **254 parent tests**
plus three successful subprocess helper summaries, with six default ignored
cases. The two actual native-worker cases were then run explicitly with
`SKATE_NATIVE_EXE` and `SKATE3_ASSET_ROOT`: **2 passed in 27.54 seconds**, including
forged score rejection, epoch retirement and delayed/lossy full-length input.
The two owned-asset game `physics::native_authority` tests also passed, exercising
native grab/grind/landing/combo publications and deterministic solver replay.
Logs: `core-final.log`, `native-worker-final.log`, `native-physics-final.log`.
This does not cover every native trick family or shared dynamic competition.

The final mods selection passed **59 library tests and 56 runtime tests**, with
four default ignored cases and two successful runtime containment subprocesses;
the profiling compile-fail doctest passed separately. The six actual C# cases
and explicit profiling measurement were run as described above.
Logs: `mods-matching-final.log`, `mods-doc-final.log`.
`cargo check --workspace --locked --all-targets` passed (`workspace-final.log`).
The final game default suite passed **369 tests**, including both new native
overlay layout regressions (`game-hud-final.log`); 107 specialized/native/private
cases remain ignored by default and are not counted as passing.

A fresh standalone **120-second / 64-owner impaired UDP soak** passed after all
admission/profiling changes: 122.155 seconds including drain, 7,450 resource
echoes (115–118 per owner), resource p95 2,189 / max 2,678 ms, 873,459 movement
samples p95 254 ms, observer p95 1,005 / max 1,965 ms and 1,987,635 encoded voice
receives p95 192 ms. It forwarded TX 241,128,845 B / RX 915,714,703 B, with
41,132 seeded drops, 53,325 sparse drops and 3,062,897 reordered deliveries.
Largest delayed link queue: 104 packets / 31,947 B. Post-warmup combined RSS
593,588 → 627,168 KiB (33,580 KiB growth), within the unchanged test bound; BODY
history reached its 258,048-entry capacity. Sampled resource output queues stayed
empty. Voice router: 33,554 accepted, 2,672 rejected, 45,811 queued recipient
drops. Exact command:

```sh
SKATE_SOAK_SECONDS=120 cargo test --locked -p skate-server --test capacity sustained_sixty_four_client_combined_traffic_soak -- --ignored --nocapture --test-threads=1
```

Log: `capacity-soak-final.log`. This is loopback impairment with simulated owners,
not native multi-worker, real WAN or physical audio acceptance.

## Final closeout

The matching native client was rebuilt after the HUD correction
(`build-hud-final.log`), and the complete two-client keyboard/browser/settings/
restart/world-rotation scenario passed again (`native7/results.json`, `ok:true`).
Its trace contains 1536 exported events under the 500 KiB export budget, with
explicit omission counts. Both clients exited successfully. The final screenshot
`native7/client0.phase05.png` was inspected: resource overlays occupy separate
rows and both native players use the replacement world. Editor Stop cleanup was
independently verified earlier; test displays and local test processes exited.

The final AST-only `graphify update .` succeeded: **26,600 nodes, 57,961 edges,
1,286 communities** (`graphify-final.log`). Nonfatal warnings remain for the
installed skill/package version mismatch and 32 zero-node inputs. Community
names derived from changed hubs are local generated metadata; no paid semantic
labeling was requested. `graphify-out/` remains ignored and was not staged.

The staged inventory was inspected: **114 source/test/documentation/resource
paths**, with the original 18,796-byte `creator-park/park.skate` as its only
binary. Private assets, credentials, databases, captures and runtime logs are
outside the patch. Credential-marker inspection found no candidate key material.
New Rust files were scoped-formatted; `git diff --cached --check` and all 71 local
Markdown file targets passed. Existing workspace-wide formatting differences
were not rewritten. The implementation is committed locally on the original
branch; the final response records its hash and post-commit status.

Remaining work is the explicit incomplete/blocked acceptance in the matrix:
exhaustive native trick families, shared dynamic native authority, longer native
multi-worker and multi-host/WAN operation, native Windows/mixed-OS, physical audio
and the private Skyline fixture. Follow
[native-acceptance.md](native-acceptance.md),
[production-validation.md](production-validation.md) and the
[prior continuation plan](../superpowers/plans/2026-10-04-platform-gaps.md#precise-continuation)
after their concrete prerequisites become available. There is no claim of full
platform acceptance or production readiness. Nothing was pushed, published or
deployed.
