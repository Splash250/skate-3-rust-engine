# Linux Asset Preparation Design

## Intent

Let Linux developers prepare owned Skate 3 assets from either an Xbox 360 ISO
or an already extracted `default.xex` plus `data/` directory without relying
on a Windows executable or a GUI. Preserve the packaged Windows setup flow and
its pinned `extract-xiso.exe` fallback.

This phase does not package game assets, change asset formats, alter Steam, or
redesign installation transactions.

## Native XISO extractor

Add a workspace crate named `skate-xiso`. Its library owns argument parsing,
path validation, destination lifecycle, and XDVDFS extraction; its binary only
prints errors and selects the process exit status. Use the MIT-licensed
`xdvdfs` 0.8.3 crate with synchronous, read-only features.

The supported CLI is exactly:

```text
skate-xiso -x <image.iso> -d <output-directory>
```

`-x` and `-d` may appear in either order. Missing values, duplicate options,
unknown options, non-Unicode option names, and positional arguments are errors.

Extraction accepts RAW and Xbox 360 offset layouts through
`xdvdfs::blockdev::OffsetWrapper`. The destination must not contain existing
entries. The extractor creates it, tracks whether it owns it, and removes that
new tree on failure so a partial disc is never mistaken for usable input.

Every image path component must be non-empty, neither `.` nor `..`, and contain
neither `/` nor `\`. Files use create-new semantics, preventing duplicate image
entries from overwriting an earlier extraction. Only directories and ordinary
files are emitted; no image entry can create a symlink.

## Python extractor selection

Replace direct calls to the downloaded Windows tool with
`xiso_extractor(base, game_exe, report)`:

1. use `SKATE_XISO` when it names an existing file;
2. use `skate-xiso` (or `skate-xiso.exe`) beside `game_exe`;
3. in a source checkout, use `target/release/skate-xiso`, then
   `target/debug/skate-xiso`;
4. on Windows only, fall back to the existing pinned XboxDev download;
5. otherwise fail with a command that tells the developer to build
   `cargo build --locked -p skate-xiso`.

Do not add an unpinned Linux binary download. Both the core and customiser ISO
paths use the same selector and invoke options before operands:
`-d <destination> -x <image>`.

The existing source-directory logic remains authoritative. A selected
directory, or a selected file named `default.xex`, resolves to the game root;
required files beneath `data/` continue to be validated by the groups that
consume them. Windows ISO and extracted-directory behavior remains intact.

## Headless setup

Extend `tools/setup.py` with `--source PATH`. When present, setup runs the same
transactional `customiser_setup.install` operation synchronously, prints
progress, writes the existing `setup-error.log` on failure, prints the optional
content summary, and returns status 0 or 2. The headless branch runs before any
Tk import or window construction.

GUI behavior remains the default. Call `window.iconbitmap` only on Windows;
all other UI behavior remains unchanged.

## Helper executable portability

Add small `skate-platform` helpers for platform executable names and hidden
Windows child processes. Use them for the setup, character-import, and updater
helpers. On Windows names still end in `.exe` and `CREATE_NO_WINDOW` remains;
on Linux names have no suffix and process configuration is a no-op.

This does not create or package Linux release helpers. The documented Linux
developer flow invokes `tools/setup.py --source` directly.

## Tests

Rust tests must first fail against the missing `skate-xiso` library and then
cover:

- both valid option orders;
- missing, duplicate, unknown, and positional arguments;
- safe nested names and rejection of empty, dot, parent, slash, and backslash
  components;
- rejection of a non-empty destination;
- cleanup of a newly created extraction directory after a forced failure.

Python tests must first fail against the old pipeline and then cover:

- `SKATE_XISO` precedence;
- adjacent and checkout native candidates;
- Windows fallback selection without changing its URL or checksum;
- Linux's actionable error when no native extractor exists;
- directory and `default.xex` source resolution;
- `--source` dispatch without importing Tk;
- headless success and failure exit codes and error-log creation;
- non-Windows GUI setup not calling `iconbitmap`.

Owned game data is not needed for these tests. A real ISO extraction remains a
manual check when no legal test image is available.

## Expected files

- Modify `Cargo.toml`
- Modify `Cargo.lock`
- Create `crates/skate-xiso/Cargo.toml`
- Create `crates/skate-xiso/src/lib.rs`
- Create `crates/skate-xiso/src/main.rs`
- Modify `crates/skate-platform/src/lib.rs`
- Create `crates/skate-platform/src/exe.rs`
- Create `crates/skate-platform/src/process.rs`
- Modify `crates/skate-game/src/setup.rs`
- Modify `crates/skate-game/src/custom_models.rs`
- Modify `crates/skate-game/src/updater.rs`
- Modify `tools/asset_pipeline/install.py`
- Modify `tools/asset_pipeline/customiser_setup.py`
- Modify `tools/setup.py`
- Create or modify focused tests under `crates/skate-xiso/src/` and `tools/`

## Verification

Run the `skate-xiso` unit tests, affected Python setup tests, existing setup
refresh/asset tests, `cargo check --workspace --locked`, Rust formatting, and
`graphify update .` after the phase's code changes.
