# Request Boundary

## Authoritative Input

- Received: 2026-07-29 (Asia/Taipei)
- Source file:
  `C:\Users\f7212\.codex\attachments\4b86d7ee-43ef-49b1-97af-89147d0b0352\pasted-text.txt`
- SHA-256:
  `484936948405B82CAB55EACC91D030E249894D724B8737175AC6D971418CCD8A`

The source explicitly requests the local Phase 1 MVP described in
`docs/PROJECT_CHARTER.md` and explicitly excludes real model use, the playmate
website, Hostinger login, and cloud deployment.

## Missing Referenced Artifacts

The source text references:

- `LATTICE_DevOS_完整專案藍圖_v1.0.md`
- `LATTICE_Codex_主建置Prompt_v1.0.md`
- `LATTICE_DevOS_Blueprint_v1.0.zip`
- a 17-file blueprint package including `08_CODEX_MASTER_PROMPT.md`

Those files were not present in the supplied attachment directory or the empty
starting workspace. An exact-name/path search in the user-scoped Documents,
Downloads, and OneDrive Documents locations also found no LATTICE blueprint or
existing `lattice-devos` repository on 2026-07-29.

This repository must therefore distinguish:

- confirmed product requirements copied into the charter/specification; and
- conservative Phase 1 design decisions recorded as ADRs.

If a missing blueprint is recovered later, compare it against the current
specification and update `PLANS.md` before changing implementation.

