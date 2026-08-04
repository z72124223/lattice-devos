# Request Boundary

> Historical source boundary. The direct user clarification recorded in
> `DIRECTION_CHANGE_2026-07-29.md` is the current product-direction authority.
> This file is retained unchanged in substance for provenance.

## Authoritative Input

- Received: 2026-07-29 (Asia/Taipei)
- Source file:
  `C:\Users\f7212\.codex\attachments\4b86d7ee-43ef-49b1-97af-89147d0b0352\pasted-text.txt`
- SHA-256:
  `484936948405B82CAB55EACC91D030E249894D724B8737175AC6D971418CCD8A`

The source explicitly requested the local Phase 1 MVP later captured by
SPEC-001 and the then-current V1 charter in repository history. It also
explicitly excluded real model use, access to the example project, managed
service login, and cloud deployment during that phase. The current
`docs/PROJECT_CHARTER.md` is the later V2 charter and must not be read back into
this historical source.

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
