# Decisions

Append-only log of significant design decisions. Each entry: D-NNN with Decided/Recorded dates (ISO 8601), status, context, options, decision, consequences, and reversal conditions. Never delete; supersede with a status flag.

Status vocabulary: Proposed | Accepted | Superseded by D-NNN | Deprecated.

---

### D-001 Language / stack
**Decided:** 2026-08-21
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** F-002, F-004, D-002, D-003, D-006

**Context.** The core, CLI, and GUI need a language/stack. Two precedents exist in the ecosystem: the Atari Disk Engine (Qt/C++) and Pontus (Rust core + Qt6 GUI via a C-ABI bridge).

**Options.**
- **A. Rust core + Qt6 GUI via C-ABI bridge (Pontus pattern).** Chosen. Memory-safety aids the untrusted-input mandate (D-006); avoids the C++ god-class gravity that hurt the ST engine; reuses the Pontus bridge.
- **B. Qt/C++ throughout (Atari Disk Engine precedent).** Rejected. Familiar and simplest for GUI integration, but risks inheriting the same architectural drift, and leaves every bounds check on untrusted input as a discipline rather than a guarantee.

**Decision.** Option A. Rust core exposing a C-ABI bridge; Qt6 GUI over that bridge from Phase 5.

Three arguments carried it beyond the original framing:

1. **The crate boundary enforces D-003 mechanically.** In C++, "no module spans two layers" is a review convention that nothing prevents an `#include` from violating — which is how the ST god-class accreted. A Cargo workspace makes each pipeline layer a crate, so a cross-layer dependency must be declared where it is visible.
2. **Memory safety and D-006 are the same decision.** AV-004 (out-of-range block pointers → wild reads) is structurally unavailable in safe Rust rather than being a discipline sustained indefinitely. D-006 is rated never-reversible; the stack should match that ambition.
3. **The binding risk was overstated.** CAPS is a closed binary with a plain C API (`bindgen` territory), and Greaseweazle is not a C library at all — it is a USB/serial protocol plus a Python host tool, so a protocol client must be written in either language.

**Consequences.** Fixes BUILD.md's toolchain (Rust + Qt6 + a C-ABI bridge). `src/<layer>/` becomes a Cargo workspace of `ade-*` crates — additive to the existing skeleton, no renames. Fuzzing (F-001) runs through `cargo-fuzz`/libFuzzer. Raises the cost of any FFI dependency, which materially shaped D-002. The concurrency model for F-014 (previously "TBD with the stack") can now be settled against Rust's model.

**Reversal conditions.** Reverse to B only if corpus-throughput measurements on real workloads (F-014) show the bridge or the Rust core to be the bottleneck by a wide margin. The flux/IPF binding half of the original reversal condition is withdrawn as unfounded — see argument 3 above.

---

### D-002 ADFlib — wrap vs reimplement
**Decided:** 2026-08-21
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** F-001, F-003, D-001, D-006, D-008, D-009, D-010

**Scope note.** As originally recorded this entry covered both ADFlib and xDMS. The two are independent choices with different deadlines — ADFlib gates Phase 1, xDMS is not needed until Phase 3 — so bundling them held Phase 1 hostage to the harder, less urgent question. Scope was narrowed to ADFlib before acceptance; xDMS moved to **D-009**.

**Context.** ADFlib is a mature C library covering OFS/FFS/RDB — the exact surface of Phase 1 — against reimplementing that handling behind ADE's own trait seams. D-001 (Rust core) was settled first and changed the arithmetic materially.

**Options.**
- **A. Wrap ADFlib via FFI.** Rejected. Fast route to a correct read-only tool over battle-tested code, but see the decision below: from Rust the cost is higher and the benefit is negative.
- **B. Pure reimplementation.** Superseded by D. Clean owned codebase and licence freedom, but discards ADFlib's accumulated correctness with nothing put in its place.
- **C. Hybrid — wrap first, reimplement layer-by-layer behind stable seams.** Rejected. Reaches a working tool quickly, but leaves the safety guarantee absent for precisely the period it is being relied upon, and is not licence-neutral: the first public release would be GPL regardless of what is later replaced.
- **D. Reimplement, with ADFlib as a black-box test oracle.** Chosen. Option B plus differential testing against ADFlib run as a separate binary.

**Decision.** Option D. Reimplement OFS/FFS/RDB handling in Rust behind ADE's own trait seams, with **no FFI dependency on ADFlib in any shipping path**. ADFlib is invoked as a separate binary in the test harness only, its output diffed against ADE's across the fixture corpus.

Format knowledge is taken from public documentation — Clévy's ADF FAQ, the Linux AFFS driver documentation, RKRM: Devices Appendix C — and **not** from reading ADFlib's GPL source. Running a GPL binary and comparing its output creates no derived work; reading its source to reimplement would muddy provenance and spend the licence freedom that is half the point of this decision.

Rationale, in short:

1. **It is the only option under which F-001 is satisfiable.** A segfault inside wrapped C is not a `Result`, is not catchable by `catch_unwind`, and takes the fuzz harness down with it. "Zero panics/segfaults across the corpus" cannot be claimed for code ADE does not own — and F-001 names adftools, ADFlib's own front-end, as the baseline to clear.
2. **Under D-001 the hybrid's speed advantage largely evaporates.** From Rust, wrapping costs `bindgen`, a `-sys` crate, `repr(C)` mirrors, and `unsafe` at every call site; reimplementation gets the defensive bounds-checking F-001 demands for free.
3. **Wrapping shapes ADE's seams around another library's API**, working against D-003.
4. **Licence freedom is preserved**, discharging D-008 immediately.

**Consequences.** Slower to a first working tool. ADE owns all of SPEC.md rather than inheriting it. The licence becomes a free choice — D-008's reversal condition fires on this entry's acceptance. Differential testing becomes a Phase 1 dependency rather than a nicety, which makes fixture provenance (**D-010**) load-bearing sooner than the roadmap assumed.

**Accepted cost.** ADFlib encodes roughly twenty-five years of edge-case handling for real-world disks that appears in no specification, and reimplementation forgoes it by default. Differential testing recovers much of it *as failing tests rather than as inherited folklore*, which is the better outcome — but only in proportion to corpus size, so the value of this decision is coupled to D-010.

**Reversal conditions.** Reverse to wrapping (A or C) if reimplementation misses Phase 1 acceptance by a wide margin on real fixtures — specifically, if differential testing shows systematic divergence that the public documentation cannot explain. Note that reversing forfeits the licence freedom already banked under D-008, so it is a genuine setback and not a cheap fallback.

---

### D-003 Layered, trait-seamed architecture; no god-class
**Decided:** 2026-08-21
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Claude (primary auditor)
**Related:** F-002, ARCHITECTURE.md

**Context.** The Atari Disk Engine grew a central god-class that still owes a refactor. That is the single most expensive lesson to carry forward.

**Options.**
- **A. Layered pipeline with a trait/interface seam per layer.** Chosen.
- **B. Pragmatic single-module core, refactor later.** Rejected — this is precisely how the ST god-class formed.

**Decision.** Option A. Flux → track → block → filesystem → object model → catalogue, each a separately-testable module; no module spans more than one layer.

**Consequences.** More upfront interface design; enables the incremental reimplementation path (D-002 option C) and independent testing.

**Reversal conditions.** Never within v1. This is the headline architectural commitment.

---

### D-004 Read before write
**Decided:** 2026-08-21
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Claude (primary auditor)
**Related:** F-001, F-010, F-019, F-025, D-018, D-024, ROADMAP.md

> **Discharged in two steps.** Creating a *new* disk was permitted on these terms by F-019 (2026-08-29) and widened by F-025. Writing *into* an existing one — the case this entry is actually about — is settled by **[D-018](#d-018-writes-into-an-existing-disk-are-buffered-saved-explicitly-and-never-over-the-file-they-came-from)** (2026-09-01), which keeps the safety property by never writing over the file an image was opened from.

**Context.** Write/format paths are where a disk tool does irreversible damage. Reading is safe and validates understanding of a format.

**Options.**
- **A. Ship read-only extraction before any write/create path.** Chosen.
- **B. Read and write together per format.** Rejected — couples risk and slows a safe first release.

**Decision.** Option A. Every write path ships only after its read path is proven on fixtures.

**Consequences.** Phase 1 is read-only; write appears from Phase 4/5. Users get a safe tool sooner.

**Reversal conditions.** Never within v1.

---

### D-005 Raw-MFM-capable internal model from day one
**Decided:** 2026-08-21
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Claude (primary auditor)
**Related:** F-007, F-008, ARCHITECTURE.md, SPEC.md §Flux

**Context.** On the Atari side, STX/Pasti (protected formats) were the hard part and suffered from being bolted on late. The Amiga analogue is extended-ADF / SCP / IPF. The trap is assuming "plain ADF" and retrofitting track data.

**Options.**
- **A. Internal model can represent a raw MFM track from the start**, even while Phase 1 only populates decoded sectors. Chosen.
- **B. Decoded-sectors-only model, extend later.** Rejected — reproduces the ST bolt-on problem.

**Decision.** Option A.

**Consequences.** Slightly heavier model early; flux support in Phase 4 slots into an existing shape rather than forcing a rewrite.

**Reversal conditions.** Never.

---

### D-006 Forensic / untrusted-input stance
**Decided:** 2026-08-21
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Claude (primary auditor)
**Related:** F-001, F-010, ATTACK_VECTORS.md (AV-001…AV-005)

**Context.** Disk images are an untrusted-input attack surface: bootblock viruses, hash-chain loops, out-of-range pointers, decompression bombs. The ST engine learned this reactively.

**Options.**
- **A. Adopt a forensic stance up front** — bounds-check everything, never execute guest code, fuzz the parsers, maintain an attack-vector register. Chosen.
- **B. Handle robustness issues as they arise.** Rejected — reactive hardening leaves windows open and is costlier.

**Decision.** Option A.

**Consequences.** F-001 and the ATTACK_VECTORS register are load-bearing from Phase 1; fuzzing is part of the Phase-1 acceptance bar.

**Reversal conditions.** Never.

---

### D-007 SCP as the open flux target; IPF read-only and optional
**Decided:** 2026-08-21
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Claude (primary auditor)
**Related:** F-003, F-006, F-007, C-003 (SPEC.md)

**Context.** IPF creation is closed (SPS-only) and the CAPS read library is restrictively licensed. SCP is the open, documented flux container, supported by the open Greaseweazle/FluxEngine toolchain.

**Options.**
- **A. Target SCP (and extended-ADF) for the open write path; treat IPF as optional read-only behind a licence-gated flag.** Chosen.
- **B. Build the flux path around IPF.** Rejected — cannot legally/openly create IPF, and the read library's licence is restrictive.

**Decision.** Option A.

**Consequences.** ADE cannot emit IPF (C-003); write-back and hardware writing go via SCP/extended-ADF. IPF-read is a compile-time optional feature.

**Reversal conditions.** Revisit if SPS opens IPF creation, or an open IPF writer appears.

---

### D-008 LICENSE deferred pending D-002
**Decided:** 2026-08-21
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Claude (primary auditor)
**Related:** D-002, README.md §License

**Context.** `LICENSE` is a Tier-1 document under the project-scaffold standard; a Tier-1 omission must be recorded as a decision. ADE's licence is genuinely undetermined because it is coupled to D-002: wrapping ADFlib (GPL) via FFI would propagate GPL, whereas a pure reimplementation leaves the choice open. Choosing a licence now would pre-empt D-002.

**Options.**
- **A. Defer the licence choice until D-002 is Accepted**, and add `LICENSE` before the first public commit. Chosen.
- **B. Pick a permissive licence now.** Rejected — may be invalidated by a GPL dependency under D-002.
- **C. Pick GPL now.** Rejected — pre-commits to the wrapping path before D-002 is decided.

**Decision.** Option A. No `LICENSE` file until D-002 lands; this entry is the audit trail for the Tier-1 omission.

**Consequences.** The repository must not be made public until a licence is added. `CLAIMS.md` and `VOCABULARY.md` are separately omitted as legitimate Tier-3 non-applicability (non-research; sibling contract not yet defined) and need no exemption entry.

**Update 2026-08-21.** D-002 was Accepted on 2026-08-21 as Option D (reimplementation, no ADFlib linkage), so no GPL obligation is inherited and the licence is now a free choice. **This entry's trigger has fired**: the deferral is discharged and licence selection is the outstanding action, not a pending one. The repository remains not-public until `LICENSE` exists. D-009 (xDMS) could in principle re-couple the licence, but only from Phase 3 and only if that decision lands on wrapping rather than porting — it is not a reason to keep deferring.

**Resolved 2026-08-21.** `LICENSE` (Apache-2.0) added before the first public commit; both trigger conditions were met on the same day. This entry is **discharged** — the licence choice itself is recorded as **D-011**, which supersedes this entry's forward-looking obligation. D-008 remains in the register as the audit trail for the period in which no `LICENSE` existed.

**Reversal conditions.** Resolve and add `LICENSE` the moment D-002 is Accepted, or immediately before any public commit — whichever comes first. *(Both met 2026-08-21; see D-011.)*

---

### D-009 xDMS — wrap vs port vs reimplement
**Decided:** 2026-08-23
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** F-003, F-016, D-002, D-006, D-008, AV-005, C-004 (SPEC.md)

**Context.** Split out of D-002, which originally bundled ADFlib and xDMS. DMS (DiskMasher) is a proprietary format fully reverse-engineered by xDMS across all compression modes including encryption. It is not needed until **Phase 3**, so this decision does not gate implementation and is deliberately left open.

The considerations differ from D-002 in two ways. First, DMS is the decompression-bomb surface (AV-005) — malformed input and password/encryption paths are exactly where resource exhaustion bites — so safety matters more here than anywhere else in the container front-end. Second, **xDMS's licence is not yet established**; D-002's GPL reasoning was specific to ADFlib and does not transfer. That must be confirmed before this entry can be decided, because it determines which options are even available.

**Options.**
- **A. Wrap xDMS via FFI.** Fastest, but carries D-002's objection in the place it matters most: a crash or unbounded allocation inside wrapped C is uncatchable from Rust, and AV-005 is precisely that failure mode.
- **B. Port xDMS to safe Rust.** Owned code, no FFI, safety across the bomb surface, and the decompressors are translated rather than rediscovered. Requires a licence permitting derivation, and requires attribution.
- **C. Reimplement from the format description, with xDMS as a black-box oracle** (the D-002 posture). Cleanest provenance, but DMS's multiple compression modes are fiddly and far less well documented than OFS/FFS — this re-solves genuinely hard reverse-engineering rather than well-specified structures.

**Decision.** Option B — **port xDMS to safe Rust**.

The licence question that blocked this entry is resolved. xDMS is **Public Domain**: "xDMS is released as public domain software. You can spread it, modify it and use it in any way you like. You can do anything with it without even asking me first." (`/usr/share/doc/xdms/copyright`, Andre Rodrigues de la Rocha and Heikki Orsila.) Derivation is permitted outright, so a port carries no obligation and ADE's Apache-2.0 licence (D-011) is unaffected. Attribution goes in `NOTICE` as a courtesy and because the author asked to hear about it.

Note the deliberate asymmetry with D-002. There, ADFlib's source is **not** to be read, because GPL provenance would cost the licence freedom. Here, reading and translating the source is legitimate and is the entire basis of this option.

Porting rather than wrapping is what gets safety across AV-005 — and that vector is no longer theoretical. The 2026-08-22 oracle work found ADFlib's `unadf` allocating **29 GB** on an ordinary uncompressed ADF and crashing on 15 of 4288 real images. A DMS decompressor handling attacker-controlled lengths is a strictly worse place to have unbounded allocation, and a fault inside wrapped C is uncatchable from Rust.

Porting rather than reimplementing (option C) is what avoids redoing reverse-engineering that was already done well: the compression modes are undocumented outside this source, and rediscovering them from DMS files would be weeks of work with no better outcome.

Note the asymmetry with D-002: there, ADFlib's source is *not* to be read, because GPL provenance would cost the licence freedom. Here, if xDMS proves permissively licensed, reading and translating the source is legitimate and is the whole basis of option B.

**Consequences.** Determines whether DMS handling is owned Rust or an FFI dependency, and whether the licence chosen under D-008 needs revisiting from Phase 3. Bounded by C-004 regardless: some DMS images are known-bad and will not round-trip, and ADE must fail loudly rather than emit a silently-bad ADF.

**Blocked on test data.** The port cannot begin responsibly: there is **not one DMS file** on this machine — the corpus is 4288 `.adf` and nothing else — and `xdms` only unpacks, so it cannot manufacture test input either. Writing a decompressor with nothing to decompress would produce code that compiles, passes its own tests, and is unverified against reality. That is precisely the failure D-002 and D-010 were written to avoid, and here it is worse than for ADF: a compression format has no self-describing structure to fixture against, so a hand-built synthetic DMS would only test the port against my own reading of the source.

**Prerequisite.** Obtain real DMS images before writing the decompressors — TOSEC ships DMS sets, and Aminet hosts freely-distributable ones. The `errdms`-tagged images matter especially, since C-004 requires ADE to fail loudly on them rather than emit a silently-bad ADF, and that behaviour cannot be implemented against files we do not have.

**Reversal conditions.** Reverse to wrapping (A) only if the port cannot reproduce `xdms`'s output byte-for-byte across a real corpus and the divergence cannot be explained. Note that `xdms` becomes this decision's oracle exactly as ADFlib is D-002's — with the difference that its source is readable, so a disagreement can be traced rather than merely observed.

---

### D-010 Test-fixture provenance
**Decided:** 2026-08-22
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** F-001, D-002, D-006, D-008, AV-001, AV-004, ROADMAP Phase 0

**Context.** Phase 0 calls for a curated TOSEC Amiga fixture set to be "checked in and labelled known-good / known-bad", and `tests/fixtures/` exists for it. But TOSEC Amiga images are overwhelmingly commercial software under copyright, and D-008 intends this repository to become public. Committing them would be unlawful distribution and a plausible takedown target. No decision currently records this, and the interaction with D-008 is direct: **the licence is not the only thing gating publication.**

D-002 raised the stakes. With ADFlib reduced to a black-box oracle, differential testing over a corpus is now the primary mechanism for recovering the edge-case knowledge that reimplementation forgoes — so fixture breadth is load-bearing for correctness, not merely for coverage.

**Options.**
- **A. Commit everything.** Rejected on sight; recorded only to note it was considered.
- **B. Commit checksums plus a fetch script.** Contributors acquire TOSEC themselves; CI needs an out-of-band corpus. Keeps the repo clean but makes the test suite non-hermetic and unrunnable on a fresh clone.
- **C. Private sibling repository for real fixtures.** Hermetic for those with access; opaque to outside contributors, and awkward for CI on a public repo.
- **D. Committed fixtures restricted to freely-distributable disks, plus hand-authored synthetic images.** Superseded by E. Fred Fish / AmigaLibDisk / permissively-licensed demo disks for the happy path, with crafted images for the malformed corpus. The original lean; its weakness is that "hand-authored" was assumed to mean committed binaries.
- **E. Generate fixtures in code; commit no image data at all.** Chosen. A fixture generator constructs images deterministically at test time; the repository holds the generator, a block-level fuzz seed corpus, and a name-plus-hash manifest for the local differential corpus.

**Decision.** Option E. **No disk image is committed to this repository, in any form, ever.** `tests/fixtures/` holds a manifest and documentation; the images themselves are built at test time by a generator kept under version control as source.

The refinement from D to E came from acquiring the corpus (4288 TOSEC images, 2026-08-22) and seeing what it could and could not do.

1. **A generator is readable where a binary is opaque.** `hash_chain_loop()` states in code what structure it builds and why; an 880 KB blob states nothing, diffs meaninglessly, and can only be trusted by whoever produced it.
2. **It covers what no corpus will supply.** The 4288-image survey contains no `DOS\4`, `DOS\6` or `DOS\7`. A generator emits all eight dostypes trivially — including the LNFS pair that no realistic collection will hand us. Since `DOS\5` was the case that exposed BUG-001, the untested dostypes are exactly where the next defect is likely to sit.
3. **AV-001 and AV-004 were always going to be code.** Hash-chain loops and out-of-range block pointers do not occur on genuine disks; they must be constructed. Committing them as binaries would have been storing the generator's output instead of the generator.
4. **The copyright question disappears** rather than being managed. There is no whitelist to maintain, no per-disk licence verification to keep current, and no standing obligation. A disk image in the repository becomes a mistake by definition.

**On the generator/parser agreement risk.** A generator written by the same hand as the parser can encode the same misreading of the spec, and both will agree. This is real and is *not* addressed by option E on its own — it is addressed by D-002's black-box oracle, which tests against an independent implementation rather than against our understanding.

**Amendment, 2026-08-24 — the oracle validates *generated* fixtures too.**

As first written, this entry assumed the oracle could only be pointed at real images, and therefore that anything ADE had to synthesise was validated by nothing but its own author. That framing was wrong, and it made whole areas of the roadmap look untestable when they are not.

ADFlib reads any structurally valid volume, regardless of who wrote it. So the loop closes without a single real image:

```
ade-fixtures generates  →  unadf reads it  →  ADE parses it  →  compare
```

An independent implementation checks the generator *and* the parser, which is precisely the shared-misreading failure this paragraph was worried about. Demonstrated on a generated 1.76 MB HD volume — `unadf` reports "Floppy HD, 80 cylinders, 22 sectors, FFS" and lists its contents — and on all eight dostypes carrying an accented filename, the case where international hashing decides whether a lookup succeeds (C-006).

The consequence is not small. Phase 2 covers HD geometry, the full dostype matrix, LNFS and RDB partitioning, and the corpus contains **none** of them: no HD image, no `RDSK`, no `DOS\4`/`\6`/`\7`, no links in 8865 entries sampled. All of it appeared to be fixture-only and therefore weakly validated. It is not: each can be generated and cross-checked.

What the oracle still cannot supply is what the *specification omits* — the accumulated real-world behaviour D-002 accepted losing. A generated fixture is only ever as good as SPEC, and SPEC is only as good as its sources. Real images remain the sole evidence of how disks actually behave, and the corpus has repeatedly earned that role: BUG-001 was confirmed on 20 real `DOS\5` images, D-012 emerged from two files no synthetic fixture would have produced, and BUG-004 surfaced because real disks and generated ones disagreed.

So the two mechanisms still cover different ground, but the boundary is not where this entry first drew it. It is not *generated versus real*; it is **conformance to the specification versus conformance to reality**. The oracle can check the first on anything. Only the corpus can check the second.

**Consequences.** `tests/fixtures/` contains no image data; the repository stays small and every fixture is reviewable as source. The test suite is hermetic and offline on a fresh clone. Differential tests against the local corpus skip cleanly when `disks/` is absent.

The cost, stated plainly: **CI never exercises a real Amiga disk, and never runs the oracle.** It verifies that the specification is implemented, not that reality matches the specification — and the survey already showed those differ, with 19% of `DOS`-magic images carrying no rootblock and only 74% a valid bootblock checksum. Running the corpus locally therefore has to be a habit rather than an optional extra, and a finding that only the corpus can catch will not be caught by a pull request.

The 2026-08-24 amendment narrows that cost without removing it. Oracle checks over generated fixtures need no corpus — only `unadf` — so they **do** now run in CI, putting an independent implementation in the loop on every push. What CI still never sees is a real Amiga disk.

**Implementation note.** Fuzzing (F-001) should target the **block** level rather than whole images: a rootblock parser takes 512-byte inputs and the container sniffer takes short headers, so seeding with 880 KB images would spend almost the entire fuzz budget on bytes no parser reads. This also keeps the committed seed corpus genuinely small.

**Manifest.** `tests/fixtures/corpus.manifest` records TOSEC canonical name plus SHA-256 for images used in differential tests. Names and hashes are metadata, not content, so nothing copyrighted is distributed, and anyone holding their own TOSEC set can reproduce a specific finding exactly.

**Update 2026-08-22.** A working corpus of **4288 TOSEC Amiga ADF images** now exists locally in `disks/`, excluded from version control by `.gitignore` (which ignores disk-image extensions repository-wide, not merely that directory). That settles the *differential-testing* half of this decision in practice: the wide corpus lives outside git, which is option B's posture without the fetch script. What remains open is the *committed-fixture* half — whether anything at all may live in `tests/fixtures/` so the suite is runnable on a fresh clone. The survey in SPEC §Corpus observations is the first return on having it, and it has already produced C-008 and confirmed BUG-001 against 20 real images.

**Reversal conditions.** Revisit only if some format proves genuinely infeasible to synthesise — a real protected-disk MFM track is the plausible candidate, at Phase 4. Even then the answer is more likely a generator that emits flux than a committed binary. Reversal means committing image data, so it requires establishing distribution rights for each file and accepting a permanent obligation to keep that verification current; it is not a convenience to be reached for when a fixture is awkward to build.

---

### D-011 Licence — Apache-2.0
**Decided:** 2026-08-21
**Recorded:** 2026-08-21
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** D-002, D-008, D-009, C-003 (SPEC.md)

**Context.** D-008 deferred the licence because D-002 might have inherited GPL by linking ADFlib. D-002 landed on reimplementation, so nothing is inherited and the choice was free. It had to be made before the first public commit: the GitHub repository is public, and absent a `LICENSE` the default is all-rights-reserved — no lawful use, fork, modification, or contribution, which is an untenable footing for a preservation tool whose value depends on adoption.

**Options.**
- **A. Apache-2.0.** Chosen.
- **B. MIT.** Rejected. Shortest and most familiar, but no patent grant and no attribution scaffolding — both of which carry more weight here than in a typical project.
- **C. MPL-2.0.** Rejected. File-level weak copyleft is a reasonable middle ground, but the concern it addresses (a closed fork of the engine) is not the operative risk for a forensic tool whose value is in adoption and scriptability.
- **D. GPL-3.0.** Rejected. It is the Amiga ecosystem norm and ADFlib's own licence, but adopting it voluntarily would spend the freedom D-002 was partly chosen to win, and would bar permissively-licensed preservation tools from embedding ADE's core.

**Decision.** Option A, Apache-2.0. Three reasons specific to ADE:

1. **The explicit patent grant.** ADE implements formats that are reverse-engineered and in places proprietary (DMS, RDB, and potential IPF interop under C-003). A licence that settles patent posture explicitly is worth the extra length here.
2. **Institutional adopters.** The target users include archives, museums, and university collections, whose legal review is routinely more comfortable with Apache-2.0 than with bare MIT.
3. **The NOTICE convention** gives attribution a defined home — relevant if D-009 lands on a port of xDMS, which would require it.

A `NOTICE` file accompanies the licence. It currently records that ADE contains **no** third-party code, and states explicitly that ADFlib is a black-box test oracle rather than a dependency, so the provenance discipline of D-002 is legible from the distribution itself and not only from this register.

**Copyright holder.** `Copyright 2026 Shane Hartley (Darian-Frey)` in `LICENSE`, `NOTICE`, and the workspace `authors` metadata — legal name with the handle the project is published under, so the notice identifies a legal person while still matching the identity contributors will encounter. Corrected on 2026-08-22 from the handle alone, before any external contribution.

**Consequences.** Discharges D-008. Source files carry the standard Apache header once code exists. Third-party attribution goes in `NOTICE`. Note two licence surfaces that remain outside this decision: the optional CAPS library for IPF-read stays restrictively licensed and compile-time-gated (C-003), and D-009 could reintroduce a licence question at Phase 3 if it lands on wrapping rather than porting.

**Reversal conditions.** Relicensing after public release requires the consent of all contributors, so this is effectively one-way once external contributions land. Before that point, revisit only if D-009 forces a GPL dependency that cannot be avoided — which would be a reason to reopen D-009, not to relicense ADE.

---

### D-012 Protection bits are metadata, not access control
**Decided:** 2026-08-22
**Recorded:** 2026-08-22
**Status:** Accepted
**Authors:** Claude (analysis); recorded for review
**Related:** D-002, D-006, F-010, F-012

**Context.** Amiga directory entries carry protection flags whose owner bits are *inverted*: bit 3 set means "not readable". The first differential run against ADFlib (D-002's oracle) disagreed on exactly two files out of 2896, and both had the same cause — `Raffles.adf:s/startup-sequence` is flagged unreadable, so `unadf` reported `adfFileOpen : access denied` and wrote a **zero-byte file**, while ADE recovered all 144 bytes: a perfectly coherent AmigaDOS startup script.

**Options.**
- **A. Honour the protection bits**, as ADFlib does, refusing to read a file the disk marks unreadable. Rejected.
- **B. Treat the bits as metadata**: report them, never enforce them. Chosen.

**Decision.** Option B. Protection flags are recorded, displayed, and reported — never obeyed.

A permission bit set on a floppy in 1989 is a statement about how AmigaDOS *would have* behaved, not an instruction to a preservation tool running thirty-seven years later. There is no user to protect and no security boundary to hold: the bit is data about the disk, and refusing to recover content because of it is the tool failing at its purpose. It would also make F-012 (undelete and salvage) incoherent — salvaging a deleted file means ignoring what the filesystem says about accessibility by definition.

Reporting still matters. A file flagged unreadable is a fact worth surfacing in the health report (F-010), and ADE renders the full `hsparwed` string.

**Consequences.** ADE extracts content ADFlib will not, which is why the oracle will always show a small residual disagreement rather than converging on 100%. That residue is expected and should not be "fixed" by matching ADFlib. The differential test's threshold accommodates it deliberately.

Rare in practice: a 143-disk sample found no read-protected files and 46 delete-protected ones, so this affects a fraction of a percent — but the fraction it affects is exactly the content a forensic tool exists to reach.

**Reversal conditions.** If ADE ever gains a write path that could damage a source image, protection bits should gate *writes* — that is a different question from reads and would need its own entry. Read behaviour should not change.


---

### D-013 LNFS is deferred on verifiability, not effort
**Decided:** 2026-08-24
**Recorded:** 2026-08-24
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** D-002, D-010, D-012, C-006, BUG-001, F-018

**Context.** LNFS (`DOS\6`, `DOS\7`) is the last structural gap in Phase 2. The field-level pass SPEC asked for is done — the long-name entry block, the `TYPE_COMMENT` block and the root block are documented at byte offsets, and both structures reconcile against the classic layout and sum exactly to 512 bytes. The parsing work is small and well understood.

What stopped it is that **nothing could check the result.**

- **No corpus material.** `DOS\6` and `DOS\7` appear in none of the 4652 images held. Neither does `DOS\4`, but that was resolvable: the generator builds it and the oracle validates it, which is exactly the shape D-010's amendment describes.
- **No oracle.** ADFlib does not implement LNFS. It decodes the dostype by bit pattern, so `DOS\6` (`0b110`) and `DOS\7` (`0b111`) come back as `OFS INTL DIRCACHE` and `FFS INTL DIRCACHE` — the trap C-006 and BUG-001 exist to describe. Asked to use that cache, `unadf -c` hunts for blocks LNFS never had, fails three checks, and prints an empty listing while **exiting 0**.
- **No documented file header.** [AOS-LNFS] declares `LongNameUserDirectoryBlock`, the comment block and the root block. It declares no long-name *file* header. Its three file-only fields would be placed by inference from the canonical file/directory block shape — sound reasoning, but not a citation.

So an LNFS reader would be checked against SPEC and against itself, with one of its structures inferred. That is precisely the position D-002 accepted a known cost to avoid: reimplementation forgoes ADFlib's accumulated edge-case knowledge, and differential testing was the mechanism chosen to recover it. Here the mechanism is unavailable.

**Options.**
- **A. Implement now against SPEC alone**, marked unverified and gated behind an opt-in. Rejected. A reader nothing can check is a claim, not a capability, and shipping one behind a flag mostly moves the problem to whoever trusts the flag. The inferred file header makes it worse: the most likely error is silent, since both layouts share a primary type, a secondary type and a checksum algorithm, and a wrong reading still checksums.
- **B. Defer until material exists.** Chosen.
- **C. Source material first, then decide.** Folded into B's reversal conditions rather than treated as a separate path — hunting is worth doing, but it is not a reason to hold the phase open.

**Decision.** Option B. LNFS identification and hashing stay (they are correct, corpus-independent, and pinned by `dostype_lnfs.rs`); the block structures are not implemented. SPEC keeps the field-level pass, so the work is ready the moment it becomes checkable.

This is the same standard D-010 set, applied where it bites. D-010's amendment permits generated fixtures *because ADFlib validates them independently* — the boundary was never "generated versus real", it was "conformance to the specification versus conformance to reality". LNFS fails both halves at once: no independent validator, and one structure not in the specification either.

**Consequences.** Phase 2 completes without LNFS, which is recorded in ROADMAP rather than quietly dropped. Any `DOS\6`/`DOS\7` image encountered is identified correctly, hashed correctly, and read as a classic volume — which will produce empty or wrong names on real long-name entries. That is a worse outcome than refusing, so a reader that mounts an LNFS volume says so: `ade check` reports `lnfs-unsupported` as a warning, stating that names and comments are being read at their classic offsets and are unreliable while block accounting and checksums are not affected. Done 2026-08-24 alongside this entry — a silent wrong answer was the one outcome this decision must not produce.

The corpus's silence is evidence of rarity, not of unimportance — `DOS\5` appears twenty-one times and is the case that exposed BUG-001.

**Reversal conditions.** A single verified real `DOS\6` or `DOS\7` image reverses this: with one real disk, the fixture generator can be checked against reality and the inferred file header confirmed or corrected. An LNFS implementation appearing in ADFlib would reverse it too, by restoring the oracle. Neither is expected soon; both are cheap to act on, because SPEC already holds the layout.

---

### D-014 Bootblock text is reported; virus identification is deferred
**Decided:** 2026-08-25
**Recorded:** 2026-08-25
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** F-011, F-013, AV-002, D-002, D-010, D-013

**Context.** F-011 asks for bootblocks to be scanned against a signature set for known historical viruses. Two things were established before writing any of it.

**No signature database is publicly available.** The established Amiga method is a CRC of the bootblock matched against a "brainfile" — the mechanism `bootblock.library` and `xvs.library` use, with multiple CRCs per entry to absorb minor variants. The mechanism is documented; the data is not. Nothing found publishes the CRCs, and the surviving tools ship them as opaque binaries.

**Naive string matching returns the opposite of the truth.** Scanning all 4652 corpus bootblocks for the names of known strains found 100 disks mentioning "VIRUS", 6 naming "Byte Bandit", and one naming nine strains at once. **Every single one is an anti-virus bootblock**, not an infection:

- `ANOTHER VIRUS PROTECTOR V5.0 PROTECT AGAINST: BYTEWARRIOR LAMER Extermin. etc.`
- `BOOTBLOCK CANNOT BE INFECTED BY THE SCA-VIRUS, BECAUSE IT WAS GENERATED WITH THE VIRUS-PROTECTOR V1.0`
- `F6: Boot Sectors. Obelisk. North Star. SCA. Byte Bandit. Byte Warrior. Revenge. IRQ. Pentagon Circle. Lamer Exterminator. H.C.S.` — a virus killer's menu

Cracking groups routinely installed virus-killer bootblocks, so the disks that name viruses are disproportionately the *protected* ones. A name-matching scanner would flag precisely the disks that were cleaned, and would do it confidently.

**Options.**
- **A. Ship a signature scanner built from names found in descriptions.** Rejected outright. It is not merely unverified, it is measurably inverted, and a false "virus detected" on a preservation tool's report is worse than silence: it invites someone to destroy a sound disk.
- **B. Invent CRCs from reasoning about how the strains behave.** Rejected. There is no way to check them, and a scanner that matches nothing looks identical to a clean corpus.
- **C. Report the bootblock's text and let a person judge; defer identification.** Chosen.

**Decision.** Option C. ADE extracts and reports printable text from the boot code, filtered so it is text rather than opcodes, and **draws no conclusion from it**. `ade info` shows what the bootblock says; whether "NO VIRUS ON BOOTBLOCK!" is a reassurance or a lie is a question the tool declines to answer.

This is the same standard D-013 applied to LNFS: the work is not hard, it is *uncheckable*, and shipping an uncheckable verdict is worse than shipping none. Here it is stronger still, because the one cheap heuristic available has been measured and found backwards.

**AV-002's real defence is structural and already holds.** The vector is *executing* guest boot code. ADE has no interpreter, no emulator and no execution path of any kind — boot code is bytes to be read, filtered and displayed. That is not a mitigation to be implemented but a property of the design, and `boot_text.rs` pins it with boot code written to be hostile to a reader (`BRA` to self, `TRAP #0`, all-ones, all-zeros).

**Consequences.** F-011 is delivered in the half that can be verified — banner extraction — and deferred in the half that cannot. AV-002's detection line stays "not implemented" for signatures while its primary defence is recorded as structural. The extracted text turns out to be worth having on its own: it carries publisher banners, Copylock notices, cracker signatures and virus-protector menus, which is catalogue material (F-013, F-014) rather than merely a security by-product.

Roughly 20% of corpus disks yield reportable text after filtering, and 91% of the runs kept contain a space — the measure used to distinguish prose from 68k opcodes that happen to land in the printable range.

**Reversal conditions.** A published, checkable signature set — CRCs or byte patterns with provenance — reverses this immediately; the extraction and reporting path is already in place and a matcher would sit on top of it. Acquiring real infected disks would also reverse it, by making a scanner testable against true positives instead of against descriptions. **Neither is a reason to ship a matcher before then.**

### D-015 The JSON surface carries a version, and an inventory enforces it
**Decided:** 2026-08-28
**Recorded:** 2026-08-28
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** F-015, F-016, IMP-001, BUG-007, D-003

**Context.** F-015 has said since 2026-08-22 that "field names and fault codes are stable". That was a promise with nothing behind it. Two things had already gone wrong under it: four commands accepted `--format=json` and printed prose for six days (BUG-007), and nobody noticed, because a machine surface that is never machine-read reports nothing when it breaks.

**A rename does not fail loudly.** This is the specific reason a promise is insufficient here. If ADE renames `bytes_recovered`, a consumer reading that key does not get an error — it gets a *missing value*, which most code treats as zero or null and carries on with. The failure surfaces later, somewhere else, as a wrong number. Compare a renamed function: the build breaks immediately, in the right place.

**Options.**
- **A. Leave it as a documented promise.** Rejected: it is what we had, and it has already failed once.
- **B. Version the output only.** Rejected on its own. A version nobody remembers to bump is *worse* than no version, because it asserts a stability that is not being maintained — the consumer now has a reason to trust what it should be checking.
- **C. Version the output, and inventory every field in a test.** Chosen.

**Decision.** Option C, in three parts.

**Every document carries `schema`, first field.** Including each line of a JSON Lines stream, which is the case versioning is *for*: a consumer reading record by record, or resuming an interrupted run, has no summary to consult. Twelve bytes a line.

**The version is added at one point.** `Value::versioned()` is called by the CLI's single JSON emission path, not by each builder — because being a document is a property of reaching stdout, not of the value. `Inspection::to_json` is a whole document under `ade info` and a nested field under `ade check`; a version stamped inside it would appear twice in the second case, on an object that is not a document. One emission point is also what BUG-007 and BUG-008 both argued for: the same omission happened twice in commands added later, and the answer is a rule one function enforces rather than one every contributor must remember.

**Major and minor mean different things.** Major: a field is renamed, removed, retyped, or **redefined under an unchanged name** — that last is the dangerous one, since a rename breaks a consumer loudly and a redefinition breaks it silently. Minor: a field is added, which is safe for any consumer that ignores what it does not recognise.

**The inventory is the mechanism.** `src/api/tests/schema.rs` lists every field path ADE can emit, per document. Any change to the output fails it, and the fix is to edit the inventory *and* move `json::SCHEMA` in one commit, where a reviewer sees both. This is the same shape as `tools/check-layering.py` (D-003) and `clippy.toml` (C-001): the policy is not that the thing cannot change, but that it cannot change **quietly**.

**Two consequences fell out of writing the inventory**, and both were defects rather than paperwork:

- **An empty array shows none of its element fields**, so a clean fixture would have inventoried `findings` as a leaf and quietly stopped covering `findings[].code`. Each document now has an input that actually produces its optional parts — a damaged image for findings, a partitioned device for `rdb.*` and `partitions[].*`.
- **`batch`'s summary keyed its findings on fault codes**, so the document's *shape* changed with the data: a healthy corpus and a broken one produced different fields, and no inventory can pin that. Changed to an array of `{code, images}` pairs, matching `containers` — deliberately **before** declaring 1.0, so it did not need a major version later.

**Consequences.** Every reporting command emits versioned JSON. The surface starts at **1.0**. Adding a field is cheap and visible; renaming one is possible and visible; doing either by accident is not possible. The inventory costs a test edit per deliberate change, which is the point.

**Reversal conditions.** If the inventory becomes a rubber stamp — updated reflexively to make the build pass, without the version moving — it has stopped working, and the honest response is to say so rather than to keep the ritual. Should ADE ever gain a JSON *parser* (it has none, and F-015 does not need one), a round-trip test would be stronger than an inventory and should replace it.

### D-016 WHDLoad is the wrong kind of dataset; OpenRetro is the right kind and out of reach
**Decided:** 2026-08-29
**Recorded:** 2026-08-29
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** F-013, D-002, D-009, D-010, D-013, D-014

**Context.** F-013's acceptance asks ADE to "content-hash the image and match against TOSEC / WHDLoad / OpenRetro datasets". TOSEC has been delivered since 2026-08-27 and names 4,586 of the 4,652 corpus images. This entry is about the other two, and they turn out to be different problems.

**WHDLoad is not an identification dataset for disk images.** The database a tool can actually obtain is Amiberry's `whdload_db.xml`, and every entry is keyed on `filename` plus the **SHA-1 of a WHDLoad LHA archive** — a pre-installed hard-disk package. ADE reads disk images. There is no hash of an ADF anywhere in it, so matching a 4,652-image corpus against it would produce exactly zero matches, correctly. The clause names WHDLoad alongside TOSEC as though they were the same kind of thing; they are not, and no amount of implementation makes them so. Adding it would be building a lookup that cannot match by construction.

**OpenRetro is the right shape and cannot be fetched.** It records ADF and IPF variants by **SHA-1 of the disk file**, which is precisely the key ADE needs. But there is no public bulk export: `openretro.org/api/*` returns 404, `oagd.net` sits behind a Cloudflare challenge, and FS-UAE Launcher — the reference consumer, whose source is public — fetches `/api/sync` only after `/api/auth`, returning early when `database_auth` is unset. The data is behind an account.

**Options.**
- **A. Implement WHDLoad matching anyway.** Rejected: it cannot match ADE's material. A feature whose measured result is always zero is worse than its absence, because it implies the corpus was checked and found wanting.
- **B. Scrape OpenRetro page by page.** Rejected on two grounds. It is impolite to a volunteer-run site that has deliberately not published a bulk endpoint, and it would produce a dataset ADE could neither redistribute nor let anyone else reproduce — which fails D-010's standard for material as surely as an uncheckable parser fails D-002's.
- **C. Deliver what the datasets in hand support; record the other two as unavailable, with the conditions that would change it.** Chosen.

**Decision.** Option C. F-013 stands as **partially delivered**: identification is real, and it is TOSEC's.

**What was delivered instead, from the dataset already here.** Every one of the 88,921 TOSEC entries carries MD5 and SHA-1 as well as CRC32, and ADE indexed only CRC32. Checking what that cost produced a better result than the datasets would have — see below.

**Consequences.** F-013's acceptance clause is not met as written and will not be by effort alone. The `VOCABULARY.md` ManifeST contract remains outstanding and is unblocked; it is ADE's own work rather than someone else's data.

**Reversal conditions.** OpenRetro publishing a bulk export, or the author deciding an account-authenticated sync is acceptable for a local tool, reverses the second half immediately: SHA-1 is implemented, tested against `sha1sum`, and already wired into identification, so the work is a fetcher and a parser. WHDLoad reverses only if ADE grows the ability to read LHA archives — a different feature, not a dataset — or if a WHDLoad database keyed on disk-image hashes appears, which none does today.

### D-017 F-017 (FUSE mount) is cut; what would be worth building is a different feature
**Decided:** 2026-08-29
**Recorded:** 2026-08-29
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** F-017, F-003, F-007, F-018, D-004

**Context.** F-017 asks for "mount an ADF/HDF as a host filesystem (read at minimum) on Linux/macOS", priority **Could**, and its own note has flagged it as "lowest-priority; candidate for cutting" since 2026-08-21. It is the last undecided entry in the register. The justification recorded with it was that it "supersedes the separate, limited fuseadf" and is "partially covered by the Linux AFFS driver".

**That justification is wrong, and measuring it is what settled this.** The Linux kernel ships an AFFS driver — `affs.ko` is present on this machine's 6.17 kernel — and SPEC's own dostype table, sourced from the driver's documentation, records what it handles: `DOS\0`–`DOS\3` read **and write**, `DOS\4`/`DOS\5` read-only, `DOS\6`/`DOS\7` unsupported. Over a 400-image random sample of the corpus:

| dostype | share | Linux AFFS |
|---|---|---|
| `DOS\0` (OFS) | 87% | read/write |
| `DOS\1` (FFS) | 6% | read/write |
| no `DOS` bootblock | 4% | — |
| `DOS\3` (FFS-INTL) | 1% | read/write |
| `DOS\5` (FFS-DC) | <1% | read-only |

**94% of the sample is read/write in the kernel already**, and essentially all of the 77% that mount at all. "Partially covered" understates it to the point of being misleading: for plain ADFs, a FUSE filesystem would reimplement a driver that has been in the kernel since 1993, worse and slower, with no root required as its only advantage.

**What is *not* covered is real, and it is not F-017.** The kernel driver mounts a block device. It cannot read an ADZ, an extended ADF, an SCP capture, or a partition of an RDB device, and it cannot present a volume ADE reconstructed from raw tracks. Those are exactly the containers ADE exists for, and a FUSE layer over *them* would let `grep`, a file manager and any other tool reach content nothing else can.

**Options.**
- **A. Build F-017 as written.** Rejected: for the images it names — ADF and HDF — it duplicates the kernel.
- **B. Keep F-017 open indefinitely.** Rejected. An entry nobody will build is noise in a register whose value is that its statuses are true, and this one has carried "candidate for cutting" for a week without either happening.
- **C. Cut F-017, and record the part that would be worth building as a candidate.** Chosen.

**Decision.** Option C. F-017 is **cut**. A FUSE mount of the containers the kernel cannot read — ADZ, extended ADF, SCP, RDB partitions, and reconstructions — is added to the candidate list, with this measurement as its justification rather than the one F-017 carried.

**A second reason the cut is right, independent of coverage.** A filesystem interface cannot express what ADE is for. `read()` returns bytes; it has no way to say that this file came from a volume reassembled out of flux with 3% of its sectors missing, or that the disk's bitmap disagrees with its directory tree, or that two dataset entries name this content. ADE's value is the reporting, and mounting is the one interface that must discard it.

**Consequences.** The register has no undecided entries. F-017's number stays allocated and its section records the cut, per the append-only convention — a reader who finds it referenced elsewhere lands on the reason rather than on nothing.

**Reversal conditions.** A concrete need to reach ADE-only containers with ordinary tools — not a preference for mounting, a case where extraction genuinely will not serve. If that arrives, it is the candidate above and not this entry: the new feature's acceptance would name the containers the kernel cannot read, and would say what happens when a reconstruction's missing sectors are read.

### D-018 Writes into an existing disk are buffered, saved explicitly, and never over the file they came from
**Decided:** 2026-09-01
**Recorded:** 2026-09-01
**Status:** Accepted; part 2 amended by D-020 (2026-10-23)
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** D-004, D-002, D-010, AV-002, C-006, F-019, F-025, IMP-010, F-031, D-024

> **Implemented 2026-10-11 as F-031** — making a directory and adding a file, from the CLI, the C ABI and the window. All four parts hold as written; part 4's "mounts under ADFlib" is enforced by the test suite, since D-002 keeps the oracle out of anything that ships, and at run time each edit is kept only if `ade check` finds nothing new.

**Context.** D-004 defers write paths until their read paths are proven, and permits one once they are. For OFS and FFS that condition is met and measurable: 4,652 corpus images, 99.36% agreement with ADFlib on extracted bytes, 3,606 volumes mounting, and a health report that already validates every structure a write would have to maintain. `ade create` was permitted on those terms in F-019 and extended in F-025.

But `create` is the easy case. It **makes a new file**, so the damage D-004 exists to prevent — writing into a disk somebody already owns and cannot get back — is structurally impossible. Saving a modified disk, creating a directory on one, injecting a file, deleting one: all of these write into an image that exists. That is the question this entry settles, and nothing in the suite may be built before it is.

**The requirement is not "be careful". It is that a wrong edit must not be able to destroy the original.** ADE's users are preserving thirty-year-old media, frequently the only copy anybody has. A tool that is careful still loses a disk when the process is killed mid-write, when the wrong row was selected, or when the edit turns out to have been a mistake an hour later.

**What ADE already does is not one thing, which is worth knowing before citing it as precedent.** Measured 2026-09-01: `ade create` refuses to overwrite, `ade extract --all` skips existing files, `ade batch --convert` skips existing outputs — but `ade convert` and single-file `ade extract` overwrite silently. Three refuse and two do not. That inconsistency is logged as **IMP-010** and is not settled here; what is settled is that the write suite follows the majority, because it is the majority that matches what this tool is for. *(2026-10-25: IMP-010 is settled the same way. `convert` had already moved to refusing with the write suite; single-file `extract`, the last to overwrite, now refuses too. Every command that writes a file refuses one that exists, or skips and reports it when it generates the names.)*

**Options.**

- **A. No writes into an existing image in v1.** The status quo. Rejected, but not because it is wrong — it is the safest possible answer and it costs the feature entirely. The whole point of options C and D is to find a way to keep A's safety property while allowing the work.

- **B. Write through immediately, as a filesystem driver does.** Rejected. Every edit is irreversible the instant it is made, a crash between two related writes leaves a structurally invalid disk, and undo would have to be implemented as a second write that may itself fail. This is how a disk editor destroys a disk, and no amount of care in the implementation removes it.

- **C. Buffer edits in memory; write only on an explicit save; never over the file that was opened.** **Chosen.**

- **D. C, plus an automatic backup of the original before saving.** Rejected as unnecessary under C: if the original is never written, there is nothing to back up. A `.bak` beside every edited disk would be clutter that fills a preservation drive to guard against a risk C has already removed.

**Decision.** Option C, in four parts.

**1. Edits are buffered.** An edit changes an in-memory image and nothing else. Nothing on disk moves until a save. This is what turns "never-reversible in v1" from a reason to refuse the suite into a property the design already has: closing without saving costs nothing, and undo is a stack in memory rather than a promise about a file. Undo and redo are therefore ordinary and cheap, and are not a separate feature to be argued about.

**2. A save never writes over the file the image was opened from.** The first save requires a destination — Save As, always. Subsequent saves may write to *that* destination, because ADE created it in this session and it is not somebody's original. This is the rule that makes a wrong edit unable to destroy anything: the disk that was opened is still on disk, byte for byte, whatever happened in the editor.

**3. A save is atomic.** Write to a temporary file in the destination's own directory, flush it, then rename over the target. Writing in place risks a torn file if the process dies mid-write, which destroys the old contents *and* fails to produce the new — the worst of both, and the one failure mode a preservation tool must not have. The cost is free space equal to the image, which is 880 KB for a floppy and at most 50 MB for a hard disk ADE can make.

**4. Every write must leave a disk ADE and ADFlib both accept.** A write operation is not finished when the bytes are placed; it is finished when the result passes `ade check` and mounts under the D-002 oracle. This is stricter than it sounds and is the point: creating a directory means allocating a block, writing a header, inserting into the parent's hash table with the *right* hash function (C-006 — a `DOS\2`–`DOS\5` volume hashes differently), marking the bitmap, moving the parent's datestamp, and on a dircache volume updating the parent's cache, or the health report will report the new block as orphaned. A write that produces a disk ADE itself calls damaged is a bug, not a partial success.

**Consequences.**

- The GUI needs a dirty-state model: a modified image says so, Save is offered only when there is something to save, and closing or replacing a modified image asks first.
- The CLI's write commands take an explicit `--output`; there is no in-place form to get wrong.
- **Creating a directory is the right first operation**, and this entry is written expecting it: one block, one hash-table insertion, one bitmap bit, everything already read and validated, and verifiable end to end because ADFlib can be asked whether the directory is there.
- Bootblock writing stays declined. AV-002's defence is structural — no interpreter, no emulator, no execution path — and writing boot code is the one operation that would make ADE a vector rather than a reader. `ade create` leaves it zeroed exactly as AmigaDOS's own `format` does without `install`, and the write suite does not change that.
- LNFS stays out (D-013). Writing a format checkable only against itself is what D-002 gave up ADFlib's knowledge to avoid, and adding a *write* path to it would be the same mistake with worse consequences.

**Reversal conditions.**

- **Part 2 loosens** if in-place saving is asked for by somebody who understands what it costs, and then only behind an explicit flag, never as a default. The measurement to bring is a case where saving to a new file genuinely will not serve — not a preference for fewer files.
- **Part 3 changes** only if a target filesystem cannot rename atomically, in which case the honest answer is to refuse to save there rather than to fall back to writing in place.
- **Part 4 weakens for nothing.** If a write cannot be verified by both ADE and the oracle, it is not ready; that is D-002's whole bargain and D-013 is the standing example of it being kept.
- The whole entry is reconsidered if ADE ever writes to **real media** through a Greaseweazle (F-006), where there is no temporary file to rename and no original to preserve. That is a different problem and would need its own entry rather than an amendment to this one.

---

### D-019 An assembly *viewer* now; the reverse-engineering suite later, and separately
**Decided:** 2026-10-22
**Recorded:** 2026-10-22
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** F-032, F-020, F-028, AV-002, D-002, D-003, D-006, ROADMAP §Atari Disk Engine feature map

**Context.** The Atari feature map records a 68000 disassembler as "needs a decision first", because the Atari engine's is a whole reverse-engineering surface — call graph, cross-references, function list, library signatures, a project format, 4,672 lines — and the prior question was whether a disk engine is where that belongs. Asked for on 2026-10-22 in a narrower form: an **Assembly** tab beside Hex, Map and Text that shows a program's code as instructions.

**Options.**
- **A. Neither.** Extract executables cleanly (F-020 already finds 7,445 of them) and let a real disassembler be a disassembler. Rejected: reading a disk is exactly when a person wants to know what a file *is*, and a hex dump of 68000 code answers that for almost nobody.
- **B. The full suite now.** Rejected for now, not forever: most of it is a reverse-engineering workbench rather than a disk tool, and none of it is useful until the decoder underneath is right.
- **C. A read-only viewer now, the suite later as its own decision.** **Chosen.**

**Decision.** Option C, scoped as follows.

1. **In:** the complete 68000 instruction set; Motorola syntax; code located the way the disk says it is — the code hunks of an AmigaDOS executable (`HUNK_HEADER`) and the boot code of a bootblock — with data hunks shown as data, BSS as a size, and relocations and symbols used to label what they point at. Any other bytes can be disassembled only when asked, and are labelled as a guess, because 68000 instructions are 2–10 bytes long and decoding from the wrong place produces confident nonsense.
2. **Out, until a later decision:** 68020+ and FPU instructions (shown as `dc.w` and said to be beyond the 68000), library-call naming, cross-references, call graphs, function lists, editing, reassembly, export.
3. **It never executes anything.** Disassembly turns bytes into text; AV-002's defence — no interpreter, no emulator, no execution path — is unchanged and remains structural. The decoder is a parser of untrusted bytes and is held to D-006: bounded, panic-free, fuzzed.
4. **Checked against an implementation nobody here wrote.** GNU objdump (`binutils-m68k-linux-gnu`, `-m m68k:68000`) is the oracle, used by tests only and never linked or read — D-002's bargain applied to a second format. Every one of the 65,536 opcode words is decoded by both, and the tests require the same validity, the same length and the same instruction.
5. **It lives in `ade-object`**, which recognises what bytes are (F-020) and is where interpreting a file's contents belongs. That crate gains `ade-endian` as a dependency — a deliberate widening of the layering policy, made here rather than in a Cargo.toml, because the decoder reads big-endian words and C-001 routes every such read through that crate.

**Consequences.** F-032 is the viewer. The suite stays on the Atari map as a candidate. F-028 gains a route to its first real processor evidence — a 68020-only instruction in a code hunk is a fact — but using it there is a separate change.

**Reversal conditions.** Part 2 loosens one item at a time, each with its own entry; the first likely one is library-call naming, which needs the base register tracked and is wrong whenever it guesses. Part 4 changes only if the oracle becomes unavailable, in which case the decoder's tests must find another independent implementation rather than check it against itself.

---

### D-020 Save may write over the disk that was opened — asked first, and the original kept
**Decided:** 2026-10-23
**Recorded:** 2026-10-23
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** D-018, D-004, F-031, IMP-010, D-024

**Context.** D-018 part 2 made every save of an opened disk a Save As: the file a disk was opened from is never written. In use, from the window: *"if I open a disk and edit it then click save it should ask me to confirm I want to save the edits and if I click yes it should then save it like normal."* That is the expectation every other editor sets, and the file dialog on every Save of an opened disk was being met as a fault. D-018's reversal condition for part 2 is exactly this — in-place saving asked for by somebody who understands the cost — and requires it be explicit, never a default.

**What it costs.** Part 2 was the rule that made a wrong edit unable to destroy anything: the opened file survived any edit, however mistaken. Saving over it gives that up for the edits that are saved. Atomicity (part 3) is unaffected — a crash still leaves the old file or the new one, never a torn one — so the risk is a *wrong* edit saved, not a broken file.

**Options.**
- **A. Keep D-018 part 2.** Rejected by the person it serves: the safety was real, and so was the friction.
- **B. Confirm, then write over.** Rejected alone: one click past a dialog that is seen on every save, and the original is gone.
- **C. Confirm, and keep the original beside it the first time.** **Chosen.**
- **D. An opt-in setting, then confirm.** Rejected as indirection: the confirmation already makes each overwrite explicit, which is what part 2's reversal condition asks.

**Decision.** Option C, in four parts.

1. **Save on an opened disk asks first**, every time, naming the file. Cancel writes nothing anywhere.
2. **The first save over it in a session keeps the original**: the file *as it is on disk* is copied beside it as `<file>.orig`, written and flushed in full before the original is touched. An existing backup is never overwritten — a later session's becomes `.orig-2`, and so on — so every original a session started from survives.
3. **A file that changed on disk after it was opened is not saved over.** The session remembers the file's length and CRC32 from when it read it (and after each save); a mismatch means somebody else wrote it, and saving would destroy changes this session never saw. Refused, with Save As offered.
4. **A disk the window itself created** (New disk) is ADE's own file: Save writes back to it without asking and without a backup, because there is no original to protect.

Unchanged: Save As always makes a new file and still refuses the opened one and any existing file the session did not make; after a Save As, Save goes to that file; every save is atomic (D-018 part 3) and every edit is checked (part 4). The CLI keeps its required `--output` — this is the window's behaviour, and a command that can overwrite its input is still one typo from doing so.

**Consequences.** `Session::save_over_original(keep_backup)` in the engine and `ade_image_save_over` in the ABI, separate from `save` and `save_as` so no existing call changes meaning. Backups accumulate beside edited disks; that is the cost of part 2, and D-018 option D rejected backups only because part 2 made them unnecessary.

**Reversal conditions.** Part 2 (the backup) could become a setting if `.orig` files are measured to be clutter nobody uses — but not removed, because it is now the only thing standing between a confirmed wrong edit and a lost disk. Part 3 does not weaken. If ADE ever saves over files it did not open itself (a batch edit, a CLI in-place flag), that is a new entry, not this one.


---

### D-021 The reverse-engineering suite on F-032: follow the code, name only what it proves, export only what rebuilds
**Decided:** 2026-10-29
**Recorded:** 2026-10-29
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** D-019, F-032, F-036, D-002, D-006, AV-002

**Context.** D-019 part 2 left library-call naming, cross-references, call graphs, function lists and export out of the Assembly view "until a later decision", one item at a time, and named library naming as the likely first, because it "is wrong whenever it guesses". Asked for on 2026-10-29 in one go: export, cross-references and a function list, library-call naming, and a call graph. This entry loosens part 2 for those five and leaves the rest of it — editing, a project format, signature matching — where it was.

**Options.**
- **A. A linear sweep with names on top.** Decode every byte of every code hunk, as the listing already does, and take references, functions and calls from that. Rejected: data in a code hunk decodes as instructions, and their "calls" and "references" would be reported as the program's. The listing can afford that because it shows bytes; a cross-reference that says *this is called from there* cannot.
- **B. Signature or heuristic naming** — recognise library bases by how they are usually loaded, or functions by their prologues. Rejected: each is a guess about code the tool cannot see the whole of, and D-019 named exactly that failure.
- **C. Follow the code, and say only what following it proves.** **Chosen.**

**Decision.** Option C, in five parts.

1. **Code is what a walk reaches.** A walk starts where the loader starts the program — byte 0 of hunk 0 — or where AmigaDOS enters boot code, byte 12, and follows every branch, jump and call whose target the instruction itself states. Each call target starts a function, walked the same way. Functions, references and the call graph come from what the walks reached; bytes no walk reaches are not read as code. What a walk cannot follow — a call or jump through a register or memory, a jump table — is counted and said, not guessed at.
2. **A library is named only where the code proves which one it is.** The walk carries what is known of each register. A base is known from two facts and nothing else: the long at address 4 is exec's base, and exec's `OpenLibrary` or `OldOpenLibrary` returns in `d0` the base of the library whose name `a1` points at, read from the program's own bytes. Boot code starts with exec in `a6`, as AmigaDOS enters it. A library call keeps `d2`–`d7` and `a2`–`a6`, as the system's calling convention promises; a call into the program's own code may change any register, so everything known is forgotten; where paths meet, only what all of them agree on survives. **A base kept in memory is not followed** — that a variable holds `DOSBase` is a claim about every write to it, which no walk of the code shown can make — so most dos and intuition calls in compiled programs stay unnamed. That is the cost, accepted.
3. **The names come from two sources that must agree.** AROS's library descriptions give each function's slot twice — in each library's `.conf` function list, and in the `AROS_LH` header of the function's own source — and the table holds only functions on which both agree, checked also against offsets known from AmigaOS. AROS-only additions (version 50 and later) and the slots AROS marks private are left out, except five exec slots whose names are AmigaOS's own. The sources stay outside the repository; the table is generated from them.
4. **Export is source that assembles back to the program's own bytes**, or it is not export. The test is mechanical: every corpus program is exported, assembled with GNU `as` in its MRI mode, each section linked at address 0, and each hunk's bytes required back exactly. An instruction an assembler would encode differently — one it shortens, a byte immediate whose ignored high byte is not zero, a 68020 addressing mode — is written as words with the instruction beside it as a comment. A relocated long that no line can rebuild — into a hunk the file does not have, or at an odd offset — is written as its number, and the source says how many there are. The library names appear in the source as `_LVO` equates, which the round trip checks too.
5. **Unchanged from D-019:** nothing is executed (AV-002); every walk is bounded (`MAX_STEPS`) and fuzzed with the decoder (D-006); objdump stays the decoder's oracle, GNU `as` becomes the export's, and neither is linked.

**Consequences.** F-036. `ade_object::flow` and `ade_object::export`, and `ade_object::lvo` generated from AROS; the listing gains function headings and named library calls; `ade disasm` gains `--functions`, `--xrefs`, `--call-graph[=dot]` and `--export`; the ABI gains `ade_disasm_view` and `ade_disasm_bootblock_view`; the Assembly tab gains a view selector and Save. Boot code has no export: it is not a program file.

**Reversal conditions.** Part 2 loosens only with a proof as strong as its two facts — following a base through memory would need every write to the variable accounted for, which means the whole program's writes, including through pointers. Part 4 does not loosen: an export that does not rebuild is a listing with a different extension. If GNU `as` stops being available, the round trip needs another assembler nobody here wrote, not a comparison with ADE's own encoder — and since 2026-11-08 it has one (below).

**Amended 2026-11-08 (IMP-020, BUG-020).** Part 4's check was too weak for what part 4 promises. Linking every section at address 0 makes a relocated long hold exactly the number the file stores, so a relocation the export dropped still gave the right bytes — and 1,479 of 3,936 sampled programs had dropped some. The export is now also assembled by **vasm**, straight to an Amiga executable, and compared with the original as the loader sees it: hunk kinds, memory, the bytes loaded and **the relocation table** — under `-devpac` and under vasm's defaults. The source carries Devpac's `OPT O-` so that no assembler shortens what it writes. GNU `as` stays, as a second assembler nobody here wrote.

---

### D-022 A program's names and comments are the user's, kept per user and matched by the program's bytes
**Decided:** 2026-11-09
**Recorded:** 2026-11-09
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** D-019, D-021, F-036, D-018, AV-002, D-006

**Context.** D-019 part 2 left "a project format" out of the Assembly view with the rest of the reverse-engineering suite; D-021 took five items from that list and left this one, because it is a different kind of thing. Everything else ADE shows is read from a disk. A project is what a *person* wrote — "this is the copper list setup", "this variable is the lives counter" — and keeping it is the first state ADE holds that it did not read. Asked for on 2026-11-09 with the rest of the disassembler surface.

**Options.**
- **Where it lives.** **A. Per user, by program** — the user's data folder, one file per program, found by a hash of the program's bytes. **B. Beside the disk image** — one sidecar per image. **C. Only where the user saves it**, as a document. **A is chosen.** The same program sits on many disks — a `c/` command on every Workbench copy, a game on its original and its cracked releases — and a name given once should be there on all of them. Nothing is written next to an image, which D-018 keeps the user's to change, and a read-only folder of images still takes notes. B ties notes to one copy; C makes a person remember to save, and to open, the right file.
- **What it holds.** Names and comments; names alone; or names, comments and overrides marking bytes as code or data. **Names and comments are chosen.** Overrides would change what the walks report, which D-021 keeps to what the code proves; that needs its own rules and is left for its own decision.

**Decision.**
1. **A project is a text file per program**, `<sha1>.adeproj` in `$ADE_PROJECTS`, or `$XDG_DATA_HOME/ade/projects`, or `~/.local/share/ade/projects`. The SHA-1 is of the program file's bytes — or of the bootblock's 1,024 bytes for boot code — so a program changed by one byte is another program, with no names: a name is a claim about particular code. The file says which program it is for, one `name` or `comment` line per place, places written as the export labels them (`h1_0042`), and is meant to be read, copied and shared as it is.
2. **A name is a label an assembler takes**: a letter, `_` or `.` first, then letters, digits, `_` and `.`, at most 64 characters; not a register, not ADE's own form (`h1_0042`, `lab_0042`, `_LVO…`), not another place's name in the same program. A comment is one line of printable text, at most 200 characters. Anything else is refused, with the reason, when it is given, and a line of a file that breaks the rules is reported and skipped when it is read — never guessed at.
3. **A name shows everywhere the place is shown**: the listing's labels and the operands that reach the place, the functions, cross-references, call graph, strings and variables, and the exported source — which must still rebuild the program byte for byte (D-021 part 4), checked with names in it. A comment shows above its line in the listing and in the source.
4. **Writing is atomic and confined**: a temporary file in the projects folder, then a rename, the folder made on first use. Nothing outside that folder is written, and a disk image never is. Reading a project is bounded (D-006): a file over 1 MB, or a line over 4 KB, is refused.
5. **Nothing a person writes changes what ADE claims.** Names and comments are shown as the user's; the walks, the library naming and the export's checks are unchanged by them.

**Consequences.** `ade_object::annotations` (the format and its rules) and `ade_core::project` (where, and the hash); the listing, the views and the export take a program's annotations; `ade annotate`; `ade_project_set_name` and `ade_project_set_comment` in the ABI; in the Assembly tab, a line's context menu names its place or comments on it.

**Reversal conditions.** Part 1's storage gains a sidecar or explicit files if notes need to travel with an image rather than with a program — added beside the per-user store, not instead of it. Overrides of code and data need their own entry. Part 5 does not loosen: the day a user's name could make ADE claim something about the code, the user's words and the disk's would be indistinguishable.

---

### D-023 AmigaDOS itself, run in FS-UAE, is a test oracle — local only, driven from outside, never shipped
**Decided:** 2026-11-11
**Recorded:** 2026-11-11
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** D-002, D-019, D-010, AV-002, BUG-023, F-012, F-031, C-005, D-013

**Context.** Every check on what ADE writes has been a reader: ADE's own health report and ADFlib (D-002). Neither boots a disk, and neither is AmigaDOS. On 2026-11-11 this machine turned out to hold FS-UAE 3.1.66 and the Amiga Forever Kickstart ROMs and Workbench disks — ROADMAP had recorded "an emulator with Kickstart ROMs, which this machine lacks" — and a scripted run was proven the same day: ADE extracts Workbench 3.1 into a folder, FS-UAE mounts it as the boot drive, a startup-sequence runs the commands and writes their output there, and a marker file tells the host to stop the emulator. A run takes 6–20 seconds. The first run found BUG-023, a disk ADE formats crashing the machine that boots it, which no reader could have seen.

**Options.**
- **A. Keep to readers.** Rejected: BUG-023 is the proof of what that misses, and several blocked items need material only a real filesystem makes — deleted headers (F-012), hard links, larger blocks (C-005).
- **B. Ship an emulator launch in ADE.** Not this decision. "Run the disk in an emulator" stays a separate GUI candidate on the Atari map.
- **C. AmigaDOS as a test-only oracle.** **Chosen.**

**Decision.**
1. **Test-only and driven from outside**, as ADFlib and objdump are: FS-UAE is a separate process given a configuration file; nothing links it, and no shipping path starts it. AV-002 is untouched — ADE still executes nothing; a program the user installed runs Amiga code the user owns.
2. **Configured, never found.** `ADE_KICKSTART` names a Kickstart 3.1 ROM and `ADE_WORKBENCH` a Workbench 3.1 disk; `fs-uae` must be on the path (or named by `ADE_FSUAE`). Without all three the tests print why and skip, so CI and a fresh clone are unaffected. ROMs and Workbench are copyrighted and never enter the repository, any more than a corpus disk does (D-010).
3. **Every run is bounded.** A requester on the Amiga side waits for a click forever, so a run that has not left its marker by its limit is killed and fails, with the emulator's log. A script that writes to a floppy locks it (`Lock DF0: ON`) and waits before the marker, because AmigaDOS writes lazily and a run stopped at the marker leaves a half-written disk — measured, not assumed.
4. **Scripts are Latin-1**, as AmigaDOS reads them; an accented name written in UTF-8 is a different name.
5. **One run at a time.** Runs are serialised within a test binary; each has its own folder and FS-UAE base directory, so nothing in the user's own FS-UAE setup is read or written.
6. **Headless where possible.** Under `xvfb-run`, when it is installed, the emulator draws to no screen; otherwise its window appears for the length of the run.

**Consequences.** `src/api/tests/amigados.rs` holds the harness and its first tests: an ADE-formatted disk does not boot (BUG-023's regression), AmigaDOS reads what ADE writes — accented names found by its own hash lookup — and ADE reads what AmigaDOS writes. F-012, hard links and C-005 gain a route to real material; each uses it as its own piece of work.

**Reversal conditions.** If FS-UAE's scripted boot proves flaky over time — runs that pass and fail with no change — the tests become opt-in behind a further variable rather than running whenever configured. Part 1 does not loosen: the day ADE starts an emulator in a shipping path is the GUI candidate's own decision.

---

### D-024 Writing to hard disk images: partitions and partition tables, on D-018's terms, checked by AmigaDOS where ADFlib is stricter than the format
**Decided:** 2026-11-13
**Recorded:** 2026-11-13
**Status:** Accepted
**Authors:** Darian-Frey (decision); Claude (analysis)
**Related:** D-004, D-018, D-020, D-023, D-002, F-018, F-038, C-002, C-005, AV-002, BUG-023, BUG-027, BUG-028, IMP-005, ROADMAP §Phase 6

> **Part 7 implemented 2026-11-13.** History is kept as the blocks each edit changed — never whole images — and Undo, Redo, groups and a failed folder deletion's rollback all work from it; the health check borrows the image rather than copying it. A `put` on a 500 MB hardfile peaks at 1.03 GB where it took 2.05 GB, and 21 files' history on a 100 MB disk is under 512 KB. One copy remained, the one an edit was made on so it could be refused: **IMP-023**, applied the same day — edits are planned on a draft, made in place with the replaced blocks kept, and put back on a refusal, so every edit command peaks at one copy of the image (516 MB on a 500 MB hardfile).

**Context.** Phase 6 (F-038) asks for hard disk images handled as fully as floppies: every edit inside each partition of an RDB disk, and partition tables created and edited. F-018's editing half has been deferred by D-004 since 2026-08-28. D-018 set the terms for writing into a disk, with floppies in mind; read against hard disks, three of its assumptions no longer hold, and this entry settles them rather than stretching D-018 to fit.

**Where things stand, 2026-11-13.**
- *Reading* partitioned disks is proven. Every command and view reads partitions correctly since BUG-027 and BUG-028 were fixed. RDB parsing agrees with ADFlib on generated devices and holds on Amiga Forever's three Workbench hard disks. AmigaDOS in FS-UAE is a second oracle (D-023), and mounts and writes hardfiles ADE makes up to its limit of a little under 1 GB unpartitioned.
- *Writing* stops at unpartitioned hardfiles. Every edit is refused on an RDB disk, and nothing writes a partition table.
- *Scale*: an edit copies the whole image and Undo keeps whole-image snapshots — about four copies at peak; a `put` on a 500 MB hardfile takes 2 GB of memory, and `ade create` now makes volumes up to 4 GB.

**How D-004 and D-018 read against this.**
- **D-004** permits a write path once its read path is proven. For partitions it is, by the evidence above; this entry cites it rather than amending it.
- **D-018 parts 1–3** — edits buffered in memory, never saved silently over the opened file (as amended by D-020), saves atomic — hold for hard disks unchanged as rules. Part 1's *implementation* does not scale, and part 3's stated cost ("at most 50 MB for a hard disk ADE can make") is out of date: an atomic save needs free space equal to the image, now up to 4 GB, and D-020's `.orig` as much again.
- **D-018 part 4** requires every write to pass `ade check` *and* mount under ADFlib. For partitioned disks that cannot be met as written. ADFlib refuses any RDB device without a filesystem stored in the RDB (`FSHD` and `LSEG` blocks), which FAQ §6.5 says is not needed (SPEC §The oracle is stricter than the format here). ADE cannot write one: the only FFS binary to put there is Commodore's, which ADE may not ship. Read literally, part 4 would mean ADE can never create a valid partitioned disk. And `ade check` examines one partition at a time, so "passes check" needs defining for a disk of several.

**Options, per question.**

*1. The oracle for partitioned disks.*
- A. ADFlib only, as part 4 says. Rejected: it refuses every RDB disk ADE could make, for a reason the format does not give.
- B. ADE's own health report only. Rejected: a writer checked only by its own reader is what D-002 exists to avoid, and BUG-023 and BUG-025 were both invisible to every reader ADE had.
- C. **AmigaDOS in FS-UAE (D-023) where ADFlib is stricter than the format; ADFlib wherever it accepts the disk.** Chosen. Kickstart 2.0 and later carry FFS in ROM, so a partitioned disk with no filesystem in its RDB mounts on a real Amiga, and AmigaDOS reading and writing every partition ADE wrote is a stronger check than ADFlib's, not a weaker one.

*2. What "passes the health check" means for a disk of several partitions.*
- **Every partition's health report, plus checks on the table itself** — chosen; there is no lesser answer that would catch a write into the wrong partition.

*3. Resizing a partition that holds files.*
- A. Move the data to fit. Rejected: the highest-risk write a disk tool can make — every block of a volume relocated, its pointers rewritten — for a feature HDToolBox itself never offered.
- B. Refuse every resize. Rejected as needlessly strict: growing into unused space touches no data.
- C. **Grow a partition only into unused cylinders after it, and only when its filesystem is extended to match; delete whole partitions; refuse to shrink or move one that holds data, with the reason.** Chosen. It is HDToolBox's own model, where a partition is resized by reformatting.

*4. Undo at the sizes a hard disk has.*
- A. Keep whole-image snapshots and cap image size. Rejected: it caps the feature at whatever memory a machine has.
- B. **Per-block change sets — each edit records the blocks it replaced — under the same rules: buffered, undoable, nothing written until a save.** Chosen. A rule-preserving change of implementation, not of policy.

*5. D-020's `.orig` backup on a multi-gigabyte disk.*
- A. Drop the backup above some size. Rejected: D-020 makes the backup the only thing between a confirmed wrong edit and a lost disk, and says it may become a setting but not be removed.
- B. **Keep it; say its size in the confirmation; refuse to start a save whose temporary file and backup will not both fit, rather than fail part-way.** Chosen.

**Decision.** Writing to hard disk images — inside partitions, and to partition tables — is permitted on D-018's terms as amended by D-020, with these additions:

1. **D-004 is satisfied for partitions** by the read-side evidence above. Writes to a partition table ship only after its reader is proven on the same footing: ADFlib on generated devices, the real RDB disks on hand, and AmigaDOS reading what ADE wrote.
2. **The oracle (D-018 part 4) for a partitioned disk** is AmigaDOS in FS-UAE where ADFlib refuses a structure the format allows, and ADFlib wherever it accepts the disk. For RDB writes this is checked by the local test suite, as D-023 runs it; CI keeps ADFlib on generated devices.
3. **An edit to a partitioned disk is kept only if** every partition's health report finds nothing new *and* the table is sound: the `RDSK` and every `PART` checksum valid; every partition inside the disk and inside the cylinders the RDB reserves for partitions; no two partitions overlapping; drive names unique; the `PART` chain ending with -1 and not looping. A table edit that would leave any of these false is refused before it is made.
4. **ADE does not write a filesystem into the RDB** (`FSHD`/`LSEG`). A partition names its dostype and relies on the FFS in Kickstart's ROM (2.0 and later), and every surface that creates a partitioned disk says so. Writing a filesystem ADE may not ship is out; one supplied by the user is a separate question for its own entry.
5. **Partition tables:** create; add a partition in unused cylinders; delete a partition (its volume goes with it, buffered and undoable like any edit); rename; change boot priority, the bootable flag and the dostype; reformat one partition without touching the others; grow one into unused cylinders after it, extending its filesystem. Shrinking or moving a partition that holds data is refused, with the reason.
6. **Partition boot blocks** are written as a floppy's are since BUG-023: the dostype and zeros, never a checksum over code (AV-002). Bootblock editing stays declined.
7. **Scale:** edits record per-block change sets rather than whole-image snapshots before anything in this entry is offered for images over 1 GB, and Undo and the dirty state work from them. D-018 part 1's rule is unchanged.
8. **Saving:** D-018 part 3 and D-020 unchanged, except that a save first checks there is room for the temporary file and, when saving over the opened file, the `.orig` too, and refuses rather than starts if there is not; and the confirmation names the backup's size.

**Consequences.**
- The edit commands, the ABI's `ade_edit_*` calls and the window's edit actions take a partition, as the read side does since BUG-028; the window's edits are enabled on partition rows.
- New table-editing surfaces: `ade partition …` subcommands (shape to be designed), ABI calls, and a partition-table view in the window.
- `ade create` gains a partitioned form.
- The emulator harness gains RDB images: AmigaDOS mounts each partition ADE wrote, reads and writes it, and makes disks with `Format` for ADE to read. HDToolBox cannot be scripted, so disks it makes are checked by hand when wanted, not in the suite.
- D-018's part 3 text about "at most 50 MB" is superseded by this entry's part 8.

**Reversal conditions.**
- **Part 2** returns to ADFlib alone if a version of ADFlib, or another independent reader, accepts an RDB with no filesystem in it; or if the FS-UAE harness proves flaky under D-023's own reversal condition, in which case partitioned writes stop being offered until another oracle exists — they do not fall back to ADE checking itself.
- **Part 4** reopens if a user wants to supply a filesystem binary for ADE to place in the RDB; that is its own entry.
- **Part 5's refusal to shrink or move** loosens only with a decision entry and a test that relocates every block of a full partition and has AmigaDOS read every file after.
- **Part 7** has no reversal: a hard disk feature that needs several copies of the disk in memory is not ready.
