# Changelog

Format follows [Keep a Changelog](https://keepachangelog.com). Reference F-, D-, C-, AV-, BUG-, and IMP- IDs for traceability.

## [Unreleased]

### Added
- Initial documentation scaffold to the project-scaffold standard: README, FEATURES (F-001…F-018), ROADMAP (Phase 0…5), ARCHITECTURE, DECISIONS (D-001…D-008), SPEC (with format constraints C-001…C-005), ATTACK_VECTORS (AV-001…AV-005), BUILD (stub), BUGS, IMPROVEMENTS, CLAUDE.
- Feature set and gap analysis derived from a survey of present-day Amiga disk tooling.
- Root `README.md` as the repository landing page; `Docs/README.md` reduced to an index of the documentation set.
- Stack-neutral directory skeleton mirroring the layered pipeline (D-003): `src/{endian,flux,track,block,filesystem,object,catalogue,container,api}`, `cli/`, `gui/`, `tests/{fixtures,fuzz,unit,integration}`, `tools/`. No build files — the tree commits to the architecture without pre-empting D-001.

- **D-009** — new decision entry: xDMS's role (wrap / port / reimplement). Split out of D-002; deferred to Phase 2; turns on xDMS's licence, which is not yet established.
- **D-010** — new decision entry: test-fixture provenance. TOSEC Amiga images are copyrighted and cannot simply be committed to a repository intended to become public; records the gap and the options.

- **D-011** — new decision entry: licence is **Apache-2.0**. `LICENSE` and `NOTICE` added at the repository root before the first public commit. NOTICE records that ADE contains no third-party code and that ADFlib is a test oracle, not a dependency.
- `.gitignore`, including a **D-010 tripwire** — disk-image extensions under `tests/fixtures/` are ignored until fixture provenance is decided, so copyrighted TOSEC images cannot be committed by accident.

### Changed
- **D-001 → Accepted** (Option A): Rust core exposing a C-ABI bridge, Qt6 GUI over it from Phase 5. The flux/IPF-binding half of the original reversal condition was withdrawn as unfounded.
- **D-002 → Accepted**, scope narrowed to ADFlib alone, decided as a new Option D: reimplement OFS/FFS/RDB in Rust with ADFlib as a **black-box differential-test oracle** — never linked, source never read. Rejects the previously-leaned hybrid, whose safety gap would have made F-001 unclaimable.
- **D-008** — deferral **discharged** and closed out: D-002 inherited no GPL obligation, and the resulting free choice was made the same day (see D-011). The entry stays in the register as the audit trail for the period without a licence.
- ARCHITECTURE, BUILD, SPEC, ROADMAP, FEATURES, ATTACK_VECTORS (AV-005), CLAUDE, and both READMEs updated for the settled stack. BUILD.md now distinguishes build dependencies from test-only ones; SPEC.md records that ADFlib's source is deliberately excluded from its reference list.

### Notes
- Planning stage: no code yet, but implementation is unblocked — D-001 and D-002 are settled.
- `CLAIMS.md` omitted (non-research); `VOCABULARY.md` deferred (ManifeST contract undefined); `LICENSE` outstanding per D-008. Publication is gated on both `LICENSE` and D-010.
