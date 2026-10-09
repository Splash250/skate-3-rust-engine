# 25. Performance: frame-time diagnostics and faster setup

Branch `feature/perf-diagnostics` (from `main` 4488651). One upstream PR, two parts:

1. **Frame-time diagnostics** (todo `frame-timing`): a frame-time counter that shows stutter, a per-frame log
   behind `SKATE_FRAME_LOG`, a graphics-menu row to turn the counter on, and a read-only `sdk.snapshot.frame`
   for mods. Observation only: the game is identical with all of it on or off.
2. **Setup speed, phases 4–5** (todo `asset-pipeline-performance`): setup runs below normal priority with
   overridable worker counts, and a map's conversion overlaps its independent work (texture and blob
   compression on threads, movable props beside the collision archive and the map writer). Every output is
   byte-identical.

Machine for all numbers: Windows 11, i7-14700KF (20 cores / 28 threads), Python 3.13.

---

## Part 1: frame-time diagnostics

### Problem

The user: "the ingame fps counter is not the best". A session felt "super stuttery when I try to do tricks"
(2026-10-02) and nothing could measure it:
- The on-screen counter (`fps_overlay.rs`) showed `FPS: N` = frames / seconds over 0.5 s windows. An average
  hides a hitch: 167 frames of 3 ms plus one 200 ms frame read **240 FPS** (unit test
  `one_hitch_is_visible_where_an_average_hides_it`). It was always on.
- The audio state log runs on the fixed 60 Hz step, which catches up after a hitch, so it shows 60 rows/s while
  rendering stutters.
- The game log has no frame times.

### Root cause of the misleading counter

It measured the right clock (`Time<Real>`, the presented frames) but reported only a 0.5 s mean. The worst frame,
the lows and the hitch count are what describe stutter; a mean cannot.

### Change

`crates/skate-game/src/frame_timing/` replaces `fps_overlay.rs`:
- `stats.rs` (pure functions, unit-tested): nearest-rank percentiles, the window summary (mean, median,
  **1 % low** = 99th-percentile frame time, **0.1 % low** = 99.9th, worst), the hitch rule (a frame longer than
  **2× the median of the previous 120 frames**, needs 30 frames of history), and the graph buckets (worst frame
  per 50 ms bucket, so a hitch always shows as a bar).
- `mod.rs`: the `FrameTiming` resource. A frame is one iteration of the app's main loop, timed with its own
  clock from the start of `First` to the start of the next `First` (what the player sees; in steady state the
  main loop waits for the pipelined render thread, so this is the presentation interval). One push per frame
  into a 5 s window (bounded at 20,000 frames); the summary is recomputed 4× per second. Systems:
  - `begin_frame` (`First`, before the time update): closes the previous frame and pushes / logs it.
  - `begin_fixed` / `end_fixed` (`RunFixedMainLoop`, before / after the fixed loop) and `count_fixed_step`
    (`FixedFirst`): the physics part of the frame.
  - `record` (`Last`): the frame's CPU split: `main_ms` (its First..Last schedules), `fixed_ms`, `fixed_steps`.
    A frame much longer than its `main_ms` waited outside the schedules (render thread, GPU, present, OS). The
    fixed steps that catch up after a long frame run in the following frame(s): the virtual clock advances
    from the render thread's timestamps.
    - Why not `Time<Real>`'s delta: with pipelined rendering it comes from the render thread and lags the main
      loop by a frame; a first version paired it with the CPU split and produced rows with `main_ms` >
      `frame_ms` (smoke run). With the own clock every row satisfies `main_ms <= frame_ms`.
  - `show` (`Last`): the overlay (top right): `FRAME ms / fps` over the last 0.25 s, `1% low`, `0.1% low`,
    `worst 5 s`, `hitches`, `CPU main / physics`, and an 80-bar graph (4 s; green ≤ 16.7 ms, yellow above, red =
    hitch). Text updates at 4 Hz, the graph at 10 Hz; hidden = no UI work.
  - `SKATE_FPS_LOG` still prints the old `SKATE_FPS_SAMPLE` lines.
- `log.rs`: `SKATE_FRAME_LOG=<file>` → one TSV row per frame, written by a background thread with a bounded 4,096-row queue. Sending never blocks: a full queue drops
  new rows, and the dropped count is reported on shutdown. The writer flushes after draining each batch
  and on drop, and is joined on drop. Unset: no thread, no file, no row work. Format v1:
  ```
  # skate3rust frame log v1
  wall_unix_s  frame  frame_ms  fixed_steps  fixed_ms  main_ms  hitch  median_ms
  ```
  `wall_unix_s` (UTC) lines rows up with `logs\game-*.stderr.log` and the audio state log; `hitch` is `HITCH` or
  empty. Reading tool (fork-local): `.claude/skills/optimisation/tools/frame_log_summary.py`.
- `graphics_menu.rs`: row 28 "Frame-time counter On/Off" in GRAPHICS (after the audio controls), saved as
  `frame_stats` in `settings/graphics.json`, **off by default** (old files load as off).
- Moddability: `sdk.snapshot.frame` (read-only): `frame, ms, fixed_steps, main_ms, fixed_ms, hitch, window_s,
  frames, fps, mean_ms, median_ms, low_1_ms, low_01_ms, worst_ms, hitches`. Defaults (zeros) in the Lua VM so it
  is never nil; documented in `sdk/ENGINE_API.md` ("Frame-time statistics") and `sdk/skate.lua`
  (`FrameSnapshot`). Mods cannot write it.

### Retail parity

Diagnostics only; nothing in retail Skate 3 to match. The proof obligation is that gameplay is unchanged:
- `systems_touch_only_their_own_state`: from Bevy's system access sets, no frame-timing system writes any
  resource but `FrameTiming`, none writes everything, and only `show` writes components (its overlay nodes).
- `game_is_identical_with_diagnostics_on_or_off`: a Bevy app with a fixed-update "game" (60 Hz, stateful)
  through 600 uneven frames (incl. 120 ms hitches), without the plugin, with it, and with the log: the fixed
  steps, every fixed delta and the state (bit pattern) are identical. Every fixed step is attributed to a logged
  frame (only the final frame's split is still pending).

### Tests

`cargo test -p skate-game --release --bin skate3rust -- frame_timing graphics_menu`, plus skate-mods
`frame_statistics_have_defaults_and_keep_host_values`. Listed in "Verification" below.

### Follow-up: Simple and Verbose modes

The full readout covers a lot of the screen for everyday play. The GRAPHICS row "Frame-time counter" now cycles
Off / Simple / Verbose instead of Off / On.
- Verbose is the readout above, unchanged.
- Simple: fps, frame time, 1 % low and worst frame on two short lines in a smaller font (11 px instead of 14 px),
  with a small spike graph of the last 2 s (40 bars, 14 px high).
- Saved as `frame_stats_simple` next to `frame_stats` in `settings/graphics.json`. Files saved before this
  change have no such field and open as Verbose when the counter was on, so nobody's counter changes by itself.
- Files: `frame_timing/mod.rs` (`readout_simple`, layout switch in `show`, done once per mode change),
  `graphics_menu.rs` (setting, row cycle, label).
- Test: `frame_counter_setting_defaults_off_and_round_trips` also checks the old file and a Simple round trip.
- Diagnostics only; the same proofs as above apply (only the overlay nodes change).

---

## Part 2: setup speed, phases 4–5

### Phase 4: priority, worker counts, overrides

New module `tools/asset_pipeline/setup_budget.py` (its name matches no fingerprint glob on purpose):
- `install()` runs setup at **below-normal priority** (`lowered_priority`, restored afterwards) and `run()` starts
  every conversion process (map jobs, the game's `--check-assets`, extract-xiso) with
  `BELOW_NORMAL_PRIORITY_CLASS`; their children inherit it.
- Overrides: `SKATE_SETUP_PRIORITY=below_normal|normal|idle`, `SKATE_SETUP_MAP_WORKERS=<n>`,
  `SKATE_SETUP_THREADS=<n>` (threads inside one map job).
- Map workers unchanged: `min(3, cpu/2)`, capped at (free RAM − 2 GiB) / 3 GiB. **Measured peaks** (2026-10-04):
  DownTown's conversion job 716 MiB working set / **1.44 GiB committed**; the game's `--check-assets` that
  follows peaks at **1.73 GiB working set / 1.9 GiB private** (University 1.33 / 1.41, Industrial 0.96 / 1.06).
  The 3 GiB per-worker budget therefore holds for the largest district. More than 3 workers barely helps: DownTown
  alone sets the map stage's length.

### Phase 5: inside one map

Measured on DownTown with `main` (`.local/perf-bench/map_bench.py`, validation stubbed): extract 11.5 s (cold
disk) / 1.0 s (warm), prepare 122–128 s, collision archive 2.7 s, write_map 26.8–29.2 s, props 8.5–10.6 s,
hash + cleanup 2.4–3.1 s. (`prepare` is dominated by stream decoding, which PR G #29 cuts; not part of this PR.)

1. **write_map compresses on threads** (`map_writer.py`):
   - `write_textures(output, root, textures, workers=None)`: the ordered window is now `job_threads()` wide
     (default cpu/4, 2..6; was 2). Writes stay in name order.
   - The vertex, index and empty blobs and the RWCM / WMET extension blobs do not depend on the textures; they are
     compressed on two more threads while the textures are written, and written in the original order.
     `stored()` is now `packed_blob()` + write (same bytes).
   - DownTown write_map (same intermediate, A/B): 2 texture threads 18.1–18.6 s, 3 → 15.0 s, 6 → 12.8–12.9 s;
     main 18.6–19.3 s.
2. **Movable props beside collision + write_map** (`install.convert_map`): the props only read the prepared
   manifest and the DMO catalog and write their own folder, so they run on one background thread from the end of
   `prepare`. `props` is now the time left waiting for them (0.0 s on DownTown). Error handling unchanged (a
   `CONTENT_ERRORS` failure still writes `<map>-availability.json` and the map still converts).

Result (all districts, below). DownTown collision + write_map + props: **37.9 → 24.4 s**.

Rejected, with numbers:
- **Extract without writing the stored BIG entries**: the 11–15 s "extract" was the cold read of the 843 MB
  archive. Warm, extracting DownTown takes 0.7–1.4 s (threaded 0.3–0.4 s), so not writing would save ≤ 1 s and
  needs the vendored stream loader to read from the archive (fingerprint of every map group). Not done.
- **Prefetching model npz files on threads in write_map**: 15.8 → 18.1 s (slower). NpzFile header parsing is
  pure Python and holds the GIL.
- B5G6R5 decoded twice / collision decode allocating per triangle: both in vendored parsers (`PARSERS`, i.e. the
  character, environment and maps fingerprints). Left open.

### Equivalence (byte-identical)

- **Every district, every output.** `.local/perf-bench/all_maps.sh` converts each of the 10 districts with
  `main`'s tools (`git archive main tools`) and with this branch, alternating per map, into the same scratch
  stage path (the WMET block embeds the stream path), and hashes every output: `maps/<map>.skate`,
  `maps/<map>.irradiance`, `native-props/<map>.skate/*`. **ALL IDENTICAL** (10/10 districts, 3–4 files each),
  including the map entry (`sha256`). Validation was stubbed in both (not under test).

  | District | collision + write_map + props, main → branch | Peak committed memory |
  |---|---|---|
  | DownTown | 37.9 → 24.4 s | 1443 → 1445 MiB |
  | University | 29.9 → 21.1 s | 1273 → 1273 MiB |
  | Industrial | 26.1 → 16.5 s | 1165 → 1167 MiB |
  | SkateSchool | 3.8 → 2.2 s | 832 → 831 MiB |
  | MaloofMoneyCup | 1.9 → 1.1 s | 825 → 845 MiB |
  | DownTownSkatePark | 1.5 → 0.9 s | 809 → 821 MiB |
  | BlackBoxPark, IndustrialSkatePark, MegaPark, StartPark | 0.4–0.7 → 0.2–0.4 s | ≤ +22 MiB |

- **Environment group** (the other user of `map_writer.write`): `backdrop.convert` with both trees → the 3
  backdrops (`DownTown`, `Industrial`, `University`) byte-identical.
- **Props failure path**: with an empty DMO catalog (StartPark), both trees write the same
  `StartPark-availability.json` status / error; only the traceback text differs (file path, line, and the
  function name `movable_props`).
- **Thread count**: `write_map` on the same DownTown intermediate with `SKATE_SETUP_THREADS` = 2, 3, 6 and with
  `main`: all byte-identical (`phase_bench.py`).

### Fingerprints

`map_writer.py` (maps + environment) and `install.convert_map` (maps) changed. Pairs in
`tools/asset_pipeline/pipeline-equivalence.json` for exactly these two groups, from `main`'s fingerprints to this
branch's: environment `e062aa4b…` → `39097274…`, maps `2d52e21d…` → `81867db8…`; `changed_groups(main, branch)` is
empty, so an existing installation rebuilds nothing. core, hud, character: unchanged. Customiser fingerprint and
stage versions: unchanged (checked against `git archive main tools`). `setup_budget.py` is in no fingerprint;
`install.py` gained no top-level import. When this PR and #28 / #29 (which also change the maps fingerprint) are
combined, the pairs have to be recomputed for the combined tree (as for F + G).

## Verification

- Rust: `cargo test -p skate-game --release --bin skate3rust --locked`: 323 passed, 2 failed - the known
  pre-existing `setup::tests::pipelines_accept_valid_group_outputs_when_fingerprint_changes` and
  `retail_render::shader_tests::sky_shader_validates`. New: 18 in `frame_timing` + `graphics_menu` (stats math,
  log format and writer, window, CPU split, app-level identity, system access, menu setting).
  `cargo test -p skate-mods --release --locked --lib`: 58 passed (new `frame_statistics_have_defaults_and_keep_host_values`);
  the `skyline_physics` integration test needs the Skyline GLB, which is not in a fresh checkout.
- Python (`py -3.13 -m unittest`): `test_map_writer` (whole-map identity across thread counts and render-only,
  blob layout, texture workers 1/2/3/7), `test_setup_budget` (overrides, priority classes, a child process really
  starts BelowNormal and the previous class returns, `run()` creation flags), plus `test_versions`,
  `test_setup_assets`, `test_setup_refresh`, `test_setup_recovery`, `test_dynamic_props`, `test_irradiance`: all OK.
- Smoke runs (muted is not available on `main`; `SKATE_REPORT_CHILD=1`, `--verify`, University, a settings copy
  with the counter on): the overlay renders (screenshot), the log is written (v1 header, 480–2,047 rows), no
  panics or errors; `frame_log_summary.py` reads it; every row has `main_ms <= frame_ms`.
- Full fresh setup into a scratch base (`.local/perf-bench/full_setup.py`, `default.xex`, 3 map workers), `main` vs
  branch, run back to back: total **445.6 → 434.6 s**; map stage 183.1 → 179.8 s (DownTown is the long pole and its
  `prepare` stream decode, cut by #29, dominates); customiser 228 → 234 s (unchanged code; runs after the maps on
  `main`). Both: 0 warnings. Single runs on a shared machine: the per-district A/B above is the reliable number.
  The full-setup gain is small on `main` because `prepare` dominates; with #29 the post-prepare phases are a larger
  share of each map.

## Files

- `crates/skate-game/src/frame_timing/{mod,stats,log}.rs` (new; replaces `fps_overlay.rs`), `main.rs`, `app.rs`,
  `graphics_menu.rs`, `modding/mod.rs`; `crates/skate-mods/src/vm.rs`; `sdk/ENGINE_API.md`, `sdk/skate.lua`.
- `tools/asset_pipeline/setup_budget.py` (new), `install.py`, `map_writer.py`, `pipeline-equivalence.json`;
  tests `test_setup_budget.py` (new), `test_map_writer.py`.

## Open questions

- Not yet run on the branch: regression-check `check_maps.py` / `check_spawns.py` against the scratch install
  (outputs are byte-identical to `main`'s, so they can only match `main`).

- Per-system split beyond main / physics / render-wait (e.g. audio, streaming): use Bevy's tracing spans
  (`TRACE_PLAY.bat`) for that; not built into the counter.
- Phase 5 leftovers in vendored parsers (above), and `hash_and_cleanup` (~3 s, I/O).
- Default on/off of the counter: off (todo); the user may prefer on.
