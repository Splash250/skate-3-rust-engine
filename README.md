<p align="center">
  <img src="docs/images/skating-crab.png" alt="Rust crab riding a skateboard" width="480">
</p>

# Skate 3 Rust Engine

A Rust and Bevy skating project built from Skate 3 reverse-engineering research.
Includes skating, tricks, grinds, offboard movement, difficulty settings and
`.skate` map support. Gameplay parity is still a work in progress.

## Community multiplayer fork

This fork develops the community-hosted multiplayer and resource platform on top
of [SK8-ENGINE's engine](https://github.com/SK8-ENGINE/skate-3-rust-engine).
It includes dedicated hosting, native Linux support, server-selected resources,
Lua/JavaScript/C# scripting, shared worlds, accounts, persistence, browser UI,
voice, creator tools and roleplay examples.

Start with the [project status and documentation index](docs/PROJECT_STATUS.md)
for implemented capabilities, recorded validation, remaining work and the
relationship to upstream. For use, see [Linux setup](docs/LINUX.md),
[dedicated hosting](docs/multiplayer/README.md), and the
[Boardwalk Borough showcase](docs/multiplayer/boardwalk-borough.md).

This is a source-development fork. The upstream downloads below do not contain
this fork's platform additions. Fork release packaging and updater identity
still need configuration before distributing fork binaries.

## Live maps and interiors

The engine now includes a live 3D map built from the active world's geometry,
textures and colors. The compact minimap follows the skater's position and
heading with a fixed angled view. Press **M** or **D-pad Left** to open the
expanded map, with pan, rotate, tilt, zoom, top-down view and recenter controls.
Local and multiplayer player markers use world-space positions, including
height, and the map updates when the active world changes.

Lua mods and server resources can add owner-scoped markers, labels, paths and
region outlines, with configurable colors and sizes. They can also set map
visibility, opacity, title and opening zoom. Server layers are distributed to
admitted clients; local layers remain private to that client. See the
[map API and controls](sdk/MAP.md), [server example](resources/programmable-map/)
and [local mod example](sdk/examples/programmable-map/).

Native interior catalogs provide building entrances, floor selection, static
GLB models, authored collision shells and return destinations. Clients and
the dedicated authority use matching collision; multiplayer travel is
server-approved. Catalogs can come from an installed package, `--locations`,
a local mod or an admitted server resource. See the
[interior and location API](sdk/LOCATIONS.md) and
[synthetic two-floor example](sdk/examples/interior-catalog/).
The [interior preparation tool](tools/prepare_interior.py) prepares models
within resource budgets while retaining provenance; third-party interior
models are not bundled with this repository.

Map presentation remains experimental. Player direction arrows, a top-screen
waypoint compass, comprehensive plugin themes and automatic skateable-route
guidance are planned improvements, not implemented features.

## History

Before this rewrite existed, **dumbad** spent more than two years reverse
engineering Skate 3 and building the tools needed to understand and work with
it. That meant countless hours digging through undocumented file formats,
animation data, and game interaction systems, then testing those discoveries
in the original game. Much of that work is collected in
[DumbadsSkate3ModdingTools](https://github.com/Ethanw05/DumbadsSkate3ModdingTools),
including tools for custom maps, meshes, collision, challenges, and DLC.

That research laid the groundwork for this project. Chasm later worked on a
recompilation and a custom renderer based on dumbad's earlier renderer work,
before moving into the Rust/Bevy rewrite. The rewrite's development time tells
only part of the story: the knowledge and tools it relies on took years of
work to establish.

## AI usage

AI coding tools were used to develop this rewrite, but none of it would have
been possible without dumbad's extraordinary effort to reverse engineer the
original game. The AI had years of hard-earned research and working tools to
build on. Describing the project as simply “AI rewriting Skate 3” leaves out
the work that made it possible in the first place.

AI helped turn that knowledge into a new implementation; it does not replace
credit for discovering how the game works. This is still a work in progress,
and using original assets or showing working tricks does not mean every
system behaves exactly like the original.

## Play

[Download upstream Experimental](https://github.com/SK8-ENGINE/skate-3-rust-engine/releases/tag/experimental).
Successful upstream `main` builds replace this prerelease. Choose **Latest** in Updates
for experimental updates; **Stable** is the default.

Extract the Windows release ZIP and run `skate3rust.exe`. Select your Skate 3
Xbox 360 ISO, or select `default.xex` in an extracted game folder. Keep its
`data` folder alongside it. Setup prepares the skater, animations and all disc maps, then
starts University. The original scoring and session-marker HUD assets are also
exported automatically during setup. No Blender, Python or Rust installation is needed.
ISO extraction needs internet access. The first conversion can take a while.

Use a compatible gamepad to play. SDL3 supports Xbox/XInput, PlayStation,
Switch and generic HID controllers; XInput remains available as a Windows
fallback. Escape opens graphics, difficulty and map settings. Maps can be
switched without restarting the game.

**Skate 3 assets are not included.** Your converted files stay in
the `data` folder beside your executable. Each freshly unpacked copy runs its
own setup; it does not adopt another installation. In-place updates refresh
only changed asset groups.

## Build

Linux developers can use [`docs/LINUX.md`](docs/LINUX.md) for native build,
owned-asset preparation, controller, and launch instructions.

Requires Windows, Rust with the MSVC toolchain, and LLVM installed in its default
location. Run `BUILD.bat` to build, then `PLAY.bat` to launch the test world.
`PLAY.bat` opens your saved map (University by default); use the in-game menu to switch maps, or drag a `.skate` file onto `PLAY.bat`. An SDL3-compatible gamepad is required for gameplay;
Escape opens difficulty and graphics settings.

Development builds use a prepared asset set in `assets/private/` or the
installed asset directory. `scripts/Build-Release.ps1` builds the portable Windows
package and requires Python 3.13. GitHub Actions builds `main` automatically;
numbered releases are published separately.

Custom animations and climbing support remain available, but no custom clips
are shipped. The included format-demo map is original procedural content.

Implementation notes are in [`docs/`](docs/). Patched Bevy dependencies and
their licenses are in [`vendor/`](vendor/). This is an unofficial project,
not affiliated with EA.

## Advanced diagnostics

For the headless server and direct client connections, see the
[dedicated multiplayer guide](docs/multiplayer/README.md).

Windows builds support opt-in [performance timeline capture](docs/performance-tracing.md)
through the `--trace` CLI option, including optional GPU pass diagnostics.

## License

Copyright (c) 2026 dumbad and the Skate 3 Rust Engine contributors.
Unless otherwise noted, this project's original code is licensed under the
[GNU General Public License version 3 only](LICENSE) (`GPL-3.0-only`).
You may use, modify, and distribute it, including commercially. If you distribute
a modified version or a binary of the covered software, you must also make its
corresponding source available under GPLv3 and preserve the required notices.

Third-party code retains its existing licenses and copyright notices, including
the vendored Bevy crates and tooling under `tools/vendor/`. This license does
not grant rights to Electronic Arts' game code, data, assets, or trademarks,
or to content supplied by other map and mod authors.
