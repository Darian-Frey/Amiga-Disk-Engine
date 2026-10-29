//! IFF FTXT formatted text (F-033): what the clipboard and word processors
//! kept as text with its formatting.
//!
//! A `FORM FTXT`: its text in one or more `CHRS` chunks, taken together as
//! one stream, with `FONS` font specifiers and any private chunks around it.
//! The reading is the FTXT document's, as the AmigaOS wiki publishes it, and
//! is its "stripping" reader: printable characters and line feeds kept,
//! ESC and CSI control sequences — the formatting — skipped, every other
//! control character dropped.

use ade_endian::u32_at;

use crate::decoded::Field;

/// The most lines shown: a document longer than this is said to be.
const MAX_LINES: usize = 2000;

/// A decoded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    /// The text, stripped of its formatting.
    pub text: String,
    /// How many formatting sequences were stripped.
    pub sequences: usize,
    /// The other chunks, by ID, in order.
    pub other: Vec<String>,
}

/// Whether `bytes` are formatted text.
#[must_use]
pub fn is_ftxt(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"FORM") && bytes.get(8..12) == Some(b"FTXT")
}

/// Read formatted text. Never fails on an FTXT: one with no `CHRS` is an
/// empty text, as the document says.
#[must_use]
pub fn parse(bytes: &[u8]) -> Option<Text> {
    if !is_ftxt(bytes) {
        return None;
    }
    let declared = u32_at(bytes, 4).map_or(0, |v| v as usize);
    let end = declared.saturating_add(8).min(bytes.len());
    let mut out = Text {
        text: String::new(),
        sequences: 0,
        other: Vec::new(),
    };
    let mut at = 12usize;
    while at.saturating_add(8) <= end {
        let id = bytes.get(at..at.saturating_add(4)).unwrap_or(&[]);
        let len = u32_at(bytes, at.saturating_add(4)).map_or(0, |v| v as usize);
        let start = at.saturating_add(8);
        let data = bytes
            .get(start..start.saturating_add(len).min(bytes.len()))
            .unwrap_or(&[]);
        if id == b"CHRS" {
            strip(data, &mut out);
        } else {
            out.other.push(id.iter().map(|&b| char::from(b)).collect());
        }
        at = start.saturating_add(len).saturating_add(len & 1);
    }
    Some(out)
}

/// One `CHRS` chunk's text, its control sequences skipped. A sequence never
/// spans two chunks.
fn strip(data: &[u8], out: &mut Text) {
    let mut bytes = data.iter().copied().peekable();
    while let Some(b) = bytes.next() {
        match b {
            b'\n' => out.text.push('\n'),
            // ESC, then SP to '/' any number of times, then '0' to '~'.
            0x1B => {
                out.sequences = out.sequences.saturating_add(1);
                while bytes.next_if(|c| (0x20..=0x2F).contains(c)).is_some() {}
                let _ = bytes.next_if(|c| (0x30..=0x7E).contains(c));
            }
            // CSI, then SP to '?' any number of times, then '@' to '~'.
            0x9B => {
                out.sequences = out.sequences.saturating_add(1);
                while bytes.next_if(|c| (0x20..=0x3F).contains(c)).is_some() {}
                let _ = bytes.next_if(|c| (0x40..=0x7E).contains(c));
            }
            0x00..=0x1F | 0x7F..=0x9F => {}
            _ => out.text.push(char::from(b)),
        }
    }
}

impl Text {
    /// What the text says, in fields for the Decoded view: a line each.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const T: &str = "Text";
        let lines: Vec<&str> = self.text.lines().collect();
        let mut out = vec![
            Field::new(T, "Characters", self.text.chars().count().to_string()),
            Field::new(T, "Lines", lines.len().to_string()),
        ];
        if self.sequences > 0 {
            out.push(Field::new(
                T,
                "Formatting",
                format!("{} sequences, not shown", self.sequences),
            ));
        }
        if !self.other.is_empty() {
            // Each kind once, with how many: a TextCraft document holds a
            // `PINF` for every paragraph.
            let mut kinds: Vec<(&String, usize)> = Vec::new();
            for id in &self.other {
                match kinds.iter_mut().find(|(k, _)| *k == id) {
                    Some((_, n)) => *n = n.saturating_add(1),
                    None => kinds.push((id, 1)),
                }
            }
            let said: Vec<String> = kinds
                .iter()
                .map(|(k, n)| {
                    if *n == 1 {
                        (*k).clone()
                    } else {
                        format!("{k} × {n}")
                    }
                })
                .collect();
            out.push(Field::new(T, "Other chunks", said.join(", ")));
        }
        out.extend(
            lines.iter().take(MAX_LINES).enumerate().map(|(i, l)| {
                Field::new("Lines", (i.saturating_add(1)).to_string(), (*l).to_owned())
            }),
        );
        if lines.len() > MAX_LINES {
            out.push(Field::new(
                "Lines",
                "…",
                format!(
                    "{} more lines, in the Text tab",
                    lines.len().saturating_sub(MAX_LINES)
                ),
            ));
        }
        out
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    reason = "tests over bytes they build, by hand rather than with the helpers that read \
              them, so the two cannot agree about a field both have wrong (C-001's exception)"
)]
mod tests {
    use super::*;

    fn ftxt(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut inner = b"FTXT".to_vec();
        for (id, data) in chunks {
            inner.extend(*id);
            inner.extend((data.len() as u32).to_be_bytes());
            inner.extend(*data);
            if data.len() % 2 == 1 {
                inner.push(0);
            }
        }
        let mut out = b"FORM".to_vec();
        out.extend((inner.len() as u32).to_be_bytes());
        out.extend(inner);
        out
    }

    #[test]
    fn formatting_is_stripped_and_the_chunks_run_on() {
        let t = parse(&ftxt(&[
            (b"FONS", b"\x01\0\0\0topaz.font\0"),
            (
                b"CHRS",
                b"Hello \x9b1mbold\x9b0m and \x1b(Bplain,\r\nsecond ",
            ),
            (b"CHRS", b"line \xFC\x07"),
        ]))
        .unwrap();
        assert_eq!(t.text, "Hello bold and plain,\nsecond line ü");
        assert_eq!(t.sequences, 3);
        assert_eq!(t.other, vec!["FONS".to_owned()]);
        let f = t.fields();
        assert!(
            f.iter()
                .any(|x| x.label == "2" && x.value == "second line ü")
        );
        assert!(parse(b"FORM\0\0\0\x04ILBM").is_none());
        assert_eq!(
            parse(&ftxt(&[])).unwrap().text,
            "",
            "no CHRS is an empty text"
        );
    }
}
