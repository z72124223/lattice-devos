# TASK-094 code review — independent final review

## Scope inspected

Writer v3 setup/manifest/rebind APIs, the Store migration state classifier and
transaction boundary, migration 0007 compatibility publication, focused
regressions and the owned live harness.

## Final independent result

`PASS` — parent-gated independent review of exact
`f19719c7bf968ce557d84b87d317946f43844bf3` found P0=0, P1=0, P2=0, and P3=0.
The feature delivery is clear at this branch's local scope. This result does not
turn pending non-force push, remote SHA/CI, or product integration into completed
delivery work.

## Findings

- P1 (review repair): Store constitution 1.12 prohibited every mutating Writer
  call, contradicting the fixed rebind implementation. Constitution 1.13 now
  permits only the fixed Writer-owned call for exact-v5 transition or exact-v6
  idempotent retry, while preserving Writer ownership/rollback boundaries.
- P2 (review repair): the prior live happy-path did not force rebind failure.
  The marker-owned live phase now injects an active Writer head and fingerprints
  exact v5 bridge history, compatibility, Writer identity, ledger and runtime
  ACL before/after the failed runner transaction.
- P2 resolution: parent foreman independently reran the owned live command as
  receipt `8125d6fe95264766b7b06161caa16a05` on dynamic port 55198. It reports
  all transition and failure-atomicity stages pass, owned-root/listener teardown,
  and unchanged 5432 PID 5200 / 58743 PID 25912 listeners. This records the
  parent command receipt only; it does not claim a raw-log artifact.
- P0/P3: none found in the local source inspection.
- The Store calls only fixed `writer_lease.writer_lease_rebind_v3()` for the
  exact-v5 transition after v5-prefix classification and ordinal-7 application,
  or for exact-v6 idempotent retry; it contains no Writer ledger mutation or
  generic SQL surface.
- The v6 current verifier checks Writer v3 identity/function/ACL closure; the
  pre-v6 bridge remains runtime quarantined.
- The harness rejects 5432, preflights ownership, uses a marker-owned temp root,
  and proves listener/root teardown.

## 2026-08-25 boundary repair

- P1 resolved locally: Store no longer depends on the Writer adapter, parses
  Writer semantic rows, or locks/mutates Writer tables. It uses its fixed
  catalog closure and one fixed zero-argument procedure call for both exact-v5
  transition and exact-v6 retry.
- P2 resolved locally: the cross-adapter fixture moved out of Store and into
  `lattice-runtime` composition. Fresh run
  `fb5817a389794a5a8e637bfff9288a61` / port `58375` asserted active-head
  SQLSTATE 55000, exact rollback fingerprint, identity/ledger/ACL drift denial,
  successful v6 transition and idempotent retry, with owned teardown.
- Residual: this test self-installs its bridge fixture and does not reorder the
  product runtime's Store-first bootstrap. A separate governed product-based
  integration task remains required before deployment can be considered.
- Historical verification residual: Store + Writer strict Clippy passed while
  runtime test-target Clippy reached 17 pre-existing `lattice-hermes-adapter`
  diagnostics; this ticket does not modify Hermes. Terminal root evidence now
  records `npm verify` exit 0 with Node 120/120, while full-workspace strict
  Clippy remains explicitly a non-PASS.

## Delivery boundary

The independent review clears this feature delivery. Non-force push, remote SHA
and CI, and product integration remain separate pending work; product bootstrap
ordering Writer v3 before Store v6/rebind is TASK-105's prerequisite and no
deployment claim is made here.
