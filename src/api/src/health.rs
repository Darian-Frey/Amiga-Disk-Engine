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

use ade_filesystem::{bitmap::Bitmap, dircache, dostype::Mode, entry::EntryKind, volume::Volume};

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
    let inspection = inspect_bytes(bytes.clone());
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
        return barren(inspection, findings);
    };

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

    let scan = scan_tree(&volume, &mut findings);
    let source: &dyn ade_block::BlockSource = match &window {
        Ok(Some(w)) => w,
        _ => &image,
    };
    let bitmap = cross_check_bitmap(&volume, source, &scan.referenced, &mut findings);

    // Worst first, so the first line a reader sees is the one that matters.
    findings.sort_by_key(|f| core::cmp::Reverse(f.severity));
    Health {
        dircache: scan.dircache.clone(),
        bitmap,
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
    // the other. Tracked by first owner so the report can name both.
    let mut owner: HashMap<u32, String> = HashMap::new();

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

    scan_dircaches(volume, &walked, &mut scan, findings);
    scan
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
    owner: &mut HashMap<u32, String>,
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
                if let Some(first) = owner.get(&b) {
                    if first != path {
                        findings.push(
                            Finding::new(
                                "cross-linked-block",
                                Severity::Error,
                                format!("block shared by {first} and {path}"),
                            )
                            .at(b),
                        );
                    }
                } else {
                    owner.insert(b, path.to_owned());
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
    findings: &mut Vec<Finding>,
) -> Option<BitmapHealth> {
    let bitmap = Bitmap::read(image, volume.geometry(), volume.rootblock()).ok()?;

    if !bitmap.flagged_valid {
        findings.push(Finding::new(
            "bitmap-flag-clear",
            Severity::Warning,
            "bitmap-valid flag is clear — the map may be stale (AV-003)",
        ));
    }
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
    let mut referenced = referenced.clone();
    referenced.insert(volume.root());
    for &b in &bitmap.blocks {
        referenced.insert(b);
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

    Some(BitmapHealth {
        marked_used: bitmap.used_count(),
        at_risk_blocks,
        orphaned_blocks,
        actually_used: referenced.len(),
        orphaned,
        referenced_but_free,
        covered: bitmap.covered(),
        flagged_valid: bitmap.flagged_valid,
    })
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
            (
                "bitmap",
                Value::opt(self.bitmap.as_ref(), |b| {
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
                }),
            ),
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
