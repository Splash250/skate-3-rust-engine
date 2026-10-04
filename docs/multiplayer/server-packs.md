# Reproducible local server packs

`tools/server_pack.py` installs an explicit set of version/digest-pinned resource
files. Recipes contain data, grants and resource settings, never shell steps,
credentials, databases, arbitrary archive extraction or native executable
acquisition. The operator supplies an existing trusted server binary. No central
service, Steam installation or paid account is required.

A format-1 recipe has this shape (the digest/length below are placeholders):

```json
{
  "format": 1,
  "id": "practice",
  "version": "1.0.0",
  "accounts_required": false,
  "resources": [{
    "id": "example", "version": "1.0.0", "provenance": "Original project resource",
    "files": [{"path": "resource.json", "source": "example/resource.json",
               "sha256": "FULL_64_LOWERCASE_HEX_DIGEST", "bytes": 123}]
  }],
  "server": {"ensure": ["example"], "grants": {"example": []}}
}
```

Every manifest, server/client/shared script and public asset is separately
pinned. `files[].source` is a confined relative path beneath `--source-root`
(default: recipe directory), or an HTTPS URL. Local source symlinks, traversal,
Windows device names, nonregular files (including FIFOs), and nonportable paths fail closed.
Local opens recheck the opened descriptor and reject final symlink substitution;
on POSIX, nonblocking open prevents FIFO substitution from hanging installation.
Downloads use verified
TLS without redirects or ambient proxy credentials. Files have a 64 MiB limit,
packs a 512 MiB/4096-file limit, and recipes a 1 MiB/128-resource limit. Downloads
have a 60-second transfer budget checked between reads and a 15-second blocking
read timeout. An in-flight read can finish up to 15 seconds after that budget.
Native executables, archives, common database and credential file extensions are
rejected. Other files must still have explicit provenance and digest pins; a
recipe is not a content license or a review of arbitrary resource scripts.

`server` accepts `ensure`, `grants`, typed `settings`, `content_limits`,
`network_budgets`, `runtime_limits`, `http_origins` and `world_rotation`. It cannot supply resource
or storage filesystem roots. Review grants and private settings before applying;
never write secrets into a recipe. Required engine features live in the existing
resource manifests and are checked by the server validator.
Settings and world-rotation entries must refer to resources pinned in the recipe;
rotation entries must be unique. Every manifest-declared file must also be pinned.

## Plan, install and run

Build the server, then review the recipe and printed installation plan:

```sh
cargo build --locked -p skate-server -p skate-accounts
python3 tools/server_pack.py plan resources/packs/free-skate.recipe.json --source-root resources
python3 tools/server_pack.py apply resources/packs/free-skate.recipe.json --source-root resources --root /tmp/skate-free-skate --server-executable "$PWD/target/debug/skate-server"
python3 tools/server_pack.py verify /tmp/skate-free-skate
```

`plan` is read-only and reports files, bytes, exact version identity, account
requirements and the persistence contract. `apply` checks all files and exact
manifest/dependency pins, then invokes the trusted server's
`--validate-resources FILE` mode for actual engine manifest/content/world/settings
validation. This mode does not execute resource scripts. Script runtime failures
remain startup failures and must be exercised on a local test server.

The installer creates:

- `server.json`: stable active configuration path, replaced atomically.
- `.pack/versions/DIGEST/resources`: immutable resource installation.
- `.pack/versions/DIGEST/receipt.json`: pinned recipe, provenance and full config.
- `.pack/transaction.json`: only present while publishing or awaiting recovery.
- `.pack/run.lock`: interprocess lock shared with the supervisor.

Resource persistence goes in `data`; account bootstrap places its private database
and TLS files in `data/accounts`, outside immutable resource versions. The
supervisor's default `data_root: "data"` therefore snapshots both resource and
account persistence together. Protect these private snapshots like the live store.
Keeping the canonical `server.json` path stable preserves the engine's storage
namespace across upgrades. Do not start the server using a version-directory
configuration. Installation refuses an existing unmanaged `server.json`.

Use the [supervisor](server-operations.md) with `pack_root` set to the installation
directory. Its lock spans child operation and restart/backoff. Apply, rollback
and account bootstrap refuse while that supervisor owns the lock. Directly
launched servers do not participate in this lock: stop them before modifying an
installation. No installer can infer arbitrary external processes reliably.

For an anonymous local practice pack:

```sh
target/debug/skate-server --test-world --bind 127.0.0.1:31030 --resources /tmp/skate-free-skate/server.json
```

For an account-required pack, initialize its private authority once:

```sh
python3 tools/server_pack.py init-accounts /tmp/skate-free-skate administrator --account-executable "$PWD/target/debug/skate-account" < /private/admin-password
target/debug/skate-server --test-world --bind 127.0.0.1:31030 --resources /tmp/skate-free-skate/server.json --accounts /tmp/skate-free-skate/accounts.json
```

The account tool enforces the existing password, permissions, TLS and store
contracts. The installer never reads or prints the password. `accounts_required`
is an explicit recipe/operator prerequisite; use the account-enabled launch
command. Stable verified profiles and competition rewards remain unavailable to
anonymous connections even when an operator omits the account flag.
Bootstrap refuses existing `accounts.json`, `data/accounts`, or a legacy top-level
`accounts` directory; it never relocates or overwrites an existing authority.
For a legacy split layout, stop the server and explicitly move the account store
under the configured recovery root and update `accounts.json` before relying on
the unified backup contract.

## Upgrade, recovery and rollback

Stop the supervisor, review a newer pinned recipe, and run `apply` again. A warm
install reuses verified installed bytes. Changed recipes produce new immutable
version directories; up to 32 versions are retained. At that limit, archive
unused versions outside the installation while stopped, preserving versions you
still need for rollback. Mutable stores are never bundled or copied.

```sh
python3 tools/server_pack.py history /tmp/skate-free-skate
python3 tools/server_pack.py recover /tmp/skate-free-skate
python3 tools/server_pack.py rollback /tmp/skate-free-skate FULL_VERSION_DIGEST --server-executable "$PWD/target/debug/skate-server"
```

Staging/download/validation failures leave the previous configuration active.
The journal records both configurations before atomic publication. Recovery
reconciles the actual configuration with that journal: an old configuration
means rollback; a new one means committed publication after verifying its files.
Unexpected third-party edits fail with an actionable error and retain the
journal. Temporary interrupted staging directories are inert and may be removed
while stopped. Neither recovery nor rollback runs recipe commands.

Rollback revalidates retained file digests, receipt identity and the engine
contract before publishing its old code/configuration. **It does not roll back
database migrations or player data.** Take a stopped-store backup through the
supervisor before an upgrade that changes persistence, and restore that backup
if the old code is incompatible. This installer provides process-interruption
recovery with flushed file publication; it does not claim cross-filesystem
power-loss atomicity or database recovery.

Tests: `python3 -m unittest tools.test_server_pack -v` covers disposable cold/warm
installs, digest/version/dependency rejection, validation failure, publication
interruption on both sides of the commit point, rollback preserving mutable
data, corrupt receipts/files, confinement and busy supervisor locks. Actual
native server startup and production-store migrations require separate evidence.
