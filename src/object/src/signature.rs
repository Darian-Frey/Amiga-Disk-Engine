//! Recognising content by its magic bytes (F-020).
//!
//! # What this is for
//!
//! A disk's directory says what its files are *called*. It does not say what
//! they *are*, and on a thirty-year-old disk the two often disagree: names are
//! truncated, extensions are absent by convention, and the interesting bytes
//! are frequently in space no directory entry points at any more.
//!
//! So this scans the image itself. A hit carries the byte offset and the block
//! it falls in, which is enough to jump to in a hex view or to hand to
//! `extract`.
//!
//! # Anchored and unanchored, and why the distinction matters
//!
//! Most magics sit at the start of a file, and an Amiga file's data begins on
//! a block boundary — so requiring block alignment removes almost all of the
//! coincidental matches a four-byte substring search would otherwise produce.
//! A few formats put their marker *inside* the file: a ProTracker module's
//! `M.K.` is 1,080 bytes in, past a name and 31 sample headers. Those must be
//! searched for anywhere, and they are the ones that need their false-positive
//! rate measured rather than assumed.
//!
//! # The table is measured, not recalled
//!
//! Every signature below was checked against the 4,652-image corpus, and the
//! counts are recorded in SPEC §Content signatures. A magic that never appears
//! in four gigabytes of real Amiga disks is reported as untested rather than
//! quietly trusted.

/// What kind of thing a signature identifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// Something the Amiga would load and run.
    Executable,
    /// A compressor or cruncher's output.
    Compressed,
    /// An archive holding other files.
    Archive,
    /// Music, in a tracker or sample format.
    Audio,
    /// A picture.
    Image,
    /// Interchange File Format, whose type is in the header.
    Iff,
    /// A disk image inside a disk image.
    DiskImage,
    /// Everything else worth naming.
    Other,
}

impl Category {
    /// A stable code for the machine surface (F-015).
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Executable => "executable",
            Self::Compressed => "compressed",
            Self::Archive => "archive",
            Self::Audio => "audio",
            Self::Image => "image",
            Self::Iff => "iff",
            Self::DiskImage => "disk-image",
            Self::Other => "other",
        }
    }
}

/// Where a magic is allowed to appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// Only at the start of a block. An Amiga file's data begins on a block
    /// boundary, so this is where a file's own header lands — and requiring it
    /// discards nearly every coincidental match.
    BlockStart,
    /// Anywhere. For markers that sit inside a file rather than at its front,
    /// which is a licence to match by accident and is measured accordingly.
    Anywhere,
}

/// One recognisable thing.
#[derive(Debug, Clone, Copy)]
pub struct Signature {
    /// What it is, for a person.
    pub name: &'static str,
    /// What kind of thing it is.
    pub category: Category,
    /// The bytes that identify it.
    pub magic: &'static [u8],
    /// Where those bytes may appear.
    pub anchor: Anchor,
}

/// One place a signature was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Byte offset into the image.
    pub offset: u64,
    /// How many bytes matched, so a longer signature can outrank a shorter one
    /// at the same offset.
    pub magic_len: usize,
    /// The block that offset falls in.
    pub block: u64,
    /// What was recognised.
    pub name: &'static str,
    /// Its category's stable code.
    pub category: Category,
    /// Consecutive blocks this signature occupies, starting at `block`.
    ///
    /// One for an ordinary file header. More means the pattern repeats block
    /// after block, which no real header does — it is filler. `Powerstyx.adf`
    /// carries `DMS!1.52` across 88 consecutive blocks, and reporting that as
    /// 88 archives would be a confident wrong answer about a damaged disk.
    pub run: u32,
}

/// The signatures ADE looks for.
///
/// Amiga-specific formats first, then the general ones that turn up on Amiga
/// disks. Sources are cited in SPEC §Content signatures; a magic nobody could
/// point at a document for is not here.
pub const SIGNATURES: &[Signature] = &[
    // --- Amiga executables and system files ---
    Signature {
        name: "Amiga hunk executable",
        category: Category::Executable,
        magic: &[0x00, 0x00, 0x03, 0xF3],
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "Amiga hunk object",
        category: Category::Executable,
        magic: &[0x00, 0x00, 0x03, 0xE7],
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "Amiga icon (.info)",
        category: Category::Other,
        magic: &[0xE3, 0x10, 0x00, 0x01],
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "AmigaGuide document",
        category: Category::Other,
        magic: b"@database",
        anchor: Anchor::BlockStart,
    },
    // --- Interchange File Format ---
    Signature {
        name: "IFF",
        category: Category::Iff,
        magic: b"FORM",
        anchor: Anchor::BlockStart,
    },
    // --- Crunchers and packers ---
    Signature {
        name: "PowerPacker PP20",
        category: Category::Compressed,
        magic: b"PP20",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "PowerPacker PP11",
        category: Category::Compressed,
        magic: b"PP11",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "Imploder",
        category: Category::Compressed,
        magic: b"IMP!",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "XPK-compressed",
        category: Category::Compressed,
        magic: b"XPKF",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "RNC ProPack",
        category: Category::Compressed,
        magic: b"RNC\x01",
        anchor: Anchor::BlockStart,
    },
    // --- Archives ---
    Signature {
        name: "LZX archive",
        category: Category::Archive,
        magic: b"LZX",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "ZIP archive",
        category: Category::Archive,
        magic: b"PK\x03\x04",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "gzip",
        category: Category::Archive,
        magic: &[0x1F, 0x8B],
        anchor: Anchor::BlockStart,
    },
    // --- Trackers. These sit inside the file, not at its front. ---
    Signature {
        name: "ProTracker module (M.K.)",
        category: Category::Audio,
        magic: b"M.K.",
        anchor: Anchor::Anywhere,
    },
    Signature {
        name: "ProTracker module (M!K!)",
        category: Category::Audio,
        magic: b"M!K!",
        anchor: Anchor::Anywhere,
    },
    Signature {
        name: "Startrekker module",
        category: Category::Audio,
        magic: b"FLT4",
        anchor: Anchor::Anywhere,
    },
    Signature {
        name: "OctaMED module",
        category: Category::Audio,
        magic: b"MMD",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "AHX / THX module",
        category: Category::Audio,
        magic: b"THX",
        anchor: Anchor::BlockStart,
    },
    // --- Pictures that are not IFF ---
    Signature {
        name: "PNG",
        category: Category::Image,
        magic: &[0x89, b'P', b'N', b'G'],
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "JPEG",
        category: Category::Image,
        magic: &[0xFF, 0xD8, 0xFF],
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "GIF",
        category: Category::Image,
        magic: b"GIF8",
        anchor: Anchor::BlockStart,
    },
    // --- Disk images inside disk images ---
    // Found by scanning the corpus, and not what it first looks like. xDMS
    // fills a track it could not decompress with `DMS!!ERR` repeated, so this
    // is not an archive at all — it is a scar. An ADF carrying it was made
    // from a DMS that did not fully unpack, and the affected tracks hold
    // filler where data should be. Listed before the plain `DMS!` so the
    // specific reading wins.
    Signature {
        name: "xDMS unpack failure filler",
        category: Category::Other,
        magic: b"DMS!!ERR",
        anchor: Anchor::BlockStart,
    },
    // The other filler xDMS leaves: its own banner, repeated. Same meaning as
    // the one above — a track that did not decompress.
    Signature {
        name: "xDMS version filler",
        category: Category::Other,
        magic: b"DMS!1.",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "DMS archive",
        category: Category::DiskImage,
        magic: b"DMS!",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "SuperCard Pro flux",
        category: Category::DiskImage,
        magic: b"SCP",
        anchor: Anchor::BlockStart,
    },
    Signature {
        name: "extended ADF",
        category: Category::DiskImage,
        magic: b"UAE-1ADF",
        anchor: Anchor::BlockStart,
    },
];

/// Every signature found in an image, in offset order.
///
/// `block_size` is the volume's, so a hit can name the block it lands in — the
/// unit everything else in ADE speaks.
#[must_use]
pub fn scan(bytes: &[u8], block_size: u32) -> Vec<Hit> {
    scan_with(bytes, block_size, SIGNATURES)
}

/// As [`scan`], against a chosen table. Exists for tests, which need a table
/// small enough to reason about.
#[must_use]
pub fn scan_with(bytes: &[u8], block_size: u32, table: &[Signature]) -> Vec<Hit> {
    let block_size = u64::from(block_size.max(1));
    let mut hits = Vec::new();

    for signature in table {
        let magic = signature.magic;
        if magic.is_empty() || magic.len() > bytes.len() {
            continue;
        }
        match signature.anchor {
            Anchor::BlockStart => {
                // Only the head of each block is examined, which is both the
                // cheap way and the correct one: a file's own header lands
                // there and nowhere else.
                let step = usize::try_from(block_size).unwrap_or(512).max(1);
                let mut at = 0usize;
                while at < bytes.len() {
                    if bytes.get(at..at.saturating_add(magic.len())) == Some(magic) {
                        hits.push(hit(at as u64, block_size, signature));
                    }
                    at = at.saturating_add(step);
                }
            }
            Anchor::Anywhere => {
                for at in 0..bytes.len().saturating_sub(magic.len()).saturating_add(1) {
                    if bytes.get(at..at.saturating_add(magic.len())) == Some(magic) {
                        hits.push(hit(at as u64, block_size, signature));
                    }
                }
            }
        }
    }

    // The most specific match at an offset wins. `DMS!!ERR` and `DMS!` both
    // match the same bytes, and reporting an ADF full of xDMS failure filler
    // as containing eleven DMS archives is worse than saying nothing: it is a
    // confident wrong answer about what damaged the disk.
    hits.sort_by_key(|h| (h.offset, std::cmp::Reverse(h.magic_len), h.name));
    hits.dedup_by_key(|h| h.offset);
    collapse_runs(hits, block_size)
}

/// Fold a signature repeating at consecutive block starts into one hit.
///
/// A file header appears once. A pattern that appears at block *n*, *n+1*,
/// *n+2*… is filler — which on this corpus is xDMS writing over tracks it
/// could not decompress. Left uncollapsed, one damaged disk reports 88 finds
/// and buries the six real ones beside it; the count that matters is how far
/// the damage runs.
fn collapse_runs(hits: Vec<Hit>, block_size: u64) -> Vec<Hit> {
    let mut out: Vec<Hit> = Vec::with_capacity(hits.len());
    for hit in hits {
        if let Some(last) = out.last_mut()
            && last.name == hit.name
            && last.offset.checked_rem(block_size) == Some(0)
            && hit.offset.checked_rem(block_size) == Some(0)
            && last.block.saturating_add(u64::from(last.run)) == hit.block
        {
            last.run = last.run.saturating_add(1);
            continue;
        }
        out.push(hit);
    }
    out
}

fn hit(offset: u64, block_size: u64, signature: &Signature) -> Hit {
    Hit {
        offset,
        magic_len: signature.magic.len(),
        run: 1,
        block: offset.checked_div(block_size).unwrap_or(0),
        name: signature.name,
        category: signature.category,
    }
}
