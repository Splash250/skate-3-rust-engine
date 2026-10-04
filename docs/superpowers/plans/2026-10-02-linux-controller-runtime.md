# Linux Controller Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Linux's unsupported controller stub with tested raw gilrs polling while preserving Windows XInput and TU3 conversion.

**Architecture:** A new `skate-platform` crate owns the shared packet/cache contract and cfg-selected transports. Linux uses pure conversion and slot-allocation helpers around one persistent gilrs backend; the game keeps its current collector through re-exports.

**Tech Stack:** Rust 2024, gilrs 0.11.2, existing `skate-core` Xbox conversion.

**Spec:** `docs/superpowers/specs/2026-10-01-linux-controller-runtime-design.md`

**Execution status:** Complete; evidence and implementation rulings are recorded in `.superpowers/sdd/2026-10-02-linux-controller-runtime/progress.md`.

## Global Constraints

- No Steam relay, Steam feature-gating, macOS, gameplay, scheduling, or TU3 conversion changes.
- Preserve Windows XInput ABI and one-second capability-cache behavior.
- Use SDL_GameControllerDB revision `555ce569a2003b22a4e134882224f1e2bdecc2e3` with its license; omit the fork-only override.
- Preserve `CONTEXT.md`, unrelated changes, and do not commit.

## Review Focus

- Negative full-scale sticks must remain `-32768`, not `-32767`.
- A disconnected first controller must not renumber a still-connected second controller.
- Hat axes must work while default gilrs filters are disabled.
- Cache errors and disconnects must not leave stale subtype data.
- Windows must not compile or link gilrs through `skate-platform`.

---

### Task 1: Shared adapter and Windows transport

**Files:** Create `crates/skate-platform/{Cargo.toml,src/lib.rs,src/input/mod.rs,src/input/xinput.rs}`; modify `Cargo.toml`.

**Interfaces:** Produces `DevicePacket`, `DeviceError`, `CapabilityCache`, `poll_cached`, and `poll` with the signatures in the spec.

- [ ] Write cache tests asserting reuse at 999 ms, refresh at 1 second, explicit invalidation, and no error caching.
- [ ] Run `cargo test -p skate-platform --locked capability_cache` and verify RED because the crate/API is absent.
- [ ] Implement the shared types/cache and move the current XInput module without behavioral changes; keep unsupported fallback for non-Windows/non-Linux.
- [ ] Run the focused cache test and `cargo check -p skate-platform --locked`; verify GREEN.

### Task 2: Linux raw conversion and persistent slots

**Files:** Create `crates/skate-platform/src/input/gilrs_raw.rs`; modify `crates/skate-platform/Cargo.toml`.

**Interfaces:** Consumes the Task 1 packet/cache API. Produces Linux `poll(index, cache)`, pure `stick`, `trigger`, `button_bits`, `dpad_bits`, and slot-reconciliation behavior internal to the module.

- [ ] Write table-driven tests with literal expectations: stick `[-1,-.5,0,.5,1] → [-32768,-16384,0,16384,32767]`; trigger `[0,.5,1] → [0,128,255]`; all XUSB button bits; four hat directions and neutral; stable `[10,20] → [None,20] → [30,20]` slots.
- [ ] Run the named module tests and verify RED because the conversion/backend is absent.
- [ ] Implement pure conversion/slot helpers, then the thread-local gilrs backend with filters and force feedback disabled, environment mappings enabled, bundled mappings disabled, and vendored mappings added.
- [ ] Run `cargo test -p skate-platform --locked input::gilrs_raw::tests`; verify GREEN.

### Task 3: Licensed mappings and game integration

**Files:** Create mapping/license files and `docs/THIRD_PARTY_NOTICES.md`; modify `crates/skate-game/{Cargo.toml,src/input/platform.rs,src/input.rs}` and `Cargo.lock`.

**Interfaces:** Consumes Task 1's public API. Existing `ControllerInput::collect` continues consuming `DevicePacket` unchanged.

- [ ] Add an integration test asserting a Linux absent slot returns `Disconnected`, clears a primed cache, and existing game controller tests still publish zero actions after disconnect.
- [ ] Run focused platform/game tests and verify the new platform assertion is RED before re-export integration.
- [ ] Fetch the pinned canonical mapping and license, record provenance, add dependencies/re-exports, and make readiness logging platform-neutral.
- [ ] Run `cargo fmt --all`, `cargo test -p skate-platform --locked`, `cargo test -p skate-game --locked input::`, and `cargo check --workspace --locked`; verify GREEN.
