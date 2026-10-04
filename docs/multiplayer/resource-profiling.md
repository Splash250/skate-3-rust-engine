# Resource performance history

The resource host captures a bounded metadata-only timeline across generation
changes. Use the existing authenticated administrative resource command surface
for profiling. Submit `{kind:"profile_read",resource:"my-resource"}` (omit the
resource or use null for all resources) or `{kind:"profile_export"}` to
`POST /v1/admin/actions`, then read the returned action ticket. These actions
require `profile.read`. The HTTPS admin renders recent-span history and percentile tables and provides a trace download. The read response includes up to 128 recent spans and bounded summaries; the complete retained history is available through a size-bounded portable export with explicit omission counts. Save the completed export result as a JSON file. The
underlying host API supports both historical summaries and portable trace export. It is not a sampling profiler of arbitrary native stacks.

`Host::profile_snapshot()` returns spans and summaries, and
`snapshot.chrome_trace()` returns JSON that opens in Chrome tracing or Perfetto.
`Host::configure_profiling(enabled,capacity,retention_ms)` changes capture limits.
Defaults are 4096 spans/60 seconds; accepted limits are 64..8192 spans and 100..300000 ms.
Expired and overflowed spans are evicted; the snapshot reports the count.
Retention spans resource restart and installed-set replacement and ends when the
host process exits. There is no unbounded disk capture or automatic upload.

## Reading a capture

Each span identifies resource, generation, language, phase, available source,
parent, monotonic start and elapsed duration. Phases include startup, lifecycle
callbacks, named events, named commands, named exports and managed IPC exchanges.
Lua callbacks include source file/definition line where available. JavaScript
and C# report manifest-selected source file names; they do not promise sampled
stack traces or exact line attribution. `@host` dispatch spans group the resource
work for an update/fixed/UI callback and let an operator correlate a long dispatch
with its expensive children on one timeline. The dedicated host also brackets
its resource platform step with an owned `Host::profile_scope("server_resource_tick")`
guard, including downstream host work. These owned guards are neither `Send` nor
`Sync`: close them on the originating thread so CPU counters and parent spans
refer to the same host thread. Physics/render work outside resource
dispatch is not measured by these spans; use the existing game Chrome trace for
whole-game scheduling analysis. `origin_unix_us` records the UNIX-microsecond
anchor for timeline zero; subsequent span offsets use a monotonic clock. Use
that anchor when correlating separately started captures and retain each
capture's clock metadata.

Summaries group resource/generation/phase and sort by p95 wall time. p50/p95/p99
use nearest-rank percentiles over retained samples. They are a window of recent
work, not a lifetime histogram. A capture with few samples cannot establish a
stable tail-latency estimate. Filtering to one generation distinguishes a stopped
version from its replacement.

| Field | Contract |
| --- | --- |
| `wall_time_us` | Inclusive elapsed time, including nested calls and waits. |
| `host_cpu_time_us` | Inclusive CPU consumed by the calling native host thread, measured on Linux/Windows; null elsewhere. |
| `exclusive_host_cpu_time_us` | Host-thread CPU after subtracting direct nested measured spans; additive without counting exports/IPC twice. |
| `worker_cpu_time_us` | Separately measured process CPU across all managed-worker threads during one IPC request; present only on IPC spans, null when unavailable. |
| `queue_wait_us` | Actual time already spent in the local event queue before dispatch; null for direct/network delivery because their prior transport queue is not measured here. |
| `ipc_receive_wait_us` | Time the host blocks receiving IPC frames, including worker execution and scheduling; not a pure queue-delay metric. |

Do not sum inclusive wall or host CPU measurements across resources. Do not
subtract host CPU from wall and call the remainder worker CPU. Worker process CPU
may exceed elapsed time when managed-runtime background threads run in parallel.
The trace contains no event payloads, argument values, settings values, storage,
script source text or exception messages. Identifier names and source paths are
visible; do not use secrets as resource/event/export names. Existing diagnostic
error text remains separately bounded and can include text supplied by scripts.

## Verification and overhead

```sh
cargo test --locked -p skate-mods --test resource_runtime resource_profile_records
cargo test --locked -p skate-mods --test resource_runtime resource_profile_instrumentation_overhead_measurement -- --ignored --nocapture --test-threads=1
SKATE_DOTNET_ROOT=/absolute/dotnet SKATE_MANAGED_HOST=/absolute/managed-host \
  cargo test --locked -p skate-mods --test managed_runtime csharp_typed_settings -- --ignored --nocapture --test-threads=1
```

The overhead workload alternates capture disabled/enabled across three paired
5000-dispatch runs with four actual Lua and four actual QuickJS resources after a
1000-dispatch warmup. It prints median dispatch nanoseconds, incremental time per
resource, ratio and retained count. Existing aggregate timing counters remain
active in both modes; the comparison isolates enabled timeline retention and
nesting overhead over the current host implementation. Results are machine/load
specific and belong in the requirement evidence ledger, not a universal budget.
A second managed benchmark alternates three paired 1000-dispatch runs after
100 warmup dispatches, including worker CPU sampling overhead only when capture
is enabled. The managed integration runs actual isolated C# callbacks and requires CPU/IPC
measurements from a freshly built trusted worker; absent runtime prerequisites
must be reported rather than replaced by a mock.

Fresh Linux measurement on 2026-10-05 (optimized repository test profile): the
mixed workload measured 138,497 ns per dispatch with capture disabled and 146,189 ns
enabled, an incremental 961 ns per resource and ratio 1.056. The actual C# workload
measured 62,483 ns disabled and 66,994 ns enabled, incremental 4,512 ns and ratio 1.072.
Both retained exactly 4096 spans. These are local measurements, not Windows or WAN
acceptance. Logs are recorded in the platform evidence ledger.
