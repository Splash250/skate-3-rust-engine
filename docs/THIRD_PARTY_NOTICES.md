# Third-party notices

## SDL_GameControllerDB

`crates/skate-platform/gamecontrollerdb.txt` is a snapshot of
[SDL_GameControllerDB](https://github.com/mdqinc/SDL_GameControllerDB) at
revision `555ce569a2003b22a4e134882224f1e2bdecc2e3`. The database is used to
map controller-specific Linux inputs to SDL's logical controller layout.

It is redistributed under the zlib license stored at
`crates/skate-platform/gamecontrollerdb.LICENSE.txt`. No locally authored
mapping overrides are included.

## xdvdfs

`skate-xiso` uses `xdvdfs` 0.8.3 for read-only Xbox disc filesystem access.
The crate is distributed under the MIT license recorded by Cargo from its
upstream package at <https://github.com/antangelo/xdvdfs>.

## Lua resource runtime

The client and dedicated server use `mlua` 0.12.1 with Lua 5.4.9 supplied by
`lua-src` 551.0.2. Their MIT notices, including the underlying Lua notice,
are preserved in [LUA-NOTICES.txt](../tools/server-package/LUA-NOTICES.txt)
and included in the standalone Windows server package. The resource host
implements its own Cfx-inspired interfaces; no citizenfx source was copied.

## JavaScript and backend services

The resource extension embeds `rquickjs`/QuickJS 0.14.0 bindings (MIT),
`rusqlite` 0.40.2 (MIT, bundled public-domain SQLite), `reqwest` 0.12.28
(MIT/Apache-2.0), Tokio and Rustls. Exact transitive versions are pinned by
Cargo.lock. Their source-package license notices, including the QuickJS engine,
are preserved in [PLATFORM-NOTICES.txt](../tools/server-package/PLATFORM-NOTICES.txt)
and packaged with the dedicated server. Identical notice texts are grouped with
all source paths retained; the inventory includes newly resolved target-specific
packages as well as native dependencies. The omitted `rquickjs-core` package
license is covered by its same-revision repository root license. The
`rsqlite-vfs` notice was retrieved from the revision in its Cargo source metadata.

QuickJS provides an embedded engine with interrupt and memory limits without
Node.js or operating-system modules. SQLite gives local transactional storage;
Rustls avoids requiring a separately installed TLS provider on Windows/Linux.
Bundling their C libraries requires the platform's normal C toolchain at build
time. This is an ongoing dependency-maintenance obligation, not a claim of
arbitrary downloaded native-code containment or C# support. No CitizenFX source
was copied for the compatibility adapters.

## Browser, accounts, voice and managed resources

The platform notice inventory also covers Wry 0.57 / Winit 0.30 and their
WebKitGTK/WebView2 adapters, Argon2/Rustls/Ring account/session protection, and
CPAL/Opus voice dependencies. Cargo.lock is the version authority. Linux uses the
system WebKitGTK and ALSA libraries; Windows requires the installed WebView2
Runtime. The Windows loader bundled by `webview2-com-sys` 0.39.1 is from Microsoft
WebView2 SDK 1.0.3800.47: its x64 static-loader bytes were matched to the official
NuGet package, and that package's original LICENSE and NOTICE are retained in
PLATFORM-NOTICES.txt. The Rust wrapper's MIT license does not replace those
notices. Player and server packaging both retain the platform notices.

The optional C# worker uses the locally installed .NET 10 runtime and Roslyn
compiler from the configured SDK. Managed-host packaging includes the SDK's
LICENSE.txt and ThirdPartyNotices.txt alongside the worker and compiler DLLs.
It does not bundle a .NET runtime or grant downloaded resources arbitrary native
libraries. Linux containment requires bubblewrap and prlimit; Windows uses
AppContainer plus Job Object limits. Native Windows behavior still needs testing.

These libraries provide maintained platform integrations and standard codec,
cryptographic and language implementations. The added operating-system runtimes,
C/C++ builds and license notices must be maintained with dependency upgrades.
