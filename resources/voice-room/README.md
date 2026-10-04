# Voice room

Enable this resource and grant all five capabilities in its manifest. Each client
opts in with `--voice` and holds **V** to speak. Proximity is available immediately;
this example adds administrator-assigned radio membership and moderation.

From the server console, list and use actual admitted player IDs:

```text
command voice_players
command voice_radio crew PLAYER_A PLAYER_B
command voice_mute PLAYER_A true
command voice_mute PLAYER_A false
command voice_clear crew
```

`voice_radio` accepts2–64 distinct admitted players and selects that resource-owned
channel on their clients. Replacing the channel removes previous membership.
`voice_clear` removes it and returns its current members to proximity. Radio
never crosses instance boundaries. A client-selected channel without membership
cannot transmit. Commands require `voice.admin` when invoked through a permission
checked command path; the local host console has operator authority.

Stopping this resource revokes channels/mute policy and clears its client channel
selection. Client local mute/deafen and the physical push-to-talk requirement
remain authoritative. The example stores no passwords and does not map caller-
supplied account labels to identity. Use the account-required server mode for
verified identities and encrypted transport.

See [voice devices, codec, limits and validation](../../docs/multiplayer/voice.md).
