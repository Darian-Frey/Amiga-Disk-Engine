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
**Related:** F-001, F-010, ROADMAP.md

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
