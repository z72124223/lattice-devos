# Branch identity recovery

Operator-only CLI for an existing user project whose canonical root, repository
and physical worktree identities are unchanged. It accepts only pending branch/ref
drift. Suspended projects, moved roots and changed repositories/worktrees are denied.
Control remains a locator; PostgreSQL Registry remains the authority.

Use the existing runtime database binding and secret/daemon authority environment.
The commands accept no filesystem path, SQL, arbitrary lifecycle or supplied observation.

```text
lattice-runtime project-registry-inspect --postgres-host 127.0.0.1 --postgres-port <port> --postgres-run-id <run-id> --project-id <exact-id>
lattice-runtime project-registry-reconcile --postgres-host 127.0.0.1 --postgres-port <port> --postgres-run-id <run-id> --project-id <exact-id> --expected-revision <reviewed-revision> --expected-receipt-digest <reviewed-receipt> --pending-observation-digest <reviewed-pending>
```

Inspect is read-only. Review its accepted/pending refs, drift and eligibility before
reconcile. Reconcile re-observes the selected repository, requires exact pending
equality, and submits the existing domain `AcceptIdentityChange` command. It never
refreshes an expected head or replaces pending data automatically. A changed head
or observation requires a new inspection and review. No task submission is performed.

Repeat the identical request after an uncertain outcome. The deterministic command
ID returns the same retained receipt without a new revision. `REPLAYED` describes
historical evidence only: inspect `current.active`, current revision and fresh
observation matches separately. Another command may have changed the project since
the retained success. `APPLIED` also reports its receipt separately from current state.

Focused checks and release build (existing Rust 1.97.1 toolchain):

```text
cargo +1.97.1 test -p lattice-runtime --lib project_bridge::recovery
cargo +1.97.1 test -p lattice-project-registry --test project_registry
cargo +1.97.1 build -p lattice-runtime --bin lattice-runtime --release
```

These unit/domain checks do not prove a live PostgreSQL write. Use a disposable
runtime for persistence acceptance, then have the responsible operator apply the
reviewed request to the intended installed runtime. No installation, service
restart or production Registry mutation is part of building this artifact.
