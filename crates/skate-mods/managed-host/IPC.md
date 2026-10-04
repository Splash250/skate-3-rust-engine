# Managed worker launch and IPC contract

The parent launches the trusted `Skate.ResourceHost.dll` under .NET 10. The
operator supplies `SKATE_DOTNET_ROOT` (the .NET install directory) and
`SKATE_MANAGED_HOST` (a directory containing the published worker and its Roslyn
assemblies). These paths are host configuration, never resource configuration.
The child receives only anonymous stdin/stdout pipes for protocol frames and a
discarded stderr. Do not inherit other handles, credentials or environment.

Linux uses bubblewrap: fresh user, PID, IPC, UTS and network namespaces; a narrow
read-only runtime and worker mount; read-only native runtime libraries; no
resource or user directory mount; no network; no writable executable files.
The worker installs a thread-synchronized seccomp filter before running resource
code, denying process launch, ptrace, cross-process memory and socket syscalls.
Managed source policy rejects unsafe/native/IO/network/process/reflection APIs.
Failure to establish isolation rejects startup. `prlimit` bounds file descriptors (256), core dumps (zero) and file size
(256 MiB, including anonymous JIT backing files). A parent deadline kills the child on timeout, crash,
excess memory, malformed output or excessive IPC. Namespace teardown removes
descendants. No shared-user RLIMIT_NPROC is changed.

Windows must launch with an AppContainer token, no capabilities, no inherited
handles other than the two explicitly listed pipes, a restricted environment,
and a kill-on-close Job Object with process/memory limits. Grant the container
read/execute access only to the trusted worker/runtime paths. The parent must
keep the job alive until it closes the child. A Windows implementation may return
a `WorkerProcess` plus an owned isolation guard from `spawn_worker`; that
guard must kill the entire job on drop. `WorkerProcess` owns optional boxed
`Read + Send` / `Write + Send` stdout/stdin streams and provides `id() -> u32`,
`kill() -> io::Result<()>`, and `wait() -> io::Result<()>`. Do not substitute an ordinary process
launch if AppContainer setup fails. Native Windows validation is a separate
requirement from implementing this adapter.

Frames are UTF-8 JSON followed by one LF. The maximum frame is 8 MiB including
LF; one bounded frame may be queued in each direction. JSON depth is bounded.
The parent sends exactly one request at a time. Child host calls are synchronous;
they may invoke a different resource through the dependency-checked export
registry, but cannot re-enter the same worker. All IDs use u32 callback IDs or
canonical decimal strings for generation/player identity.

Parent to worker:

```json
{"op":"init","sources":[{"name":"main.cs","code":"..."}],"metadata":{"id":"demo","side":"server","generation":"1","grants":[]},"maxCallbacks":64,"maxValueBytes":262144}
{"op":"invoke","callback":1,"args":[{"value":1},"0"]}
{"op":"return","request":1,"ok":true,"value":null}
{"op":"return","request":1,"ok":false,"error":"capability denied"}
```

Worker to parent (between a request and its completion):

```json
{"op":"call","request":1,"method":"resource.state.set","args":["value",1]}
{"op":"register","request":2,"kind":"on","name":"changed","callback":1,"permission":""}
{"op":"done","ok":true,"value":null}
{"op":"done","ok":false,"error":"resource error"}
```

`kind` is `lifecycle`, `on`, `on_net`, `export` or `command`. The lifecycle names
and host methods are the shared Lua/JavaScript allowlist. The Rust parent validates
every field and callback registration quota independently of the worker. The
parent injects resource identity, owner, grants and generation through the common
host; resource IPC never chooses them. C# implements `IResourceScript.Start(Resource)`
and registers synchronous callbacks returning `JsonNode?`. No Tasks, unmanaged
assemblies, assembly assets, arbitrary dependencies or async callback returns are
accepted. Compile all manifest-selected `.cs` files together inside the worker.
