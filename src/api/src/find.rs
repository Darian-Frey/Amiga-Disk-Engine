//! Searching an image, and saying where a hit landed (F-021).
//!
//! The byte search is [`ade_object::find`]. What this adds is the answer a hex
//! editor cannot give: **which file owns the block a match fell in**, or that
//! nothing does. "Found at offset 322,205" sends someone to a hex view;
//! "found in `s/startup-sequence`" ends the question, and "found in a block no
//! directory entry points at" is frequently the more interesting of the two.
use crate::json::Value;
/// Where a hit landed. Defined by [`crate::layout`], because it is a property
/// of the disk rather than of the search — the whole-disk map (F-022) and a
/// single hit's attribution must never disagree about what a block is.
pub use crate::layout::Region;
use ade_object::find::{Match, Pattern, search};

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
            Ok(image) => Self::over(&image, bytes, pattern),
            Err(_) => Self::over_unmountable(bytes, pattern),
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
        let bytes = image.read_range(0, image.geometry().total_bytes());
        Self::over(image, &bytes, pattern)
    }

    /// The shared body: search `bytes`, attribute through `image`.
    fn over(image: &crate::Image, bytes: &[u8], pattern: &Pattern) -> Self {
        let block_size = image.geometry().block_size();
        let map = crate::layout::attribute(image).0;
        let matches = search(bytes, pattern, block_size)
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
            scanned: bytes.len() as u64,
            was_hex: pattern.is_hex,
        }
    }

    /// A container that will not mount is still searchable — a quarter of real
    /// images are exactly that, and they are the ones worth searching.
    fn over_unmountable(bytes: &[u8], pattern: &Pattern) -> Self {
        Self {
            matches: search(bytes, pattern, 512)
                .into_iter()
                .map(|at| Found {
                    at,
                    owner: None,
                    owner_block: None,
                    region: Region::Unclaimed,
                })
                .collect(),
            scanned: bytes.len() as u64,
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
