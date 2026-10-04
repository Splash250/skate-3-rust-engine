# Maintaining repository agent instructions

Consult this when changing the instruction system, not before routine coding.
Research reviewed on 2026-10-04. The aim is less repeated discovery and fewer
avoidable mistakes; speed improvements require observation, not an assumption
that adding instructions helps.

## Structure and rationale

| File | Owns |
| --- | --- |
| [Root AGENTS.md](../../AGENTS.md) | Graph navigation, task routing, shared boundaries and completion |
| [crates/AGENTS.md](../../crates/AGENTS.md) | Rust boundaries, package/target selection and build scope |
| [crates/skate-game/AGENTS.md](../../crates/skate-game/AGENTS.md) | Scheduling and game-specific test prerequisites |
| [tools/AGENTS.md](../../tools/AGENTS.md) | Python imports, focused checks and installation invariants |
| [sdk/AGENTS.md](../../sdk/AGENTS.md) | Mod contracts and the distinction between syntax and runtime validation |

Each child adds only what differs for that scope. Root routing explicitly covers
sibling paths such as `mods/`; an SDK guide does not inherit into those paths.
Existing docs and build/CI files remain the sources for detailed procedures.
Avoid a directory inventory, copied dependency versions, transient machine
failures or a second set of instructions in another agent-specific filename.

This uses ordinary `AGENTS.md`, with no per-user configuration or override files.
Codex's startup discovery follows the path from repository root to its working
directory, so the root also tells agents to read scoped files before editing
deeper paths. `AGENTS.override.md` can shadow the regular file at a directory;
account for that when diagnosing missing guidance. See
[OpenAI's discovery rules](https://learn.chatgpt.com/docs/agent-configuration/agents-md).

## Keep a change only if it earns its place

1. Identify a recurring wrong decision, expensive lookup or non-obvious invariant.
   Put its corrective instruction at the narrowest useful scope.
2. Give references a trigger: which task needs the document? Link existing
   documentation rather than making every agent reread it or copying its body.
3. Verify paths, commands, working directory, prerequisites and actual test
   selection. Distinguish a command checked against configuration from one run.
4. Review root plus the affected child chain for contradictions and duplication.
   Keep the root to roughly one short screenful per major concern; use 100 lines
   as a review trigger, not a target. Nested files should usually be shorter.
5. Remove the rule when code/tooling makes it unnecessary. Update commands with
   their owning workflow; avoid appending lessons indefinitely.

For a discovery smoke check in a fresh agent session, ask it to list the active
instruction files from the repository root and from `crates/skate-game`, `tools`
and `sdk`. For a task begun at root, confirm it follows the matching scoped link.
Check for shadowing overrides and truncation if it misses a file. Codex's default
combined project-instruction budget is 32 KiB; that is a ceiling, not a goal.
[Discovery and verification documentation](https://learn.chatgpt.com/docs/agent-configuration/agents-md).

Evaluate changes on representative tasks: a data-parser fix, a setup recovery
fix, a shader change and a Lua mod. Compare successful validation, unnecessary
file reads, zero-test selections, repeated approval requests and elapsed time.
Hold task/model/settings constant where possible and repeat runs before claiming
a speedup. Delete guidance that adds work without preventing a real error.

## Research behind the choices

- The [AGENTS.md project](https://agents.md/) defines a plain Markdown format
  and scoped nested guidance. We use one shared root and a few distinct scopes.
- [OpenAI's guidance on instruction maintenance](https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra)
  recommends contextual document pointers, pruning obsolete requirements and
  avoiding compulsory broad reading for small changes. We apply that through
  routing, focused check selection and explicit local-work autonomy.
- [Gloaguen et al., Evaluating AGENTS.md, v3](https://arxiv.org/abs/2602.11988v3)
  found no general task-success improvement and higher average inference cost;
  non-standard practices are more defensible content than repository overviews.
- [Lulla et al., On the Impact of AGENTS.md, v2](https://arxiv.org/abs/2601.20404v2)
  observed efficiency improvements in a different sample of repositories and
  tasks. Together these studies motivate local evaluation; they do not establish
  a universal best template or a guaranteed speed gain for this repository.
