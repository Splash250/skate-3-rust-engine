# Dedicated Server Implementation Plan

**Goal:** Ship a headless dedicated server and a game client path for synchronized
skating, tricks, bodies, player collision and shove fighting.

**Architecture:** Extend the transport-neutral session with a dedicated policy
and a server authority layer. Reuse owner BODY/POSE streams; use acknowledged
server effect queues for shared interactions. A small UDP binary hosts the state
machine; game integration consumes effects in fixed simulation.

**Tech stack:** Rust/std UDP, existing skate-net codecs and serde, existing Bevy
client and recovered physics. No new third-party dependencies.

**Spec:** [dedicated server baseline](../specs/2026-10-04-dedicated-server.md).

## Constraints and review focus

- Preserve all existing local Linux/instruction changes and legacy network modes.
- Current checkout contains needed local work; implement in place with disjoint
  file ownership. Root coordinates Cargo runs to avoid target-directory locks.
- No commits, external deployment, resource downloads or plugin functionality.
- Review stale actor effects, loss between consecutive impacts, forged authority,
  duplicate local/server collision response and invalid packet resource growth.

## Tasks

- [x] Network: add dedicated Session constructors/handshake and policy. Add typed
  Gameplay, ShoveRequest, Effect/EffectBatch/EffectAck and ClientEffects. Implement
  `dedicated::Server::{new,receive,service,player_count}` with `Config` containing
  session/server_id/map/max_players. Test admission, replication, validation,
  collision/shove authority, bounded reliability and legacy compatibility.
- [x] Executable: add `skate-server` workspace crate with argument parsing,
  streaming map fingerprinting and a bounded nonblocking UDP service loop. Test
  command errors, real sockets and executable startup/shutdown. Document use.
- [x] Client: add `--connect`, dedicated startup/status/player filtering, built-in
  gameplay capture/presentation and accepted effects in fixed preparation. Wire
  right-bumper shove intent and existing animation/wipeout lifecycle. Disable
  duplicate proxy reaction and dedicated mod/appearance transfer. Test helpers.
- [x] Integration: run networking/server suites serially, check the game target,
  review the complete diff and fix regressions. Update Graphify after code edits.

Each implementation task starts with a regression test and records its failing
then passing result. Primary baseline: `cargo test --locked -p skate-net` passed
all 40 existing tests before implementation. New crate registration may update
the workspace lockfile without changing dependency versions.

## Results (2026-10-04)

- `cargo test --locked -p skate-net`: 58 passed, including 17 dedicated tests and
  bounded visitor-history coverage. Original legacy, high-speed, interpolation,
  application and blob suites still pass.
- `cargo test --locked -p skate-server`: 10 passed, including executable startup,
  two-client real UDP body/pose/trick replication, shared collision/shove effects,
  acknowledgement/deduplication and departure.
- Game `dedicated` filter: 7 passed. Velocity-routing filter: 2 passed. The two
  normally ignored native-asset tests were explicitly run with locally prepared
  assets: both passed. They exercise actual wipeout lifecycle, one-time impulse
  application and the stock Shove animation after releasing the bumper before
  server acceptance.
- `cargo build --locked -p skate-game --bin skate3rust --features dev-dynamic`
  and `cargo check --workspace --locked`: passed.
- `cargo build --locked --release -p skate-server`: passed; the release
  executable's `--help` smoke check also passed.
- Two graphical clients and the headless server ran concurrently on loopback.
  Both clients exited successfully and captured the other player and two-player
  roster. Their saved reports recorded 360 and 351 native physics ticks; logs
  reported one remote peer each and no physics failure. The initial wrapper
  incorrectly required `GAME_VERIFY_OK` from the disabled Bevy LogPlugin;
  readback instead validated the completed screenshot/report artifacts and
  runtime diagnostics. No product change was required for that harness issue.
- Independent source review findings were fixed and regression-tested: stuck
  long respawns, asymmetric readmission/restart acknowledgement state, stale
  shove replay, unbounded departed-player link records and offboard double
  impulse application. No actionable findings remained in final review.
- `git diff --check`: passed. Documentation links were checked.
- `graphify update .`: completed (22,408 nodes, 48,471 edges). It warned about
  the installed skill/package version mismatch, refreshed community labels and
  non-code files that yielded no AST nodes; no semantic/API extraction was run.

The graphical smoke covers startup, remote rendering and session integration;
it is not controller-driven gameplay or WAN playtesting. Conservative server
player proxies and owner-predicted terrain/limb physics remain the documented
authority boundary. No commits, pushes or deployment were performed.
