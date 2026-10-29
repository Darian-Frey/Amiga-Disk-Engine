//! Locale catalogs (F-033): a program's words in one language, for
//! Workbench 2 and later's `locale.library`.
//!
//! A `FORM CTLG`: `FVER`, a `$VER:` string naming the catalog and its
//! version; `LANG`, the language; `CSET`, the code set; and `STRS`, every
//! string as an ID, a length and the bytes, each entry padded to an even
//! length and then to a multiple of four. The reading is AROS's
//! `OpenCatalogA()` (`workbench/libs/locale/opencataloga.c`), read locally
//! and not committed: the same padding, the same 30-byte limit on a
//! language's name.

use core::fmt::Write as _;

use ade_endian::u32_at;

use crate::decoded::Field;

/// A decoded catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    /// The `$VER` string, without `$VER: `.
    pub version: String,
    /// The language, as the catalog names it.
    pub language: String,
    /// The `CSET` code set; 0 is the Amiga's own, ISO 8859-1.
    pub code_set: Option<u32>,
    /// Every string, by ID, in the order the catalog holds them.
    pub strings: Vec<(u32, Vec<u8>)>,
    /// Bytes of `STRS` an entry claimed beyond the chunk's end.
    pub overrun: usize,
    /// Bytes at the end of `STRS` too few to be an entry: none, when the
    /// padding is read as it was written.
    pub left: usize,
}

/// Why bytes are not a catalog ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotACatalog(pub String);

impl core::fmt::Display for NotACatalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotACatalog {}

/// Whether `bytes` are a catalog.
#[must_use]
pub fn is_catalog(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"FORM") && bytes.get(8..12) == Some(b"CTLG")
}

/// Latin-1 text to the first NUL.
fn text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect()
}

/// Read a catalog.
///
/// # Errors
/// [`NotACatalog`] when it is not one, or holds no strings.
pub fn parse(bytes: &[u8]) -> Result<Catalog, NotACatalog> {
    if !is_catalog(bytes) {
        return Err(NotACatalog("not a FORM CTLG".to_owned()));
    }
    let declared = u32_at(bytes, 4).map_or(0, |v| v as usize);
    let end = declared.saturating_add(8).min(bytes.len());
    let mut catalog = Catalog {
        version: String::new(),
        language: String::new(),
        code_set: None,
        strings: Vec::new(),
        overrun: 0,
        left: 0,
    };
    let mut found_strings = false;
    let mut at = 12usize;
    while at.saturating_add(8) <= end {
        let id = bytes.get(at..at.saturating_add(4)).unwrap_or(&[]);
        let len = u32_at(bytes, at.saturating_add(4)).map_or(0, |v| v as usize);
        let start = at.saturating_add(8);
        let data = bytes
            .get(start..start.saturating_add(len).min(bytes.len()))
            .unwrap_or(&[]);
        match id {
            b"FVER" => {
                let v = text(data);
                v.strip_prefix("$VER: ")
                    .unwrap_or(&v)
                    .trim()
                    .clone_into(&mut catalog.version);
            }
            b"LANG" => catalog.language = text(data.get(..data.len().min(30)).unwrap_or(&[])),
            b"CSET" => catalog.code_set = u32_at(data, 0).ok(),
            b"STRS" => {
                found_strings = true;
                strings(data, &mut catalog);
            }
            _ => {}
        }
        at = start.saturating_add(len).saturating_add(len & 1);
    }
    if !found_strings {
        return Err(NotACatalog("it has no STRS, so no strings".to_owned()));
    }
    Ok(catalog)
}

/// The entries of a `STRS` chunk: an ID, a length, the bytes, padded to an
/// even length and then to four. Each is read as far as the chunk goes.
fn strings(data: &[u8], catalog: &mut Catalog) {
    let mut at = 0usize;
    while at.saturating_add(8) <= data.len() {
        let id = u32_at(data, at).unwrap_or(0);
        let len = u32_at(data, at.saturating_add(4)).map_or(0, |v| v as usize);
        let start = at.saturating_add(8);
        let stop = start.saturating_add(len);
        catalog.overrun = catalog
            .overrun
            .saturating_add(stop.saturating_sub(data.len()));
        let string = data.get(start..stop.min(data.len())).unwrap_or(&[]);
        // A catalog's strings are C strings: the length counts what is
        // there, and the NUL may be inside it or in the padding.
        let string = string.split(|&b| b == 0).next().unwrap_or(&[]).to_vec();
        catalog.strings.push((id, string));
        let mut size = 8usize.saturating_add(len).saturating_add(len & 1);
        if size & 3 != 0 {
            size = size.saturating_add(4usize.saturating_sub(size & 3));
        }
        at = at.saturating_add(size);
    }
    catalog.left = data.len().saturating_sub(at);
}

/// A string as one line: line breaks and other control codes shown.
fn shown(s: &[u8]) -> String {
    let mut out = String::new();
    for &b in s {
        match b {
            b'\n' => out.push('⏎'),
            b'\t' => out.push('⇥'),
            0x00..=0x1F | 0x7F..=0x9F => {
                let _ = write!(out, "\\x{b:02X}");
            }
            _ => out.push(char::from(b)),
        }
    }
    out
}

impl Catalog {
    /// What the catalog holds, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const C: &str = "Catalog";
        let mut out = vec![
            Field::new(C, "Version", self.version.clone()),
            Field::new(C, "Language", self.language.clone()),
            Field::new(
                C,
                "Code set",
                match self.code_set {
                    Some(0) => "0, the Amiga's own (ISO 8859-1)".to_owned(),
                    Some(n) => n.to_string(),
                    None => "not given".to_owned(),
                },
            ),
            Field::new(C, "Strings", self.strings.len().to_string()),
        ];
        if self.overrun > 0 {
            out.push(Field::new(
                C,
                "Cut short",
                format!(
                    "the strings claim {} bytes more than the file holds",
                    self.overrun
                ),
            ));
        }
        out.extend(
            self.strings
                .iter()
                .map(|(id, s)| Field::new("Strings", id.to_string(), shown(s))),
        );
        out
    }
}

#[cfg(test)]
#[allow(
    clippy::trivially_copy_pass_by_ref,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    reason = "tests over bytes they build, by hand rather than with the helpers that read \
              them, so the two cannot agree about a field both have wrong (C-001's exception)"
)]
mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend((data.len() as u32).to_be_bytes());
        c.extend(data);
        if data.len() % 2 == 1 {
            c.push(0);
        }
        c
    }

    /// A catalog of `strings`, each entry padded to four as CatComp writes
    /// it, with or without its NUL inside the length.
    fn catalog(strings: &[(u32, &[u8])], nul_inside: bool) -> Vec<u8> {
        let mut strs = Vec::new();
        for (id, s) in strings {
            let len = s.len() + usize::from(nul_inside);
            strs.extend(id.to_be_bytes());
            strs.extend((len as u32).to_be_bytes());
            strs.extend(*s);
            if nul_inside {
                strs.push(0);
            }
            // Each entry starts on four, so padding it to four is padding
            // the whole to four.
            while strs.len() % 4 != 0 {
                strs.push(0);
            }
        }
        let mut inner = b"CTLG".to_vec();
        inner.extend(chunk(b"FVER", b"$VER: test.catalog 1.2 (29.10.26)\0"));
        inner.extend(chunk(b"LANG", b"deutsch\0"));
        inner.extend(chunk(b"CSET", &[0u8; 32]));
        inner.extend(chunk(b"STRS", &strs));
        let mut out = b"FORM".to_vec();
        out.extend((inner.len() as u32).to_be_bytes());
        out.extend(inner);
        out
    }

    #[test]
    fn a_catalog_reads_its_version_language_and_every_string() {
        // Lengths 1, 2, 3 and 5 cover every padding to four.
        let strings: &[(u32, &[u8])] = &[(0, b"a"), (1, b"bc"), (7, b"def"), (8, b"\xFCber\n")];
        for nul_inside in [false, true] {
            let c = parse(&catalog(strings, nul_inside)).unwrap();
            assert_eq!(c.version, "test.catalog 1.2 (29.10.26)");
            assert_eq!(c.language, "deutsch");
            assert_eq!(c.code_set, Some(0));
            let got: Vec<(u32, &[u8])> =
                c.strings.iter().map(|(i, s)| (*i, s.as_slice())).collect();
            assert_eq!(got, strings, "NUL inside the length: {nul_inside}");
            assert_eq!((c.overrun, c.left), (0, 0));
        }
        let f = parse(&catalog(strings, true)).unwrap().fields();
        assert!(f.iter().any(|x| x.label == "8" && x.value == "über⏎"));
    }

    #[test]
    fn a_string_longer_than_its_chunk_is_counted_not_read_past() {
        let mut bytes = catalog(&[(0, b"abc")], false);
        // The one entry claims 100 bytes.
        let at = bytes.windows(4).position(|w| w == b"STRS").unwrap() + 8 + 4;
        bytes[at..at + 4].copy_from_slice(&100u32.to_be_bytes());
        let c = parse(&bytes).unwrap();
        assert!(c.overrun > 0);
        assert_eq!(c.strings[0].1, b"abc");
        assert!(parse(b"FORM\0\0\0\x04CTLG").is_err(), "no STRS");
    }
}
