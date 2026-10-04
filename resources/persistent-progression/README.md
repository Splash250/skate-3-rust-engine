# Persistent progression example

This server-only resource uses the actual asynchronous SQLite backend. It has no
downloaded client script or private-asset prerequisite. Add
`persistent-progression` to the server resource configuration's `ensure` array
and grant `resource.database`, `resource.events`, and `resource.commands` to it.

After `Progression ready` appears, use the dedicated console:

```text
command progress_award local-account 100
command progress_buy local-account
command progress_inspect local-account
```

The balance is 75 and inventory contains one `deck_blue`. Stop the process and
restart with the same configuration path and storage directory; inspection keeps
the same balance and item. Three more purchases reach zero. A further purchase
fails the nonnegative-balance constraint and rolls back the whole transaction,
including the inventory increment. A purchase on an unknown account fails its
foreign-key constraint. Concurrent purchases serialize as SQLite transactions.
`busy` means retry the whole operation after the outstanding work completes.

The administrator supplies account labels. This example does **not** implement
authentication or convert connection IDs/client-supplied labels into trusted
identities. Player-facing rewards must bind these operations to a verified
account and independently verified server gameplay before exposing them to
network events. `progression.admin` gates each command; only the local trusted
console currently supplies that authority.

Schema version 1 is replayed and checked on resource/process restart. Keep applied
migrations immutable; append version 2 for upgrades. Ordinary resource stop,
restart or unload retains the database. Server scripts and databases stay outside
public resource content. See [backend services](../../docs/multiplayer/backend-services.md)
for limits, cancellation semantics, ownership and backup considerations.

The shipped manifest and script are exercised without replacement by
`cargo test --locked -p skate-server --test backend_example`. This starts real
server processes, sends the commands above, and verifies successful purchases,
rollback, resource restart and whole-process restart through the script's actual
completion logs. The SQLite concurrency and isolation checks are in
`cargo test --locked -p skate-services`.
