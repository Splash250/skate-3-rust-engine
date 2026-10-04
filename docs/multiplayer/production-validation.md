# Bounded production validation

These fixtures exercise the implemented resource platform with synthetic data.
They are acceptance probes with explicit workload limits, not a certification of
Internet capacity, physical audio or native Windows behavior. Run Cargo commands
from the repository root; serialize them with other builds sharing `target/`.

## Combined traffic and delay

```sh
cargo test --locked -p skate-server --test capacity -- --nocapture --test-threads=1
SKATE_SOAK_SECONDS=120 cargo test --locked -p skate-server --test capacity sustained_sixty_four_client_combined_traffic_soak -- --ignored --exact --nocapture --test-threads=1
```

The ordinary suite retains the original 64-owner burst workload and adds an
8-owner sustained workload. The opt-in soak runs 64 owners for 30–300 seconds
(default 120), followed by at most 20 seconds to drain outstanding resource work.
Each owner uses an actual UDP socket and the real admission, movement, resource,
shared-object and voice protocols. Server Lua echoes 1 KiB requests while two
shared objects move and eight talkers send real Opus-encoded synthetic tone
frames. One tone frame is encoded at setup and reused; this measures encoded
packet routing under load, not concurrent encoder CPU. Receivers validate voice
framing/routing; this fixture does not run
64 voice decoders, audio devices or native skating rigs. Content download has its
own integration suite; these simulated owners explicitly acknowledge the fixture
content set.

The sustained profile independently delays each direction by 25–75 ms and drops
one percent of datagrams with fixed seeds. Delivery by due time produces measured
reordering. The existing sparse deterministic movement/receive drops remain in
addition to that loss, and OS socket loss is possible. This is a user-space
loopback impairment model, not a real WAN measurement. Fixed seeds reproduce
impairment decisions for a given datagram sequence; OS scheduling changes actual
packet sequences and measured timings.

Each owner offers at most one request per second and retains at most two
outstanding requests. Every submitted request must complete once; every peer
must complete at least two, the greatest per-peer completion count may be no
more than twice the least, and every peer must observe every other actor, both moving objects
and every eligible talker. A resource request older than 15 seconds fails the
run. The acceptance bounds are movement p95 below 2500 ms, encoded voice packet
p95 below 1000 ms, and resource echo p95 below 10000 ms. End-of-run observer
snapshot ages must have p95 below 2500 ms and maximum below 10000 ms, so stale
observers cannot hide behind the latency of newly arriving packets. These are conservative
regression gates, not recommended player experience targets. The output reports
actual p95/max ages, minimum/maximum per-peer completions, byte counts, loss,
reordering and queue peaks so a passing bound cannot conceal the measured latency.

Measurement histograms use constant storage. Each impairment queue is capped at
4096 datagrams and 2 MiB; filling a queue fails instead of allocating indefinitely.
Linux combined server/client RSS is sampled every five seconds. Each client retains bounded 64-revision BODY and POSE histories for every
observed actor, so 64 simulated clients need substantially more memory than a
single server. The RSS gate begins only after measured priority BODY history occupancy reaches
95% of its combined bound; growth of 64 MiB or more then fails. Optional POSE
histories are reported separately: pose detail uses remaining bandwidth and need
not populate every theoretical history slot under this saturated workload. Runs of at least
120 seconds must include three post-warmup samples on Linux; shorter runs can
finish before that window and cannot establish memory stability. Non-Linux hosts
report unavailable RSS instead of zero. This detects large sustained regressions
over the selected duration, not every memory leak.
Queued impairment bytes are test-network queues; they are not production-server
queue telemetry. Five-second samples separately record the Lua heap and resource
output queue measurements, and reject resource retirement or callback errors.
Reliable resource completion and existing transport/runtime
budget regressions independently exercise production backpressure.

## Abrupt process failure and offline backup

```sh
cargo test --locked -p skate-services --test recovery -- --nocapture
cargo test --locked -p skate-services --test backend -- --nocapture
```

The recovery parent launches the actual service in a child process. The child
commits a synthetic balance of 100, begins a debit transaction, and executes a
long query before commit. The parent waits for a rollback journal containing
pages, verifies a separate reader still sees 100, and verifies the writer lock
is active before forcibly terminating the child. Reopening through the service
must preserve 100, accept the existing migration, allow another write, and pass
SQLite integrity and foreign-key checks.

After stopping all service workers, the fixture copies the entire service data
directory. It then changes the original balance and restores the copied directory
to another location. The restored service must retain the backup's balance and
migration and accept independent writes. No real account data or credentials are
used. The ignored child helper is invoked only by its parent with temporary
paths; running every ignored service test directly is not a supported command.

This establishes process-kill handling during an active transaction and the
stopped-directory backup procedure. It does not simulate power loss, storage
firmware failures, a kill at every commit I/O boundary, online backup, live
filesystem copying, account-database backup or native Windows filesystem behavior.
See [backend services](backend-services.md) for cancellation and commit ambiguity.

## Linux continuation measurements, 2026-10-04

The first completed 120-second run passed with 64 simulated owners, two moving objects,
eight encoded-voice talkers and sustained resource traffic. The full workload
including resource drain lasted 122.078 seconds. This is the combined process,
not an isolated measurement of dedicated-server memory.

| Measurement | Observed |
| --- | --- |
| Resource echoes | 7,465; 115–118 per owner; p95 2,163 ms; maximum 2,795 ms |
| Movement | 904,000 received samples; p95 source age 253 ms |
| End observer snapshot age | p95 923 ms; maximum 2,612 ms |
| Encoded voice packets received | 2,055,760; p95 age 189 ms |
| Actual client socket bytes | TX 241,875,646; RX 929,345,574 |
| Controlled loss/reordering | 42,086 seeded drops; 54,646 existing sparse drops; 3,140,593 reordered datagrams |
| Largest individual impairment queue | 104 datagrams; 28,568 bytes |
| Combined RSS | 596,268 KiB at measured warmup; 624,340 KiB at 120 seconds; growth 28,072 KiB |
| Retained replication history | 258,048 BODY revisions at capacity; 129 optional POSE revisions |
| Resource runtime | Lua heap 61,028–61,108 bytes at sampled intervals; output queues empty; no callback errors |
| Voice router | 34,654 accepted frames; 2,777 rejected frames; 44,868 queued recipient copies dropped |

All simulated receivers accepted encoded packets from every eligible talker.
Reordering and loss remain present;
this run does not claim lossless audio. Optional pose detail was rarely scheduled
under the combined 64-owner load, so this is not high-detail presentation
acceptance for 64 graphical clients.

The earlier five-second baseline passed, and the eight-owner impaired probe
completed 56 echoes (seven per owner) with p95 1,478 ms. Two preliminary memory
gates failed because they assumed a 15-second warmup, then assumed every optional
POSE history would fill. Source review and measured occupancy corrected those
assumptions before the final passing run; the RSS growth limit stayed 64 MiB.
The [evidence ledger](platform-extension-evidence.md) records local log paths and
retains these diagnostic failures separately from the passing run.

## Focused trust-boundary review

The continuation review read the service owner/queue, SQLite authorizer and HTTP
origin code, the native input protocol and dedicated-session routing, the native
server manager, its resource integration, and client prediction/retirement code.
It checked current source against the documented attacker boundaries and relevant
regressions. It did not audit every native physics/scoring path, browser or managed
runtime implementation, operating system, or dependency. Passing this review is
not a general security guarantee.

The service checks found no new validated defect in the inspected scope. Owner
identity and active generation protect submission; cancellation retains bounded
queue slots; SQL parameters, authorization and result/storage limits protect the
database; exact administrator-approved HTTP origins, disabled redirects/proxies,
and deadlines bound HTTP behavior. New tests verify that views and triggers cannot
bypass migration-metadata protection. Private approved origins and the shared
SQLite allocator ceiling retain the documented [backend limitations](backend-services.md).

Two native-authority issues were identified:

- **Worker availability, fixed and regression tested.** A participant could send
  one valid tick just before each two-second inactivity timeout and retain one
  of the limited workers for far longer than the attempt's 60 Hz duration. The
  manager now enforces an absolute host-clock deadline of the configured physical
  duration plus 20 seconds, covering initial client loading and delivery/replay
  grace. It checks that deadline before accepting queued outcomes and preserves
  the existing inactivity checks. A regression first reproduced the retained
  attempt, then passed after the fix. Four focused server tests pass, including
  worker reaping and cancellation on owner/world/instance/disconnect changes.
- **Client reconnect, fixed and regression tested.** The client retained
  only the numeric completed movement epoch across retirement. A new session
  reusing that number could therefore fail to start its native replay. Retirement
  now clears the terminal epoch and acknowledged tick. The regression verifies
  repeated terminal records remain suppressed within an attempt while a new
  connection can reuse the same epoch after retirement.
  This is a participation/lifecycle defect; the review did not establish a forged
  scoring path. The regression passes in the fresh 364-test game suite.

A separate actual native-worker integration passes a 300-tick heelflip outcome
with native finalized award 22 while ignoring a forged million-point client
`Gameplay` score, then verifies approved-teleport cancellation. This supports that
specific authoritative input path; exhaustive trick-family, native Windows and
cross-platform numerical acceptance remain open in the
[native authority contract](native-skating-authority.md).

End-to-end integration also exposed two routing defects: the common Lua operation
allowlist rejected native start/cancel before the server, and a C# resource with
no selected-side source unnecessarily started an empty worker. Both are fixed
with regressions covering active owner/generation, grants and absent runtime
prerequisites. Real Lua/JavaScript routing and four actual isolated managed-worker
integrations pass. These were availability/reachability defects; review did not
establish an authority bypass through either path. The final independent review
found no further concrete defect in the inspected routing, prediction and
retirement paths. See the evidence ledger for exact runs and remaining scope.

## Shared-contact sampling regression

A fresh two-client crate approach sometimes produced a real contact but less
than the unchanged 3 cm displacement gate. A diagnostic pair passed with the
current floor and failed with the historical floor, ruling out a deterministic
floor-only regression. Bounded samples showed the server pairing an inbound
animation-root position with a wheel velocity that had already reversed during
impact. A transport regression reproduced this independently: the accepted root
advanced while the returned proxy velocity pointed away.

The scoped repair derives coarse proxy velocity from consecutive accepted root
observations within the current epoch. It uses the larger source/arrival interval,
requires fresh nonzero-time samples, rejects reset baselines and discontinuities,
and caps speed by the observed rig and existing 200 m/s shared-object bound.
Approved teleport placement still replaces the old proxy without sweeping its
old path. BODY admission, course-v1 collision validation and the articulated
native authority companion are unchanged. This improves coherence of an existing
coarse shared-contact approximation; it is not full-rig authority. The ledger
records focused regressions and actual reruns separately from the initial failure.

## Integrated rerun after the contact correction

The same 120-second profile passed again after the final network change, with
matching binaries and no concurrent builds or graphical runs. It lasted
122.193 seconds including drain. The acceptance bounds and memory methodology
were unchanged. Local log: `/tmp/skate-platform-followup-20261004/capacity-soak120-integrated.log`.

| Measurement | Integrated result |
| --- | --- |
| Resource echoes | 7,430; 114–117 per owner; p95 2,198 ms; maximum 2,708 ms |
| Movement | 867,536 samples; p95 source age 255 ms |
| End observer age | p95 991 ms; maximum 1,965 ms |
| Encoded voice receives | 1,999,044; p95 age 192 ms |
| Client socket bytes | TX 241,357,253; RX 916,846,411 |
| Controlled impairment | 41,167 seeded drops; 53,373 sparse drops; 3,068,942 reordered datagrams |
| Largest individual delayed queue | 102 datagrams; 32,961 bytes |
| Combined RSS | 590,144 KiB at measured warmup; 623,664 KiB at 120 s; growth 33,520 KiB |
| History | 258,048 BODY revisions; 160 optional POSE revisions |
| Voice router | 33,701 accepted; 2,369 rejected; 47,229 queued recipient copies dropped |

Sampled runtime output queues were empty and no callback errors occurred. These
figures retain the same combined-process, simulated-owner, encoded-packet and
loopback limits above. Two minutes of bounded operation does not establish
long-duration or WAN production readiness.
