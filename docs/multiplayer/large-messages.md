# Large resource messages

API1 adds explicit request keys, acknowledgement progress, cancellation and
deadlines to the existing bounded bulk transport. Use this for resource data
whose delivery needs to be observed; ordinary events over384 encoded JSON bytes
already use the same bulk lane automatically.

```lua
resource.on("transfer_progress", function(event)
    sdk.log(event.key .. ": " .. event.state)
end)
resource.transfer.start("inventory-page", "inventory_data", data, {
    recipient=player_id, -- server requires one admitted player; omit on client
    timeout_ms=10000,
})
resource.transfer.cancel("inventory-page")
```

JavaScript exposes `resource.transfer.start/cancel`; C# exposes the equivalent
shared resource API. Requires `resource.events` and a live generation. Received
payloads arrive through the usual authenticated `resource.on_net` handler.
Server transfers address one explicit recipient; broadcast callers choose their
recipient set and handle each result. Client transfers address the server.

Progress events contain `key`, `state`, `acknowledged_bytes`, `total_bytes` and
`error`. States are queued, sending, delivered, cancelled, timed_out or failed.
Byte counts include the encoded message envelope. Delivered means the peer's
transport acknowledged the complete value, not that its script callback
succeeded. An application needing commit confirmation must send its own reply.
A cancellation cannot undo a callback that already ran at the remote peer.

The negotiated `network_budgets.value_bytes` applies to both ordinary and explicit
messages: default16KiB, hard256KiB. The sender's runtime payload budget must also
allow the value. All lanes share queue byte and pending-count limits; backpressure
returns a per-request error. Each host permits128 tracked requests and each
resource16, with1..120000ms deadlines. Keys are unique while pending. The wire
uses192-byte chunks, only acknowledges exact sent boundaries and keeps bounded
cancellation tombstones until acknowledged. Movement remains separately scheduled.

Handles are bound to resource generation, recipient and activation epoch.
Disconnect, instance migration, reconfiguration and resource retirement cannot
reassign a handle to a new connection. Resource teardown cancels its pending work;
callbacks are suppressed after ownership retirement. The last256 wire completions
are retained for bounded progress lookup; expired handles report failure.

The graphical client reads optional `network-budgets.json` from its resource
cache directory (`SKATE3_RESOURCE_CACHE` overrides the default). It accepts the
same network-budget fields as the server and negotiates the minimum of both
sides. For example, `{"value_bytes":262144}` allows values up to256KiB when the
server also enables them. Client VM payload/count budgets follow the negotiated
limits while retaining the64MiB aggregate allocation guard. This local file
must be a regular file no larger than16KiB; malformed or out-of-range budgets
fail activation with a diagnostic.
