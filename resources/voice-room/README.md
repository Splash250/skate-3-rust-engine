# Voice room

Enable this trusted server resource and grant the capabilities in its manifest.
Its private `proximity_meters` setting defaults to 12 m and applies live. Each
client still opts in with `--voice` and holds **V** to speak. The resource also
adds administrator-assigned radio membership and moderation.

From the server console, list and use actual admitted player IDs:

```text
command voice_players
command voice_radio crew PLAYER_A PLAYER_B
command voice_mute PLAYER_A true
command voice_mute PLAYER_A false
command voice_clear crew
```

`voice_radio` accepts 2–64 distinct admitted players and selects that resource-owned
channel on their clients. Replacing the channel removes previous membership.
`voice_clear` removes it and returns its current members to proximity. Radio
never crosses instance boundaries. A client-selected channel without membership
cannot transmit. Commands require `voice.admin` when invoked through a permission
checked command path; the local host console has operator authority.

The reserved `pizza_dispatch` channel is controlled by the server export
`dispatch_member({actor,enabled})`. The pizza job renews its 15-second membership
lease while a shift is active. Only admitted actor IDs can be added; disconnects,
lease expiry, and job cleanup remove the member, including when the job resource
is retired before it can send a cleanup request. Radio remains instance-scoped and local
push-to-talk, mute, and deafen controls remain in effect.

Stopping this resource revokes channels/mute policy and clears its client channel
selection. Client local mute/deafen and the physical push-to-talk requirement
remain authoritative. The example stores no passwords and does not map caller-
supplied account labels to identity. Use the account-required server mode for
verified identities and encrypted transport.

See [voice devices, codec, limits and validation](../../docs/multiplayer/voice.md).
