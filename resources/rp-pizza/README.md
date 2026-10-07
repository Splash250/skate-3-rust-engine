# Slice of Life Pizza

Slice of Life is a server-owned delivery shift for Boardwalk Borough. Clock in at `pizza_counter`, collect the order, and skate to the server-selected address. The job checks the current admitted actor, public-world instance, observed map marker, route order, and minimum travel time. Client payloads cannot choose the destination, payout, account or sender.

The wallet credit uses a stable operation ID saved with the pending order before submission. After a resource restart the job checks that same wallet operation and either completes once or retries the same ID. The `voice-room` dispatch membership is enabled for active shifts and removed on finish, cancel, expiry or disconnect cleanup.

Open **Pizza Shift** from the Phone app list or the player interface menu. Operators can adjust `delivery_reward` after restart and `minimum_delivery_seconds` or `shift_timeout_seconds` live.
