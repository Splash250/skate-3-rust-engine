# Shared objects and private rooms

This server resource creates a primitive floor, a dynamic crate, and a green
travel marker in each of instances 0 and 7. It needs no private assets. Request
and grant `resource.entities`, `resource.teleport`, and `resource.commands`.

From the server console, use `command objects_room PLAYER_ID 0` to place a player
on the demonstration floor. `command objects_push 0` applies a server impulse.
Walking to the green marker at `(8, 1, 0)` moves a player to the other room at a
predefined destination. The resource checks the server's latest accepted player
observation, with a two-second cooldown. This is a resource policy demonstration;
it does not independently simulate player movement.

Two players in the same room should see and push the same crate. A later join
receives its current position. Players in different rooms receive different
objects and player rosters. Restarting the resource replaces its objects with a
new generation. Console commands require `objects.admin` through the server
command boundary; no client-supplied coordinates are accepted as destinations.

See [shared object contracts](../../docs/multiplayer/shared-entities.md) for
current bounds and verification limits. Graphical two-client acceptance still
requires an actual game run.
