//! The health report (F-010).
//!
//! ADE's headline forensic surface: everything that can be said about an
//! image's condition, gathered in one pass and ranked by how much it matters.
//!
//! # It reports, it does not judge
//!
//! Real disks are full of oddities. 567 of 4652 corpus images carry a
//! datestamp of day zero; 260 have a stale bitmap flag; a quarter hold no
//! AmigaDOS volume at all. If everything unusual were an error, the report
//! would be noise and the genuinely broken disks would hide in it. So findings
//! carry a [`Severity`], and the cosmetic is separated from the dangerous.
//!
//! The distinction that matters most is **would this lose data**. A day-zero
//! datestamp would not. A block referenced by a file but marked free in the
//! bitmap would: the next write puts something else there.
//!
//! # Not covered here
//!
//! F-010's acceptance also names bad sectors and weak bits. Both are
//! flux-level properties — a sector is "bad" because it would not read off the
//! physical medium, and weak bits only exist in a flux capture. Neither is
//! knowable from a decoded image, so both belong to Phase 4 and are absent
//! rather than faked.

use std::collections::{HashMap, HashSet};

/// How many cache disagreements to report before saying so in one line.
const MAX_DIRCACHE_FINDINGS: usize = 20;

use ade_filesystem::{
    bitmap::Bitmap,
    dircache::{self, T_DIRCACHE},
    dostype::Mode,
    entry::{EntryKind, T_DATA, T_HEADER, T_LIST},
    volume::Volume,
};

use crate::{
    inspect::{Inspection, inspect_bytes},
    json::Value,
};

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Worth recording; not a problem.
    Info,
    /// Odd, or a sign of past trouble. Common on real disks.
    Warning,
    /// Would lose or corrupt data.
    Error,
}

impl Severity {
    /// Lower-case identifier, stable for the JSON surface (F-015).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl core::fmt::Display for Severity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One thing found wrong, or worth noting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Stable machine-readable identifier. Part of the F-015 contract.
    pub code: &'static str,
    /// How much it matters.
    pub severity: Severity,
    /// Human-readable description. May be reworded; the code may not.
    pub message: String,
    /// The block it concerns, where there is one.
    pub block: Option<u32>,
}

impl Finding {
    fn new(code: &'static str, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            block: None,
        }
    }
    fn at(mut self, block: u32) -> Self {
        self.block = Some(block);
        self
    }
}

/// What the bitmap says, and whether it is telling the truth.
#[derive(Debug, Clone)]
pub struct BitmapHealth {
    /// Blocks the bitmap marks in use.
    pub marked_used: usize,
    /// The specific blocks in use by files but marked free. Naming them is the
    /// difference between "something is wrong" and "here is what to repair".
    pub at_risk_blocks: Vec<u32>,
    /// The specific blocks marked in use but unreachable.
    pub orphaned_blocks: Vec<u32>,
    /// Blocks actually reachable from the directory tree.
    pub actually_used: usize,
    /// Marked in use, but nothing references them. Lost space, or deleted
    /// files whose blocks were never freed.
    pub orphaned: usize,
    /// Referenced by a file, but the bitmap says free. **Dangerous**: the next
    /// write would overwrite live data.
    pub referenced_but_free: usize,
    /// Blocks the bitmap covers at all.
    pub covered: u32,
    /// Whether `bm_flag` claimed the map was valid.
    pub flagged_valid: bool,
    /// On a volume with no entries, the blocks the bitmap marks free that
    /// hold something other than zeros or a formatter's fill (IMP-019),
    /// whether or not deleting left them. Always zero on a volume with
    /// entries.
    pub free_holding_data: usize,
}

/// The condition of one image.
#[derive(Debug)]
pub struct Health {
    /// What the image is, and what its headers say.
    pub inspection: Inspection,
    /// Bitmap cross-check, where a volume was mountable.
    pub bitmap: Option<BitmapHealth>,
    /// Directory-cache cross-check, on a `DOS\4`/`DOS\5` volume only.
    pub dircache: Option<DirCacheHealth>,
    /// Directories reached.
    pub directories: usize,
    /// Files reached.
    pub files: usize,
    /// Bytes recovered across every file.
    pub bytes_recovered: u64,
    /// Everything found, worst first.
    pub findings: Vec<Finding>,
    /// What was actually examined: the volume's label, and the partition it
    /// came from where the image is a device.
    ///
    /// `None` means nothing mounted, which is why a report can be empty without
    /// the image being sound.
    pub examined: Option<Examined>,
    /// What the disk is, in the terms a person browsing it needs: above all,
    /// why a disk with no files has none.
    pub kind: DiskKind,
}

/// What the directory-cache cross-check found.
///
/// Present only on a cached volume. `disagreements` of zero is the interesting
/// case to state plainly: two independent descriptions of every directory were
/// compared and matched.
#[derive(Debug, Clone, Default)]
pub struct DirCacheHealth {
    /// Directories that carry a cache.
    pub directories: usize,
    /// Cache blocks across all of them.
    pub blocks: usize,
    /// Records across all of them.
    pub records: usize,
    /// How many ways the caches and the directories differ.
    pub disagreements: usize,
}

/// What a disk is, read from the evidence the report already has.
///
/// A quarter of real images hold no AmigaDOS files, and "no rootblock at
/// block 880" is true of all of them and explains none. Measured 2026-10-28
/// over the 4,653 corpus images: of the 1,166 with nothing to list, 702 boot
/// into a loader of their own, 339 are data disks that cannot boot — 322 of
/// them disk 2 or later of a set — 87 are games behind an empty root
/// (IMP-019), 5 are raw-track images of protected disks, and the rest are
/// blank or every block marked used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskKind {
    /// An AmigaDOS volume with something on it.
    Files,
    /// A partitioned hard disk: its volumes are in its partitions.
    Device,
    /// An AmigaDOS volume with nothing on it, and nothing in its free space.
    Blank,
    /// No volume, and every byte zero: a disk never formatted, or a dump of
    /// nothing.
    Unformatted,
    /// An AmigaDOS volume with nothing on it, whose free blocks hold only
    /// what deleting left.
    Emptied,
    /// An empty AmigaDOS volume over a game its own loader reads: data in
    /// blocks the bitmap marks free, or blocks marked used that no file
    /// reaches (IMP-019).
    GameBehindEmptyRoot,
    /// No AmigaDOS volume, and a bootblock Kickstart will run: the disk boots
    /// into a loader of its own, which reads the tracks directly.
    OwnLoader,
    /// No AmigaDOS volume, and a bootblock that cannot boot — no `DOS`, a bad
    /// checksum, or no code: read by another disk's loader.
    DataDisk,
    /// Raw tracks rather than sectors, without a volume: a custom format, as
    /// copy protection used, that only its own loader reads.
    CustomTracks,
    /// Not readable as a disk at all: no geometry, a container ADE does not
    /// unpack, or an image shorter than its geometry.
    Unreadable,
}

impl DiskKind {
    /// Stable identifier for the JSON surface (F-015).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Device => "device",
            Self::Blank => "blank",
            Self::Unformatted => "unformatted",
            Self::Emptied => "emptied",
            Self::GameBehindEmptyRoot => "game-behind-empty-root",
            Self::OwnLoader => "own-loader",
            Self::DataDisk => "data-disk",
            Self::CustomTracks => "custom-tracks",
            Self::Unreadable => "unreadable",
        }
    }

    /// Whether the disk holds AmigaDOS files a person could list.
    #[must_use]
    pub const fn has_files(self) -> bool {
        matches!(self, Self::Files | Self::Device)
    }

    /// A few words for a row or a status line.
    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Files => "AmigaDOS files",
            Self::Device => "a hard disk; its volumes are in its partitions",
            Self::Blank => "blank: an AmigaDOS volume with nothing on it",
            Self::Unformatted => "unformatted: every byte is zero",
            Self::Emptied => "no files; its free space holds what deleting left",
            Self::GameBehindEmptyRoot => "a game behind an empty volume, read by its own loader",
            Self::OwnLoader => "boots into its own loader; no AmigaDOS files",
            Self::DataDisk => "a data disk for another disk's loader; no AmigaDOS files",
            Self::CustomTracks => "custom track format, read by its own loader",
            Self::Unreadable => "not readable as a disk",
        }
    }

    /// What it means, for a person looking at a disk with nothing listed.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::Files => "An AmigaDOS volume: its files and folders are listed.",
            Self::Device => {
                "A hard disk holds no volume of its own: each partition is a volume, listed \
                 beneath it."
            }
            Self::Blank => {
                "An AmigaDOS volume with nothing on it, and nothing left in its free space."
            }
            Self::Unformatted => {
                "Every byte of this disk is zero: it was never formatted, or the dump holds \
                 nothing."
            }
            Self::Emptied => {
                "An AmigaDOS volume with no files, whose free blocks still hold what deleting \
                 left. Carve may recover it."
            }
            Self::GameBehindEmptyRoot => {
                "An AmigaDOS volume with nothing in it, so Workbench shows a named disk, over a \
                 game that its own loader reads straight from the tracks. Writing a file to it \
                 would overwrite the game, so ADE asks first."
            }
            Self::OwnLoader => {
                "This disk boots into code of its own, which reads the tracks directly instead \
                 of through AmigaDOS, as most games did. There is no directory to list."
            }
            Self::DataDisk => {
                "This disk neither boots nor holds AmigaDOS files. Another disk's loader reads \
                 it directly, usually the first disk of its set."
            }
            Self::CustomTracks => {
                "This disk is kept as raw tracks, not all of them AmigaDOS sectors: a custom \
                 format, often copy protection, that only its own loader reads."
            }
            Self::Unreadable => "ADE cannot read this image as a disk; the container says why.",
        }
    }
}

/// What a disk with no mountable volume is.
fn kind_without_volume(inspection: &Inspection) -> DiskKind {
    if !inspection.partitions.is_empty() {
        return DiskKind::Device;
    }
    // Before the geometry: a raw-track image with no ordinary sector on any
    // track has none, and is a protected disk, not an unreadable file.
    if inspection.tracks.is_some() || inspection.flux.is_some() {
        return DiskKind::CustomTracks;
    }
    if inspection.geometry.is_none() {
        return DiskKind::Unreadable;
    }
    match &inspection.bootblock {
        // Kickstart runs a bootblock that says `DOS` and sums correctly; one
        // with no code would boot into nothing.
        Some(bb) if bb.is_dos() && bb.checksum_valid && bb.has_boot_code => DiskKind::OwnLoader,
        Some(_) => DiskKind::DataDisk,
        None => DiskKind::Unreadable,
    }
}

/// What a mounted volume with nothing on it is.
fn kind_of_empty(findings: &[Finding], bitmap: Option<&BitmapHealth>) -> DiskKind {
    let has = |code: &str| findings.iter().any(|f| f.code == code);
    if has("free-blocks-hold-data") || bitmap.is_some_and(|b| b.orphaned > 0) {
        DiskKind::GameBehindEmptyRoot
    } else if has("free-blocks-hold-deleted-files") {
        DiskKind::Emptied
    } else {
        DiskKind::Blank
    }
}

/// The volume a health report describes.
#[derive(Debug, Clone)]
pub struct Examined {
    /// The volume label from its rootblock.
    pub volume: String,
    /// The rootblock's block number, within the volume.
    pub rootblock: u32,
    /// The partition it came from, where the image is a partitioned device.
    pub partition: Option<String>,
}

impl Health {
    /// The worst severity present, if anything was found.
    #[must_use]
    pub fn worst(&self) -> Option<Severity> {
        self.findings.iter().map(|f| f.severity).max()
    }

    /// How many findings of each severity.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut c: (usize, usize, usize) = (0, 0, 0);
        for f in &self.findings {
            match f.severity {
                Severity::Info => c.0 = c.0.saturating_add(1),
                Severity::Warning => c.1 = c.1.saturating_add(1),
                Severity::Error => c.2 = c.2.saturating_add(1),
            }
        }
        c
    }

    /// Whether nothing worse than `Info` was found.
    #[must_use]
    pub fn is_sound(&self) -> bool {
        self.worst().is_none_or(|s| s < Severity::Warning)
    }
}

/// Examine an image and report its condition.
///
/// Never fails on a bad image: an unreadable or unmountable one is a *finding*,
/// not an error, which is the whole point of a health report.
#[must_use]
pub fn examine(bytes: Vec<u8>) -> Health {
    examine_partition(bytes, None)
}

/// Examine one partition of a device, or the image's own volume.
///
/// `partition` names a partition by drive name or index. On a partitioned
/// device `None` means the first partition, since a device holds no volume of
/// its own — checking such an image without naming a partition should examine
/// something rather than report that there is nothing there.
///
/// # Panics
/// Never: an unreadable image is a finding, not a failure.
#[must_use]
pub fn examine_partition(bytes: Vec<u8>, partition: Option<&str>) -> Health {
    examine_into(bytes, partition, &mut HashSet::new())
}

/// [`examine`], and every block the directory tree reaches — the set the
/// bitmap is checked against, which a bitmap repair writes back.
pub(crate) fn examine_in_use(bytes: Vec<u8>) -> (Health, HashSet<u32>) {
    let mut in_use = HashSet::new();
    let health = examine_into(bytes, None, &mut in_use);
    (health, in_use)
}

fn examine_into(bytes: Vec<u8>, partition: Option<&str>, in_use: &mut HashSet<u32>) -> Health {
    // The inspection unwraps an ADZ for itself; everything below reads blocks
    // directly, so it needs the same bytes the inspection saw. Passing the
    // still-compressed ones reported a truncated image on a sound disk.
    let (bytes, _) = crate::inspect::unwrap_container(bytes);
    // The inspection sees the container as it is — it does its own assembly
    // and reports the result — while the block reads below need the
    // reconstruction. Inspecting the assembled bytes instead would report a
    // protected disk's container as a plain ADF (F-007).
    let inspection = inspect_bytes(bytes.clone());
    let bytes = crate::inspect::assemble_for_reading(bytes);
    let mut findings: Vec<Finding> = inspection
        .faults()
        .into_iter()
        .map(|f| Finding::new(f.code, severity_of(f.code), f.message))
        .collect();

    let Some(geometry) = inspection.geometry else {
        findings.push(Finding::new(
            "container-unsupported",
            Severity::Info,
            inspection
                .volume_absent
                .clone()
                .unwrap_or_else(|| "no geometry could be established".to_owned()),
        ));
        return barren(inspection, findings);
    };

    // Asked here, while the bytes are at hand, for the one kind a disk with
    // no volume cannot be told apart from without them. Stops at the first
    // non-zero byte, which on a real disk is in the first few.
    let unformatted = bytes.iter().all(|&b| b == 0);
    let Ok(image) = ade_container::RawImage::new(bytes, geometry) else {
        findings.push(Finding::new(
            "image-truncated",
            Severity::Error,
            "the image is shorter than the geometry it claims",
        ));
        return barren(inspection, findings);
    };
    // A partitioned device has no volume at its own rootblock; the volume to
    // examine is inside a partition.
    let window = select_window(&image, &inspection, partition, &mut findings);
    let chosen_name = window.as_ref().ok().and_then(|w| w.as_ref()).and_then(|w| {
        inspection
            .partitions
            .iter()
            .find(|p| p.first_block == w.start())
            .map(|p| p.name.clone())
    });
    let mounted = match &window {
        Ok(Some(w)) => Volume::mount(w),
        Ok(None) => Volume::mount(&image),
        Err(()) => {
            return barren(inspection, findings);
        }
    };
    let Ok(volume) = mounted else {
        // Not an error: a quarter of real images are not AmigaDOS disks.
        findings.push(Finding::new(
            "no-volume",
            Severity::Info,
            inspection
                .volume_absent
                .clone()
                .unwrap_or_else(|| "no AmigaDOS volume".to_owned()),
        ));
        let mut health = barren(inspection, findings);
        if unformatted {
            health.kind = DiskKind::Unformatted;
        }
        return health;
    };

    volume_prelude(&inspection, &volume, &mut findings);

    let scan = scan_tree(&volume, &mut findings);
    let source: &dyn ade_block::BlockSource = match &window {
        Ok(Some(w)) => w,
        _ => &image,
    };
    // Nothing reached but the root: no file, directory or link.
    let empty = scan.referenced.len() <= 1;
    let bitmap = cross_check_bitmap(&volume, source, &scan.referenced, empty, &mut findings);
    in_use.clone_from(&scan.referenced);
    let kind = if !inspection.partitions.is_empty() {
        DiskKind::Device
    } else if empty {
        kind_of_empty(&findings, bitmap.as_ref())
    } else {
        DiskKind::Files
    };

    // Worst first, so the first line a reader sees is the one that matters.
    findings.sort_by_key(|f| core::cmp::Reverse(f.severity));
    Health {
        dircache: scan.dircache.clone(),
        bitmap,
        kind,
        directories: scan.directories,
        files: scan.files,
        bytes_recovered: scan.bytes,
        findings,
        examined: Some(Examined {
            volume: volume.rootblock().name_lossy(),
            rootblock: volume.root(),
            partition: window
                .as_ref()
                .ok()
                .and_then(|w| w.as_ref())
                .and_then(|_| chosen_name.clone()),
        }),
        inspection,
    }
}

struct TreeScan {
    directories: usize,
    files: usize,
    bytes: u64,
    referenced: HashSet<u32>,
    dircache: Option<DirCacheHealth>,
}

fn scan_tree(volume: &Volume<'_>, findings: &mut Vec<Finding>) -> TreeScan {
    let mut scan = TreeScan {
        directories: 0,
        files: 0,
        bytes: 0,
        referenced: HashSet::new(),
        dircache: None,
    };
    scan.referenced.insert(volume.root());

    let Ok(walked) = volume.walk(volume.root()) else {
        findings.push(Finding::new(
            "tree-unreadable",
            Severity::Error,
            "the directory tree could not be walked",
        ));
        return scan;
    };
    if walked.hit_limit {
        // A fault in ADE, not the disk (IMP-003).
        findings.push(Finding::new(
            "walk-capped",
            Severity::Error,
            "the tree walk hit its structural cap — cycle detection failed",
        ));
    }

    // A block referenced by two files is cross-linked: writing either corrupts
    // the other. Tracked by first owner so the report can name both — by its
    // header block, not its path: two entries of one name share a path, and
    // keyed by path their shared blocks looked like one file's (BUG-014).
    let mut owner: HashMap<u32, (u32, String)> = HashMap::new();

    for (path, entry) in &walked.entries {
        if entry.kind.is_directory() {
            scan.directories = scan.directories.saturating_add(1);
            scan.referenced.insert(entry.block);
            continue;
        }
        if !entry.kind.is_file() {
            scan.referenced.insert(entry.block);
            continue;
        }
        scan.files = scan.files.saturating_add(1);
        scan_file(volume, path, entry, &mut owner, &mut scan, findings);
    }

    scan_damaged_directories(&walked, findings);
    scan_dircaches(volume, &walked, &mut scan, findings);
    scan_hash_slots(volume, &walked, findings);
    scan_parents(volume, &walked, findings);
    scan_duplicate_names(volume, &walked, findings);
    scan_root_in_tables(volume, &walked, findings);
    scan
}

/// How many damaged directories to report before saying so in one line.
const MAX_DAMAGED_DIRECTORIES: usize = 20;

/// Directories whose hash table could not be listed in full (BUG-016).
///
/// `ade ls` has always printed these faults; the health report walked past
/// them, so a disk could read as whole with entries missing from it. One
/// finding per directory, naming what kind of damage it is, because they
/// mean different things: a pointer to nothing or to a non-entry may hide
/// entries; a loop hides them *and* hangs AmigaDOS listing the directory —
/// which crack groups used on purpose, a README whose chain points at itself
/// in a directory named for the group; a merge of two chains loses nothing,
/// but is a structure AmigaDOS never writes. Measured 2026-10-26 over 3,606
/// corpus volumes: 29 directories on 22 disks — 22 pointing at non-entries
/// (3 of them with merged chains too) and 7 loops.
fn scan_damaged_directories(walked: &ade_filesystem::volume::Walk, findings: &mut Vec<Finding>) {
    for (i, d) in walked.damaged.iter().enumerate() {
        if i == MAX_DAMAGED_DIRECTORIES {
            findings.push(Finding::new(
                "directory-damaged",
                Severity::Warning,
                format!(
                    "{} more directories are damaged",
                    walked.damaged.len().saturating_sub(i)
                ),
            ));
            break;
        }
        let place = if d.path.is_empty() {
            "the root directory".to_owned()
        } else {
            d.path.clone()
        };
        let mut said = Vec::new();
        if let Some(first) = d.faults.first() {
            said.push(match d.faults.len() {
                1 => format!("1 hash-table pointer leads to no entry ({first})"),
                n => format!("{n} hash-table pointers lead to no entry (first: {first})"),
            });
        }
        if let Some(first) = d.cycles.first() {
            said.push(format!(
                "a {first}, so listing it the way AmigaDOS does never finishes"
            ));
        }
        if let Some(first) = d.merges.first() {
            said.push(format!(
                "two hash chains merge at block {first}, which AmigaDOS never writes"
            ));
        }
        findings.push(
            Finding::new(
                "directory-damaged",
                Severity::Warning,
                format!("{place}: {}", said.join("; ")),
            )
            .at(d.block),
        );
    }
}

/// How many misplaced entries to report before saying so in one line.
const MAX_SLOT_FINDINGS: usize = 20;

/// Entries filed under a hash slot their name does not hash to (IMP-012).
///
/// A walk visits every slot, so such an entry is listed — by ADE, and by
/// ADFlib's `unadf -l` — but a lookup by name hashes the name and looks in
/// one slot, so AmigaDOS cannot open it — confirmed against ADFlib, whose
/// `unadf -p` cannot find one by name either. Measured 2026-10-23 over the
/// corpus: 30 entries on 6 of 3,606 volumes, every one consistent with a hand
/// edit or damage (a stale duplicate of a renamed file, directory-art names,
/// names holding NUL bytes) and none with the C-006 mistake of hashing with
/// the wrong function. An entry reachable from its right slot as well as a
/// wrong one — two chains merged — is found by name, and is not reported.
fn scan_hash_slots(
    volume: &Volume<'_>,
    walked: &ade_filesystem::volume::Walk,
    findings: &mut Vec<Finding>,
) {
    let directories = core::iter::once((String::new(), volume.root())).chain(
        walked
            .entries
            .iter()
            .filter(|(_, e)| e.kind == EntryKind::Directory)
            .map(|(path, e)| (path.clone(), e.block)),
    );
    let mut reported = 0usize;
    for (dir_path, dir) in directories {
        let Ok(found) = volume.misplaced(dir) else {
            continue;
        };
        for m in found {
            if reported >= MAX_SLOT_FINDINGS {
                reported = reported.saturating_add(1);
                continue;
            }
            reported = reported.saturating_add(1);
            let name: String = m.name.iter().map(|&b| char::from(b)).collect();
            let path = if dir_path.is_empty() {
                name
            } else {
                format!("{dir_path}/{name}")
            };
            let mode = if m.other_mode {
                " It fits the other hash function, which is the mistake C-006 describes."
            } else {
                ""
            };
            findings.push(
                Finding::new(
                    "entry-wrong-slot",
                    Severity::Warning,
                    format!(
                        "{path}: filed under hash slot {}, but its name hashes to {}; it is \
                         listed, and AmigaDOS cannot open it by name.{mode}",
                        m.slot, m.expected
                    ),
                )
                .at(m.block),
            );
        }
    }
    if reported > MAX_SLOT_FINDINGS {
        findings.push(Finding::new(
            "entry-wrong-slot",
            Severity::Warning,
            format!(
                "{} more entries are filed under the wrong hash slot",
                reported.saturating_sub(MAX_SLOT_FINDINGS)
            ),
        ));
    }
}

/// Entries whose header names a directory other than the one listing them
/// (BUG-018).
///
/// Every entry's `parent` field names its directory, and AmigaDOS follows
/// it: `ParentDir()` does, and so does every edit, which finds the entry's
/// directory through it. Measured 2026-10-27 over 3,606 corpus volumes: 18
/// entries on 3 disks, all Putty's, listed in the root and naming other
/// blocks — some not directories at all. The volume's own row (a root listed
/// in a hash table, IMP-015) is the root, not an entry, and is left to its
/// own finding.
fn scan_parents(
    volume: &Volume<'_>,
    walked: &ade_filesystem::volume::Walk,
    findings: &mut Vec<Finding>,
) {
    let directories = core::iter::once((String::new(), volume.root())).chain(
        walked
            .entries
            .iter()
            .filter(|(_, e)| e.kind == EntryKind::Directory)
            .map(|(path, e)| (path.clone(), e.block)),
    );
    let mut reported = 0usize;
    for (dir_path, dir) in directories {
        let Ok(listing) = volume.list(dir) else {
            continue;
        };
        for e in listing
            .entries
            .iter()
            .filter(|e| e.kind != EntryKind::Root && e.parent != dir)
        {
            reported = reported.saturating_add(1);
            if reported > MAX_SLOT_FINDINGS {
                continue;
            }
            let name = e.name_lossy();
            let path = if dir_path.is_empty() {
                name
            } else {
                format!("{dir_path}/{name}")
            };
            let named = match volume.entry_at(e.parent) {
                _ if e.parent == volume.root() => "the root".to_owned(),
                Ok(p) if p.kind.is_directory() => format!("the directory {}", p.name_lossy()),
                Ok(p) => format!("{}, which is not a directory", p.name_lossy()),
                Err(_) => "a block that is not an entry".to_owned(),
            };
            let listed = if dir == volume.root() {
                "the root".to_owned()
            } else {
                dir_path.clone()
            };
            findings.push(
                Finding::new(
                    "entry-wrong-parent",
                    Severity::Warning,
                    format!(
                        "{path}: listed in {listed}, but its header names block {} as its \
                         directory — {named}",
                        e.parent
                    ),
                )
                .at(e.block),
            );
        }
    }
    if reported > MAX_SLOT_FINDINGS {
        findings.push(Finding::new(
            "entry-wrong-parent",
            Severity::Warning,
            format!(
                "{} more entries name a directory other than the one listing them",
                reported.saturating_sub(MAX_SLOT_FINDINGS)
            ),
        ));
    }
}

/// Directories holding two or more entries of one name (IMP-016).
///
/// AmigaDOS cannot create them — a name is how it finds a file — but disks
/// carry them. Only one of a group can be opened by name, and a listing shows
/// them all without saying which, so the finding names it: the first match
/// down the slot the name hashes to, which is what AmigaDOS opens. Measured
/// 2026-10-25: 11 groups on 5 of 3,606 corpus volumes, 8 of them reachable
/// by name from none — every copy filed in a wrong slot.
fn scan_duplicate_names(
    volume: &Volume<'_>,
    walked: &ade_filesystem::volume::Walk,
    findings: &mut Vec<Finding>,
) {
    let directories = core::iter::once((String::new(), volume.root())).chain(
        walked
            .entries
            .iter()
            .filter(|(_, e)| e.kind == EntryKind::Directory)
            .map(|(path, e)| (path.clone(), e.block)),
    );
    let mut reported = 0usize;
    for (dir_path, dir) in directories {
        let Ok(found) = volume.duplicates(dir) else {
            continue;
        };
        for d in found {
            reported = reported.saturating_add(1);
            if reported > MAX_SLOT_FINDINGS {
                continue;
            }
            let name: String = d.name.iter().map(|&b| char::from(b)).collect();
            let path = if dir_path.is_empty() {
                name
            } else {
                format!("{dir_path}/{name}")
            };
            let blocks: Vec<String> = d.blocks.iter().map(u32::to_string).collect();
            let opened = match d.reached {
                Some(b) => format!(
                    "AmigaDOS opens block {b} by name, and the others appear only in a listing"
                ),
                None => "AmigaDOS can open none of them by name: each is filed under a wrong \
                         hash slot"
                    .to_owned(),
            };
            findings.push(
                Finding::new(
                    "duplicate-name",
                    Severity::Warning,
                    format!(
                        "{path}: {} entries share this name (blocks {}); {opened}.",
                        d.blocks.len(),
                        blocks.join(", ")
                    ),
                )
                .at(d.reached.or(d.blocks.first().copied()).unwrap_or(0)),
            );
        }
    }
    if reported > MAX_SLOT_FINDINGS {
        findings.push(Finding::new(
            "duplicate-name",
            Severity::Warning,
            format!(
                "{} more names are shared by several entries",
                reported.saturating_sub(MAX_SLOT_FINDINGS)
            ),
        ));
    }
}

/// A directory whose hash table leads to the rootblock (IMP-015).
///
/// Measured 2026-10-25: 7 of 3,606 corpus volumes, every one the root's own
/// table listing the root under the volume's name — a cracker's signature,
/// by the names (`Billiard from NightMaster`, `ACKERLIGHT 'The French
/// Light'`). It breaks nothing ADE can see: the walk does not descend into it
/// and it holds no data. Information, so a listing that shows the volume as
/// an entry of itself is explained rather than mistaken for damage.
fn scan_root_in_tables(
    volume: &Volume<'_>,
    walked: &ade_filesystem::volume::Walk,
    findings: &mut Vec<Finding>,
) {
    let directories = core::iter::once((String::new(), volume.root())).chain(
        walked
            .entries
            .iter()
            .filter(|(_, e)| e.kind == EntryKind::Directory)
            .map(|(path, e)| (path.clone(), e.block)),
    );
    for (dir_path, dir) in directories {
        let Ok(slots) = volume.slots_reaching_root(dir) else {
            continue;
        };
        if slots.is_empty() {
            continue;
        }
        let place = if dir_path.is_empty() {
            "the root's own hash table".to_owned()
        } else {
            format!("the hash table of {dir_path}")
        };
        let slots: Vec<String> = slots.iter().map(u32::to_string).collect();
        findings.push(
            Finding::new(
                "root-in-hash-table",
                Severity::Info,
                format!(
                    "slot {} of {place} leads to the rootblock, so the volume is listed as an \
                     entry of itself. It holds no data and nothing descends into it.",
                    slots.join(", ")
                ),
            )
            .at(dir),
        );
    }
}

/// Read every directory cache and account for the blocks it occupies.
///
/// Two jobs, and the first is the one that matters most. A dircache block is
/// marked used in the bitmap; if nothing reaches it, the bitmap cross-check
/// calls it orphaned. Before this ran, every `DOS\5` disk in the corpus
/// reported lost space that was not lost — 19 blocks on the Workbench 3.1
/// install disk alone.
///
/// The second job is the cross-check SPEC asks for: the cache duplicates what
/// the hash chains hold, so a disagreement means the disk is damaged or the
/// cache is stale. Neither side is preferred; the difference is reported.
fn scan_dircaches(
    volume: &Volume<'_>,
    walked: &ade_filesystem::volume::Walk,
    scan: &mut TreeScan,
    findings: &mut Vec<Finding>,
) {
    if !volume.has_dircache() {
        return;
    }

    // Only real directories carry a cache. A hard link to a directory has an
    // `extension` field like anything else, but it does not point at one, and
    // following it would walk whatever happens to be there.
    let directories = core::iter::once(volume.root()).chain(
        walked
            .entries
            .iter()
            .filter(|(_, e)| e.kind == EntryKind::Directory)
            .map(|(_, e)| e.block),
    );

    let mut summary = DirCacheHealth::default();
    let mut disagreements = 0usize;
    for dir in directories {
        let Ok(chain) = volume.dircache(dir) else {
            continue;
        };
        if !chain.blocks.is_empty() {
            summary.directories = summary.directories.saturating_add(1);
        }
        summary.blocks = summary.blocks.saturating_add(chain.blocks.len());
        summary.records = summary.records.saturating_add(chain.records().len());
        for block in chain.block_numbers() {
            scan.referenced.insert(block);
        }
        for fault in &chain.faults {
            findings.push(Finding::new(
                "dircache-chain-broken",
                Severity::Error,
                format!("directory cache for block {dir}: {fault}"),
            ));
        }

        let Ok(listing) = volume.list(dir) else {
            continue;
        };
        let differences = dircache::compare(&chain.records(), &listing.entries);
        summary.disagreements = summary.disagreements.saturating_add(differences.len());
        for difference in differences {
            // Capped: a systematically stale cache would otherwise produce one
            // finding per entry on the disk, burying everything else.
            if disagreements >= MAX_DIRCACHE_FINDINGS {
                continue;
            }
            disagreements = disagreements.saturating_add(1);
            findings.push(Finding::new(
                "dircache-disagrees",
                Severity::Warning,
                difference.to_string(),
            ));
        }
    }

    scan.dircache = Some(summary);

    if disagreements >= MAX_DIRCACHE_FINDINGS {
        findings.push(Finding::new(
            "dircache-disagrees",
            Severity::Warning,
            format!(
                "at least {MAX_DIRCACHE_FINDINGS} cache disagreements — \
                 the directory cache is systematically stale, not locally damaged"
            ),
        ));
    }
}

/// Everything one file contributes: its blocks, its cross-links, its contents.
fn scan_file(
    volume: &Volume<'_>,
    path: &str,
    entry: &ade_filesystem::entry::Entry,
    owner: &mut HashMap<u32, (u32, String)>,
    scan: &mut TreeScan,
    findings: &mut Vec<Finding>,
) {
    {
        if !entry.checksum_valid {
            findings.push(
                Finding::new(
                    "entry-checksum",
                    Severity::Warning,
                    format!("{path}: file header checksum does not match"),
                )
                .at(entry.block),
            );
        }

        if let Ok(blocks) = volume.file_blocks(entry) {
            for b in blocks {
                if let Some((first_block, first)) = owner.get(&b) {
                    if *first_block != entry.block {
                        // Equal paths are two entries of one name; their
                        // header blocks are what tells them apart.
                        let message = if first == path {
                            format!(
                                "block shared by {first} (header {first_block}) and {path} \
                                 (header {})",
                                entry.block
                            )
                        } else {
                            format!("block shared by {first} and {path}")
                        };
                        findings.push(
                            Finding::new("cross-linked-block", Severity::Error, message).at(b),
                        );
                    }
                } else {
                    owner.insert(b, (entry.block, path.to_owned()));
                }
                scan.referenced.insert(b);
            }
        }

        match volume.read_file(entry) {
            Ok(contents) => {
                scan.bytes = scan.bytes.saturating_add(contents.bytes.len() as u64);
                if contents.short_by > 0 {
                    findings.push(
                        Finding::new(
                            "file-short",
                            Severity::Warning,
                            format!(
                                "{path}: recovered {} of {} declared bytes",
                                contents.bytes.len(),
                                contents.declared_size
                            ),
                        )
                        .at(entry.block),
                    );
                }
                for fault in &contents.faults {
                    findings.push(
                        Finding::new(
                            "data-block-structure",
                            Severity::Warning,
                            format!("{path}: {fault}"),
                        )
                        .at(fault.first_block),
                    );
                }
                if contents.exceeded_volume {
                    findings.push(
                        Finding::new(
                            "file-exceeds-volume",
                            Severity::Error,
                            format!("{path}: read stopped at the volume's size"),
                        )
                        .at(entry.block),
                    );
                }
            }
            Err(e) => findings.push(
                Finding::new("file-unreadable", Severity::Error, format!("{path}: {e}"))
                    .at(entry.block),
            ),
        }
    }
}

fn cross_check_bitmap(
    volume: &Volume<'_>,
    image: &dyn ade_block::BlockSource,
    referenced: &HashSet<u32>,
    empty: bool,
    findings: &mut Vec<Finding>,
) -> Option<BitmapHealth> {
    let bitmap = Bitmap::read(image, volume.geometry(), volume.rootblock()).ok()?;

    // The flag is not re-reported here: `Inspection` already raises
    // `bitmap-flag-clear` from the rootblock, and `examine` inherits every
    // inspection fault as a finding. Emitting it again listed the same warning
    // twice in slightly different words — found by a batch run, where the code
    // showed up on exactly twice as many images as carried it.
    for &bad in &bitmap.bad_checksums {
        findings.push(
            Finding::new(
                "bitmap-checksum",
                Severity::Warning,
                "bitmap block checksum does not match",
            )
            .at(bad),
        );
    }
    if bitmap.incomplete {
        findings.push(Finding::new(
            "bitmap-incomplete",
            Severity::Warning,
            "the bitmap does not cover the whole volume",
        ));
    }

    // The bitmap blocks and the rootblock are allocated but are not *reached*
    // by a tree walk — they are the filesystem's own overhead. Counting them as
    // orphans would report a false positive on every healthy disk.
    // Only the bitmap blocks that exist: a pointer outside the volume names
    // no block, and counting it as one in use called it live data the map
    // had marked free (BUG-015).
    let mut referenced = referenced.clone();
    referenced.insert(volume.root());
    for &b in &bitmap.blocks {
        if !bitmap.out_of_range.contains(&b) {
            referenced.insert(b);
        }
    }
    if !bitmap.out_of_range.is_empty() {
        findings.push(Finding::new(
            "bitmap-pointer-invalid",
            Severity::Warning,
            format!(
                "the rootblock names {} for the bitmap outside the volume ({}) — the map \
                 is not all where it says, and what it says about some blocks is missing",
                if bitmap.out_of_range.len() == 1 {
                    "1 block".to_owned()
                } else {
                    format!("{} blocks", bitmap.out_of_range.len())
                },
                summarise(&bitmap.out_of_range)
            ),
        ));
    }

    // The two directions are not equally serious.
    let at_risk_blocks = bitmap.referenced_but_free(&referenced);
    let orphaned_blocks = bitmap.orphaned(&referenced);
    let referenced_but_free = at_risk_blocks.len();
    let orphaned = orphaned_blocks.len();

    if referenced_but_free > 0 {
        // Live data the filesystem believes is free: the next write destroys
        // it. This is the finding that justifies the whole cross-check.
        findings.push(Finding::new(
            "referenced-but-free",
            Severity::Error,
            format!(
                "{referenced_but_free} blocks are in use by files but marked free \
                 ({}) — writing to this volume would overwrite live data",
                summarise(&at_risk_blocks)
            ),
        ));
    }
    if orphaned > 0 {
        findings.push(Finding::new(
            "orphaned-blocks",
            Severity::Warning,
            format!(
                "{orphaned} blocks are marked in use but unreachable ({}) — lost \
                 space, or deleted files whose blocks were never freed",
                summarise(&orphaned_blocks)
            ),
        ));
    }

    let free_holding_data = if empty {
        report_free_data(image, volume.geometry(), &bitmap, &referenced, findings)
    } else {
        0
    };

    Some(BitmapHealth {
        marked_used: bitmap.used_count(),
        at_risk_blocks,
        orphaned_blocks,
        actually_used: referenced.len(),
        orphaned,
        referenced_but_free,
        covered: bitmap.covered(),
        flagged_valid: bitmap.flagged_valid,
        free_holding_data,
    })
}

/// On a volume with no entries, report free blocks that hold data, and
/// return how many there are.
fn report_free_data(
    image: &dyn ade_block::BlockSource,
    geometry: &ade_block::Geometry,
    bitmap: &Bitmap,
    referenced: &HashSet<u32>,
    findings: &mut Vec<Finding>,
) -> usize {
    // A game that loads its own tracks can still carry a valid, empty root,
    // so Workbench shows a named disk — and leave the blocks it reads marked
    // free, since it never asked AmigaDOS for them. Such a disk reads as empty
    // and unused, and the next write lands on the game (IMP-019). Measured
    // 2026-10-28: of 120 floppy-sized corpus volumes with no entries, 87 hold
    // data in free blocks, 68 of them in more than a thousand. Asked only of
    // an empty volume, because free blocks holding deleted files are ordinary.
    let (holding, listed) = free_holding_data(image, geometry, bitmap, referenced);
    if holding.is_empty() {
        return 0;
    }
    let (blocks, hold) = if holding.len() == 1 {
        ("1 block".to_owned(), "holds")
    } else {
        (format!("{} blocks", holding.len()), "hold")
    };
    if listed == holding.len() {
        // Only what deleting left: the volume was emptied, and nothing says a
        // loader reads it. Still counted, and still guarded by an edit
        // session, because a game mastered as files and then cleared looks the
        // same less one stray block — Starush disk 2 is 1,755 of 1,756.
        findings.push(Finding::new(
            "free-blocks-hold-deleted-files",
            Severity::Info,
            format!(
                "the volume holds no files, but {blocks} the bitmap marks free {hold} what \
                 deleting left ({}) — `ade carve` may recover it, and writing to the volume \
                 would overwrite it",
                summarise(&holding)
            ),
        ));
        return holding.len();
    }
    let deleted = match listed {
        0 => String::new(),
        1 => "; 1 of them listed by a deleted file's header".to_owned(),
        n => format!("; {n} of them listed by deleted files' headers"),
    };
    findings.push(Finding::new(
        "free-blocks-hold-data",
        Severity::Warning,
        format!(
            "the volume holds no files, but {blocks} the bitmap marks free {hold} data \
             ({}{deleted}) — on a disk like this it is usually a game its own loader reads, \
             and writing to the volume would overwrite it",
            summarise(&holding)
        ),
    ));
    holding.len()
}

/// The bitmap cross-check as JSON, split out to keep `Health::to_json`
/// readable.
fn bitmap_json(b: &BitmapHealth) -> Value {
    Value::Obj(vec![
        ("marked_used", Value::Num(b.marked_used as u64)),
        (
            "at_risk_blocks",
            Value::Arr(
                b.at_risk_blocks
                    .iter()
                    .map(|x| Value::Num(u64::from(*x)))
                    .collect(),
            ),
        ),
        ("actually_used", Value::Num(b.actually_used as u64)),
        ("orphaned", Value::Num(b.orphaned as u64)),
        (
            "referenced_but_free",
            Value::Num(b.referenced_but_free as u64),
        ),
        ("flagged_valid", Value::Bool(b.flagged_valid)),
        ("free_holding_data", Value::Num(b.free_holding_data as u64)),
        ("covered", Value::Num(u64::from(b.covered))),
        (
            // Integer arithmetic: a percentage derived from
            // block counts has no business going through a
            // float and back.
            "fill_percent",
            Value::Num(
                (b.marked_used as u64)
                    .saturating_mul(100)
                    .checked_div(u64::from(b.covered))
                    .unwrap_or(0),
            ),
        ),
    ])
}

/// Blocks the bitmap marks free, that nothing reaches, holding something
/// other than one 32-bit word repeated — zeros, or the fill a formatter
/// wrote: `DOS\0` from AmigaDOS's, `0xE5` from a PC's, and others the corpus
/// shows — with how many of them deleting left behind.
///
/// Deleting leaves what was deleted where it was: a file's or directory's
/// header, its extension blocks, its OFS data blocks and a directory's cache
/// blocks each still carry their type and a valid checksum, and a file's
/// header still lists its data blocks, which on FFS carry nothing of their
/// own. When every block is one of those, the volume was emptied by deleting,
/// which is information rather than a warning: without this, deleting the
/// last file on a disk called it a game. When some are not, it is a warning,
/// and every block counts, because a disk mastered by writing files and then
/// clearing the directory leaves exactly that for its loader to read.
fn free_holding_data(
    image: &dyn ade_block::BlockSource,
    geometry: &ade_block::Geometry,
    bitmap: &Bitmap,
    referenced: &HashSet<u32>,
) -> (Vec<u32>, usize) {
    let read = |b: u32| {
        let mut raw = vec![0u8; geometry.block_size() as usize];
        ade_block::read_at(image, ade_block::BlockIndex(u64::from(b)), &mut raw)
            .ok()
            .map(|()| raw)
    };
    let start = u64::from(geometry.reserved());
    let end = start
        .saturating_add(u64::from(bitmap.covered()))
        .min(geometry.total_blocks());
    let mut holding = Vec::new();
    let mut deleted: HashSet<u32> = HashSet::new();
    for b in start..end {
        let Ok(b) = u32::try_from(b) else { break };
        if bitmap.is_allocated(b) || referenced.contains(&b) || bitmap.blocks.contains(&b) {
            continue;
        }
        let Some(raw) = read(b) else { continue };
        let word = raw.get(..4).unwrap_or_default();
        if raw.chunks(4).all(|w| w == word) {
            continue;
        }
        holding.push(b);
        if !ade_block::checksum::normal_valid(&raw) {
            continue;
        }
        let kind = ade_endian::u32_at(&raw, 0).unwrap_or(0);
        if matches!(kind, T_LIST | T_DATA | T_DIRCACHE) {
            deleted.insert(b);
        } else if kind == T_HEADER {
            match ade_filesystem::entry::Entry::parse(&raw, b) {
                Ok(e) if e.looks_like_an_entry() && e.kind == EntryKind::File => {
                    claim_deleted(&read, b, raw, &mut deleted);
                }
                Ok(e) if e.looks_like_an_entry() => {
                    deleted.insert(b);
                }
                _ => {}
            }
        }
    }
    let listed = holding.iter().filter(|b| deleted.contains(b)).count();
    (holding, listed)
}

/// Add a deleted file's header, its extension blocks and every data block
/// they list to `claimed`. Bounded by a visited set and a cap, since what it reads is
/// whatever was left in free space (AV-001).
fn claim_deleted(
    read: &dyn Fn(u32) -> Option<Vec<u8>>,
    header: u32,
    raw: Vec<u8>,
    claimed: &mut HashSet<u32>,
) {
    /// More extension blocks than a double-density floppy could need.
    const CAP: usize = 64;
    let size = raw.len();
    let mut seen = HashSet::new();
    let (mut block, mut raw) = (header, raw);
    while seen.len() < CAP && seen.insert(block) {
        claimed.insert(block);
        let high = ade_endian::u32_at(&raw, 8).unwrap_or(0) as usize;
        for index in 0..high.min(size / 4) {
            let at = size
                .checked_sub(204)
                .and_then(|base| base.checked_sub(index.saturating_mul(4)));
            match at.map(|at| ade_endian::u32_at(&raw, at)) {
                Some(Ok(0)) => {}
                Some(Ok(data)) => {
                    claimed.insert(data);
                }
                _ => break,
            }
        }
        let next = size
            .checked_sub(8)
            .and_then(|at| ade_endian::u32_at(&raw, at).ok())
            .unwrap_or(0);
        let Some(next_raw) = (next != 0).then(|| read(next)).flatten() else {
            break;
        };
        (block, raw) = (next, next_raw);
    }
}

/// Severity for the faults `Inspection` already knows about.
fn severity_of(code: &str) -> Severity {
    match code {
        // Cosmetic: 567 corpus images have a day-zero datestamp and are
        // otherwise perfect.
        "datestamp-day-zero" | "datestamp-minutes-range" | "datestamp-ticks-range" => {
            Severity::Info
        }
        // The volume header itself is damaged.
        "rootblock-checksum" => Severity::Error,
        // Everything else — structural, but survivable and common: a stale
        // bitmap flag, undocumented dostype bits, an over-long name length.
        _ => Severity::Warning,
    }
}

impl Health {
    /// The report as a JSON value (F-015).
    #[must_use]
    pub fn to_json(&self) -> Value {
        let (info, warning, error) = self.counts();
        Value::Obj(vec![
            ("image", self.inspection.to_json()),
            (
                "kind",
                Value::Obj(vec![
                    ("code", Value::str(self.kind.as_str())),
                    ("summary", Value::str(self.kind.summary())),
                ]),
            ),
            (
                "examined",
                Value::opt(self.examined.as_ref(), |e| {
                    Value::Obj(vec![
                        ("volume", Value::str(e.volume.clone())),
                        ("rootblock", Value::Num(u64::from(e.rootblock))),
                        ("partition", Value::opt(e.partition.as_ref(), Value::str)),
                    ])
                }),
            ),
            (
                "tree",
                Value::Obj(vec![
                    ("directories", Value::Num(self.directories as u64)),
                    ("files", Value::Num(self.files as u64)),
                    ("bytes_recovered", Value::Num(self.bytes_recovered)),
                ]),
            ),
            (
                "dircache",
                Value::opt(self.dircache.as_ref(), |d| {
                    Value::Obj(vec![
                        ("directories", Value::Num(d.directories as u64)),
                        ("blocks", Value::Num(d.blocks as u64)),
                        ("records", Value::Num(d.records as u64)),
                        ("disagreements", Value::Num(d.disagreements as u64)),
                    ])
                }),
            ),
            ("bitmap", Value::opt(self.bitmap.as_ref(), bitmap_json)),
            (
                "summary",
                Value::Obj(vec![
                    ("info", Value::Num(info as u64)),
                    ("warning", Value::Num(warning as u64)),
                    ("error", Value::Num(error as u64)),
                    (
                        "worst",
                        Value::opt(self.worst(), |s| Value::str(s.as_str())),
                    ),
                ]),
            ),
            (
                "findings",
                Value::Arr(
                    self.findings
                        .iter()
                        .map(|f| {
                            Value::Obj(vec![
                                ("code", Value::str(f.code)),
                                ("severity", Value::str(f.severity.as_str())),
                                ("message", Value::str(f.message.clone())),
                                ("block", Value::opt(f.block, |b| Value::Num(u64::from(b)))),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// Render a block list compactly: a health report naming 15,863 blocks
/// individually is worse than one naming none.
fn summarise(blocks: &[u32]) -> String {
    const SHOWN: usize = 6;
    let head: Vec<String> = blocks.iter().take(SHOWN).map(u32::to_string).collect();
    if blocks.len() > SHOWN {
        format!(
            "{}, and {} more",
            head.join(", "),
            blocks.len().saturating_sub(SHOWN)
        )
    } else {
        head.join(", ")
    }
}

/// Findings that describe the *volume* rather than anything found in it.
///
/// Pushed before the tree walk because they change how everything after them
/// should be read.
fn volume_prelude(inspection: &Inspection, volume: &Volume<'_>, findings: &mut Vec<Finding>) {
    // F-007: everything below reads a reconstruction, not the file. Said
    // before any of it, because "880 orphaned blocks" means something quite
    // different on a disk half of which could not be decoded — there the
    // blocks are not lost, they were never recovered.
    if let Some(assembly) = &inspection.assembly {
        findings.push(Finding::new(
            "volume-reconstructed",
            Severity::Info,
            format!(
                "this volume was reassembled from a raw-track container — {} of {} sectors \
                 recovered ({}% of a disk). Missing sectors read as zeros, so unreachable \
                 blocks and short files below are expected rather than damage",
                assembly.sectors_placed,
                assembly.sectors_total,
                assembly.percent_complete()
            ),
        ));
    }

    // D-013: the long-name block layout is not implemented, and a classic read
    // of one does not fail — it silently produces the wrong name. The entry
    // blocks share a primary type, a secondary type and a checksum algorithm
    // with classic ones, and a wrong reading still checksums, so nothing
    // downstream will notice. Say so rather than present names that cannot
    // have been read correctly.
    if volume
        .dostype()
        .is_some_and(|d| d.mode() == Mode::LongNames)
    {
        findings.push(Finding::new(
            "lnfs-unsupported",
            Severity::Warning,
            "this volume uses the long-name layout (LNFS), which is not implemented \
             (D-013) — names and comments are being read at their classic offsets \
             and are unreliable; block accounting and checksums are not affected",
        ));
    }
}

/// Choose the window to examine, if the image is a partitioned device.
///
/// `Ok(None)` means the image is a single volume and should be examined as it
/// stands. `Err(())` means the caller named a partition that is not there, so
/// examining anything at all would answer a question nobody asked — a finding
/// is pushed and the report stops.
fn select_window<'a>(
    image: &'a ade_container::RawImage,
    inspection: &Inspection,
    wanted: Option<&str>,
    findings: &mut Vec<Finding>,
) -> Result<Option<ade_container::Window<'a>>, ()> {
    for fault in &inspection.partition_faults {
        findings.push(Finding::new(
            "partition-table-broken",
            Severity::Error,
            fault.clone(),
        ));
    }
    if inspection.partitions.is_empty() {
        if let Some(name) = wanted {
            findings.push(Finding::new(
                "no-partition-table",
                Severity::Error,
                format!("no partition table, so no partition {name:?}"),
            ));
            return Err(());
        }
        return Ok(None);
    }

    // A name is matched first, then an index, because a drive legitimately
    // named "0" should reach itself rather than the first partition.
    let index = match wanted {
        None => 0,
        Some(name) => {
            let found = inspection
                .partitions
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(name))
                .or_else(|| {
                    name.parse::<usize>()
                        .ok()
                        .filter(|i| *i < inspection.partitions.len())
                });
            let Some(i) = found else {
                findings.push(Finding::new(
                    "no-such-partition",
                    Severity::Error,
                    format!("this device has no partition {name:?}"),
                ));
                return Err(());
            };
            i
        }
    };
    let Some(chosen) = inspection.partitions.get(index) else {
        return Ok(None);
    };

    let blocks = u32::try_from(chosen.blocks).unwrap_or(u32::MAX);
    match ade_container::Window::new(
        image,
        chosen.first_block,
        blocks,
        chosen.block_size,
        chosen.reserved,
    ) {
        Ok(w) => Ok(Some(w)),
        Err(e) => {
            findings.push(Finding::new(
                "partition-out-of-range",
                Severity::Error,
                format!(
                    "partition {index} ({}) does not fit the device: {e}",
                    chosen.name
                ),
            ));
            Err(())
        }
    }
}

/// A report for an image that yielded no volume to examine.
///
/// The findings say why; everything a tree walk would have filled in is empty
/// rather than zeroed-and-plausible.
fn barren(inspection: Inspection, findings: Vec<Finding>) -> Health {
    Health {
        kind: kind_without_volume(&inspection),
        inspection,
        bitmap: None,
        dircache: None,
        directories: 0,
        files: 0,
        bytes_recovered: 0,
        findings,
        examined: None,
    }
}
