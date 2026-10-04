# Managed cross-language example

Original redistributable example under the repository GPL-3.0-or-later license.
Install `js-rules`, `lua-rule-adapter` and `managed-language-demo`, granting their
declared capabilities. Both client and server need the trusted published managed
worker and .NET 10 runtime configured as described in [the SDK](../../sdk/RESOURCES.md#c-resources).

The server calls C# → Lua → JavaScript with fixed input, persists its start count,
and replies privately to the admitted client's ready event. The client displays
the server-calculated preview through `engine.ui`. These are source resources;
the client receives `client.cs` and the dependency's public files, never
`server.cs`. The worker compiles selected source inside the OS boundary. The
sample is deliberately separate from the default Lua/JavaScript launch config
so hosts without the managed runtime reject only an explicitly selected C# pack.
