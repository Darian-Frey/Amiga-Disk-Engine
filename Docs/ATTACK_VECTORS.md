# Attack Vectors

Project-specific failure modes ADE must be resilient against. Disk images are untrusted input (D-006); this register is the forward-looking checklist that pairs with the backward-looking [BUGS.md](BUGS.md).

Severity: Critical (must hold) | Major (regression on release blocks) | Minor (track only).

> **Detection status.** ADE is planning-stage: no code exists, so every vector below is `Detection: not implemented`. This is honest signal, not an oversight — each entry states the check that *would* detect it. Entries move from "not implemented" to a concrete test path via the **History** field as the code lands (per the standard's category-three → category-one transition).

## Parsing robustness

### AV-001 Directory hash-chain loops
**Severity:** Critical
**Description.** A corrupt or malicious directory hash chain that points back into itself causes unbounded traversal → hang or OOM. This is the exact class of bug ADFlib had to add loop detection for.

**Wider than first recorded.** The 2026-08-22 SPEC research found that cycles are reachable on **structurally valid, non-corrupt disks**: AmigaDOS permits hard links to directories, "which opens the way to endless recursion" (Clévy, ADF FAQ §4.6). Cycle detection is therefore a correctness requirement for ordinary images, not only a defence against hostile input — and it must be a visited-set over block numbers rather than a depth limit, since a legitimate deep tree and a two-block cycle are indistinguishable by depth alone.
**Detection.** Not implemented (would require a cycle-detecting traversal carrying a visited-set of block numbers, exercised by both a malformed-chain fuzz fixture *and* a legitimate directory-hardlink fixture).
**Related decisions.** D-006. **Related features.** F-001, F-012. **Related constraints.** SPEC §Links.
**History.** Identified 2026-08-21 during initial scaffolding, from the ADFlib precedent. Scope widened 2026-08-22 to include legitimate directory hard links.

### AV-004 Out-of-range block pointers
**Severity:** Critical
**Description.** File-header/extension/rootblock pointers that fall outside device geometry cause wild reads if dereferenced unchecked.
**Detection.** Not implemented (would require bounds-checking every pointer against computed geometry before dereference, exercised by a crafted-pointer fixture).
**Related decisions.** D-006. **Related features.** F-001.
**History.** Identified 2026-08-21 during initial scaffolding.

## Resource exhaustion

### AV-005 Decompression edge cases
**Severity:** Major
**Description.** DMS encryption/password paths and malformed gzip (ADZ/HDZ) can trigger resource exhaustion or crashes in the decompressor.
**Detection.** Not implemented (would require capped output sizes and fuzzing the DMS/gzip front-ends; `errdms` fixtures as known-bad inputs).
**Related decisions.** D-006, D-009. **Related constraints.** C-004 (SPEC.md).
**History.** Identified 2026-08-21 during initial scaffolding.

## Data integrity on write

### AV-003 Corrupt bitmap-valid flag
**Severity:** Major
**Description.** A cleared or corrupt bitmap-valid flag, trusted blindly, leads to mis-allocation on any write path.
**Detection.** Not implemented (would require treating the flag as advisory and offering a defensive bitmap rebuild, verified against a tampered-bitmap fixture).
**Related decisions.** D-004, D-006. **Related features.** F-010. **Related constraints.** SPEC §Bitmap.
**History.** Identified 2026-08-21 during initial scaffolding. Corroborated 2026-08-22 by the Linux AFFS driver documentation, which warns the flag "may not be accurate when the system crashes while an affs partition is mounted" — so this is a routine real-world condition, not only an attack.

> **Note on polarity.** A **set** bitmap bit means the block is **free**; cleared means allocated (ADF FAQ §4.3). This is the opposite of the usual convention and an easy inversion to make, which would mis-report every block on every disk. Worth a dedicated test.

## Guest content

### AV-002 Malicious bootblock code
**Severity:** Major
**Description.** Bootblocks may contain historical viruses (e.g. the "Saddam"/"Lazarus" strains). Executing guest boot code would be a serious breach.
**Detection.** Not implemented (would require: never executing bootblock code — enforced structurally by the flux/block layers — plus a signature scan flagging known strains, per F-011).
**Related decisions.** D-006. **Related features.** F-011.
**History.** Identified 2026-08-21 during initial scaffolding.
