# Linux Asset Preparation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prepare owned game assets on Linux from an ISO or extracted directory through a native tested extractor and headless setup command.

**Architecture:** A `skate-xiso` library provides testable CLI/path/destination logic and a thin binary. Python selects native extractors deterministically, retains the pinned Windows fallback, and exposes setup before importing Tk.

**Tech Stack:** Rust 2024, xdvdfs 0.8.3, Python unittest and pathlib.

**Spec:** `docs/superpowers/specs/2026-10-02-linux-asset-preparation-design.md`

**Execution status:** Complete; evidence and implementation rulings are recorded in `.superpowers/sdd/2026-10-02-linux-asset-preparation/progress.md`.

## Global Constraints

- Preserve Windows setup transactions, download URL/checksum, and extracted-folder behavior.
- Do not download an unpinned Linux extractor or include owned game data.
- No Steam or unrelated asset-format changes; preserve `CONTEXT.md`; do not commit.

## Review Focus

- Image traversal and duplicate names must not overwrite outside or inside the extraction root.
- A failed extraction must not leave a directory that appears complete.
- Headless setup must not import or initialize Tk.
- Native-extractor lookup must not silently select a missing/non-file override.
- Refreshes from `default.xex` must retain the exact source-root/hash behavior.

---

### Task 1: Safe native XISO library and CLI

**Files:** Create `crates/skate-xiso/{Cargo.toml,src/lib.rs,src/main.rs}`; modify workspace manifest/lock.

**Interfaces:** Produces `Options { image: PathBuf, destination: PathBuf }`, `parse_args`, `checked_component`, `prepare_destination`, `extract`, and `run`.

- [ ] Write tests asserting both valid argument orders and literal errors for missing, duplicate, unknown, positional, and non-Unicode options; assert safe components and rejection of `""`, `"."`, `".."`, slash, and backslash; assert non-empty destination rejection and owned-directory cleanup on injected failure.
- [ ] Run `cargo test -p skate-xiso --locked`; verify RED because the crate is absent.
- [ ] Implement minimal parsing, validation, destination guard, xdvdfs traversal, create-new file output, and the thin main.
- [ ] Run `cargo test -p skate-xiso --locked`; verify GREEN.

### Task 2: Native extractor selection and extracted sources

**Files:** Modify `tools/asset_pipeline/{install.py,customiser_setup.py}`; create/modify focused `tools/test_setup_*.py`.

**Interfaces:** Produces `xiso_extractor(base: Path, game_exe: Path, report) -> Path`; existing `_install` and customiser install consume it.

- [ ] Write Python tests for override, adjacent, release/debug checkout candidates, Windows fallback, Linux missing-native error, option order, directory input, and `default.xex` input.
- [ ] Run the focused unittest module and verify RED against current direct `dependency(...)` calls.
- [ ] Implement selection and route both ISO paths through it without changing the pinned Windows constants or `source_directory` validation.
- [ ] Run focused tests plus `tools.test_setup_refresh` and `tools.test_setup_assets`; verify GREEN.

### Task 3: Headless setup and portable helper names

**Files:** Modify `tools/setup.py`, `crates/skate-platform/src/{lib.rs,exe.rs,process.rs}`, and game setup/custom-model/updater call sites; modify tests.

**Interfaces:** Produces `setup.main(argv=None)`, `setup.headless(args)`, `exe::name(&str) -> String`, and `process::hidden(&mut Command) -> &mut Command`.

- [ ] Write tests asserting `--source` avoids Tk, success returns 0, failure returns 2 and writes `setup-error.log`, non-Windows icon setup skips `iconbitmap`, and executable naming preserves `.exe` only on Windows.
- [ ] Run focused Python/Rust tests and verify RED.
- [ ] Implement headless dispatch before Tk import and use portable helpers at the three game call sites.
- [ ] Run `cargo fmt --all`, `cargo test -p skate-xiso --locked`, `cargo test -p skate-platform --locked`, affected Python tests, and `cargo check --workspace --locked`; verify GREEN.
