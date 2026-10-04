# Linux Developer Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Linux developers asset-free CI plus reliable commands to build, prepare assets, and launch without Steam relay staging.

**Architecture:** Small POSIX launch/build wrappers are behavior-tested with fake executables. Documentation binds those commands into one workflow; a separate Ubuntu CI job runs checks without owned assets.

**Tech Stack:** POSIX shell, Python unittest, GitHub Actions, Rust.

**Spec:** `docs/superpowers/specs/2026-10-02-linux-developer-workflow-design.md`

**Execution status:** Complete; evidence and implementation rulings are recorded in `.superpowers/sdd/2026-10-02-linux-developer-workflow/progress.md`.

## Global Constraints

- Linux scripts never select or stage `skate-steam-relay`.
- Preserve Windows scripts and release behavior; no assets, Steam libraries, deploy, or publish actions.
- Preserve `CONTEXT.md`, unrelated changes, and do not commit.

## Review Focus

- Paths containing spaces must survive script argument forwarding.
- Existing `RUSTFLAGS` must survive the musl adjustment.
- Launch must distinguish missing assets from a valid executable-adjacent installation.
- CI must not require a display, controller, ISO, or extracted game files.
- Crash diagnostics must fall back cleanly if `uname` cannot run.

---

### Task 1: Linux build and launch scripts

**Files:** Create `BUILD.sh`, `PLAY.sh`, and `tools/test_linux_scripts.py`.

**Interfaces:** `BUILD.sh [--release]`; `PLAY.sh [game args...]`; environment contracts `SKATE_RELEASE`, `SKATE3_MODS`, and existing `RUSTFLAGS`.

- [ ] Write behavior tests with fake `cargo`, `ldd`, and game executables asserting debug/release commands, no relay package, musl flag preservation, absolute asset argument, adjacent-data fallback, mods default, and exact argument forwarding.
- [ ] Run `python3 -m unittest tools.test_linux_scripts -v`; verify RED because scripts are absent.
- [ ] Implement both POSIX scripts and make them executable.
- [ ] Run the behavior tests and `sh -n BUILD.sh PLAY.sh`; verify GREEN.

### Task 2: Linux documentation, CI, and diagnostics

**Files:** Create `docs/LINUX.md`, `.github/workflows/linux.yml`, `crates/skate-platform/src/crash.rs`; modify `README.md`, platform lib, and game crash report.

**Interfaces:** Produces `crash::os_version() -> String`; CI invokes the commands fixed in the spec.

- [ ] Write Rust tests for trimmed successful OS text and fallback text through an injected command-output formatter.
- [ ] Run focused tests and verify RED because the crash helper is absent.
- [ ] Implement the helper and game integration, then write the Linux guide, README link, and asset-free Ubuntu workflow.
- [ ] Run focused tests, Python script/setup tests, YAML readback with Python, and script syntax checks; verify GREEN.

### Task 3: Full Linux validation and graph refresh

**Files:** All files changed by P0-P2 plus `graphify-out/` generated updates.

**Interfaces:** Consumes every earlier task; produces the verified working tree and explicit manual-check list.

- [ ] Run `cargo fmt --all -- --check`, focused platform/XISO/game tests, affected Python tests, and script behavior tests.
- [ ] Run `cargo check --workspace --locked`, `cargo build --locked -p skate-game --bin skate3rust`, `cargo build --locked -p skate-xiso`, and `sh -n BUILD.sh PLAY.sh`; verify all exit 0.
- [ ] Cross-check Windows cfg with an installed Windows target if available; otherwise inspect cfg boundaries and report it unexecuted.
- [ ] Run `graphify update .`, inspect `git diff --check`, confirm `CONTEXT.md` remains untracked/unmodified, and perform the final whole-tree review.
