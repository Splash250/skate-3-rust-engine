# Rust crate guidance

## Boundaries

- `skate-core` owns engine-independent recovered calculations: no rendering,
  filesystem I/O or ECS. `skate-data` owns data loading and validation; stateless
  codec math stays in core. Both crates forbid unsafe code.
- `skate-game` owns scheduling and host integration; read
  [its guide](skate-game/AGENTS.md) before changing that crate.
- For mod runtime/API work, read [the SDK guide](../sdk/AGENTS.md). Keep runtime
  wrappers, host behavior and public declarations consistent.
- Keep platform-specific implementations behind the existing target gates and
  `skate-platform` abstractions. Linux setup prerequisites are in
  [docs/LINUX.md](../docs/LINUX.md).
- Bevy uses local workspace patches. For changes affecting those patches, read
  [vendor/README.md](../vendor/README.md), preserve upstream licenses and update
  the patch explanation alongside the implementation.

## Choose the validation scope

From the repository root, substitute the actual package/test/filter:

```sh
cargo check --locked -p <package>
cargo test --locked -p <package> <filter>
cargo test --locked -p <package> --test <integration-test-target>
```

Choose the applicable check, not every command. Integration-test target names
come from `tests/` or `cargo metadata --no-deps --format-version 1`.
For example, the collision fixture suite is
`cargo test --locked -p skate-data --test retail_collision`.
Use `-- --list` when uncertain about a test filter and confirm tests ran.

For cross-crate interface or workspace dependency changes, check affected
consumers and use `cargo check --workspace --locked` as appropriate.
Keep the lockfile stable unless the task changes dependencies.

Development profiles already use optimization level 3. Reuse the existing
`target/` and select a package before triggering a full game/workspace build.
Keep formatting scoped to changed code; the repository is not uniformly
formatted. `rustfmt --edition 2024 --check <file.rs>` checks a selected file
(and may traverse its child modules); inspect the diff before applying formatting.
