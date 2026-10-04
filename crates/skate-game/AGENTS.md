# Game integration guidance

## Runtime changes

- Preserve the scheduling contract in [src/app.rs](src/app.rs): fixed updates
  run Input → Controls → Physics; frame updates run Assets → Physics → Animation
  → Verification. Trace producers and consumers before changing ordering.
- Keep recovered calculations in `skate-core` and loading in `skate-data`;
  integrate them here. For mod host work, also read
  [sdk/AGENTS.md](../../sdk/AGENTS.md).
- Keep the default `dev-dynamic` feature for normal iteration. Static release
  packaging has its own path in [scripts/Build-Release.ps1](../../scripts/Build-Release.ps1).
  Linux build/run prerequisites are in [docs/LINUX.md](../../docs/LINUX.md).

## Tests and prerequisites

This crate has a test-bearing binary, **not a library target**. From repo root:

```sh
cargo check --locked -p skate-game --bin skate3rust
cargo test --locked -p skate-game --bin skate3rust <filter>
```

For retail WGSL/interface changes, the headless Naga validation command is:

```sh
cargo test --locked -p skate-game --bin skate3rust retail_render::shader_tests
```

The module is named `shader_tests` despite the filename `retail_shader_tests.rs`;
filtering by the filename selects no tests. This suite needs no GPU/window, but
compilation still needs the game build dependencies.

Select ignored tests individually after reading their prerequisites. Some stock
tests require `SKATE3_ASSET_ROOT`; normal runtime asset selection uses
`SKATE3_ASSETS`. Other tests have their own environment variables or GPU needs.
Use `-- --ignored` only with the intended test filter and available prerequisites.
Report compile, synthetic-fixture, GPU and owned-asset validation separately;
none alone establishes original-game gameplay or numerical parity.
