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
        let image = crate::Image::from_bytes(bytes.to_vec()).ok();
        let block_size = image.as_ref().map_or(512, |i| i.geometry().block_size());
        let map = image
            .as_ref()
            .map(|i| crate::layout::attribute(i).0)
            .unwrap_or_default();
        let matches = search(bytes, pattern, block_size)
            .into_iter()
            .map(|at| {
                let (region, owner) = map
                    .get(&at.block)
                    .map_or((Region::Unclaimed, None), |(r, o, _)| (*r, o.clone()));
                Found { at, owner, region }
            })
            .collect();
        Self {
            matches,
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
                                ("region", Value::str(m.region.name())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}
