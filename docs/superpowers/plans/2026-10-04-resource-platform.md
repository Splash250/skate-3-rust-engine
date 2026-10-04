# Resource Platform Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development; work in place with explicit ownership. No commits.

**Goal:** Implement server-selected Lua resources, persistent verified distribution, isolated lifecycle and server-authorized resource networking in the existing dedicated engine.

**Architecture:** A new transport-independent `skate-resources` crate owns canonical contracts, immutable content, bounded HTTP distribution and inventory. `skate-mods` hosts resource Lua using the existing validated engine adapter. Dedicated admission advertises the required set, gates gameplay until activation, and routes bounded resource records with connection-derived identity and generation checks.

**Tech Stack:** Rust, existing serde/serde_json/BLAKE3/mlua, standard TCP HTTP and UDP; no Steam requirement and no native plugin downloads.

**Spec:** [Approved request](2026-10-04-resource-platform-request.txt).

## Global constraints

- Preserve existing dirty/untracked work; work in this checkout; no commit/push/publication/deployment.
- Lua first; language-neutral manifest/transport boundary; JavaScript/C# are unsupported runtimes.
- Client publication excludes server scripts and configuration; full digests identify bytes.
- Root coordinates Cargo runs. Contributors must request a build slot before Cargo.
- Graphify guides navigation; verify source and update graph after editing.
- Server authority is permanent; local mod behavior and legacy networking remain compatible.

## Review focus

- Canonical path case, reserved names, traversal, symlinks and cross-platform identity.
- Interrupted/corrupt transfer and failed activation cannot publish partial sets.
- Generation/session confusion, resource impersonation, replay and event flooding.
- Startup exceptions and teardown must release all engine-owned state even without callbacks.
- Cache reuse across servers must not share permissions or persistence.

## 1. Contract, content, cache and HTTP (content agent)

Files: new `crates/skate-resources/` except `src/protocol.rs`; root owns workspace registration.

Interfaces: `Manifest` (format/api/id/version/language/client_scripts/server_scripts/shared_scripts/files/dependencies/exports/capabilities), `Resource`, `ResourceSet`, `Cache`, `HttpServer`, `download_set`. Export contracts from crate root; coordinate precise signatures before integration.

- [x] Add contract/path/dependency/compatibility tests, then implement strict validation and deterministic set identity.
- [x] Add filesystem/socket cache tests for cold/warm/restart/cross-server/update/corruption/cancellation/limits, then implement immutable content and atomic staged publication.
- [x] Implement HTTP content allowlist, bounded workers/timeouts/transfer sizes and persistent bounded audit with inspect/prune CLI.
- [x] Verify server-only exclusion and same-name/different-content behavior.

## 2. Lua resources and engine adapter (runtime agent)

Files: `crates/skate-mods/src/resource*`, `vm.rs`, `lib.rs`, crate dependency and focused tests.

Interfaces: side-aware resource host consumes manifests + materialized roots, exposes lifecycle, incoming network event/state, outgoing resource operations, engine `Command` drain and retired owner drain. Client host integrates with existing Manager dispatch/ownership so existing rendering/physics APIs remain usable.

- [x] Tests first for dependency lifecycle, failure containment, exports/local events/timers and stale generations.
- [x] Add server VM capabilities, commands/permissions, isolated bounded persistence and server-owned replicated state.
- [x] Add client integration hooks with explicit grants and generic command validation; preserve legacy mod scan/runtime.
- [x] Verify cleanup and side separation through runtime tests.

## 3. Dedicated transport and real server (root)

Files: `skate-net` dedicated/lobby resource routing; `skate-server` configuration/runtime; `skate-resources/src/protocol.rs`.

- [x] Define bounded generation-tagged event/state envelopes and admission requirements; test unready/spoofed/stale senders before implementation.
- [x] Configure resource root, persistence/grants and lifecycle stdin commands; build server set and start HTTP independently of UDP.
- [x] Connect actual Lua runtime ticks, player observation, events, state, persistence and lifecycle set changes.
- [x] Keep UDP server without resources working and run existing suites.

## 4. Game activation and examples (root then integration agent)

Files: game `modding` and `multiplayer` integration; new `resources/` examples; resource SDK documentation.

- [x] Background bounded download and verified cache publication; cancellation on disconnect/update.
- [x] Dependency-ordered activation before readiness; teardown resources on disconnect and failure; gameplay gating.
- [x] Connect real engine observations/commands and server resource messages, without enabling legacy peer mod replication.
- [x] Ship challenge + dependency/export + custom UI/asset + persistence resources and explicit grants.

## 5. Evidence, documentation and review

Files: `docs/multiplayer/`, `sdk/`, `.github/workflows/networking.yml`, focused integration tests.

- [x] Research official Cfx docs and citizenfx source; record sourced compatibility matrix and actual API limits.
- [x] Exercise real HTTP/UDP/process/filesystem cold/warm/restart/cross-server/update and rejected-content paths.
- [x] Add Windows/Linux CI; check affected consumers/workspace and available Windows target.
- [x] Independent review; fix evidenced defects, update graph, report executed checks and unavailable graphical/native Windows checks.

## Decisions and execution ledger

- Initial ruling: use `resource.json` with explicit canonical paths and exact dependency versions, rather than execute manifests. This keeps inspection safe and identity portable; FiveM manifest syntax is a documented difference.
- Initial ruling: the built-in content endpoint is bounded HTTP on the server's TCP port (separate from UDP). Content digest verification gives integrity relative to the negotiated set, not authenticated server identity; TLS/authentication can be supplied by hosting infrastructure in a later transport adapter.
- Initial ruling: retain API-2 local mods unchanged; resource API-1 wraps the validated client engine SDK with per-resource grants and a separate server API.


## Completed execution evidence

- Added the contract/content/cache crate, real client/server Lua hosts, bounded
  generation-aware transport, game activation/retirement, packaged examples and
  authoring/operations/compatibility documentation.
- Completed independent implementation and review tracks in this checkout.
  Regressions cover review findings in queue fairness, immutable publication,
  manifest restart closure, native ownership, finalization and asset decoding.
- Fresh integrated validation: 108 network/content/server tests, 80 supported
  Lua/mod tests, 25 focused game tests and 35 Python tests passed. The Linux
  workspace check and graphical executable build passed. Real graphical cold,
  warm-process and live-stop checks succeeded.
- The network/content crates passed the Windows MSVC-target check. Server
  cross-checking is limited by missing Windows C headers/toolchain for Lua;
  native Windows execution remains a CI/platform check, not a claimed local pass.
  An existing private skyline GLB fixture also limits the optional full mod suite.
- Graphify's AST-only update succeeded; local changes remain uncommitted. See
  [the validation record](../../multiplayer/resource-validation.md) for exact
  commands, boundaries and process/filesystem/socket evidence.
