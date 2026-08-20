---
ticket_id: TASK-074
title: Bounded local-model development assist activation
spec_id: SPEC-002
spec_version: 31
module_id: lattice-cli
constitution_version: 1.0
status: completed
parallel_safe: true
depends_on: []
allowed_paths:
  - docs/tickets/TASK-074-local-model-development-assist.md
  - PLANS.md
  - HANDOFF.md
branch: feature/task-051-p0-platform-live-acceptance
---

# TASK-074 Bounded local-model development assist activation

## Objective

Activate an already-installed local coding model as an untrusted development
assistant while keeping CPU compilation and tests authoritative. Bind the
model only to loopback, verify exact runtime/model identity and GPU offload,
exercise one CPU/GPU parallel check, and record the model's permitted and
forbidden roles. This ticket changes no LATTICE product code or public API.

## Acceptance Criteria

1. The CUDA llama.cpp runtime and Qwen2.5-Coder 7B Q4_K_M model are bound by
   exact SHA-256 and loaded from existing local files; no download occurs.
2. The server listens only on `127.0.0.1`, and `/health` plus `/v1/models`
   succeed before any prompt is sent. `0.0.0.0`, `::`, public listeners,
   external APIs, credentials, and user data are forbidden.
3. GPU offload is observable for the exact server process. A focused LATTICE
   Rust test runs concurrently on CPU and remains the authoritative result.
4. A low-risk structured test-output extraction is manually checked. A
   security/recovery classification benchmark is also run; any incorrect
   answer permanently restricts this model from architecture, security,
   authorization, merge, or commit decisions.
5. Model output is advisory and may assist summaries, candidate test lists,
   or mechanical review only. Every claim must be revalidated by source,
   compiler, deterministic tests, or the primary agent before use.
6. Record whether the exact process remains intentionally running or was
   terminated. No project write, Git mutation, MCP mutation, database access,
   model/provider credential, external network, push, merge, deployment,
   payment, or account change is permitted.

## Completion Evidence

- Runtime: `llama-server.exe`, SHA-256
  `A00ACD129ACC95BBC266ADA2FB46B12D1D3134DCFB9FDA214C300A2CD4A47F72`.
- Model: `qwen2.5-coder-7b-instruct-q4_k_m.gguf`, SHA-256
  `509287F78CB4D4CF6B3843734733B914B2C158E43E22A7F4BF5E963800894D3C`.
- The stale listener was identified and stopped; PID `26732` was then started
  hidden with `--host 127.0.0.1 --port 18181 --n-gpu-layers all`, and both
  health and model discovery returned successfully. It remains intentionally
  running for bounded local assistance at ticket closure.
- In parallel, the focused Hermes false-completed Rust test passed 1/1. The
  model correctly extracted that test result into a four-field summary.
- The model failed the recovery-security classification benchmark by proposing
  a broader retry policy. That failure is retained as acceptance evidence and
  enforces the non-authoritative restriction above; it is not reported as a
  security-review capability.

## Non-Goals

No autonomous coding, file edit, command execution, credential access,
security decision, architecture approval, ticket transition, commit, merge,
provider/model substitution, production Hermes acceptance, public endpoint,
durable memory, model download, fine-tuning, or deployment.
