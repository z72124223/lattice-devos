---
name: lattice-managed-work
description: Use the local LATTICE formal-work entry for authorized engineering tasks in an already registered repository, and follow Life-Harness diagnostic notifications in its owned Codex execution turns. Use when implementing or repairing code through LATTICE. Ordinary questions and unregistered projects do not require managed execution.
---

# LATTICE managed engineering work

Use the current Codex task for reasoning and reporting. LATTICE's running Control service owns the separate native execution turn and durable work identity. Registration alone does not enroll the current App task or let it reuse another turn's claim.

## Already executing a formal task

If the current task has a verified `LATTICE_TASK` binding and formal execution instructions, continue that work; do not delegate it again. When the service supplies a Life-Harness candidate notification, run its exact `life-harness-client.mjs prepare` command. If it returns `decision=prepared`, execute `command` verbatim in a separate native shell call, then execute `readbackCommand` separately. Keep the same active turn. Do not wrap, redirect, concatenate, evaluate or regenerate the receipt command. Report any rejection without substituting invented IDs or context. Diagnosis does not grant new repair authority.

## Entering from an ordinary Codex App task

Read the local installation record at `%LOCALAPPDATA%/Programs/LATTICE/active-backend.json`. Its `root` identifies the installed package; it does not prove the server is running. Use that package's `node.exe` and `apps/lattice-control/src/formal-work-client.mjs`.

1. Run `state` and `projects`; require the running loopback Control to expose formal work. Match the actual repository against the registered project root; do not choose by a similar name. If unavailable or mismatched, report the limitation and perform only the separately authorized local work without claiming enrollment.
2. Run `list --project <id>` to avoid duplicate work. Read `status --project <id> --task <taskRef>` before resuming an existing item. Preserve its worktree and original requirements. Do not restart interrupted historical probes merely because their titles look related.
3. For a new authorized implementation, save a small JSON request with the returned `projectId`, a stable `clientRequestId`, `objective` and concrete `successCriteria`. Run `create --input <file>` once; retain the returned `taskRef`. Reuse the same request if the response is uncertain. `create` does not start execution.
4. Run `start --project <id> --task <taskRef>` for that work. Read status at reasonable intervals while doing useful independent work. Present pending questions and results in the original App task. The client does not bypass approvals or answer questions automatically.

This entry creates an isolated native executor through the existing Control lifecycle. It currently requires a runnable artifact and a Node test for independent verification; do not force unrelated tasks into that contract. Inspect the registered repository HEAD before dispatch: uncommitted source is not automatically copied into its managed worktree. Keep a single writer for each scope.

Life-Harness automatically considers failures only in enabled, owned execution turns. A saved candidate or steering intent is not a completed diagnostic. Use the real diagnostic readback and formal completion fields to report their separate outcomes. JEV stays disabled. Do not restore schedules or old work, broaden permissions, or claim coverage of other hosts or a reboot without evidence.
