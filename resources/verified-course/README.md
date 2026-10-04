# Verified practice course

Ensure `verified-course` and grant its manifest capabilities. Its dependency
selects the redistributable `community-park` world. From the server console:

```text
command course_players
command course_room PLAYER_ID 7
command course_start PLAYER_ID
command course_cancel PLAYER_ID
```

Starting returns the admitted player to the world-approved spawn and creates a
fresh movement baseline. Pass the four checkpoint spheres along the x-axis
from x=-4 through x=8, near y=1/z=0. The server awards275 points only after all
four ordered checkpoints and the gold pickup at(-2,1,-2) pass its trajectory and
terrain checks. Approach within1.25m of the gold sphere; the checkpoints contribute
250 points and the pickup25. It awards once per attempt and stays visible for other participants.
Elapsed time comes from the server clock. The replicated panel shows the latest
result only to players in that competition instance.

`course_room PLAYER_ID 0|7` cancels any active attempt and moves an admitted
player to the approved predefined spawn in the public or private competition
instance. Both instances have their own course, visible pickup entity and scoped
results. Run two players in different rooms to exercise private competition
state. Return with `course_room PLAYER_ID 0`. Instance movement never completes
an attempt. Additional courses use the same reusable host API and owned rules.

The resource receives host-generated `competition_result` callbacks. It has no
client event that submits points, trick names, checkpoint claims or finish times.
Speed/acceleration violations, missing observations, unexpected instance/epoch
changes, and terrain crossings reject the attempt and return the player to the
approved spawn. This strict course can reject a legitimate but delayed or
unsupported native movement sequence; it is not a full skating simulation.

Course policy is reusable through `resource.competition.submit`; no resource ID
has privileged scoring logic. See the [rules and limitations](../../docs/multiplayer/verified-competitions.md).
