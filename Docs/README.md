> **Status:** Active
> **Provenance:** Claude (primary auditor / initial scaffolding, 2026-08-21)
> **Last reviewed:** 2026-08-21
> **Why this status:** Documentation-first scaffold in place and internally consistent; no code yet. The blocking stack decisions (D-001, D-002) were Accepted on 2026-08-21 and the licence settled (D-011). D-009 and D-010 remain open; neither blocks Phase 1.

# ADE documentation

Index of the Amiga Disk Engine documentation set. The project overview, formats, repository layout, and licence position live in the [root README](../README.md).

## The set

| Document | Contents |
|---|---|
| [FEATURES.md](FEATURES.md) | Capability list F-001…F-018, with priorities, effort, phase, and acceptance criteria |
| [ROADMAP.md](ROADMAP.md) | Phased plan, Phase 0…5, referencing feature IDs |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Layered pipeline, module responsibilities, invariants, cross-cutting concerns |
| [DECISIONS.md](DECISIONS.md) | Append-only design-decision log D-001…D-011, with reversal conditions |
| [SPEC.md](SPEC.md) | Authoritative disk and filesystem format reference; format constraints C-001…C-005 |
| [ATTACK_VECTORS.md](ATTACK_VECTORS.md) | Failure modes for untrusted disk-image input, AV-001…AV-005 |
| [BUILD.md](BUILD.md) | Environment, toolchain, build commands (stub until the first build succeeds) |
| [BUGS.md](BUGS.md) | In-repo bug catalogue, BUG-NNN (empty; populated during implementation) |
| [IMPROVEMENTS.md](IMPROVEMENTS.md) | In-repo refactor / code-quality catalogue, IMP-NNN (empty) |
| [CHANGELOG.md](CHANGELOG.md) | Version history, referencing F-/D-/C-/AV-/BUG-/IMP- IDs |
| [CLAUDE.md](CLAUDE.md) | Handoff contract for AI-assisted sessions |

## Which document answers what

- **What it does** → FEATURES.md
- **When** → ROADMAP.md
- **How it is put together** → ARCHITECTURE.md
- **Why it is put together that way** → DECISIONS.md
- **How the formats work** → SPEC.md
- **What could go wrong** → ATTACK_VECTORS.md (forward-looking) and BUGS.md (backward-looking)

## Conventions

Append-only identifier registers (`F-`, `D-`, `C-`, `AV-`, `BUG-`, `IMP-`); entries are never deleted, only superseded via a status flag. Fixed status vocabularies per document. Every decision carries reversal conditions. Cross-references are added in both directions. British English; ISO 8601 dates.

## Deliberate omissions

- **`CLAIMS.md`** — omitted. ADE is a systems and forensic tool making no empirical or theoretical research assertions (legitimate Tier-3 non-applicability; no exemption entry required).
- **`VOCABULARY.md`** — deferred until the ManifeST catalogue-integration contract (F-013) is defined.
- **`LICENSE`** — **no longer omitted.** Deferred under **D-008** pending D-002; resolved on 2026-08-21 as Apache-2.0 (**D-011**) and added before the first public commit, with an accompanying `NOTICE`. D-008 stays in the register as the audit trail for the period in which no licence existed.
