# Repository agent guide

This is a Rust/Bevy engine with Python asset tooling and a Lua mod SDK.
Commands below and in scoped guides run from the repository root unless noted.
Before editing a path, read its applicable nested `AGENTS.md` files; they extend
this guide, with the closest file taking precedence for its scope.

## Find the relevant context

- When `graphify-out/graph.json` exists, begin codebase investigation with
  `graphify query "<question>"`. Use `graphify path "<A>" "<B>"` for relationships
  and `graphify explain "<concept>"` for focused concepts. Verify findings in
  current source before editing; the graph is a navigation aid.
- Use `graphify-out/wiki/index.md` for broad navigation when present. Read
  `graphify-out/GRAPH_REPORT.md` only for an architecture review or when scoped
  queries are insufficient. Dirty graph files are expected and do not justify
  skipping it. For `/graphify`, read the installed skill first.
- If the graph is absent or the command fails, report the limitation and use
  targeted `rg` searches. Skip graph queries when investigating incorrect graph
  output or when the user explicitly requests it.
- Read only the guides relevant to the task:

| Work | Guidance |
| --- | --- |
| Rust crates, tests, dependency boundaries | [crates/AGENTS.md](crates/AGENTS.md) |
| Game scheduling, rendering, runtime integration | [crates/skate-game/AGENTS.md](crates/skate-game/AGENTS.md) |
| Python tooling or `BUILD.sh` / `PLAY.sh` | [tools/AGENTS.md](tools/AGENTS.md) |
| Lua mods, SDK or mod host APIs, including work in `mods/` | [sdk/AGENTS.md](sdk/AGENTS.md) |
| Linux build, launch, controllers, owned-asset setup | [docs/LINUX.md](docs/LINUX.md) |
| Multiplayer terminology and intended authority model | [CONTEXT.md](CONTEXT.md); verify implementation separately |
| Vendored Bevy changes | [vendor/README.md](vendor/README.md) and workspace `[patch.crates-io]` |

## Work within the requested change

- Inspect `git status --short`; preserve unrelated edits and untracked files.
  Keep fixes scoped, and reuse existing abstractions and dependencies.
- Complete authorized local edits and focused checks without adding approval
  checkpoints for routine reversible choices. Ask when an unresolved choice
  materially changes behavior, compatibility, dependencies, persistence or scope.
- Delegate independent work with explicit ownership when it helps; verify the
  result. Avoid concurrent writes to the same files or builds sharing `target/`.
- Keep retail dumps, extracted assets, private maps and credentials out of
  patches. Use synthetic or existing redistributable fixtures. Preserve
  third-party licenses and provenance, including under `vendor/` and `tools/vendor/`.
- Commit, push, publish, deploy or change external systems only when requested.

## Finish with evidence

- Run the smallest check that exercises the change; broaden for affected callers
  or shared interfaces. Use scoped guides for commands. For documentation-only
  edits, check links, command references and the diff; a game build is unnecessary.
- Report actual results, skipped prerequisites and pre-existing failures.
  A zero-test run is not evidence that the intended regression passed.
- After code changes, run `graphify update .` (AST-only, no API cost) and report
  failures. Documentation-only edits do not require a full graph rebuild.
- Keep these instructions focused on recurring repository-specific decisions.
  When changing their structure, use [the maintenance note](docs/agent-guidance/README.md).
