//! Amiga bitmap fonts (F-033): the `.font` contents file in `FONTS:`, and
//! the file for each size beside it in a folder of the font's name.
//!
//! The contents file names the sizes: an ID (`$0F00`, `$0F02` when the
//! entries carry tags, `$0F03` for an outline font), a count, and that many
//! 260-byte entries — a file name, a height, a style and flags. Each size is
//! a hunk executable whose one hunk starts `MOVEQ #-1,D0; RTS`, so running it
//! does nothing, and then holds a `DiskFontHeader` with the font's
//! `TextFont` inside it. The pointers in it are offsets into that hunk,
//! which loading turns into addresses.
//!
//! The layouts are `diskfont/diskfont.h` and `graphics/text.h`; the reading
//! follows AROS's `diskfont.library` (`workbench/libs/diskfont/
//! diskfont_io.c`), and the drawing its `graphics.library` `Text()`
//! (`rom/graphics/text.c`): a character's kerning moves the pen before its
//! glyph is drawn, and its spacing — or the font's width, when it has no
//! spacing table — moves it after. All read locally, not committed.

use ade_endian::{i16_at, u8_at, u16_at, u32_at};

use crate::decoded::Field;
use crate::hunk;

/// `FCH_ID`, `TFCH_ID`, `OFCH_ID`: the contents file of a bitmap font, one
/// whose entries carry tags, and an outline font's.
const CONTENTS_IDS: [u16; 3] = [0x0F00, 0x0F02, 0x0F03];
/// `DFH_ID`: the `DiskFontHeader` of a font size's file.
const DFH_ID: u16 = 0x0F80;
/// Bytes of one `FontContents` entry.
const ENTRY: usize = 260;
/// Where the `DiskFontHeader`'s ID sits in the hunk: after the 4-byte
/// `MOVEQ`/`RTS` and the 14-byte `Node`.
const DFH_AT: usize = 18;
/// Where its `TextFont` starts: after the ID, revision, segment and 32-byte
/// name. The `TextFont`'s own 20-byte `Message` comes first.
const TF_AT: usize = 58;

/// The tallest font ADE draws, bounding what a header can make it allocate
/// (D-006).
const MAX_HEIGHT: usize = 256;
/// The widest specimen line ADE draws.
const MAX_WIDTH: usize = 2048;

/// Text style bits, `FSF_*`.
const UNDERLINED: u8 = 1;
const BOLD: u8 = 2;
const ITALIC: u8 = 4;
const EXTENDED: u8 = 8;
const COLOUR: u8 = 0x40;
/// Font flag bits, `FPF_*`.
const TALLDOT: u8 = 0x08;
const WIDEDOT: u8 = 0x10;
const PROPORTIONAL: u8 = 0x20;
const DESIGNED: u8 = 0x40;

/// One size a contents file lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Size {
    /// The file holding it, relative to `FONTS:`: `topaz/11`.
    pub file: Vec<u8>,
    /// Its height in pixels.
    pub height: u16,
    /// `FSF_*` style bits.
    pub style: u8,
    /// `FPF_*` flags.
    pub flags: u8,
}

/// A `.font` contents file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contents {
    /// `$0F00`, `$0F02` or `$0F03`.
    pub id: u16,
    /// The sizes it lists.
    pub sizes: Vec<Size>,
}

/// One size of a font, from its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    /// `dfh_Name`, which is often empty: AmigaDOS fills it in on loading.
    pub name: Vec<u8>,
    /// `dfh_Revision`.
    pub revision: u16,
    /// `tf_YSize`: the height.
    pub height: u16,
    /// `tf_XSize`: the nominal width, and every character's on a font with
    /// no spacing table.
    pub width: u16,
    /// `tf_Baseline`: rows above the baseline, counted from the top.
    pub baseline: u16,
    /// `tf_BoldSmear`.
    pub bold_smear: u16,
    /// `FSF_*` style bits.
    pub style: u8,
    /// `FPF_*` flags.
    pub flags: u8,
    /// The first and last characters it has.
    pub first: u8,
    /// The last.
    pub last: u8,
    /// Each glyph's bit offset into a row of the bitmap, and width; one per
    /// character from `first` to `last`, then the glyph for any other.
    pub glyphs: Vec<(u16, u16)>,
    /// How far each character moves the pen, when the font says.
    pub spacing: Option<Vec<i16>>,
    /// How far each moves it before drawing, when the font says.
    pub kerning: Option<Vec<i16>>,
    /// The bitmap: `height` rows of `modulo` bytes.
    pub bitmap: Vec<u8>,
    /// `tf_Modulo`: bytes in a row of it.
    pub modulo: usize,
    /// Bytes of its tables that run past the end of the file, read as zero.
    /// 21 corpus fonts end a few bytes short of their last table — the
    /// entry for the "any other character" glyph, usually — where the Amiga
    /// read on into whatever followed.
    pub short_by: usize,
}

/// Why bytes are not a font ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAFont(pub String);

impl core::fmt::Display for NotAFont {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

fn fail(why: impl Into<String>) -> NotAFont {
    NotAFont(why.into())
}

/// Whether `name` and `bytes` look like a `.font` contents file: by name,
/// since the ID is two common bytes.
#[must_use]
pub fn is_contents(name: &[u8], bytes: &[u8]) -> bool {
    let named = name.len() > 5
        && name
            .get(name.len().saturating_sub(5)..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(b".font"));
    named && u16_at(bytes, 0).is_ok_and(|id| CONTENTS_IDS.contains(&id))
}

/// Read a `.font` contents file.
///
/// # Errors
/// [`NotAFont`] when it is cut short of the entries it counts.
pub fn contents(bytes: &[u8]) -> Result<Contents, NotAFont> {
    let id = u16_at(bytes, 0).map_err(|_| fail("the contents file is cut short"))?;
    let count = usize::from(u16_at(bytes, 2).map_err(|_| fail("the contents file is cut short"))?);
    let mut sizes = Vec::new();
    for i in 0..count {
        let at = 4usize.saturating_add(i.saturating_mul(ENTRY));
        let entry = bytes
            .get(at..at.saturating_add(ENTRY))
            .ok_or_else(|| fail(format!("it lists {count} sizes, and holds {i}")))?;
        let file = entry
            .iter()
            .take(256)
            .take_while(|&&b| b != 0)
            .copied()
            .collect();
        sizes.push(Size {
            file,
            height: u16_at(entry, 256).unwrap_or(0),
            style: u8_at(entry, 258).unwrap_or(0),
            flags: u8_at(entry, 259).unwrap_or(0),
        });
    }
    Ok(Contents { id, sizes })
}

/// The bytes of a font size's one hunk, when `bytes` is one.
fn font_hunk(bytes: &[u8]) -> Option<&[u8]> {
    let exe = hunk::parse(bytes)?;
    let first = exe.hunks.first()?;
    let data = bytes.get(first.file_offset..first.file_offset.checked_add(first.file_len)?)?;
    (u16_at(data, DFH_AT).ok()? == DFH_ID).then_some(data)
}

/// Whether `bytes` are a font size's file.
#[must_use]
pub fn is_font(bytes: &[u8]) -> bool {
    hunk::is_executable(bytes) && font_hunk(bytes).is_some()
}

/// Read one size of a font.
///
/// # Errors
/// [`NotAFont`] when it is not a font size's file, a table runs out of the
/// hunk, or the font is larger than ADE draws.
pub fn parse(bytes: &[u8]) -> Result<Font, NotAFont> {
    let data =
        font_hunk(bytes).ok_or_else(|| fail("not a font: no DiskFontHeader where one goes"))?;
    let word = |at: usize| u16_at(data, TF_AT.saturating_add(at)).unwrap_or(0);
    let byte = |at: usize| u8_at(data, TF_AT.saturating_add(at)).unwrap_or(0);
    let long = |at: usize| u32_at(data, TF_AT.saturating_add(at)).unwrap_or(0) as usize;
    let (height, style, flags) = (word(20), byte(22), byte(23));
    let (first, last) = (byte(32), byte(33));
    if last < first {
        return Err(fail(format!("it runs from character {first} to {last}")));
    }
    if usize::from(height) > MAX_HEIGHT {
        return Err(fail(format!(
            "{height} pixels tall is larger than ADE draws"
        )));
    }
    let count = usize::from(last.saturating_sub(first)).saturating_add(2);
    // A table is read as far as the file goes and the rest taken as zero —
    // but one that starts outside the file is no table.
    let mut short_by = 0usize;
    let mut table = |at: usize, each: usize, what: &str| -> Result<Vec<u8>, NotAFont> {
        let want = count.saturating_mul(each);
        let got = data
            .get(at..)
            .filter(|rest| !rest.is_empty())
            .ok_or_else(|| fail(format!("its {what} table starts outside the file")))?;
        let mut out = got.get(..want.min(got.len())).unwrap_or(&[]).to_vec();
        short_by = short_by.saturating_add(want.saturating_sub(out.len()));
        out.resize(want, 0);
        Ok(out)
    };
    let glyphs: Vec<(u16, u16)> = table(long(40), 4, "character location")?
        .chunks_exact(4)
        .map(|c| (u16_at(c, 0).unwrap_or(0), u16_at(c, 2).unwrap_or(0)))
        .collect();
    let mut signed = |at: usize, what: &str| -> Result<Option<Vec<i16>>, NotAFont> {
        if at == 0 {
            return Ok(None);
        }
        Ok(Some(
            table(at, 2, what)?
                .chunks_exact(2)
                .map(|c| i16_at(c, 0).unwrap_or(0))
                .collect(),
        ))
    };
    let spacing = signed(long(44), "spacing")?;
    let kerning = signed(long(48), "kerning")?;
    let modulo = usize::from(word(38));
    let at = long(34);
    let bitmap = data
        .get(at..at.saturating_add(modulo.saturating_mul(usize::from(height))))
        .ok_or_else(|| fail("its bitmap runs out of the file"))?
        .to_vec();
    let name = data
        .get(26..58)
        .unwrap_or(&[])
        .iter()
        .take_while(|&&b| b != 0)
        .copied()
        .collect();
    Ok(Font {
        name,
        revision: u16_at(data, 20).unwrap_or(0),
        height,
        width: word(24),
        baseline: word(26),
        bold_smear: word(28),
        style,
        flags,
        first,
        last,
        glyphs,
        spacing,
        kerning,
        bitmap,
        modulo,
        short_by,
    })
}

/// The lines of a specimen.
pub const SPECIMEN: [&[u8]; 3] = [
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    b"abcdefghijklmnopqrstuvwxyz",
    b"0123456789 .,;:!?'\"()&",
];

impl Font {
    /// The glyph index for a character, as `Text()` chooses it.
    fn index(&self, c: u8) -> usize {
        if c < self.first || c > self.last {
            self.glyphs.len().saturating_sub(1)
        } else {
            usize::from(c.saturating_sub(self.first))
        }
    }

    /// Whether one bit of the bitmap is set.
    fn ink(&self, row: usize, bit: usize) -> bool {
        self.bitmap
            .get(row.saturating_mul(self.modulo).saturating_add(bit / 8))
            .is_some_and(|b| b & (0x80 >> (bit % 8)) != 0)
    }

    /// Lines of text set in the font, as `Text()` sets them: one row of
    /// `height` per line, `true` where there is ink. Colour fonts are not
    /// drawn — their glyphs are in planes ADE does not read.
    #[must_use]
    pub fn specimen(&self, lines: &[&[u8]]) -> Option<(usize, usize, Vec<bool>)> {
        if self.style & COLOUR != 0 || self.height == 0 {
            return None;
        }
        let height = usize::from(self.height);
        // Lay each line out first, to find how wide the picture is and how
        // far left a negative kerning reaches.
        let mut placed: Vec<Vec<(i64, usize)>> = Vec::new();
        let (mut left, mut right) = (0i64, 0i64);
        for line in lines {
            let mut x = 0i64;
            let mut row = Vec::new();
            for &c in *line {
                let i = self.index(c);
                if let Some(k) = self.kerning.as_ref().and_then(|k| k.get(i)) {
                    x = x.saturating_add(i64::from(*k));
                }
                let (_, w) = self.glyphs.get(i).copied().unwrap_or((0, 0));
                left = left.min(x);
                right = right.max(x.saturating_add(i64::from(w)));
                row.push((x, i));
                x = x.saturating_add(match self.spacing.as_ref().and_then(|s| s.get(i)) {
                    Some(s) => i64::from(*s),
                    None => i64::from(self.width),
                });
                right = right.max(x);
            }
            placed.push(row);
        }
        let width = usize::try_from(right.saturating_sub(left))
            .ok()?
            .min(MAX_WIDTH);
        let total = height.saturating_mul(lines.len());
        let mut ink = vec![false; width.saturating_mul(total)];
        for (n, row) in placed.iter().enumerate() {
            for &(x, i) in row {
                let (at, w) = self.glyphs.get(i).copied().unwrap_or((0, 0));
                for gx in 0..usize::from(w) {
                    let step = i64::try_from(gx).unwrap_or(i64::MAX);
                    let Ok(px) = usize::try_from(x.saturating_sub(left).saturating_add(step))
                    else {
                        continue;
                    };
                    if px >= width {
                        continue;
                    }
                    for y in 0..height {
                        if self.ink(y, usize::from(at).saturating_add(gx)) {
                            let at = n
                                .saturating_mul(height)
                                .saturating_add(y)
                                .saturating_mul(width)
                                .saturating_add(px);
                            if let Some(p) = ink.get_mut(at) {
                                *p = true;
                            }
                        }
                    }
                }
            }
        }
        Some((width, total, ink))
    }

    /// The shape of a pixel it was drawn for, width to height: `FPF_TALLDOT`
    /// for a high-resolution non-interlaced screen's, `FPF_WIDEDOT` for a
    /// low-resolution interlaced one's.
    #[must_use]
    pub fn pixel_shape(&self) -> (u8, u8) {
        if self.flags & TALLDOT != 0 {
            (1, 2)
        } else if self.flags & WIDEDOT != 0 {
            (2, 1)
        } else {
            (1, 1)
        }
    }

    /// What the font says, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const F: &str = "Font";
        let mut out = Vec::new();
        if !self.name.is_empty() {
            out.push(Field::new(F, "Name", latin1(&self.name)));
        }
        out.push(Field::new(F, "Height", format!("{} pixels", self.height)));
        out.push(Field::new(
            F,
            "Width",
            if self.flags & PROPORTIONAL != 0 {
                format!("proportional, nominally {}", self.width)
            } else {
                format!("{} pixels, every character", self.width)
            },
        ));
        out.push(Field::new(
            F,
            "Baseline",
            format!("{} rows from the top", self.baseline),
        ));
        out.push(Field::new(
            F,
            "Characters",
            format!(
                "{} to {} ({} glyphs, and one for any other)",
                char_name(self.first),
                char_name(self.last),
                usize::from(self.last.saturating_sub(self.first)).saturating_add(1)
            ),
        ));
        out.push(Field::new(F, "Style", style_words(self.style)));
        out.push(Field::new(
            F,
            "Kerning",
            if self.kerning.is_some() {
                "a table, one value a character"
            } else {
                "none"
            },
        ));
        out.push(Field::new(
            F,
            "Drawn for",
            match self.pixel_shape() {
                (1, 2) => "a high-resolution screen, not interlaced (pixels twice as tall as wide)",
                (2, 1) => "a low-resolution interlaced screen (pixels twice as wide as tall)",
                _ => "any screen",
            },
        ));
        if self.flags & DESIGNED != 0 {
            out.push(Field::new(
                F,
                "Designed",
                "at this size, not scaled from another",
            ));
        }
        if self.revision != 0 {
            out.push(Field::new(F, "Revision", self.revision.to_string()));
        }
        if self.short_by > 0 {
            out.push(Field::new(
                F,
                "Cut short",
                format!(
                    "its tables run {} bytes past the end of the file; ADE takes them as zero",
                    self.short_by
                ),
            ));
        }
        out
    }
}

impl Contents {
    /// What the contents file says, in fields.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const F: &str = "Font";
        let mut out = vec![Field::new(
            F,
            "Kind",
            match self.id {
                0x0F00 => "bitmap font",
                0x0F02 => "bitmap font, its entries carrying tags",
                _ => "outline font, drawn at any size from its .otag file",
            },
        )];
        if self.sizes.is_empty() && self.id == 0x0F03 {
            out.push(Field::new(F, "Sizes", "none kept as bitmaps"));
        }
        for s in &self.sizes {
            let style = style_words(s.style);
            out.push(Field::new(
                "Sizes",
                format!("{} pixels", s.height),
                if style == "plain" {
                    latin1(&s.file)
                } else {
                    format!("{}, {style}", latin1(&s.file))
                },
            ));
        }
        out
    }
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn char_name(c: u8) -> String {
    match c {
        b' ' => "space".to_owned(),
        0x21..=0x7E | 0xA1..=0xFF => format!("{:?} ({c})", char::from(c)),
        _ => c.to_string(),
    }
}

fn style_words(style: u8) -> String {
    let words: Vec<&str> = [
        (UNDERLINED, "underlined"),
        (BOLD, "bold"),
        (ITALIC, "italic"),
        (EXTENDED, "extended"),
        (COLOUR, "colour"),
    ]
    .iter()
    .filter(|(bit, _)| style & bit != 0)
    .map(|(_, w)| *w)
    .collect();
    if words.is_empty() {
        "plain".to_owned()
    } else {
        words.join(", ")
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

    /// A font of 'A' and 'B', 2 rows tall, as a one-hunk executable. Its
    /// bitmap row is 16 bits: A at bit 0, 3 wide, all ink; B at bit 3, 2
    /// wide, ink in its first column; the "other" glyph at bit 5, 1 wide.
    fn font(spacing: Option<[i16; 3]>, kerning: Option<[i16; 3]>, cut: usize) -> Vec<u8> {
        let mut h = vec![0x70, 0xFF, 0x4E, 0x75];
        h.extend([0u8; 14]); // Node
        h.extend(0x0F80u16.to_be_bytes());
        h.extend(1u16.to_be_bytes()); // revision
        h.extend([0u8; 4]); // segment
        let mut name = [0u8; 32];
        name[..4].copy_from_slice(b"test");
        h.extend(name);
        h.extend([0u8; 20]); // the TextFont's Message
        h.extend(2u16.to_be_bytes()); // YSize
        h.push(0); // style
        h.push(if spacing.is_some() { 0x20 } else { 0 }); // flags
        h.extend(4u16.to_be_bytes()); // XSize
        h.extend(1u16.to_be_bytes()); // baseline
        h.extend(1u16.to_be_bytes()); // bold smear
        h.extend(0u16.to_be_bytes()); // accessors
        h.push(b'A');
        h.push(b'B');
        let at = h.len();
        h.extend([0u8; 4 + 2 + 4 + 4 + 4]); // pointers, modulo
        let put = |h: &mut Vec<u8>, off: usize, v: u32| {
            h[at + off..at + off + 4].copy_from_slice(&v.to_be_bytes());
        };
        let bitmap = h.len() as u32;
        h.extend([0b1111_0100, 0, 0b1111_0000, 0]);
        put(&mut h, 0, bitmap);
        h[at + 4..at + 6].copy_from_slice(&2u16.to_be_bytes());
        let loc = h.len() as u32;
        for (pos, width) in [(0u16, 3u16), (3, 2), (5, 1)] {
            h.extend(pos.to_be_bytes());
            h.extend(width.to_be_bytes());
        }
        put(&mut h, 6, loc);
        for (off, table) in [(10, spacing), (14, kerning)] {
            if let Some(t) = table {
                let p = h.len() as u32;
                for v in t {
                    h.extend(v.to_be_bytes());
                }
                put(&mut h, off, p);
            }
        }
        h.truncate(h.len() - cut);
        while h.len() % 4 != 0 {
            h.push(0);
        }
        let mut exe = Vec::new();
        for v in [
            0x3F3u32,
            0,
            1,
            0,
            0,
            (h.len() / 4) as u32,
            0x3E9,
            (h.len() / 4) as u32,
        ] {
            exe.extend(v.to_be_bytes());
        }
        exe.extend(h);
        exe.extend(0x3F2u32.to_be_bytes());
        exe
    }

    #[test]
    fn a_font_reads_its_header_and_tables() {
        let f = parse(&font(None, None, 0)).unwrap();
        assert_eq!((f.height, f.width, f.first, f.last), (2, 4, b'A', b'B'));
        assert_eq!(f.name, b"test");
        assert_eq!(f.glyphs, vec![(0, 3), (3, 2), (5, 1)]);
        assert_eq!((f.spacing, f.kerning, f.short_by), (None, None, 0));
        assert!(is_font(&font(None, None, 0)));
    }

    #[test]
    fn a_fixed_font_sets_each_character_its_width_apart() {
        let f = parse(&font(None, None, 0)).unwrap();
        let (w, h, ink) = f.specimen(&[b"AB?"]).unwrap();
        assert_eq!((w, h), (12, 2));
        let row: Vec<bool> = ink[..12].to_vec();
        // A: 3 ink; B at 4: its first column; '?' is not in the font, so
        // the "other" glyph, at 8: one column.
        let want = [
            true, true, true, false, true, false, false, false, true, false, false, false,
        ];
        assert_eq!(row, want);
    }

    #[test]
    fn kerning_moves_the_pen_before_a_glyph_and_spacing_after() {
        let f = parse(&font(Some([5, 3, 2]), Some([0, -1, 0]), 0)).unwrap();
        let (w, _, ink) = f.specimen(&[b"AB"]).unwrap();
        // A at 0, advance 5; B kerned to 4, advance 3 to 7.
        assert_eq!(w, 7);
        let row: Vec<usize> = (0..w).filter(|&x| ink[x]).collect();
        assert_eq!(row, vec![0, 1, 2, 4]);
    }

    #[test]
    fn a_table_cut_short_is_read_as_far_as_it_goes() {
        // The last two bytes of the kerning table gone: the "other" glyph's.
        let f = parse(&font(Some([5, 3, 2]), Some([0, -1, 7]), 2)).unwrap();
        assert_eq!(f.short_by, 2);
        assert_eq!(f.kerning, Some(vec![0, -1, 0]));
        assert!(f.fields().iter().any(|x| x.label == "Cut short"));
    }

    #[test]
    fn a_contents_file_lists_its_sizes() {
        let mut b = vec![0x0F, 0x00, 0, 2];
        for (name, h) in [(&b"ruby/8"[..], 8u16), (b"ruby/12", 12)] {
            let mut e = [0u8; ENTRY];
            e[..name.len()].copy_from_slice(name);
            e[256..258].copy_from_slice(&h.to_be_bytes());
            e[258] = BOLD;
            b.extend(e);
        }
        assert!(is_contents(b"ruby.font", &b));
        assert!(!is_contents(b"ruby", &b), "by name too");
        let c = contents(&b).unwrap();
        assert_eq!(c.sizes.len(), 2);
        assert_eq!(
            (c.sizes[1].file.as_slice(), c.sizes[1].height),
            (&b"ruby/12"[..], 12)
        );
        assert_eq!(c.fields()[2].value, "ruby/12, bold");
        b.truncate(200);
        assert!(contents(&b).unwrap_err().0.contains("holds 0"));
    }
}
