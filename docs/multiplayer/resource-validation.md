# Resource platform validation

Validation recorded on 2026-10-04 in the existing Linux checkout. The checks
below use synthetic/redistributable fixtures unless explicitly described as a
graphical run with the user's locally installed assets. No retail inputs are
included in the resource examples or patches.

## Executed checks

| Check | Result |
| --- | --- |
| `cargo test --locked -p skate-net -p skate-server -p skate-resources` | 108 tests passed: 73 network, 18 content/cache, 17 server. Includes real HTTP/UDP and separate server/client processes. |
| `cargo test --locked -p skate-mods --lib --test resource_runtime --test physics_api --test validate_example` | 80 tests passed: 57 library, 3 physics API, 19 resource runtime, 1 example validation. Two performance benchmarks are intentionally ignored. The ignored subprocess helper is executed by its passing parent regression. |
| Python setup, asset setup, Linux launcher, server packaging and Steam relay staging suites | 35 tests passed. |
| Game binary tests with `--features dev-dynamic`: `dedicated_resource`, `multiplayer::`, `physics::network::delta` | 25 tests passed: 10 resource/engine cleanup/asset checks, 13 multiplayer checks, 2 network physics delta checks. Two optional owned-import tests were ignored. |
| `cargo check --workspace --locked` | Passed on Linux. |
| `cargo build --locked -p skate-game --bin skate3rust --features dev-dynamic` | Passed on Linux; executable used for graphical checks. |
| `cargo check --locked -p skate-net -p skate-resources --target x86_64-pc-windows-msvc` | Passed. This is cross-compilation checking, not native Windows execution. |
| Windows-target check including `skate-server` | Blocked in vendored Lua's C compilation: no Windows `windows.h` / supported MSVC C toolchain on this Linux host. |
| `git diff --check`; `sh -n BUILD.sh PLAY.sh` | Passed. |
| `graphify update .` | AST-only update succeeded. The tool warned about its skill/package version mismatch and 13 configuration/source files yielding no graph nodes; no semantic API call or relabeling was requested. |

The Python command was:

```sh
python3 -m unittest tools.test_setup_linux tools.test_setup_refresh tools.test_setup_assets tools.test_linux_scripts tools.test_package_server tools.test_stage_steam_relay -v
```

The all-integration-test `skate-mods` run encountered the existing
`skyline_every_component_is_real_and_drives_through_ground_contact` prerequisite:
its private `skyline.glb` fixture is absent. The test was not weakened. CI runs
the explicit fixture-free suites above, including legacy local-mod coverage.

## Graphical integration

Three fresh Linux game processes connected to a real local dedicated server,
using the procedural test world, locally installed character/animation assets
and an isolated temporary cache/storage directory. All three exited with code
zero and `GAME_VERIFY_OK`; their logs contained no error, panic or resource
activation failure. Captures were inspected locally and were not added to the
repository.

- Cold client: downloaded 4,476 bytes and activated the downloaded challenge;
  the rendered LANDING CLUB HUD showed enrollment, the 0/3 target and the round.
- Fresh warm process: downloaded zero bytes and reused all 4,476 bytes.
- Live `stop landing-challenge`: negotiated a second set, downloaded zero bytes,
  reused the 691-byte dependency and removed the challenge HUD.

Each clean client exit also notified the server, which removed the player within
three seconds instead of waiting for the admission timeout. The post-run cache
had zero active source pins. All 14 inventory entries were
verified; seven recorded successful activation. This checks actual rendering,
activation and cleanup, but is not a live two-machine or mixed-OS gameplay test.
Native Windows execution and live mixed-OS Steam gameplay remain unverified.

## Behavioral evidence

| Required behavior | Exercised evidence |
| --- | --- |
| Cold download and execution | Real UDP server advertises a set; HTTP transfers it; a separate client Lua host runs the downloaded script, reads its declared UI asset and exchanges events/state with actual server Lua. |
| Warm reconnect and full process restart | Filesystem cache tests plus a real server process and two separately spawned client test processes; the second client downloads zero unchanged bytes. |
| Cross-server reuse, updates and same-name separation | Real HTTP endpoints reuse identical verified blobs across sources, fetch changed files only, and distinguish exact content despite identical labels. |
| Corrupt, truncated, interrupted or oversized content | Digest, length, compatibility, transfer-deadline and cache-budget rejection tests; incomplete materializations never become executable sets. Verified partial-download blobs survive retry. Corrupt set metadata is isolated and repaired without losing reusable blobs. |
| Server confidentiality | Client projection excludes server scripts; HTTP serves only selected digest blobs and rejects private/unknown paths. Private server configuration and persistence are never part of the set. |
| Dependencies and lifecycle | Missing versions, cycles, dependency exports, cascade stop/restart, initial startup rollback, incremental stopped-version upgrades and stale generation rejection. A changed dependent manifest rejects restart before any running generation or offer changes. |
| Stable live content | Editing an unrelated running resource does not publish those bytes when another resource stops. Its script-only restart publishes new bytes with a new generation. |
| Sender/authority boundaries | Gameplay gating before resource readiness; connection-derived sender, resource/generation validation, spoofed app-key and state-write rejection, payload/queue/rate limits, replay/ACK checks and fair state delivery under sustained events. Full 64-bit identities round-trip through Lua as decimal strings. |
| Permissions and private state | Source-specific client grants, denied nested/native operations, server/client API separation, per-source/resource storage, separate server configurations sharing a storage directory, and bounded inventory/history. |
| Cleanup and failure containment | Runtime callbacks/timers/exports/queued commands retire on failure, stop and disconnect. Native physics scopes cannot cross exports/deferred callbacks/unload. A bounded child-process regression rejects user finalizers that previously deadlocked shutdown. |
| Downloaded assets | GLB external URI, image/geometry expansion, malformed accessor/index, cyclic graph and texture amplification rejection; safe metadata introspection does not import files or images. |
| Portable identities | Golden full-digest manifest/resource/set fixtures run on Linux and are included in the Windows/Linux CI matrix. Native Windows execution has not been observed locally. |
| Existing behavior | Dedicated protocol, direct UDP and legacy peer/local-mod tests remain enabled; no Steam installation is needed for the resource server. |

The server subprocess helper is intentionally ignored in the ordinary test
enumeration and is invoked twice by the process-restart test. Those helper runs
are not counted twice as additional independent regressions.

## CI and review

The Windows/Linux networking workflow runs the shared protocol, server, content
cache and fixture-free Lua suites. Windows packaging includes only explicit
redistributable example files and Lua notices. The extracted executable is used
for both UDP package and resource-process tests through `SKATE_SERVER_EXE`.
Linux game CI also checks resource policy, asset validation and engine cleanup.
Workflow definitions were updated locally; no CI run was dispatched or observed.

Independent reviews covered content/cache isolation, native ownership scopes,
state delivery fairness, lifecycle publication and downloaded asset decoding.
Reproduced defects were fixed with regression coverage; the protocol/cache
re-review reported no remaining finding in its scoped source inspection. This
is bounded engineering validation, not a claim of exhaustive security proof.

## Product boundaries

Lua is the implemented resource language. JavaScript/C#, GTA natives, browser
NUI, native DLL/SO plugins and a dedicated Steam adapter are not implemented.
Manifests are data-only `resource.json`, with exact dependency versions and
canonical lowercase paths. Replicated resource state is server-owned. HTTP and
direct UDP do not add authenticated publisher identity, encryption or anti-cheat.
The example validates enrollment and rules but relies on owner-reported landing
observations. Engine limitations are detailed in the
[compatibility and API audit](resource-compatibility.md).

Existing uncommitted and untracked work was retained. No commit, push,
publication, deployment or external CI dispatch was performed.
