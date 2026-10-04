# Resource platform extension implementation plan

> Agent execution: use subagent-driven development with explicit file ownership;
> continue in the existing checkout. The user authorizes implementation without
> additional design approval. Local commits were subsequently authorized by the
> user on 2026-10-04. Do not push, publish or deploy.

**Goal:** Extend the resource platform through the eleven capability areas in
the approved request, recording acceptance evidence separately from implementation.

**Architecture:** Retain transport-neutral Rust contracts and the current
resource lifecycle. Server authority owns identities, instances and shared
objects. Resource grants gate host operations; every asynchronous operation and
replica carries resource generation and connection/instance epoch. Movement stays
on its bounded priority path; bulk data and backend I/O have independent budgets.

**Tech stack:** Existing Rust/Bevy, serde, Lua/mlua and UDP foundation; select
additional runtime, browser, audio and database libraries only after inspecting
their containment, maintenance, licensing and platform requirements.

**Spec:** User's approved request is preserved in
`2026-10-04-platform-extension-request.txt`. The previous resource platform plan
describes the baseline; its deferral of JavaScript/C# is superseded.

## Constraints and review focus

- Branch `cross-platform-dedicated-server`, baseline
  `69ed3779342bd61ecff7f8d40628269152a79afc`; initially clean and exactly at baseline.
- Preserve unrelated work and local/peer behavior; direct UDP requires no Steam.
- Parent coordinates Cargo runs; agents do not share concurrent target builds.
- No paid service, credential or private fixture is needed for local examples.
- Reject stale generations/epochs after restart, disconnect and instance change.
- Bound allocations before decoding and queues before enqueueing; report overload.
- Distinguish verified simulation from client-reported presentation and scores.
- Native Windows, graphical and real audio acceptance require actual execution.

## Dependency-ordered work

### 1. Movement reset and instance contracts

Files: `crates/skate-net/src/dedicated.rs`, `lobby.rs`, dedicated tests;
`crates/skate-game/src/multiplayer/` and physics network integration.

- [x] Add server-approved teleport with validated destination and a reset epoch.
- [x] Test immediate reset, duplicate/lost/reordered messages, forged requests,
  stale input and swept-collision preservation; integrate owner and observers.
- [x] Add instance membership, scoped replication/state/events and collider cleanup.

### 2. Bounded capacity and transfer

Files: `crates/skate-net/src/resources.rs`, new bounded transfer module;
`crates/skate-resources/`, server configuration and workload tests.

- [x] Negotiate validated budgets; expand capacity without unbounded queues.
- [x] Implement chunking, flow control, cancellation and errors separate from movement.
- [x] Exercise at least 64 real socket simulated peers with resource/shared traffic;
  report traffic, memory, latency and loss using measured evidence.

### 3. Runtime portability and storage

Files: `crates/skate-mods/src/resources.rs`, resource compatibility Lua, runtime
tests and SDK declarations.

- [x] Add cooperative Lua threads/waits, vectors, timers/events/exports subset,
  preserving callback and coroutine execution budgets.
- [x] Configure bounded storage/value/key/execution limits; test data above old caps.
- [x] Implement actual JavaScript and isolated managed C# through shared host
  contracts; test cross-language exports and failures before claiming support.

### 4. Services, identity and administration

Files: new `crates/skate-services/`, integration in server resource host, service
tests and examples. Root coordinates workspace/lockfile edits.

- [x] Implement asynchronous bounded local SQLite transactions/migrations and
  HTTP requests with deadline, cancellation and result caps.
- [x] Connect capability/generation-scoped APIs and progression example to resources.
- [x] Add protected account authentication, persistent roles/inheritance,
  revocation, moderation and audited administration tied to real server state.

### 5. Shared entities and gameplay verification

Files: transport-neutral world contract, server simulation, client world adapter.

- [x] Stable resource/generation entity IDs, ownership, interest and late snapshots.
- [x] Shared dynamic collision, interpolation/reconciliation and disconnect recovery.
- [x] Independently validated competition outcomes; document exact physics/trick
  limits and keep unverifiable outcomes open.

### 6. Browser and voice

Files: platform-specific browser/audio adapters, resource host APIs, client runtime.

- [x] Local packaged browser assets, messaging, permission policy, input ownership
  and lifecycle; actual Linux integration.
- [ ] Execute native Windows browser integration (requires Windows host).
- [x] Encoded capture/transport/playback, proximity/radio, devices/mute/deafen,
  instance filtering and cleanup; deterministic audio through actual ALSA devices.
- [ ] Physical microphone/speaker and native Windows audio validation (devices/host unavailable).

### 7. Parks, authoring, animation and reusable resources

Files: data import, game mounting/streaming, native rail and animation adapters,
resource examples and SDK docs.

- [x] Resource map stages: download, validate, mount visual/collision/rails, admit.
- [x] Native rail authoring, prop/collider/marker placement save/load/export.
- [x] Rig-validated animation import/blends/events, presentation replication and
  owned versioned gameplay/scoring hooks.
- [x] Reusable examples covering every approved gameplay scenario.

### 8. Integration, documentation and review

- [x] Fresh baseline guarantee regression suites plus new socket/process/filesystem
  tests; preserve private skyline prerequisite.
- [x] Windows/Linux packaging and CI definitions updated; independent review
  findings fixed and locally verified. Hosted CI execution remains unavailable.
- [x] Graphify AST update; current evidence matrix and exact continuation record
  saved in the ledger. Native platform and scoring limits below remain open.

## Decisions

- Extend existing host/lifecycle rather than introduce a replacement platform.
- Use server-issued discontinuity epochs, not elapsed-time heuristics, for
  authorized teleports; retain swept guards for ordinary movement.
- In-process managed-language runtimes are not assumed to provide containment.
  Downloaded arbitrary native libraries remain excluded.

## Continuation architecture decisions

- Shared entities continue in the dedicated authority and native dynamics adapters;
  the movement agent owns those files, with explicit root-owned resource API integration.
- Account services must bind verified identities to protected transport admission;
  a persistent database label is not authentication. The backend agent owns the
  new account service and proposes the admission contract before integration.
- C# uses a separate bounded worker process and OS isolation, not an in-process
  managed runtime security claim. Linux user namespaces are available locally;
  Windows execution still requires a native verification environment.
- Browser UI uses the maintained Wry webview abstraction (WebKitGTK on Linux and
  WebView2 on Windows) in a companion process with bounded IPC, per-resource
  ownership, packaged local assets and explicit navigation/network policy.
  Keep engine canvas/menu APIs. Verify Linux graphically; do not infer Windows
  runtime behavior from cross-compilation.

## Acceptance limits retained

The current [evidence ledger](../../multiplayer/platform-extension-evidence.md)
is authoritative for results. Native Windows and mixed-platform execution have
not occurred; cross-compilation and CI definitions do not close those checks.
The private skyline fixture remains unavailable and its tests are unchanged.
Full server-derived articulated stock trick/landing/combo scoring is still an
open requirement; implemented course-v1 verifies its explicitly narrower rules.
An additional ignored native marker reset test remains unresolved, with identical
failures under the old and new teleport method in a controlled comparison.
