# Call diagnostics dashboard

This plugin supplies an actual packaged in-game dashboard for `phone-calls`.
Install the exact dependencies from `resource.json`; the complete
[RP configuration](../rp-server.json) installs and grants them. The shared
interaction registry exposes **Call diagnostics** in the master menu and phone
only when the configured policy admits the `calls.diagnostics` permission.

The server independently checks the real authenticated sender for every read.
Running **Run invariant test** additionally requires `calls.test`. Grant these
permissions through existing verified account roles; hiding an entry grants
nothing. Different operators can receive read-only diagnostics or testing
rights. This plugin uses the generic `resource.authorized(sender, permission)`
API with the explicit `resource.authorization` capability.

The dashboard shows real counts of ringing, connecting, active and retiring
calls. The explicit read-only test checks current call indexes, distinct
participants, allowed phases, retirement consistency and the 16-slot capacity.
It never creates a synthetic call, joins a voice channel, opens a microphone,
executes commands or evaluates scripts. It reports the actual number of checks
and bounded errors; it does not expose player identities, account data or paths.
These consistency checks complement the native UDP/Opus integration test; they
do not establish microphone or speaker acceptance.

Client requests use unique bounded sequence IDs, one outstanding request and a
two-second deadline. Closing the dashboard discards pending responses. Reads
refresh every 1.1 seconds; private content expires after two seconds without a
fresh authorized result. Denial clears it immediately. The server limits each
actor to one authorized request per second and sends responses privately to
that same actor. Input claims about identity, roles or permissions are ignored.

Keyboard: Tab/arrows navigate, Enter selects, Escape closes. Controller: D-pad
or stick navigation, A selects, B closes. The dashboard has no gameplay binding
of its own; enter through the master menu or phone. Editing its packaged
HTML/CSS/JavaScript changes the presentation without rebuilding the engine.

The two `phone-calls` server exports used here are `diagnostics` and
`invariant_test`. Only declared dependency resources with export grants can
call them; client code cannot call a server VM's exports. Keep authorization
at the network boundary when exposing either to players. No call state is
changed by these exports.
