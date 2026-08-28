# Attack Vectors

Project-specific failure modes ADE must be resilient against. Disk images are untrusted input (D-006); this register is the forward-looking checklist that pairs with the backward-looking [BUGS.md](BUGS.md).

Severity: Critical (must hold) | Major (regression on release blocks) | Minor (track only).

> **Detection status, audited 2026-08-28.** All five vectors now have implemented detection, each with a named test path. The header this replaces still said "no code exists, so every vector below is `Detection: not implemented`" — true when written on 2026-08-21, and left standing for a week after it stopped being true. AV-002 and AV-003 had been updated in place; AV-001, AV-004 and AV-005 had not, so this register was claiming ADE undefended against the three vectors its type system and fuzz harness were built around.
>
> A vector is never "closed" here. Detection existing is the start of the entry's useful life, not the end of it: what each one now records is *which* mechanism holds it and *what would prove the mechanism broken*.

## Parsing robustness

### AV-001 Directory hash-chain loops
**Severity:** Critical
**Description.** A corrupt or malicious directory hash chain that points back into itself causes unbounded traversal → hang or OOM. This is the exact class of bug ADFlib had to add loop detection for.

**Wider than first recorded.** The 2026-08-22 SPEC research found that cycles are reachable on **structurally valid, non-corrupt disks**: AmigaDOS permits hard links to directories, "which opens the way to endless recursion" (Clévy, ADF FAQ §4.6). Cycle detection is therefore a correctness requirement for ordinary images, not only a defence against hostile input — and it must be a visited-set over block numbers rather than a depth limit, since a legitimate deep tree and a two-block cycle are indistinguishable by depth alone.
**Detection.** **Implemented** (audited 2026-08-28; landed 2026-08-22 with `ade ls`). Every chain that can be followed carries a visited set of block numbers — directory hash chains, file extension chains, dircache chains, the RDB `PART` list, and the whole-volume walk, which carries **one** set across the entire traversal so a hard link pointing back up the tree terminates rather than recursing. Tests exercise both halves the entry asked for, malformed and legitimate: `a_two_block_hash_cycle_terminates`, `a_directory_cycle_terminates_a_tree_walk`, `a_self_looping_extension_chain_terminates`, `a_two_block_extension_cycle_terminates`, `a_looping_cache_chain_terminates_and_is_reported`.

**Behind the visited set sits a structural cap** that does not depend on it: a volume cannot hold more entries than it has blocks. The redundancy is deliberate, and IMP-003 records why — removing the visited set alone made ADE allocate 28.8 GB and take the host down, so this Critical vector rested on a single `HashSet::insert` returning true. The cap turns that into a reported fault.

**Depth is carried explicitly as well as count.** Bounding the entry count is not enough: each path string is built from its parent's, so a cycle makes the *strings* grow without bound — `a/b/a/b/…` — while the count stays inside its cap. Found by mutation-testing `walk`, where the first version of the cap still reached 4 GB.
**Related decisions.** D-006. **Related features.** F-001, F-012. **Related constraints.** SPEC §Links.
**History.** Identified 2026-08-21 during initial scaffolding, from the ADFlib precedent. Scope widened 2026-08-22 to include legitimate directory hard links.

### AV-004 Out-of-range block pointers
**Severity:** Critical
**Description.** File-header/extension/rootblock pointers that fall outside device geometry cause wild reads if dereferenced unchecked.
**Detection.** **Implemented, at the type level** (audited 2026-08-28; landed 2026-08-21 with `ade-block`). `BlockSource::read_block` does not take a number — it takes a `ValidBlock`, whose only constructor is `Geometry::validate` and whose field is private to `ade-block`. An unchecked index cannot reach a backing store because no code can build the token that would carry it there, so this vector is enforced by the compiler rather than by a check somebody must remember to write.

**It holds across partition boundaries too.** `Window` re-validates every block number after translating it to the underlying device, so a partition cannot be used to address outside itself — AV-004 within a volume and AV-004 across a device are different claims, and both are checked.
**Related decisions.** D-006. **Related features.** F-001.
**History.** Identified 2026-08-21 during initial scaffolding.

## Resource exhaustion

### AV-005 Decompression edge cases
**Severity:** Major
**Description.** DMS encryption/password paths and malformed gzip (ADZ/HDZ) can trigger resource exhaustion or crashes in the decompressor.
**Detection.** **Implemented for gzip** (audited 2026-08-28; landed 2026-08-25). The inflater caps output up front and checks the cap **before each write, never after** — a limit tested afterwards has already allocated. The cap is the caller's policy rather than a constant, so a legitimate 100 MB HDZ hitting a floppy-sized cap is reported as such rather than treated as an attack. gzip's declared `ISIZE` is verified after the fact and never trusted to size anything beforehand, which is BUG-003's lesson with the attacker holding the pen.

**Not implemented for DMS**, which is not parsed at all yet (D-009, F-003). The `errdms` fixtures this entry asks for cannot exist before the parser does.
**Related decisions.** D-002, D-006, D-009. **Related constraints.** C-004 (SPEC.md).

> **Observed in ADE itself, 2026-08-24 — BUG-003.** `read_file` reserved `Vec::with_capacity(byte_size)` from a `u32` read off the disk, so a crafted file header claiming 4 GB caused a 4 GB allocation on an 880 KB floppy, before any data block was read. Fixed by clamping the reservation to the volume's own size.
>
> Two lessons. This vector reaches the **plain ADF path**, not only decompression where it was first expected. And it survived 900,000 fuzz cases undetected, because the harness bounds *output* while a `with_capacity` that merely succeeds produces none — allocation is invisible to an output-bounds check unless something asserts on it directly.
>
> **Observed in the reference implementation, 2026-08-22.** This vector is not hypothetical, and it is not confined to compressed formats. Running ADFlib's `unadf` over the 4288-image corpus produced **15 crashes**, and on `Bomb Busters_Disk1.adf` — an ordinary uncompressed 901,120-byte game disk — it allocated **29 GB** before the kernel OOM killer terminated it *and the surrounding session*. ADE reads the same disk in 2.8 MB and reports its volume cleanly.
>
> Two consequences. First, unbounded allocation belongs to the plain-ADF parse path too, not just to decompression — the fuzz harness must cover both. Second, every invocation of the D-002 oracle now runs under `ulimit -v` and `timeout`, because a test that can take down the developer's machine is not a test.
**History.** Identified 2026-08-21 during initial scaffolding.

## Data integrity on write

### AV-003 Corrupt bitmap-valid flag
**Severity:** Major
**Description.** A cleared or corrupt bitmap-valid flag, trusted blindly, leads to mis-allocation on any write path.
**Detection.** **Implemented 2026-08-24** (F-010). ADE does not ask the flag whether the bitmap is trustworthy — it reads the bitmap and checks it against the blocks the directory tree actually reaches, in both directions:

- *referenced but marked free* — live file data the filesystem believes is available. Rated **error**: the next write destroys it. Found on real disks, e.g. block 25 of `Rolling Thunder.adf`, a genuine `T_DATA` block owned by file header 910.
- *marked used but unreachable* — lost space, or deleted files whose blocks were never freed. Rated **warning**.

The flag itself is reported as an observation. Over a 776-disk sample: 76 had a stale flag, 65 had orphaned blocks (15,863 in total), and 2 had live data marked free.

**Rebuild implemented 2026-08-24.** `Bitmap::rebuild` computes the map the volume should have and the report names the offending blocks — `Rolling Thunder.adf` reads "1 blocks are in use by files but marked free (25)", block 25 being a `T_DATA` block owned by file header 910. Naming them is the difference between knowing something is wrong and knowing what to repair.

The rebuild is **computed, never applied**. D-004 defers write paths to Phase 4 and is marked never-reversible within v1; AV-003 asks for a rebuild to be *offered*, which this is. The separation is not bureaucratic: a bitmap rebuilt from a misread tree and written to the only copy of a disk destroys exactly what the tool exists to preserve. Correctness is still provable without writing — build the map, read it back, and check it describes the set it was built from.
**Related decisions.** D-004, D-006. **Related features.** F-010. **Related constraints.** SPEC §Bitmap.
**History.** Identified 2026-08-21 during initial scaffolding. Corroborated 2026-08-22 by the Linux AFFS driver documentation, which warns the flag "may not be accurate when the system crashes while an affs partition is mounted" — so this is a routine real-world condition, not only an attack.

> **Note on polarity.** A **set** bitmap bit means the block is **free**; cleared means allocated (ADF FAQ §4.3). This is the opposite of the usual convention and an easy inversion to make, which would mis-report every block on every disk. Worth a dedicated test.

## Guest content

### AV-002 Malicious bootblock code
**Severity:** Major
**Description.** Bootblocks may contain historical viruses (e.g. the "Saddam"/"Lazarus" strains). Executing guest boot code would be a serious breach.
**Detection.** **Primary defence holds structurally** (2026-08-25): ADE has no interpreter, no emulator and no execution path of any kind, so boot code is only ever bytes to be read, filtered and displayed. `boot_text.rs` pins this with boot code written to be hostile to a reader — `BRA` to self, `TRAP #0`, all-ones, all-zeros. Signature scanning is **deferred by D-014**: no checkable signature database is published, and matching strain names is measurably *inverted* — all 107 corpus disks naming a virus carry anti-virus bootblocks, not infections. Bootblock text is reported instead, with no verdict drawn.
**Related decisions.** D-006. **Related features.** F-011.
**History.** Identified 2026-08-21 during initial scaffolding.
