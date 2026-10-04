# Voice

Launch a dedicated client with `--voice` to opt in to microphone and playback
devices. Hold **V** to talk; **Ctrl+M** toggles local mute, **Ctrl+D** toggles local
deafen. Losing window focus, opening the game menu, or focusing resource browser
UI releases push-to-talk. A resource cannot bypass the opt-in, physical key,
local mute, or local deafen. Voice is off by default and only runs in a connected
dedicated session. LAN hosts retain their existing trust model; use the
[account-required TLS/AEAD mode](accounts-and-administration.md) for authenticated,
encrypted voice and gameplay.

The game opens default input/output devices on its audio worker after connection.
Device discovery and selection are available through `resource.voice.submit`
with `engine.voice` granted to the client resource:

```lua
resource.voice.submit({kind="devices"})
resource.on("voice_result", function(event)
    -- Device enumeration: event.kind == "devices", event.value.inputs/outputs.
    -- Opening/ready/error events describe the actual asynchronous device work.
end)
resource.voice.submit({kind="configure", input_device="0:device name",
    output_device="0:device name", muted=false, deafened=false})
resource.voice.submit({kind="transmit", pressed=true, channel="voice-room/crew"})
-- Use channel="" for proximity. pressed=true still requires physical V.
```

Omit device identifiers to select OS defaults. Identifiers come from the current
device list, are at most256 bytes, and can change when devices are removed.
Missing devices produce a bounded error; refresh the list and select again.
The shared Lua, JavaScript, and C# resource surfaces submit the same JSON schema.
Commands require a live owner generation. Stop, restart, disconnect, connection
replacement, instance change, and movement reset retire applicable buffered audio
and stale callbacks. Resource device overrides are removed on owner teardown;
the user's opted-in proximity voice returns to default devices. Local user mute
and deafen remain in effect.

On the server, grant `resource.voice` and use these operations:

```lua
resource.voice.submit({kind="channel", name="crew", members={actorA, actorB}})
resource.voice.submit({kind="remove_channel", name="crew"})
resource.voice.submit({kind="mute", player=actorA, muted=true})
resource.voice.submit({kind="proximity", meters=15})
```

Actor IDs are canonical decimal strings from `resource.players()`. Radio channel
names are resource-owned (`resource-id/name`), at most64 ASCII letters, digits,
underscores, periods or hyphens per segment. Membership permits transmission
and reception. A client cannot grant itself membership. Radio bypasses distance
but always respects instance isolation. Resource mute policies combine by union;
active proximity policies combine by minimum radius (default20m, allowed1–100m).
Stopping or restarting an owner removes its channels, mute list and radius.
The server invokes `voice_result` with the actual operation result. These API
grants confer policy authority, so only grant them to trusted server resources.

Voice is carried by protocol kinds60/61 over the existing admitted UDP transport.
The host validates the actual sender endpoint, admitted actor, current movement
epoch, routing revision, and sequence before forwarding. The account-required
transport adds direction-separated ChaCha20-Poly1305 and its own replay window.
Voice has a separate128-packet replay window as well. Position and instance come
from the server's gameplay observation; proximity inherits the game's
owner-reported movement authority and is not a claim of fully authoritative
character movement. Recipients reject stale routing revisions and movement
epochs before decoded samples enter the worker. Policy changes flush pending
speech and are repeated to clients every100ms to recover UDP loss.

## Codec and budgets

The codec is real Opus: mono48kHz,960 samples/20ms,24kbps target, complexity5,
DTX, in-band FEC and10% expected loss. Native device audio is downmixed and
resampled to48kHz; playback is resampled to the output device rate. Supported
native formats are f32, i16 and u16, with8–192kHz and1–32 channels. A known
device buffer is requested at480 frames and capped at4096; driver-reported
unknown buffers use the driver's default. End-to-end device latency therefore
depends on the platform/driver and is not guaranteed by the20ms wire frame.

| Boundary | Limit |
| --- | --- |
| Encoded Opus payload |400 bytes/frame, exact20ms decoder duration |
| Per-sender ingress |50 frames/s, burst5 |
| Admitted voice players |64 |
| Selected/mixed speakers per recipient |8 |
| Server recipient queue |8 senders ×2 frames each; oldest from that sender dropped |
| Host voice egress |512 datagrams/tick by default, after gameplay; fair across recipients and senders |
| Radio channels |16/resource,64 total;64 members/channel |
| Voice owners |128 generation-tracked IDs |
| Client network queue |64 packets |
| Worker commands / PCM input / PCM output / encoded output |128 /8 /4 /8 |
| Decoder jitter queue |4 frames/speaker; one-frame initial delay |
| Loss concealment |FEC from next frame or PLC, at most3 consecutive missing frames |
| Device callback events |32; device errors4 |
| Resource command JSON |4KiB plus typed field validation |

`SKATE_VOICE_EGRESS_PACKETS` may select16–512 datagrams per Host tick; invalid
values fail before listening. The CLI targets100Hz, so the default permits up to
51200 voice/control datagrams per second. One simultaneous frame from each of
eight talkers produces504 recipient copies across64 players;512 permits that
batch to drain in one Host step even when load reduces the tick frequency.
Eight full-rate talkers require25200 datagrams/s before periodic control traffic.
At the400-byte payload ceiling, worst-case voice traffic alone is roughly12MiB/s before
UDP/IP and authentication overhead. Actual24kbps Opus speech is substantially
smaller. Slower host embedding or a smaller configured budget reduces capacity.
`Host::voice_metrics()` exposes accepted/rejected frames, queue drops and sent
packet/byte totals. Queue drops count sender-queue overflow and replacement by a
nearer selected sender; intentional policy/epoch retirement is not counted.
The sent totals count router output, before socket success or simulated loss.
Under congestion the server drops old frames per sender and
rotates sender selection instead of allowing the last speakers in every batch to
starve the first ones. Queues remain bounded and gameplay is emitted first.
The client retires idle decoders after2 seconds. Audio callbacks use bounded
nonblocking channels; device open/enumeration/codec work occurs on one worker,
outside gameplay. Native driver calls can themselves stall: teardown mutes
immediately and waits at most100ms, leaving a stalled worker to finish without
blocking world shutdown. A game holds one worker and resource restarts do not
create additional audio workers. Physical device/driver failure remains an OS
boundary; no realtime scheduling or hardware latency guarantee is claimed.

## Dependencies and verification

`skate-voice` keeps routing/wire support independent of codec and device features.
The dedicated server uses `default-features=false` and does not initialize audio
hardware. The game enables `devices`, which includes the codec. CPAL0.15.3
(Apache-2.0, already used by the game's audio stack) provides platform device
backends. `opus`0.4.0 (MIT/Apache-2.0) wraps `opusic-sys`0.7.5 (BSD-3-Clause),
which builds its bundled upstream Opus source with CMake. This avoids requiring
players to install a system Opus library. The new crate forbids unsafe Rust;
native audio and codec FFI remains inside those established dependencies.
Preserve their distributed license/copyright files in binary redistribution.

Run `cargo test --locked -p skate-voice --features devices` and
`cargo test --locked -p skate-server --test voice`. The deterministic suite sends
generated PCM through the actual Opus encoder, real loopback UDP sockets, server
routing, and the decoder. A separate Linux child process uses CPAL's actual ALSA
capture/playback streams with a synthetic input file and null playback device,
then checks mute/deafen and stream descriptor cleanup. It does not record a
person or transmit audio externally. The server integration uses real TLS login,
encrypted UDP, server Lua radio/mute/teleport policy and resource retirement.

Physical microphone/speaker acoustics, native Windows devices and desktop
permission prompts still require native manual validation. This is the Skate
voice transport and API, not Mumble wire compatibility or a binary FiveM voice
resource drop-in.
