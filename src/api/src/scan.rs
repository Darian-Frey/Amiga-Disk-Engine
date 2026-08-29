//! Scanning an image for recognisable content (F-020).
//!
//! The engine side is [`ade_object::signature`]; this wires it to an image and
//! to the JSON surface. Scanning the **whole image** rather than each file is
//! the point: on a thirty-year-old disk the interesting bytes are often in
//! space no directory entry points at any more.

use ade_object::signature::{self, Hit};

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
