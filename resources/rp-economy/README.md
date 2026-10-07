# RP Economy

Server-only wallet and transaction ledger for the roleplay showcase. Durable
balances are keyed by the verified account returned by `platform-profiles`;
client payloads cannot supply an account ID.

Exports:

- `balance({actor})` returns a cached balance or queues an asynchronous read.
- `submit({actor, operation_id, kind, amount, reason})` queues an idempotent
  charge or credit and returns a pending receipt. `kind` is `charge` or
  `credit`; the stable operation ID is scoped to the verified account.
- `operation({actor, operation_id})` reads a cached receipt or queues recovery
  from SQLite. Callers retry a pending wallet operation with the same ID.

The SQLite transaction inserts the operation receipt and updates the wallet
atomically. Duplicate IDs return the original receipt; content mismatches are
rejected. A conditional update records insufficient funds and balance-limit
rejections without changing the balance. Starter balance, balance ceiling, and
per-operation limit are private operator settings.
