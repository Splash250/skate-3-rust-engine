# Linux Controller Runtime Design

## Intent

Make the current game usable with controllers during Linux development while
preserving its existing Windows XInput behavior and TU3 input conversion. This
phase changes only platform input. Asset preparation, Linux scripts, CI, Steam
work, and dedicated-server work are separate phases.

Success means:

- Linux polling returns controller packets instead of `UnsupportedPlatform`.
- Windows continues to call the same XInput APIs with the same structures,
  error handling, subtype lookup, and cache lifetime.
- Linux input reaches `skate_core::input::xbox::convert` as XInput-compatible
  button, trigger, and stick values without a second gameplay mapping layer.
- Automated tests cover mapping, numeric conversion, D-pad hats,
  disconnection, stable slots, and capability caching.

## Constraints

- Do not import Linux Steam relay support, Steam feature gates, or
  `libsteam_api.so` handling.
- Do not change gameplay, controller action mappings, TU3 conditioning, or
  Bevy scheduling.
- Linux is the only new supported platform. Other non-Windows targets remain
  unsupported; macOS support is explicitly out of scope.
- Keep Bevy's `GilrsPlugin` disabled. The platform adapter is the single owner
  of controller polling.
- Preserve the untracked `CONTEXT.md` and unrelated working-tree changes.
- The reference fork has no repository-level license. Reimplement its design;
  copy only the separately licensed SDL controller database with its license
  and provenance intact.

## Architecture

Create a `skate-platform` workspace crate containing the platform-neutral
packet/error/cache interface and OS-specific transports. The existing
`skate-game::input::platform` module becomes a narrow re-export so current game
call sites and tests do not need structural changes.

The public adapter contract remains:

```rust
pub struct DevicePacket {
    pub number: u32,
    pub state: XboxState,
    pub subtype: u8,
}

pub enum DeviceError {
    Disconnected,
    State(u32),
    Capabilities(u32),
    UnsupportedPlatform,
}

pub fn poll_cached(
    index: usize,
    cache: &mut CapabilityCache,
) -> Result<DevicePacket, DeviceError>;
```

`poll_cached` accepts only the four existing device slots. Windows dispatches
to XInput, Linux dispatches to the gilrs transport, and other targets return
`UnsupportedPlatform`.

## Windows transport

Move the existing implementation without behavioral changes:

- retain the installed Windows SDK ABI layouts and size assertions;
- call `XInputGetState` for every host sample;
- map error 1167 to `Disconnected` and retain other numeric errors;
- invalidate capability metadata after state errors;
- call `XInputGetCapabilities(index, 1, ...)` through the existing one-second
  cache;
- retain subtype `7` handling in the game controller collector.

The Windows module remains behind `cfg(windows)`. No gilrs dependency is built
for Windows by this crate.

## Linux transport

The Linux module owns one thread-local backend because gilrs polling and state
updates must share a persistent instance. The game polls on its existing main
thread system.

Initialization uses `GilrsBuilder` with:

- default event filters disabled, avoiding gilrs deadzone and jitter changes;
- environment mappings enabled for `SDL_GAMECONTROLLERCONFIG` overrides;
- gilrs's bundled mapping snapshot disabled so the selected vendored snapshot
  is the single reproducible built-in source;
- the vendored SDL controller database added explicitly;
- force feedback disabled because this adapter does not expose rumble.

Each host polling pass drains pending events so gilrs updates its cached device
state. A slot allocator then:

1. removes disconnected device IDs from their existing slots;
2. keeps every still-connected ID in its previous slot;
3. assigns newly connected IDs, sorted by gilrs ID, to the lowest free slots;
4. ignores connected devices beyond the four-slot game limit.

Polling an empty slot invalidates its capability cache and returns
`Disconnected`. Successful samples increment a wrapping packet number and use
subtype `1`, matching an ordinary gamepad.

## XInput-compatible conversion

Conversion helpers are pure functions so they can be tested without hardware.

- Negative sticks scale by 32768, making `-1.0` exactly `-32768`.
- Positive sticks scale by 32767, making `1.0` exactly `32767`.
- Stick inputs are clamped to `[-1.0, 1.0]` and rounded to the nearest integer.
- Analog trigger/button values are clamped to `[0.0, 1.0]`, multiplied by
  255, and rounded.
- When a trigger has no analog button value, its `LeftZ`/`RightZ` axis fallback
  maps `[-1.0, 1.0]` to `[0, 255]`.
- Face, shoulder, thumb, menu, and D-pad buttons map to the existing XUSB bit
  values expected by `XboxState`.

With default filters disabled, evdev hats can remain `DPadX`/`DPadY` axes.
Their values supplement button-derived D-pad bits using thresholds:

- `DPadY > 0.5`: up (`0x0001`)
- `DPadY < -0.5`: down (`0x0002`)
- `DPadX < -0.5`: left (`0x0004`)
- `DPadX > 0.5`: right (`0x0008`)

Button and hat sources are ORed, so either representation works without
discarding a valid button mapping.

## Mapping data and provenance

Vendor revision `555ce569a2003b22a4e134882224f1e2bdecc2e3` of the canonical
SDL_GameControllerDB and its zlib license as `gamecontrollerdb.txt` and
`gamecontrollerdb.LICENSE.txt`. Add the source URL and revision to
`docs/THIRD_PARTY_NOTICES.md`.

Do not copy the fork's private `EXTRA_MAPPINGS` string. Its surrounding fork
has no license and the exact override is not separately attributed. Users can
provide equivalent hardware-specific mappings through
`SDL_GAMECONTROLLERCONFIG`; a later change may add the mapping if it appears in
the licensed canonical database.

## Game integration

Add `skate-platform` to the workspace and to `skate-game`. Replace
`crates/skate-game/src/input/platform.rs` with crate-local re-exports of the
adapter API. Keep `ControllerInput::collect` and
`skate_core::input::xbox::convert` unchanged.

Update the readiness log from Windows-specific wording to a neutral raw-device
message. No input scheduling, focus selection, multiplayer controller choice,
or action publication changes are required.

## Tests

Tests live beside the new platform adapter and exercise real conversion and
state-management code through pure inputs:

- every supported logical button maps to its literal XUSB bit;
- stick endpoints, zero, representative fractional values, clamping, and the
  asymmetric negative endpoint convert correctly;
- trigger analog values and axis fallback convert to literal byte values;
- hat axes produce each D-pad bit, neutral values produce none, and button and
  hat bits combine;
- slot allocation preserves connected device positions, clears a disconnected
  slot, and fills the lowest free slot on reconnection;
- polling an absent slot reports `Disconnected` and invalidates cached
  capability data;
- the one-second capability cache refreshes at expiry, never caches errors, and
  honors explicit invalidation;
- existing `skate-game` controller tests continue proving packets reach TU3
  conversion, disconnections zero gameplay actions, and subtype errors remain
  visible.

Hardware validation is separate from automated verification: if no controller
is available, the final report must say that hotplugging, real evdev mappings,
and live axis behavior remain manual checks.

## Expected files

- Modify `Cargo.toml`
- Modify `Cargo.lock`
- Create `crates/skate-platform/Cargo.toml`
- Create `crates/skate-platform/src/lib.rs`
- Create `crates/skate-platform/src/input/mod.rs`
- Create `crates/skate-platform/src/input/xinput.rs`
- Create `crates/skate-platform/src/input/gilrs_raw.rs`
- Create `crates/skate-platform/gamecontrollerdb.txt`
- Create `crates/skate-platform/gamecontrollerdb.LICENSE.txt`
- Modify `crates/skate-game/Cargo.toml`
- Modify `crates/skate-game/src/input/platform.rs`
- Modify `crates/skate-game/src/input.rs`
- Create `docs/THIRD_PARTY_NOTICES.md`

## Verification

Run, in order:

1. focused `skate-platform` unit tests;
2. existing `skate-game` controller tests;
3. `cargo test -p skate-platform --locked`;
4. the narrowest complete `skate-game` test target that owns input tests;
5. `cargo check --workspace --locked`;
6. `cargo fmt --all -- --check` after formatting changed Rust;
7. a Windows target check if an appropriate target is already installed;
8. `graphify update .` after code changes.

Any pre-existing failures must be named separately from regressions. No owned
game assets are required for this phase.
