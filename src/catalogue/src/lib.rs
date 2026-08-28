//! Content hashing and dataset matching (F-013).
//!
//! A disk image says almost nothing about itself. Its filename is whatever the
//! last person to touch it chose, its volume label is often `Empty`, and
//! neither survives a copy. What does survive is the bytes — so identification
//! means hashing the content and asking a dataset what it is.
//!
//! # The dataset
//!
//! TOSEC publishes Logiqx XML datfiles: one `<rom>` element per known image,
//! carrying a name, a size, and CRC32/MD5/SHA1 hashes. 88,833 Amiga entries
//! across 98 files identify **98% of the 4652-image corpus** by CRC32 alone,
//! and recover the proper TOSEC name — year, publisher, disk number and the
//! `[cr]`/`[b]`/`[m]` provenance tags that a renamed file has lost.
//!
//! # CRC32 is a content hash, not an identity
//!
//! Measured across the dataset: **71 CRC32 collisions** among 88,833 entries,
//! and size does not disambiguate them — the colliding pairs are the same
//! length. None involves an `.adf`, and no corpus image lands on one, so the
//! risk is currently theoretical for Amiga floppies.
//!
//! It is theoretical rather than absent, which is why [`Catalogue::identify`]
//! returns **every** match rather than one. A caller that wants certainty
//! should compare the MD5 or SHA1 this also parses.

pub mod sha1;

use std::collections::BTreeMap;
use std::path::Path;

use ade_block::checksum::crc32;

/// One known image, as a datfile describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The name the dataset gives it — TOSEC's full convention.
    pub name: String,
    /// Size in bytes, as declared.
    pub size: u64,
    /// CRC32 of the content.
    pub crc32: u32,
    /// MD5, where the datfile carries one.
    pub md5: Option<String>,
    /// SHA1, where the datfile carries one.
    pub sha1: Option<String>,
    /// Which datfile this came from — the dataset's own name for the set.
    pub source: String,
}

impl Entry {
    /// Whether this entry's declared size matches some bytes.
    #[must_use]
    pub const fn size_matches(&self, len: u64) -> bool {
        self.size == len
    }
}

/// A loaded set of datfiles, indexed for lookup.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    by_crc: BTreeMap<u32, Vec<Entry>>,
    files: usize,
}

/// Why a catalogue could not be loaded.
#[derive(Debug)]
pub enum CatalogueError {
    /// The directory could not be read.
    Io(std::io::Error),
    /// No datfiles were found where they were expected.
    NoDatfiles {
        /// Where ADE looked.
        looked_in: String,
    },
}

impl core::fmt::Display for CatalogueError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::NoDatfiles { looked_in } => {
                write!(f, "no .dat files in {looked_in}")
            }
        }
    }
}

impl core::error::Error for CatalogueError {}

impl From<std::io::Error> for CatalogueError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl Catalogue {
    /// How many entries are indexed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_crc.values().map(Vec::len).sum()
    }

    /// Whether the catalogue holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_crc.is_empty()
    }

    /// How many datfiles were loaded.
    #[must_use]
    pub const fn files(&self) -> usize {
        self.files
    }

    /// Load every `.dat` in a directory.
    ///
    /// A file that cannot be read or parsed is skipped rather than fatal: a
    /// dataset is a pile of third-party files and one bad member should not
    /// cost you the other ninety-seven.
    ///
    /// # Errors
    /// [`CatalogueError`] if the directory cannot be read, or holds no datfiles.
    pub fn load_dir(dir: &Path) -> Result<Self, CatalogueError> {
        let mut catalogue = Self::default();
        for entry in std::fs::read_dir(dir)? {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("dat") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let source = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_owned();
            catalogue.add(&text, &source);
            catalogue.files = catalogue.files.saturating_add(1);
        }
        if catalogue.is_empty() {
            return Err(CatalogueError::NoDatfiles {
                looked_in: dir.display().to_string(),
            });
        }
        Ok(catalogue)
    }

    /// Parse one datfile's text into the index.
    pub fn add(&mut self, text: &str, source: &str) {
        for entry in parse(text, source) {
            self.by_crc.entry(entry.crc32).or_default().push(entry);
        }
    }

    /// Everything in the dataset whose content hash matches these bytes.
    ///
    /// The entries only. [`Self::identify_detailed`] says what several matches
    /// *mean*, which is usually the more useful answer.
    #[must_use]
    pub fn identify(&self, bytes: &[u8]) -> Vec<&Entry> {
        self.identify_detailed(bytes).entries
    }

    /// Match these bytes against the dataset, and classify what was found.
    ///
    /// # Several matches is usually not a problem
    ///
    /// A dataset lists content, and the same content can appear under more
    /// than one name — the same CD audio track as track 6 and track 10, the
    /// same ISO in two sets. Reporting that as an ambiguity implies ADE could
    /// not tell which one it is, when in truth **both names are correct**.
    ///
    /// The alarming case is different content sharing a CRC32, and it is worth
    /// distinguishing because a reader's response should differ: duplicate
    /// names are a property of the catalogue, a collision is a reason to
    /// distrust the match. Measured across all 88,921 Amiga entries, there are
    /// **77 duplicate groups and zero collisions**.
    ///
    /// # SHA-1 is computed only when it would change the answer
    ///
    /// CRC32 is table-driven and cheap; SHA-1 is neither. The CRC32 bucket is
    /// fetched first and SHA-1 runs only when more than one candidate
    /// survives — over the whole corpus, never.
    #[must_use]
    pub fn identify_detailed(&self, bytes: &[u8]) -> Identification<'_> {
        let len = bytes.len() as u64;
        let Some(bucket) = self.by_crc.get(&crc32(bytes)) else {
            return Identification::unknown();
        };
        // The size is checked too: it costs nothing and rules out an unrelated
        // file that happens to hash the same.
        let candidates: Vec<&Entry> = bucket.iter().filter(|e| e.size_matches(len)).collect();
        match candidates.len() {
            0 => Identification::unknown(),
            1 => Identification {
                entries: candidates,
                kind: Match::Named,
            },
            _ => Self::classify(bytes, candidates),
        }
    }

    /// Decide what several candidates mean, using SHA-1.
    fn classify<'a>(bytes: &[u8], candidates: Vec<&'a Entry>) -> Identification<'a> {
        let digest = sha1::hex(&sha1::sha1(bytes));
        let declared: Vec<&str> = candidates
            .iter()
            .filter_map(|e| e.sha1.as_deref())
            .collect();
        if declared.len() != candidates.len() {
            // Some candidate carries no SHA-1, so nothing can be concluded.
            // Saying so beats guessing in either direction.
            return Identification {
                entries: candidates,
                kind: Match::Unverified,
            };
        }

        let (ours, theirs): (Vec<&Entry>, Vec<&Entry>) = candidates
            .into_iter()
            .partition(|e| e.sha1.as_deref() == Some(digest.as_str()));
        if ours.is_empty() {
            // Every candidate agrees on the CRC32 and none on the content.
            // This is what a real collision looks like, and it has never been
            // seen in the Amiga set.
            return Identification {
                entries: theirs,
                kind: Match::Collision,
            };
        }
        let kind = if ours.len() == 1 {
            Match::Named
        } else {
            Match::Duplicated
        };
        Identification {
            entries: ours,
            kind,
        }
    }
}

/// What matching some bytes against the dataset found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// Nothing in the dataset holds these bytes.
    Unknown,
    /// One entry, and it is the answer.
    Named,
    /// Several entries of **identical content** — the dataset lists one file
    /// under more than one name. Every name returned is correct; this is a
    /// property of the catalogue rather than a doubt about the disk.
    Duplicated,
    /// Several entries of **different content** sharing a CRC32 and a size:
    /// a real hash collision, and none of them is these bytes. Never observed
    /// across the 88,921 Amiga entries.
    Collision,
    /// Several entries, and the dataset gives no SHA-1 to tell them apart.
    /// Not a collision and not a duplicate — an unanswerable question.
    Unverified,
}

impl Match {
    /// Whether the dataset named these bytes at all.
    #[must_use]
    pub const fn is_named(self) -> bool {
        matches!(self, Self::Named | Self::Duplicated)
    }
}

/// The result of matching bytes against the dataset.
#[derive(Debug, Clone)]
pub struct Identification<'a> {
    /// The entries that matched.
    pub entries: Vec<&'a Entry>,
    /// What those entries mean.
    pub kind: Match,
}

impl Identification<'_> {
    /// Nothing matched.
    #[must_use]
    fn unknown() -> Self {
        Self {
            entries: Vec::new(),
            kind: Match::Unknown,
        }
    }
}

/// Extract the `<rom>` entries from a Logiqx datfile.
///
/// A scanner rather than an XML parser, because the shape is narrow and fixed
/// and ADE has no dependencies. It reads attributes by name so attribute order
/// cannot break it, and resolves the five XML entities; only `&amp;` actually
/// occurs, 8772 times across the Amiga set.
#[must_use]
pub fn parse(text: &str, source: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for tag in text.split("<rom ").skip(1) {
        let Some(end) = tag.find('>') else { continue };
        let Some(fields) = tag.get(..end) else {
            continue;
        };
        let Some(name) = attribute(fields, "name") else {
            continue;
        };
        let Some(crc) = attribute(fields, "crc").and_then(|c| u32::from_str_radix(&c, 16).ok())
        else {
            continue;
        };
        let size = attribute(fields, "size")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        out.push(Entry {
            name: unescape(&name),
            size,
            crc32: crc,
            md5: attribute(fields, "md5"),
            sha1: attribute(fields, "sha1"),
            source: source.to_owned(),
        });
    }
    out
}

/// Read one double-quoted attribute out of a tag's text.
fn attribute(fields: &str, key: &str) -> Option<String> {
    let needle = alloc_needle(key);
    let at = fields.find(&needle)?;
    let start = at.checked_add(needle.len())?;
    let rest = fields.get(start..)?;
    let end = rest.find('"')?;
    rest.get(..end).map(ToOwned::to_owned)
}

/// `key="`, the thing an attribute starts with.
fn alloc_needle(key: &str) -> String {
    let mut s = String::with_capacity(key.len().saturating_add(2));
    s.push_str(key);
    s.push_str("=\"");
    s
}

/// Resolve the five XML entities.
fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        // Last, or an escaped ampersand in an entity name would be mangled.
        .replace("&amp;", "&")
}
