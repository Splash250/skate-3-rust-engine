# Linux Developer Workflow Design

## Intent

Provide documented, repeatable Linux commands for building, preparing owned
assets, and launching the current game, plus CI that checks the Linux-specific
code without owned assets. Steam relay work remains untouched and absent from
Linux scripts.

## Build and launch scripts

Add POSIX `BUILD.sh` and `PLAY.sh` at the repository root.

`BUILD.sh` accepts an optional `--release` and no other options. It runs locked
Cargo builds for `skate-game --bin skate3rust` and `skate-xiso` as two explicit
commands. It never selects `skate-steam-relay`. On a musl host it appends
`-C target-feature=-crt-static` to `RUSTFLAGS`, retaining any existing flags,
because the system Wayland/X11/audio libraries are ordinarily dynamic.

`PLAY.sh` accepts game arguments verbatim. It selects the debug executable by
default and the release executable when `SKATE_RELEASE=1`. If the executable is
missing it prints the matching build command and exits. It sets `SKATE3_MODS`
to the repository's `mods/` only when the variable is unset. If a repository
`assets/` directory exists it launches with `--assets <absolute path>`;
otherwise it relies on the installation under the executable-adjacent `data/`
directory created by the documented headless setup command.

Both scripts resolve the repository from their own location, quote paths, use
`set -eu`, and remain valid under `/bin/sh`.

## Documentation

Create `docs/LINUX.md` and link it from `README.md`. Document:

- Rust and Python requirements;
- representative Debian/Ubuntu and Fedora packages for libudev, ALSA,
  pkg-config, clang/libclang, X11/Wayland, and Vulkan loader/tools;
- a working Vulkan driver as a runtime requirement;
- debug and release builds;
- ISO and pre-extracted headless setup commands using executable-adjacent data;
- `SKATE_XISO`, `SKATE3_ASSETS`, `SKATE3_MODS`, and
  `SDL_GAMECONTROLLERCONFIG` overrides;
- raw Linux controller semantics and the four-controller limit;
- glibc versus dynamically linked musl considerations;
- launch commands and the absence of Linux Steam relay support;
- the fact that legal game assets are user-supplied and never needed by CI.

Keep existing Windows build and packaged setup instructions unchanged.

## Linux CI

Add `.github/workflows/linux.yml` for pushes and pull requests. On Ubuntu it
installs only build dependencies, uses stable Rust and Python, caches Cargo,
then runs:

- `cargo fmt --all -- --check`;
- focused `skate-platform` and `skate-xiso` tests;
- affected Python setup tests;
- `cargo check --workspace --locked`;
- `sh -n BUILD.sh PLAY.sh`.

The workflow neither obtains owned assets nor launches the game. It does not
build or stage a Linux Steam relay explicitly; the required workspace check
may compile the repository's existing platform-independent relay crate but
imports no Linux relay implementation or Steam library.

## Crash diagnostics

As a small isolated portability change, non-Windows diagnostic reports may
include `uname -sr` through `skate-platform::crash::os_version`. Failure or
empty output falls back to the current `OS version unavailable` text. Windows
native reporting and popup behavior are unchanged. No Linux GUI popup is
added.

## Tests and verification

- Script behavior tests execute copies against a fake `cargo`/game executable
  and assert selected packages, profile, forwarded arguments, asset selection,
  and absence of relay staging.
- A Rust unit test covers non-empty/fallback OS-version formatting at the
  helper boundary without requiring a particular kernel string.
- Run `sh -n`, script behavior tests, affected Python tests, focused Rust
  tests, `cargo check --workspace --locked`, and the relevant Linux build.
- If shellcheck is installed, run it; otherwise report it as unavailable.
- Hardware controller and Vulkan window launch checks remain manual when the
  environment has no controller or display.

## Expected files

- Create `BUILD.sh`
- Create `PLAY.sh`
- Create `docs/LINUX.md`
- Modify `README.md`
- Create `.github/workflows/linux.yml`
- Create `tools/test_linux_scripts.py`
- Modify `crates/skate-platform/src/lib.rs`
- Create `crates/skate-platform/src/crash.rs`
- Modify `crates/skate-game/src/crash_report.rs`

## Verification

After phase tests, run the complete required workspace check, relevant Python
suite, a real Linux game/extractor build, formatting, script syntax checks,
working-tree review, and `graphify update .`.
