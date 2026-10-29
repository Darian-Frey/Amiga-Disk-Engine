//! Printer drivers (F-033): what a `DEVS:Printers` driver says of its
//! printer.
//!
//! A driver is a loadable executable whose first hunk is a `PrinterSegment`
//! as it sits in memory — the loader's link to the next segment comes before
//! the hunk — so the hunk opens with `ps_runAlert`, a `MOVEQ`/`RTS` that
//! makes running the file do nothing, then the driver's version and
//! revision, then its `PrinterExtendedData`: a pointer to the printer's
//! name, four function pointers, the printer's class and colour class, how
//! many columns, character sets and dot rows it has, the dots it can print
//! across and down, and its resolution. The layout and constants are AROS's
//! `devices/prtbase.h`, read locally and not committed.
//!
//! A driver is recognised by its relocations: the name and the four
//! functions all relocated where the structure puts them, and no `RomTag`
//! where the version goes — a library opens with the same `MOVEQ`/`RTS`, and
//! then its `RomTag`.

use ade_endian::{i32_at, u8_at, u16_at, u32_at};

use crate::decoded::Field;
use crate::hunk;

/// Where the fields sit in the first hunk.
const VERSION_AT: usize = 4;
const PED_AT: usize = 8;

/// The longest name read: a printer's is a few words.
const MAX_NAME: usize = 64;

/// A decoded printer driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Driver {
    /// The printer's name, as the driver gives it: the first line of its
    /// name string.
    pub name: String,
    /// A further line of the name string, where there is one: Commodore
    /// Frankfurt's drivers follow the name with `© Commodore Ffm.`.
    pub note: Option<String>,
    /// The driver's version and revision.
    pub version: (u16, u16),
    /// `ped_PrinterClass`: graphics, colour, and the V44 flags.
    pub class: u8,
    /// `ped_ColorClass`.
    pub colour_class: u8,
    /// `ped_MaxColumns`: characters across a line of text.
    pub columns: u8,
    /// `ped_NumCharSets`: character sets it knows.
    pub char_sets: u8,
    /// `ped_NumRows`: dots the head prints in one pass.
    pub rows: u16,
    /// `ped_MaxXDots`, `ped_MaxYDots`: the dots it prints across and down;
    /// zero for no limit.
    pub max_dots: (u32, u32),
    /// `ped_XDotsInch`, `ped_YDotsInch`.
    pub dpi: (u16, u16),
    /// `ped_TimeoutSecs`.
    pub timeout: i32,
}

/// Why bytes are not a driver ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotADriver(pub String);

impl core::fmt::Display for NotADriver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotADriver {}

/// A hunk's bytes as the file holds them.
fn hunk_bytes<'a>(bytes: &'a [u8], h: &hunk::Hunk) -> Option<&'a [u8]> {
    bytes.get(h.file_offset..h.file_offset.checked_add(h.file_len)?)
}

/// The executable, its first hunk's bytes and the hunk the name is in, when
/// `bytes` are a driver.
fn driver(bytes: &[u8]) -> Option<(hunk::Executable, Vec<u8>, u32)> {
    let exe = hunk::parse(bytes)?;
    let first = exe.hunks.first()?;
    let data = hunk_bytes(bytes, first)?.to_vec();
    let alert = u32_at(&data, 0).ok()?;
    // MOVEQ #n,D0 then RTS.
    if alert & 0xFF00_FFFF != 0x7000_4E75 || u16_at(&data, VERSION_AT).ok()? == 0x4AFC {
        return None;
    }
    let target_of = |at: usize| {
        first
            .relocs
            .iter()
            .find(|r| r.offset as usize == at)
            .map(|r| r.target)
    };
    let name_hunk = target_of(PED_AT)?;
    let functions =
        (1..=4usize).all(|i| target_of(PED_AT.saturating_add(i.saturating_mul(4))).is_some());
    functions.then_some((exe, data, name_hunk))
}

/// Whether `bytes` are a printer driver.
#[must_use]
pub fn is_driver(bytes: &[u8]) -> bool {
    hunk::is_executable(bytes) && driver(bytes).is_some()
}

/// Read a printer driver.
///
/// # Errors
/// [`NotADriver`] when it is not one, or its name lies outside its hunk.
pub fn parse(bytes: &[u8]) -> Result<Driver, NotADriver> {
    let (exe, data, name_hunk) =
        driver(bytes).ok_or_else(|| NotADriver("not a printer driver".to_owned()))?;
    let ped = |at: usize| PED_AT.saturating_add(at);
    let name_at = u32_at(&data, ped(0)).map_or(usize::MAX, |v| v as usize);
    let named = exe
        .hunks
        .get(name_hunk as usize)
        .and_then(|h| hunk_bytes(bytes, h))
        .and_then(|h| h.get(name_at..))
        .ok_or_else(|| NotADriver("its printer's name lies outside the file".to_owned()))?;
    // To a NUL, or to the first control character that is not a line break:
    // some drivers run straight on from a line of text into code.
    let string: String = named
        .iter()
        .take(MAX_NAME)
        .take_while(|&&b| b >= 0x20 || b == b'\r' || b == b'\n')
        .map(|&b| char::from(b))
        .collect();
    let mut lines = string
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|l| !l.is_empty());
    let name = lines.next().unwrap_or_default().to_owned();
    let note = lines.next().map(str::to_owned);
    let byte = |at: usize| u8_at(&data, ped(at)).unwrap_or(0);
    let word = |at: usize| u16_at(&data, ped(at)).unwrap_or(0);
    let long = |at: usize| u32_at(&data, ped(at)).unwrap_or(0);
    Ok(Driver {
        name,
        note,
        version: (
            u16_at(&data, VERSION_AT).unwrap_or(0),
            u16_at(&data, VERSION_AT.saturating_add(2)).unwrap_or(0),
        ),
        class: byte(20),
        colour_class: byte(21),
        columns: byte(22),
        char_sets: byte(23),
        rows: word(24),
        max_dots: (long(26), long(30)),
        dpi: (word(34), word(36)),
        timeout: i32_at(&data, ped(50)).unwrap_or(0),
    })
}

impl Driver {
    /// What the driver says, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const P: &str = "Printer driver";
        let class = match self.class & 3 {
            0 => "black and white, text only",
            1 => "black and white, text and graphics",
            2 => "colour, text only",
            _ => "colour, text and graphics",
        };
        let mut class = class.to_owned();
        if self.class & 4 != 0 {
            class.push_str("; a V44 driver with extended features");
        }
        let colours = match self.colour_class & !0x10 {
            0x01 => "black only",
            0x02 => "yellow, magenta and cyan",
            0x03 => "yellow, magenta and cyan, or black",
            0x04 => "yellow, magenta, cyan and black",
            0x08 | 0x0C => "blue, green, red and white",
            0x09 => "white on black",
            0x0A => "blue, green and red",
            0x0B => "blue, green and red, or black and white",
            _ => "unknown",
        };
        let mut colours = colours.to_owned();
        if self.colour_class & 0x10 != 0 {
            colours.push_str(", a pass a colour");
        }
        let dots = |n: u32| {
            if n == 0 {
                "no limit".to_owned()
            } else {
                n.to_string()
            }
        };
        let mut out = vec![Field::new(P, "Printer", self.name.clone())];
        if let Some(note) = &self.note {
            out.push(Field::new(P, "Also says", note.clone()));
        }
        out.extend([
            Field::new(
                P,
                "Version",
                format!("{}.{}", self.version.0, self.version.1),
            ),
            Field::new(P, "Prints", class),
            Field::new(P, "Colours", colours),
            Field::new(
                P,
                "Resolution",
                format!("{} × {} dots an inch", self.dpi.0, self.dpi.1),
            ),
            Field::new(
                P,
                "Largest",
                format!(
                    "{} dots across, {} down",
                    dots(self.max_dots.0),
                    dots(self.max_dots.1)
                ),
            ),
            Field::new(P, "Head", format!("{} dots a pass", self.rows)),
            Field::new(
                P,
                "Text",
                format!(
                    "{} columns, {} character sets",
                    self.columns, self.char_sets
                ),
            ),
            Field::new(P, "Gives up after", format!("{} seconds", self.timeout)),
        ]);
        out
    }
}

#[cfg(test)]
#[allow(
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

    /// A one-hunk driver for a 120 × 72 dot-matrix printer named `name`,
    /// with `version` where the version goes and its name and functions
    /// relocated — all but the name's when `relocate_name` is false.
    fn driver(name: &[u8], version: u16, relocate_name: bool) -> Vec<u8> {
        let mut h = vec![0x70, 0x00, 0x4E, 0x75];
        h.extend(version.to_be_bytes());
        h.extend(40u16.to_be_bytes());
        let ped = h.len();
        h.resize(ped + 62, 0);
        let put =
            |h: &mut Vec<u8>, at: usize, v: u32| h[at..at + 4].copy_from_slice(&v.to_be_bytes());
        for i in 1..=4 {
            put(&mut h, ped + i * 4, 0);
        }
        h[ped + 20] = 1; // black and white, graphics
        h[ped + 21] = 1;
        h[ped + 22] = 80;
        h[ped + 23] = 10;
        h[ped + 24..ped + 26].copy_from_slice(&8u16.to_be_bytes());
        put(&mut h, ped + 26, 960);
        h[ped + 34..ped + 36].copy_from_slice(&120u16.to_be_bytes());
        h[ped + 36..ped + 38].copy_from_slice(&72u16.to_be_bytes());
        put(&mut h, ped + 50, 30);
        let name_at = h.len();
        h.extend(name);
        h.push(0);
        put(&mut h, ped, name_at as u32);
        while h.len() % 4 != 0 {
            h.push(0);
        }
        let mut relocs: Vec<u32> = (1..=4).map(|i| (ped + i * 4) as u32).collect();
        if relocate_name {
            relocs.push(ped as u32);
        }
        let longs = (h.len() / 4) as u32;
        let mut exe = Vec::new();
        for v in [0x3F3u32, 0, 1, 0, 0, longs, 0x3E9, longs] {
            exe.extend(v.to_be_bytes());
        }
        exe.extend(h);
        exe.extend(0x3ECu32.to_be_bytes());
        exe.extend((relocs.len() as u32).to_be_bytes());
        exe.extend(0u32.to_be_bytes());
        for r in relocs {
            exe.extend(r.to_be_bytes());
        }
        exe.extend(0u32.to_be_bytes());
        exe.extend(0x3F2u32.to_be_bytes());
        exe
    }

    #[test]
    fn a_driver_says_its_printer_s_name_and_what_it_prints() {
        let bytes = driver(b"CBM_MPS1000", 35, true);
        assert!(is_driver(&bytes));
        let d = parse(&bytes).unwrap();
        assert_eq!((d.name.as_str(), d.note.as_deref()), ("CBM_MPS1000", None));
        assert_eq!(d.version, (35, 40));
        assert_eq!(
            (d.class, d.colour_class, d.columns, d.char_sets),
            (1, 1, 80, 10)
        );
        assert_eq!(
            (d.rows, d.max_dots, d.dpi, d.timeout),
            (8, (960, 0), (120, 72), 30)
        );
        let f = d.fields();
        let value = |l: &str| f.iter().find(|x| x.label == l).unwrap().value.clone();
        assert_eq!(value("Prints"), "black and white, text and graphics");
        assert_eq!(value("Resolution"), "120 × 72 dots an inch");
        assert_eq!(value("Largest"), "960 dots across, no limit down");
    }

    #[test]
    fn a_name_string_s_second_line_is_a_note_and_code_after_it_is_not_read() {
        let d = parse(&driver(
            b"CBM MPS2xxx\r\n\xA9 Commodore Ffm.\x0e\x0b",
            33,
            true,
        ))
        .unwrap();
        assert_eq!(d.name, "CBM MPS2xxx");
        assert_eq!(d.note.as_deref(), Some("© Commodore Ffm."));
    }

    #[test]
    fn a_library_or_an_unrelocated_name_is_not_a_driver() {
        assert!(
            !is_driver(&driver(b"x", 0x4AFC, true)),
            "a RomTag where the version goes"
        );
        assert!(!is_driver(&driver(b"x", 35, false)));
    }
}
