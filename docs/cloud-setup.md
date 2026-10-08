# Linux cloud setup

This entry runs LATTICE Control, PostgreSQL and the existing Codex App Server on
one Linux host. Life-Harness is enabled in owned execution turns. JEV is excluded.
Codex remains the only agent loop. All listeners bind to `127.0.0.1`.

## Prerequisites

Run as a non-root user with Node 24.15 or newer, Rust 1.97.x, Git, PostgreSQL
server tools, and a native Linux `codex` binary already installed. Use a signed
package source or independently verified dependency artifact when provisioning
these tools. This script does not install OS packages or change authentication.
Keep the checkout on the intended Git branch and verify its commit before setup;
the runtime's project inspection expects a branch rather than a detached HEAD.
The selected PostgreSQL version must support the repository's actual migrations;
`--postgres-initialize` and `--postgres-bootstrap` must both pass.

The native `codex app-server` must return authenticated ChatGPT readiness from
`account/read`. A cloud task's outer login or `codex login status` alone is not
that evidence. Use cloud-supported account authentication. Do not copy a Windows
`auth.json`. Keep LATTICE state under a writable directory such as `/workspace`.
The native app-server also needs a platform-supported writable Codex home and
authentication arrangement: a login located on a read-only `CODEX_HOME` does not
prove that app-server can start or persist its own sessions. This script does not
copy or relocate login files to work around that boundary.

## Install and start

Set these paths to existing host components. The state root must be outside the
checkout, neither its ancestor nor descendant, owned by the invoking user, and
mode `0700`. Its parent must already exist. The default ports are 55432 and 4317;
optional `LATTICE_CLOUD_PG_PORT` and `LATTICE_CONTROL_PORT` must be distinct.

```bash
export LATTICE_CLOUD_STATE_ROOT=/workspace/lattice-state
export LATTICE_PG_BIN=/usr/lib/postgresql/18/bin
export LATTICE_CODEX_BIN="$(command -v codex)"
# CODEX_HOME already points to this host's authenticated Codex configuration.
bash scripts/cloud/setup.sh --build
bash scripts/cloud/start.sh
```

`--build` compiles `latticed` from this checkout using Cargo.lock and the installed
Rust 1.97.x. It can take several minutes and needs substantial free disk. To use
an already built native artifact, set `LATTICE_LATTICED=/absolute/path/latticed`
and omit `--build`. Setup records binary hashes; a changed binary requires a new
installation state root. It never treats a renamed Windows executable as Linux.

Setup creates a dedicated cluster, random credentials, and private `runtime.toml`
and `installation.json` under the state root. Never upload these files as task
evidence. It does not modify the user's default MCP configuration. Re-running a
completed setup preserves the database identity and credentials. Failed partial
installation data is retained for diagnosis; an unknown cluster is never erased
or adopted. Use a separate new state root after inspecting a partial failure.
When the runtime environment contract changes, use a new state root so the private
MCP configuration is generated with the same authority identity as bootstrap.

Start checks the cluster identity, effective bind address and configuration hashes
before using it, applies the actual runtime bootstrap, and starts Control in the
foreground with `--no-auto-restore`. The cloud environment's Start skill or host
service manager owns process lifetime; this script adds no supervisor or scheduler.
Stopping Control leaves the cluster and facts intact. Do not run two starts for
one state root. An interrupted process may leave `.operation`; remove that empty
lock directory only after confirming its original setup/Control process has ended.

## Graphify and readiness

Optional `LATTICE_GRAPHIFY_RUNTIME_ROOT` must be supplied at first setup and point
to an independently provisioned, reviewed runtime. Current native pins require
Ubuntu 26.04, Python 3.14.4, bubblewrap 0.11.1, and the complete fixed Graphify
payload plus `install-report.json`. Their hashes are checked by the Rust adapter.
The repository's standalone wheel downloader cannot reconstruct that full payload.
A standard Debian cloud image therefore remains explicitly degraded.

Start performs the real identity preflight and, when it passes, native
`--graphify-refresh`. Readiness requires a persisted receipt showing an actual
analysis on this source commit, not merely a reused graph or `PREPARED` status.
It retains that evidence across starts and reruns the pinned identity check.
Graphify failure leaves Control and PostgreSQL available and visible as degraded.

In another terminal with the same state-root variable:

```bash
bash scripts/cloud/readiness.sh
```

The JSON separately reports `coreReady`, `overallReady`, PostgreSQL identity,
Control's Life-Harness enrollment, native account readiness, and Graphify evidence.
Exit 0 means all prerequisites are verified; exit 2 means readiness is incomplete;
exit 1 means the configuration or operation was rejected. Graphify configuration
status `PREPARED` alone never means `overallReady`. Native account failure also
keeps `coreReady` false. These checks make no paid model request and do not run a
formal engineering task. A separate real task must prove task completion and the
Life-Harness failure → diagnostic receipt → repair → verifier sequence.

## Persistence and delivery boundary

The PostgreSQL directory survives starts within this host/state root. A Codex
cloud task's filesystem snapshot is suitable for acceptance, but does not prove
shared storage across tasks or a continuously running service. Production durable
PostgreSQL needs a persistent cloud host/volume, backups and restore verification.
Keep LATTICE and PostgreSQL in the same network namespace under this loopback-only
contract. Do not point this setup at remote TCP PostgreSQL or open public ports.

After publishing a cloud environment, dot must select that real environment and
start a real cloud task. Retain its task URL, environment identity, checked-out
commit, public readiness output, and actual formal-task/diagnostic receipts.
Environment creation, dependency installation, setup success and HTTP 200 are
separate from complete three-core acceptance.
