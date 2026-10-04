# SDK and mod guidance (API 2)

Use this for SDK changes and, via the root guide, mod work in `mods/` or the Rust
runtime/host. A requested mod is delivered under `mods/` as a folder or `.zip`.
Manifests use `"api": 2`; the entry Lua file returns a callback table.

## Read the relevant API contract

- [GENERAL_API.md](GENERAL_API.md): capabilities, engine access, commands and results.
- [ENGINE_API.md](ENGINE_API.md): player/scoring observations and controls.
- [DEFORMATION.md](DEFORMATION.md): optional deformation and its limits.
- [skate.lua](skate.lua): language-server declarations, not executable runtime code.
- [examples/physics-sandbox/](examples/physics-sandbox/): working primitive-based mod.

For implementation changes, follow the chain from
[runtime wrappers](../crates/skate-mods/src/api.lua) through the
[command schema](../crates/skate-mods/src/vm.rs) to the
[game host](../crates/skate-game/src/modding/) and
[dynamics](../crates/skate-dynamics/). Update applicable declarations and examples
when changing the contract; also read the Rust guides for files you edit.

## Build with primitives

Vehicle, wheel/suspension, injury and challenge rules belong in Lua. Use bodies,
colliders, forces, joints/motors, sensors, graphics, player attachment and camera
controls; there is no `sdk.vehicle`. Any added host API must be reusable across
mods, without depending on a specific mod ID or game mode.

## Validate the delivered package

From the repository root, replace the final argument with the actual package:

```sh
cargo run --locked -p skate-mods --example check_mod -- sdk/examples/physics-sandbox
```

Manifest must use `"api": 2`. Return a callback table from the entry Lua file. Optional `"enabled_by_default": false` keeps a dev / test mod off until the player enables it (or `SKATE3_MODS_ENABLE=<id>[,<id>]` for one run); a saved preference always wins.

## Building blocks

Physics bodies (box/sphere/capsule/convex/`mesh` from named GLB objects), extra
`add_collider` hulls, forces/joints(+motors)/sensors, graphics meshes/overlays,
player attach/detach, camera follow/set, `sdk.assets.objects`, `sdk.input.pad`,
settings/log/timers. No `sdk.vehicle`.

This checks the manifest, bounded entry file and Lua syntax. It does **not**
execute callbacks or verify gameplay. For runtime behavior changes, run the
relevant `skate-mods` tests; for in-game behavior, report the exercised scenario
and any unavailable assets/controller/GPU prerequisites.
