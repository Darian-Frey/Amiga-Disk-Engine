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

- Cargo workspace over the existing skeleton: `ade-endian`, `ade-block`, `ade-container`, `ade-track`, `ade-flux`, `ade-filesystem`, `ade-object`, `ade-catalogue`, `ade-core`, and the `ade-cli` binary. Layer crates depend downward on abstractions only — `ade-block` defines the `BlockSource` seam that `ade-container` and `ade-track` implement, so it depends on neither.
- `ade-endian` — the C-001 seam: bounds-checked, overflow-safe big-endian accessors with typed errors carrying the failing offset.
- `ade-block` — `Geometry` (DD/HD floppy constants, configurable block size), `BlockIndex`, and the `BlockSource` trait. AV-004 is enforced by the type system: `read_block` takes a `ValidBlock`, whose only constructor is `Geometry::validate` and whose field is private to the crate, so an unchecked index cannot reach a backing store.
- `ade-filesystem::dostype` — dostype decoding by documented flag bits (FFS / INTL / dircache), preserving unrecognised bits rather than discarding them. The authoritative table stays deferred to SPEC, so no unverified enumeration is committed.
- Enforcement of two invariants moved from prose into the build: `clippy.toml` disallows raw byte-order conversions outside `ade-endian` (C-001), and `tools/check-layering.py` fails on any cross-layer crate dependency (D-003). Both were verified against deliberate violations.
- Workspace lint set treating the untrusted-input mandate as machine-checkable: `unsafe_code` forbidden, and `unwrap_used` / `expect_used` / `panic` / `indexing_slicing` / `arithmetic_side_effects` denied (D-006, F-001).
- GitHub Actions CI: fmt, clippy, tests, docs-without-warnings, and the layering check.

- **SPEC.md filled in from primary sources** (2026-08-22): bootblock, both checksum algorithms, the full dostype table, rootblock, directory hashing including the international variant, directory blocks, file header / extension / data blocks, bitmap, dircache, links, RDB / partition / FSHD blocks, hardfiles, container magics, and protection-flag semantics. Every section cited to Clévy's ADF FAQ, the Linux AFFS driver docs, the AmigaOS wiki, or the SuperCard Pro specification. ADFlib's source remains deliberately excluded per D-002.
- **C-006** — new constraint: dircache and LNFS imply international hashing. `DOS\4` and `DOS\5` set the dircache bit and leave the INTL bit clear yet are international; `DOS\6`/`DOS\7` always are. Deciding the hash function from bit 1 alone breaks directory lookup silently.
- **C-007** — new constraint: the bootblock's rootblock pointer reads 880 even on HD volumes whose rootblock is at 1760. It must not be trusted; the location is computed.
- `Mode` enum in `ade-filesystem::dostype` (`Classic` / `DirCache` / `LongNames`) and `Dostype::intl_flag_set()`, so callers cannot reconstruct the bit-decoding trap for themselves.
- `Geometry::reserved()`, `Geometry::FLOPPY_RESERVED`, and `GeometryError::ReservedExceedsVolume`.

### Fixed
- **BUG-001** (high) — `Dostype::is_international()` was the naive bit test and wrong for `DOS\4`/`DOS\5`, which are international with the INTL bit clear. Investigation found a second error the report had missed: `DOS\6`/`DOS\7` are the bit combinations the classic encoding left unused *because* dircache implies international, which is why LNFS claimed them — so they are dostypes, not bit patterns, and the old `has_dircache()` wrongly reported them as dircache volumes. Replaced bit predicates with a whole-byte `Mode` resolution. Verified against 4288 TOSEC images: 20 would have hashed wrongly before the fix.
- **BUG-002** (low) — `Geometry::midpoint()` used `total_blocks / 2`; renamed to `root_block()` and reimplemented as the documented `(numReserved + highKey) / 2` (ADF FAQ §4.2). `Geometry` carries `reserved`; `new()` takes it and rejects `reserved >= total_blocks`. Fixed ahead of its deferral because the API change was cheapest before anything depended on it.

- **D-010 Accepted** (Option E) — no disk image is committed to this repository in any form. Fixtures are generated in code at test time; `tests/fixtures/` holds a manifest and documentation only. Chosen over the earlier lean towards committed freely-distributable disks because a generator is reviewable where a binary is opaque, covers the three dostypes absent from the 4288-image corpus, and removes the copyright question entirely rather than managing it. The stated cost: CI never exercises a real disk, so the local corpus run must be habit.

- **`ade-fixtures`** — the D-010 fixture generator. Builds structurally valid Amiga volumes in code: any geometry including 81–83 cylinders, all eight dostypes, OFS and FFS data layout with the reversed `data_blocks[]` table, hash-table insertion with same-hash chaining, an accurate bitmap (set bit = free), and both checksum algorithms as separate functions so they cannot be confused. A `corrupt` module supplies the structures no genuine disk contains: AV-001 self-cycles and two-block cycles, AV-003 bitmap-flag invalidation, AV-004 out-of-range pointers, plus non-`DOS` bootblocks, truncation and trailing junk drawn from the survey. Depends on no other ADE crate — it states the format independently so a misreading in a layer crate cannot cancel out against it.
- **Corpus differential test** — the generator's checksum arithmetic checked against 3976 real `DOS` images: 3226 of 3229 well-typed rootblocks validate, and the bootblock rate reproduces the 74.1% measured independently in Python. Skips cleanly when `disks/` is absent, so a fresh clone passes offline.
- **`ade-fixtures --bin manifest`** — emits `sha256 size name` rows for a corpus directory, with a dependency-free SHA-256 verified against `sha256sum`.
- **`tests/fixtures/README.md`** — states the no-images policy, how to build fixtures, and that CI never exercises a real disk.

- **`ade info <image>`** — the first working command, and the first vertical slice through the pipeline. Reports the container kind *with the evidence behind it*, the bootblock, and the volume as three independent facts (C-008), then lists faults.
- **`ade-block::checksum`** — both algorithms as separately named functions (`normal`, `boot`) rather than one with a flag, because confusing them is a silent-corruption bug.
- **`ade-container`** — `RawImage` (an in-memory `BlockSource`) and `sniff`, an evidence cascade rather than a magic lookup. Handles the fact that plain ADF has no signature, that 81–83-cylinder images exist, and that a `DOS` prefix is neither necessary nor sufficient.
- **`ade-filesystem`** — `bootblock` and `rootblock` read-only inspection, plus `datestamp` decoding with a dependency-free civil-date conversion. Out-of-range datestamps are reported, never normalised: folding 90 minutes into an hour would destroy the evidence.
- **`ade-core::inspect`** — wires the layers; the single seam the CLI consumes.
- **CLI exit codes** (F-015): 0 clean, 1 faults, 2 usage, 3 unreadable, **4 no AmigaDOS volume**. 4 is separate deliberately — 1054 of 4288 real images have no rootblock where one should be, and reporting those as "clean" would mislead while calling them faulty would be wrong.
- Verified against the full corpus: 4288 images, **zero crashes**, container detection matching the independent census exactly. Findings included 248 stale bitmap-valid flags (AV-003 in the wild), 3 rootblock checksum failures and 3 dostypes carrying undocumented bits — each matching the earlier Python analysis.

### Changed
- **SPEC §Corpus observations** — new section recording a survey of 4288 TOSEC Amiga ADF images, explicitly labelled measurement rather than specification. Magic distribution, the 300 non-`DOS` images across 144 distinct bootloaders, bootblock-checksum and rootblock validity rates, and the non-canonical size distribution.
- **SPEC §Extended-ADF** — the `UAE-1ADF` layout, derived from eleven corpus images and verified arithmetically (`12 + tracks × 12 + Σ space` equals file size for all eleven). Track type 0 is standard sector data and 1 is raw MFM; `length` is in **bits**, `space` in bytes. Mixed types within one image are the copy-protection signature.
- **C-008** — new constraint: ADF identification is heuristic and must be reported as such. Neither magic, size, bootblock checksum, nor the `DOS` prefix is decisive, and each claim is now backed by a measured figure.
- **SPEC §Geometry** — 80 cylinders is the norm, not the limit; 81/82/83-cylinder images occur and land exactly on `cylinders × 2 × 11 × 512`.
- **D-010** — records that a 4288-image corpus now exists locally outside version control, settling the differential-testing half of the decision in practice; the committed-fixture half stays open.
- **AV-001 widened.** Directory cycles are reachable on structurally valid disks, because AmigaDOS permits hard links to directories. Cycle detection is a correctness requirement for ordinary images, not only a defence against hostile input, and must use a visited-set rather than a depth limit.
- **AV-003 corroborated** by the Linux AFFS documentation, which warns the bitmap-valid flag may be inaccurate after a crash — a routine condition, not only an attack. Noted also that a *set* bitmap bit means *free*, the opposite of the usual convention.
- **F-003** records the sniffing cascade: a plain ADF has no magic number, so content dispatch cannot be a magic lookup.
- Copyright holder in `LICENSE`, `NOTICE`, and workspace `authors` corrected from the bare handle to `Shane Hartley (Darian-Frey)`, so the notice names a legal person. Done before any external contribution, while it is still a one-party change. Recorded under D-011.
- **D-001 → Accepted** (Option A): Rust core exposing a C-ABI bridge, Qt6 GUI over it from Phase 5. The flux/IPF-binding half of the original reversal condition was withdrawn as unfounded.
- **D-002 → Accepted**, scope narrowed to ADFlib alone, decided as a new Option D: reimplement OFS/FFS/RDB in Rust with ADFlib as a **black-box differential-test oracle** — never linked, source never read. Rejects the previously-leaned hybrid, whose safety gap would have made F-001 unclaimable.
- **D-008** — deferral **discharged** and closed out: D-002 inherited no GPL obligation, and the resulting free choice was made the same day (see D-011). The entry stays in the register as the audit trail for the period without a licence.
- ARCHITECTURE, BUILD, SPEC, ROADMAP, FEATURES, ATTACK_VECTORS (AV-005), CLAUDE, and both READMEs updated for the settled stack. BUILD.md now distinguishes build dependencies from test-only ones; SPEC.md records that ADFlib's source is deliberately excluded from its reference list.

### Notes
- The workspace builds, tests (18 passing), lints clean, and documents without warnings. The engine is still a scaffold: no image is parsed yet, and `ade` has no commands.
- Next: the first vertical slice, `ade info <image>` — geometry, dostype, bootblock checksum — chosen over building layers bottom-up so that every seam is exercised before integration rather than after.
- `CLAIMS.md` omitted (non-research); `VOCABULARY.md` deferred (ManifeST contract undefined); `LICENSE` outstanding per D-008. Publication is gated on both `LICENSE` and D-010.
