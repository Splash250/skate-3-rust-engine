# JavaScript and Lua cooperation

These original scripts use the repository GPL-3.0 license and contain no game
assets. `cross-language-demo` runs real JavaScript on client and server. Its
server calls the Lua `lua-rule-adapter` dependency, which calls JavaScript
`js-rules`; the result crosses both VM boundaries as JSON. Server starts persist
in resource-owned JSON storage. A ready event sends a private greeting to the
admitted client, whose script displays it through the existing engine UI.

Select `cross-language-demo` in a server configuration's `ensure` list. Grant
`resource.exports` to `js-rules` and `lua-rule-adapter`; grant the capabilities
listed in `cross-language-demo/resource.json` to that resource. The resolver starts
dependencies automatically. Run the server with `--test-world --resources` and
the configuration path, then connect a game client with `--test-world` as described
in the main resource examples guide. No Node.js installation is needed: the
server/client binaries embed QuickJS.

The expected label is “Checkpoint rule preview: 75”; restarting the server or
resource increments the displayed starts counter. This fixed calculation shows
language interoperability; it is not server-verified skating or an authoritative
competition. Resource stop/restart owns callback/UI cleanup. Server scripts and
storage do not enter downloaded client content.
