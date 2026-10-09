# Linux development

Linux builds run without Steam by default and use raw local controller input. You need a
current stable Rust toolchain, Python 3.13 or newer for asset preparation, a
working Vulkan driver, and the native development libraries used by Bevy and
gilrs.

On Debian or Ubuntu, a representative dependency set is:

```sh
sudo apt install build-essential cmake pkg-config clang libclang-dev libudev-dev \
  libasound2-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev \
  libwayland-dev libxkbcommon-dev libvulkan-dev vulkan-tools
```

On Fedora, use the corresponding packages:

```sh
sudo dnf install gcc gcc-c++ cmake pkgconf-pkg-config clang clang-devel systemd-devel \
  alsa-lib-devel libX11-devel libXcursor-devel libXi-devel libXrandr-devel \
  wayland-devel libxkbcommon-devel vulkan-loader-devel vulkan-tools
```

## Build and prepare assets

Build a debug copy with `./BUILD.sh`, or a release copy with
`./BUILD.sh --release`. The script builds the game and native `skate-xiso`
extractor with the lockfile. It does not build or stage the Steam relay.

Add `--steam` only when you want the optional Steam lobby helper described
[below](#optional-steam-lobbies).

Legal Skate 3 game assets must be supplied by you and are never included in
the repository or needed by CI. Prepare an ISO beside the debug executable:

```sh
python3 tools/setup.py --base target/debug/data --game-exe target/debug/skate3rust \
  --source /path/to/owned-skate3.iso
```

For an already extracted disc, point `--source` to its `default.xex` (or its
containing directory). For a release build, replace both `debug` path segments
with `release`. Setup publishes transactionally into the executable-adjacent
`data/` directory.

`SKATE_XISO` may name another trusted native extractor. `SKATE3_ASSETS` selects
an existing prepared asset directory, `SKATE3_MODS` selects the mods directory,
and `SDL_GAMECONTROLLERCONFIG` adds local SDL controller mappings. Repository
`assets/` is passed explicitly by `PLAY.sh` when it exists.

## Run

Run `./PLAY.sh` for debug or `SKATE_RELEASE=1 ./PLAY.sh` for release. Additional
arguments are forwarded exactly, for example `./PLAY.sh --map University.skate`.
The game requires a working Vulkan driver and desktop graphics/audio stack.

Linux controller input is local and Steam-independent. SDL/gilrs mappings are
translated to the game's XInput-shaped packet format, including raw evdev hat
axes for the D-pad. Up to four controllers are exposed.

glibc is the normal development target. On musl hosts `BUILD.sh` preserves
existing `RUSTFLAGS` and disables static CRT linkage so system X11, Wayland,
audio, Vulkan, and udev libraries can remain dynamically linked.

## Live map

The compact live 3D map is enabled during gameplay in singleplayer and
multiplayer. Press **M** or **D-pad Left** to toggle the larger live map. It
shows the active world, **YOU**, and connected players with their available
names. In the expanded map, left-drag or arrow keys pan freely. Drag the compass
or use Alt + Left/Right to rotate; click it or press Shift + Ctrl + Up to face
north. The 2D/3D button or Ctrl + D switches to/from top-down; its vertical
slider adjusts tilt. Double-click zooms in (Alt-double-click zooms out);
wheel, +/- buttons, Ctrl + +/- and D-pad Up/Down also zoom. YOU or Ctrl + L
recenters on the skater. Pixel trackpad scrolling pans; Linux's current backend
does not expose native pinch/rotation gestures.

Expanded navigation blocks local skating input while the simulation continues.
Opening a menu collapses it; map controls are inactive during menus, replay,
loading, and browser input. View switching preserves the free center and zoom,
and closing returns to the player-following compact map.

The overview is generated from the loaded geometry, including distant streamed
areas, and is replaced when a map change commits. Dense geometry is reduced to
keep its cost bounded. Worlds without usable geometry show **Map unavailable**.

## Optional Steam lobbies

The existing Steam lobby mode can use a separate native helper on x86-64 Linux
with glibc (`x86_64-unknown-linux-gnu`). Build and stage it explicitly:

```sh
./BUILD.sh --steam
# Or combine the options in either order:
./BUILD.sh --release --steam
```

The relay is built for that explicit native target. The staging script locates
the locked `steamworks-sys` 0.13.0 SDK through Cargo metadata, then copies
`skate-steam-relay` and `libsteam_api.so` into `target/debug/steam-relay/`, or
`target/release/steam-relay/` for release. Executable permissions are preserved.
It rejects musl and other Rust hosts, missing files, unexpected SDK versions,
and `STEAM_SDK_LOCATION` overrides. Other architectures have not been enabled
by these scripts.

These launch scripts require Cargo's default native output layout. Leave
`build.target` unset in Cargo configuration, and unset `CARGO_BUILD_TARGET`
and `CARGO_TARGET_DIR`; `--steam` rejects `CARGO_BUILD_TARGET` before building.
The standalone staging helper respects Cargo's reported target directory, but
`PLAY.sh` looks under the repository's `target/` directory.

Open the native Linux Steam client and sign in, then launch with `./PLAY.sh`
(or `SKATE_RELEASE=1 ./PLAY.sh`) and select Steam multiplayer in the game. The
game launches the helper from its staged directory and supplies that directory
in the helper's library search path. No global `LD_LIBRARY_PATH` change is
needed. Two Steam players need separate Steam accounts. The helper currently
uses the development App ID 480.

The ordinary `./BUILD.sh` and `./BUILD.sh --release` commands remain
Steam-independent. [Dedicated multiplayer](multiplayer/README.md) uses direct
UDP with `--connect`; it does not need Steam or this helper. Steam lobbies do
not provide a Steam transport for the dedicated server.

## Resource platform companions

`./BUILD.sh --browser` builds the optional `skate-browser-host` companion beside
`skate3rust`. Building it requires the WebKitGTK4.1 development package (on
Debian/Ubuntu, `libwebkit2gtk-4.1-dev`); running it needs a working X11/Wayland
session and delegated cgroup v2 memory/pids controllers. The page process fails
closed if those controls are unavailable. See the [browser host guide](multiplayer/browser-interfaces.md)
for bounds and diagnostics. Existing canvas and menu resources do not need it.

Voice builds the Opus codec and uses ALSA through CPAL (`libasound2-dev` at build
time). Capture requires the game's explicit `--voice` option and physical V
push-to-talk. Resource radio policy cannot override local opt-out, mute or deafen.
See [voice setup and device tests](multiplayer/voice.md).

For C# resources, install .NET10 and `bubblewrap`/`util-linux`, publish the trusted
worker from `crates/skate-mods/managed-host`, and configure `SKATE_DOTNET_ROOT` and
`SKATE_MANAGED_HOST` as documented in the [resource SDK](../sdk/RESOURCES.md).
A downloaded C# resource contains source, not arbitrary DLL/SO plugins. The
worker requires unprivileged user namespaces and its sandbox prerequisites; it
fails closed when they are unavailable.

Server Lua and local Lua mods can customize the map through the [map SDK](../sdk/MAP.md).

## Native interior packages

The native location loader discovers a catalog beside a map at
`locations/<lowercase-map-stem>/catalog.json`; for `maps/DownTown.skate` this is
`maps/locations/downtown/catalog.json`. Use `--locations DIRECTORY` on the game
or dedicated server to select an explicit package. An absent optional catalog
leaves the map usable. Dedicated clients receive the server's catalog through
required resource admission; their local package cannot override it.

See [the interior SDK](../sdk/LOCATIONS.md) and
[the two-floor example](../sdk/examples/interior-catalog/). Engine code owns model
loading, native collision, input, markers, minimap context and travel. Mods supply
bounded catalogs; server resources must declare and receive both
`resource.locations` and `resource.teleport` for catalog travel.

The Downtown placement configuration is in
[`resources/interiors/downtown`](../resources/interiors/downtown/). Large source and
runtime payloads remain in ignored local storage. The supplied apartment's source
JPEGs visibly contain diagonal distortion; a successful GLB validation does not
establish texture correctness. Do not treat that runtime candidate as a visually
accepted apartment installation until an unmodified source is available.
