//! Scanning an image for recognisable content (F-020).
//!
//! The engine side is [`ade_object::signature`]; this wires it to an image and
//! to the JSON surface. Scanning the **whole image** rather than each file is
//! the point: on a thirty-year-old disk the interesting bytes are often in
//! space no directory entry points at any more.

use ade_object::signature::{self, Hit};

use ade_container::bytes::{ByteSource, Flat};

use crate::input::pieces;
use crate::json::Value;

/// What a scan found.
#[derive(Debug, Clone)]
pub struct Scan {
    /// Every hit, in offset order.
    pub hits: Vec<Hit>,
    /// Bytes examined.
    pub scanned: u64,
}

impl Scan {
    /// Scan an image's bytes.
    #[must_use]
    pub fn of(bytes: &[u8], block_size: u32) -> Self {
        Self {
            hits: signature::scan(bytes, block_size),
            scanned: bytes.len() as u64,
        }
    }

    /// Scan bytes too many to hold, a piece at a time.
    ///
    /// # Errors
    /// They could not all be read: part of a scan reads as a scan of the whole.
    pub fn over(bytes: &dyn ByteSource, block_size: u32) -> std::io::Result<Self> {
        let scan = Self::readable(bytes, block_size);
        if scan.scanned < bytes.size() {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        Ok(scan)
    }

    /// Scan one partition of a device: its own bytes, offsets from its start
    /// (BUG-028), as far as it can be read.
    ///
    /// # Errors
    /// No partition table, or no partition `wanted` names.
    pub fn of_partition(image: &crate::Image, wanted: &str) -> Result<Self, String> {
        let area = image.area(wanted)?;
        let source = area.source.get();
        Ok(Self::readable(
            &Flat(source),
            source.geometry().block_size(),
        ))
    }

    fn readable(bytes: &dyn ByteSource, block_size: u32) -> Self {
        let mut found = signature::Pieces::new(block_size);
        let overlap = signature::longest_magic().saturating_sub(1);
        let scanned = pieces(bytes, block_size, overlap, |piece, at, own| {
            found.add(piece, at, own);
            true
        });
        Self {
            hits: found.finish(),
            scanned,
        }
    }

    /// Whether anything was recognised.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// The scan as JSON (F-015).
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("scanned", Value::Num(self.scanned)),
            ("found", Value::Num(self.hits.len() as u64)),
            (
                "hits",
                Value::Arr(
                    self.hits
                        .iter()
                        .map(|h| {
                            Value::Obj(vec![
                                ("name", Value::str(h.name)),
                                ("category", Value::str(h.category.code())),
                                ("offset", Value::Num(h.offset)),
                                ("block", Value::Num(h.block)),
                                // Blocks the pattern runs for. More than one
                                // means filler rather than a file header.
                                ("blocks", Value::Num(u64::from(h.run))),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}
