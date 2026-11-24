//! Searching an image, and saying where a hit landed (F-021).
//!
//! The byte search is [`ade_object::find`]. What this adds is the answer a hex
//! editor cannot give: **which file owns the block a match fell in**, or that
//! nothing does. "Found at offset 322,205" sends someone to a hex view;
//! "found in `s/startup-sequence`" ends the question, and "found in a block no
//! directory entry points at" is frequently the more interesting of the two.
use crate::input::pieces;
use crate::json::Value;
/// Where a hit landed. Defined by [`crate::layout`], because it is a property
/// of the disk rather than of the search — the whole-disk map (F-022) and a
/// single hit's attribution must never disagree about what a block is.
pub use crate::layout::Region;
use ade_container::bytes::{ByteSource, Flat};
use ade_object::find::{Match, Pattern, search_at};

/// Every match in `bytes`, read a piece at a time, and how many were read.
fn search_all(bytes: &dyn ByteSource, pattern: &Pattern, block_size: u32) -> (Vec<Match>, u64) {
    let overlap = pattern.bytes.len().saturating_sub(1);
    let mut out = Vec::new();
    // A match needs the overlap whole, so one that starts in the overlap does
    // not fit in this piece and is the next one's alone.
    let scanned = pieces(bytes, block_size, overlap, |piece, at, _| {
        out.extend(search_at(piece, pattern, block_size, at));
        true
    });
    (out, scanned)
}

/// One match, with the file it belongs to where there is one.
#[derive(Debug, Clone)]
pub struct Found {
    /// Where it is.
    pub at: Match,
    /// The path of the entry whose blocks cover this one, if any.
    pub owner: Option<String>,
    /// That entry's own block, if any. The path names the owner for a reader;
    /// this identifies it for a program — a front end listing hits can offer
    /// the owning file for extraction from the block alone.
    pub owner_block: Option<u32>,
    /// What part of the disk it landed in.
    pub region: Region,
}
/// What a search found.
#[derive(Debug, Clone)]
pub struct Search {
    /// Every match, in offset order.
    pub matches: Vec<Found>,
    /// Bytes examined.
    pub scanned: u64,
    /// Whether the pattern was read as hex.
    pub was_hex: bool,
}
impl Search {
    /// Search an image, attributing each hit to a file where possible.
    #[must_use]
    pub fn run(bytes: &[u8], pattern: &Pattern) -> Self {
        // Mounted once. Both the block size and the owner map come from the
        // same image, and each `from_bytes` copies the whole disk — twice over
        // a 100 MB hardfile is the kind of quiet cost IMP-006 was about.
        match crate::Image::from_bytes(bytes.to_vec()) {
            Ok(image) => Self::over(&image, &bytes, pattern),
            Err(_) => Self::over_unmountable(&bytes, pattern),
        }
    }

    /// Search an image that is already open.
    ///
    /// For a caller holding a mounted [`crate::Image`] — the C ABI does, and so
    /// does anything that opened the disk to show it. [`Self::run`] mounts one
    /// from bytes; doing that again over a handle that exists is a second copy
    /// of the whole disk and a second walk of its directory tree, which is the
    /// cost IMP-006 was about.
    #[must_use]
    pub fn of_image(image: &crate::Image, pattern: &Pattern) -> Self {
        Self::over(image, &Flat(image.source()), pattern)
    }

    /// Search one partition of a device alone: its own bytes, its offsets
    /// and blocks counted from its start, its files named by their paths
    /// within it — as a floppy is searched (BUG-028).
    ///
    /// # Errors
    /// No partition table, or no partition `wanted` names.
    pub fn of_partition(
        image: &crate::Image,
        wanted: &str,
        pattern: &Pattern,
    ) -> Result<Self, String> {
        let (map, _, area) = crate::layout::attribute_partition(image, wanted)?;
        let source = area.source.get();
        let block_size = source.geometry().block_size();
        Ok(Self::attributed(&Flat(source), pattern, block_size, &map))
    }

    /// Search a file's bytes where they are, naming hits from `image`, its
    /// mount, if it has one.
    pub(crate) fn in_place(
        image: Option<&crate::Image>,
        bytes: &dyn ByteSource,
        pattern: &Pattern,
    ) -> std::io::Result<Self> {
        let found = match image {
            Some(image) => Self::over(image, bytes, pattern),
            None => Self::over_unmountable(bytes, pattern),
        };
        if found.scanned < bytes.size() {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        Ok(found)
    }

    /// The shared body: search `bytes`, attribute through `image`.
    fn over(image: &crate::Image, bytes: &dyn ByteSource, pattern: &Pattern) -> Self {
        let block_size = image.geometry().block_size();
        let map = crate::layout::attribute(image).0;
        Self::attributed(bytes, pattern, block_size, &map)
    }

    /// Search `bytes` and name each hit from `map`.
    fn attributed(
        bytes: &dyn ByteSource,
        pattern: &Pattern,
        block_size: u32,
        map: &std::collections::BTreeMap<u64, crate::layout::Owned>,
    ) -> Self {
        let (found, scanned) = search_all(bytes, pattern, block_size);
        let matches = found
            .into_iter()
            .map(|at| {
                let (region, owner, owner_block) = map
                    .get(&at.block)
                    .map_or((Region::Unclaimed, None, None), |(r, o, b)| {
                        (*r, o.clone(), *b)
                    });
                Found {
                    at,
                    owner,
                    owner_block,
                    region,
                }
            })
            .collect();
        Self {
            matches,
            scanned,
            was_hex: pattern.is_hex,
        }
    }

    /// A container that will not mount is still searchable — a quarter of real
    /// images are exactly that, and they are the ones worth searching.
    fn over_unmountable(bytes: &dyn ByteSource, pattern: &Pattern) -> Self {
        let (found, scanned) = search_all(bytes, pattern, 512);
        Self {
            matches: found
                .into_iter()
                .map(|at| Found {
                    at,
                    owner: None,
                    owner_block: None,
                    region: Region::Unclaimed,
                })
                .collect(),
            scanned,
            was_hex: pattern.is_hex,
        }
    }
    /// The search as JSON (F-015).
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("scanned", Value::Num(self.scanned)),
            ("hex", Value::Bool(self.was_hex)),
            ("found", Value::Num(self.matches.len() as u64)),
            (
                "matches",
                Value::Arr(
                    self.matches
                        .iter()
                        .map(|m| {
                            Value::Obj(vec![
                                ("offset", Value::Num(m.at.offset)),
                                ("block", Value::Num(m.at.block)),
                                // Null means no directory entry points at this
                                // block — deleted, unallocated, or outside the
                                // filesystem entirely.
                                ("file", Value::opt(m.owner.as_ref(), Value::str)),
                                (
                                    "file_block",
                                    Value::opt(m.owner_block.as_ref(), |b| {
                                        Value::Num(u64::from(*b))
                                    }),
                                ),
                                ("region", Value::str(m.region.name())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}
