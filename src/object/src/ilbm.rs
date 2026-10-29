//! IFF ILBM pictures (F-033): the Amiga's own picture format, from Deluxe
//! Paint onwards.
//!
//! An IFF file is a `FORM` of chunks, each an ID, a length and the bytes,
//! padded to an even length. A picture's are `BMHD` (its size, depth and
//! compression), `CMAP` (its colours), `CAMG` (the display mode it was drawn
//! for) and `BODY` (the bitplanes, a row at a time, every plane of a row in
//! turn, optionally with a mask plane after them and optionally compressed
//! with ByteRun1).
//!
//! The reading follows AROS's `ilbm.datatype`
//! (`workbench/classes/datatypes/ilbm/ilbmclass.c`) and
//! `datatypes/pictureclass.h`, read locally and not committed: ByteRun1 is
//! unpacked a row at a time, the mask plane skipped; extra-half-brite
//! (`CAMG` bit 7) takes its second 32 colours from the first 32 at half
//! brightness; hold-and-modify (`CAMG` bit 11) treats the top two planes as
//! a control and the rest as a value; depth 24 is true colour; a 4-bit
//! colour map stored in the high nibbles is widened. Where AROS and the
//! hardware differ, ADE takes the hardware: a hold-and-modify line starts
//! from colour 0, the background, where AROS starts from black.
//!
//! An ACBM — "Amiga contiguous bitmap", AmigaBASIC's and some games'
//! form — is the same picture with its planes stored whole, one after
//! another, uncompressed, in an `ABIT` chunk where an ILBM has `BODY`. AROS
//! does not read it; ffmpeg does, and the corpus test compares every pixel
//! with ffmpeg's. ADE lays an `ABIT` out as rows and draws it as an ILBM.
//!
//! SPEC §IFF ILBM has the table.

use ade_endian::{i16_at, u8_at, u16_at, u32_at};

use crate::decoded::Field;

/// The widest or tallest picture ADE will draw, bounding what a header can
/// make it allocate (D-006). Larger than any Amiga display mode.
const MAX_SIDE: usize = 4096;

/// `CAMG` bits, from `graphics/modeid.h` and `graphics/view.h`.
const LACE: u32 = 1 << 2;
const SUPERHIRES: u32 = 1 << 5;
const EXTRA_HALFBRITE: u32 = 1 << 7;
const HAM: u32 = 1 << 11;
const HIRES: u32 = 1 << 15;

/// `bmh_Masking`: a mask plane follows each row's bitplanes.
pub(crate) const MASK_PLANE: u8 = 1;

/// A decoded picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ilbm {
    /// Pixels across and down.
    pub width: usize,
    /// Pixels down.
    pub height: usize,
    /// Bitplanes.
    pub depth: u8,
    /// `bmh_Masking`: 0 none, 1 a mask plane, 2 a transparent colour, 3
    /// lasso.
    pub masking: u8,
    /// `bmh_Compression`: 0 none, 1 ByteRun1.
    pub compression: u8,
    /// `bmh_Transparent`: the colour that is see-through, under masking 2.
    pub transparent: u16,
    /// `bmh_XAspect`, `bmh_YAspect`: the shape of a pixel.
    pub aspect: (u8, u8),
    /// `bmh_PageWidth`, `bmh_PageHeight`: the screen it was drawn on.
    pub page: (i16, i16),
    /// The colour map, as `0xRRGGBB`.
    pub colours: Vec<u32>,
    /// The `CAMG` display mode, when the file has one.
    pub mode: Option<u32>,
    /// One `0xFFRRGGBB` per pixel, opaque: the picture as it looked on its
    /// own screen. Empty for a colour map with no picture.
    ///
    /// A transparent colour (masking 2) is drawn as its colour, not as a
    /// hole: Deluxe Paint marks colour 0 transparent on 2,590 of the
    /// corpus's 4,670 pictures, full-screen ones included, for when a
    /// picture is pasted as a brush.
    pub argb: Vec<u32>,
    /// The other chunks, by ID, in order: what ADE did not draw.
    pub other_chunks: Vec<String>,
    /// Bytes of `BODY` left after the last row, which a sound encoder never
    /// leaves.
    pub body_left: usize,
    /// Whether it is an ACBM: its planes whole, in `ABIT`.
    pub contiguous: bool,
    /// Bytes an ACBM's `ABIT` is short of its planes, drawn as zero.
    pub missing: usize,
}

/// Why bytes are not a picture ADE can draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAPicture(pub String);

impl core::fmt::Display for NotAPicture {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `bytes` start like an IFF ILBM, or an ACBM.
#[must_use]
pub fn is_ilbm(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"FORM") && matches!(bytes.get(8..12), Some(b"ILBM" | b"ACBM"))
}

fn fail(why: impl Into<String>) -> NotAPicture {
    NotAPicture(why.into())
}

/// A chunk: its ID and its bytes.
pub(crate) type Chunk<'a> = ([u8; 4], &'a [u8]);

/// The chunks of a `FORM`.
pub(crate) fn chunks(bytes: &[u8]) -> Result<Vec<Chunk<'_>>, NotAPicture> {
    let declared = u32_at(bytes, 4).map_err(|_| fail("the FORM header is cut short"))? as usize;
    // A FORM may claim more than the file holds; read what is there. It may
    // also claim less: the chunks are found within the length it states,
    // but each is read to its own length, as far as the file goes, as ffmpeg
    // reads it — *Thromulus*'s screens and *Italy '90 Soccer*'s pictures
    // state a FORM 28 to 116 bytes short of the chunks inside it (BUG-019).
    let end = declared.saturating_add(8).min(bytes.len());
    let mut at = 12usize;
    let mut out = Vec::new();
    while at.saturating_add(8) <= end {
        let mut id = [0u8; 4];
        id.copy_from_slice(bytes.get(at..at.saturating_add(4)).unwrap_or(&[0; 4]));
        let len = u32_at(bytes, at.saturating_add(4)).unwrap_or(0) as usize;
        let start = at.saturating_add(8);
        let stop = start.saturating_add(len);
        let Some(data) = bytes.get(start..stop.min(bytes.len())) else {
            break;
        };
        out.push((id, data));
        // Chunks are padded to an even length.
        at = stop.saturating_add(len & 1);
    }
    Ok(out)
}

/// Unpack one row's worth of ByteRun1 into `out`, returning how many bytes
/// of `src` it used. A run past the row is cut at the row, as AROS cuts it.
fn byterun1(src: &[u8], out: &mut [u8]) -> usize {
    let (mut i, mut o) = (0usize, 0usize);
    while o < out.len() {
        let Some(&n) = src.get(i) else { break };
        i = i.saturating_add(1);
        let n = i8::from_ne_bytes([n]);
        if n >= 0 {
            for _ in 0..=n {
                let (Some(&b), Some(slot)) = (src.get(i), out.get_mut(o)) else {
                    return i;
                };
                *slot = b;
                i = i.saturating_add(1);
                o = o.saturating_add(1);
            }
        } else if n != -128 {
            let Some(&b) = src.get(i) else { break };
            i = i.saturating_add(1);
            for _ in 0..=n.unsigned_abs() {
                let Some(slot) = out.get_mut(o) else { break };
                *slot = b;
                o = o.saturating_add(1);
            }
        }
    }
    i
}

/// Read a picture.
///
/// # Errors
/// [`NotAPicture`] when a required chunk is missing, the header asks for
/// something ADE cannot draw, or the body is shorter than the picture.
pub fn parse(bytes: &[u8]) -> Result<Ilbm, NotAPicture> {
    if !is_ilbm(bytes) {
        return Err(fail("an ILBM starts with FORM, a length, and ILBM"));
    }
    let contiguous = bytes.get(8..12) == Some(b"ACBM");
    let chunks = chunks(bytes)?;
    let find = |id: &[u8; 4]| chunks.iter().find(|(c, _)| c == id).map(|(_, d)| *d);
    let bmhd = find(b"BMHD").ok_or_else(|| fail("it has no BMHD, so its size is unknown"))?;
    if bmhd.len() < 20 {
        return Err(fail("its BMHD is cut short"));
    }
    let width = usize::from(u16_at(bmhd, 0).unwrap_or(0));
    let height = usize::from(u16_at(bmhd, 2).unwrap_or(0));
    let depth = u8_at(bmhd, 8).unwrap_or(0);
    let masking = u8_at(bmhd, 9).unwrap_or(0);
    let compression = u8_at(bmhd, 10).unwrap_or(0);
    let transparent = u16_at(bmhd, 12).unwrap_or(0);
    let aspect = (u8_at(bmhd, 14).unwrap_or(0), u8_at(bmhd, 15).unwrap_or(0));
    let page = (i16_at(bmhd, 16).unwrap_or(0), i16_at(bmhd, 18).unwrap_or(0));

    let cmap = find(b"CMAP");
    // A colour map with no picture: Deluxe Paint's `.col` files and the
    // `palette.ilbm` of preferences, a BMHD of nothing and a CMAP.
    let mode = find(b"CAMG").and_then(|c| u32_at(c, 0).ok());
    let mut picture = Ilbm {
        width: 0,
        height: 0,
        depth,
        masking,
        compression,
        transparent,
        aspect,
        page,
        colours: colour_map(cmap.unwrap_or_default()),
        mode,
        argb: Vec::new(),
        other_chunks: other(&chunks),
        body_left: 0,
        contiguous,
        missing: 0,
    };
    let pixels = if contiguous { b"ABIT" } else { b"BODY" };
    if cmap.is_some() && (find(pixels).is_none() || width == 0 || height == 0 || depth == 0) {
        return Ok(picture);
    }
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(fail(format!(
            "{width} × {height} pixels is not a picture ADE draws"
        )));
    }
    if compression > 1 && !contiguous {
        return Err(fail(format!(
            "compression {compression}; ADE reads none (0) and ByteRun1 (1)"
        )));
    }
    if !(1..=8).contains(&depth) && depth != 24 {
        return Err(fail(format!(
            "{depth} bitplanes; ADE draws 1 to 8, and 24 for true colour"
        )));
    }

    let colours = &picture.colours;
    if depth != 24 && colours.is_empty() {
        return Err(fail("it has no CMAP, so its colours are unknown"));
    }
    let (ham, ehb) = picture.modes();

    let rows;
    let body = if contiguous {
        // An ACBM's planes are never compressed, whatever its header says:
        // two corpus files claim ByteRun1 over an ABIT exactly the size of
        // its planes uncompressed, and ffmpeg reads them so too.
        let abit = find(b"ABIT").ok_or_else(|| fail("it has no ABIT, so no pixels"))?;
        let (laid_out, missing) = interleave(abit, (width, height, depth), masking == MASK_PLANE)?;
        rows = laid_out;
        picture.missing = missing;
        rows.as_slice()
    } else {
        find(b"BODY").ok_or_else(|| fail("it has no BODY, so no pixels"))?
    };
    let (argb, used) = draw(
        body,
        (width, height, depth),
        masking == MASK_PLANE,
        compression == 1 && !contiguous,
        (ham, ehb),
        colours,
    )?;
    picture.width = width;
    picture.height = height;
    picture.argb = argb;
    picture.body_left = body.len().saturating_sub(used);
    Ok(picture)
}

/// An ACBM's `ABIT` — each plane whole, then the next — as the rows an ILBM
/// `BODY` holds, every plane of a row in turn, with how many bytes were
/// missing and drawn as zero.
///
/// A short `ABIT` is drawn when what is missing lies within the last plane
/// and is at most half the picture: *Thromulus* stores its screens 116 bytes
/// short, and ffmpeg draws them so too. Anything shorter is refused before
/// it is allocated (D-006).
fn interleave(
    abit: &[u8],
    (width, height, depth): (usize, usize, u8),
    mask_plane: bool,
) -> Result<(Vec<u8>, usize), NotAPicture> {
    let row_bytes = width.div_ceil(16).saturating_mul(2);
    let planes = usize::from(depth).saturating_add(usize::from(mask_plane));
    let plane_bytes = row_bytes.saturating_mul(height);
    let total = plane_bytes.saturating_mul(planes);
    let missing = total.saturating_sub(abit.len());
    if missing > plane_bytes || missing > total / 2 {
        return Err(fail(format!(
            "its ABIT of {} bytes cannot hold {width} × {height} in {planes} planes",
            abit.len()
        )));
    }
    let mut rows = Vec::with_capacity(total);
    for y in 0..height {
        for p in 0..planes {
            let at = p
                .saturating_mul(plane_bytes)
                .saturating_add(y.saturating_mul(row_bytes));
            let have = abit.get(at.min(abit.len())..at.saturating_add(row_bytes).min(abit.len()));
            let have = have.unwrap_or(&[]);
            rows.extend_from_slice(have);
            rows.resize(
                rows.len()
                    .saturating_add(row_bytes.saturating_sub(have.len())),
                0,
            );
        }
    }
    Ok((rows, missing))
}

/// The pixels a `BODY` holds, and how many of its bytes they used.
fn draw(
    body: &[u8],
    dims: (usize, usize, u8),
    mask_plane: bool,
    byterun: bool,
    modes: (bool, bool),
    colours: &[u32],
) -> Result<(Vec<u32>, usize), NotAPicture> {
    let (rows, used) = unpack(body, dims, mask_plane, byterun)?;
    Ok((render(&rows, dims, mask_plane, modes, colours), used))
}

/// A `BODY` unpacked into its rows — each row every plane in turn, the mask
/// plane last where there is one — and how many of its bytes they used.
pub(crate) fn unpack(
    body: &[u8],
    (width, height, depth): (usize, usize, u8),
    mask_plane: bool,
    byterun: bool,
) -> Result<(Vec<u8>, usize), NotAPicture> {
    let row_bytes = width.div_ceil(16).saturating_mul(2);
    let planes = usize::from(depth).saturating_add(usize::from(mask_plane));
    let row_len = row_bytes.saturating_mul(planes);
    // Refused before anything is allocated if the body could not hold the
    // picture even at ByteRun1's best, 128 bytes from 2 (D-006): otherwise a
    // header claiming 4096 × 4096 reserves 64 MB on the strength of a few
    // bytes.
    let most = if byterun {
        body.len().saturating_mul(64)
    } else {
        body.len()
    };
    if most < row_len.saturating_mul(height) {
        return Err(fail(format!(
            "its BODY of {} bytes cannot hold {width} × {height} in {planes} planes",
            body.len()
        )));
    }
    let mut rows = vec![0u8; row_len.saturating_mul(height)];
    let mut at = 0usize;
    for (y, row) in rows.chunks_exact_mut(row_len.max(1)).enumerate() {
        if byterun {
            let used = byterun1(body.get(at..).unwrap_or(&[]), row);
            at = at.saturating_add(used);
            if at > body.len() || (used == 0 && row_len > 0) {
                return Err(fail(format!("its BODY runs out at row {y} of {height}")));
            }
        } else {
            let src = body
                .get(at..at.saturating_add(row_len))
                .ok_or_else(|| fail(format!("its BODY runs out at row {y} of {height}")))?;
            row.copy_from_slice(src);
            at = at.saturating_add(row_len);
        }
    }
    Ok((rows, at))
}

/// Rows of planes as `0xFFRRGGBB` pixels.
pub(crate) fn render(
    rows: &[u8],
    (width, height, depth): (usize, usize, u8),
    mask_plane: bool,
    (ham, ehb): (bool, bool),
    colours: &[u32],
) -> Vec<u32> {
    let row_bytes = width.div_ceil(16).saturating_mul(2);
    let planes = usize::from(depth).saturating_add(usize::from(mask_plane));
    let row_len = row_bytes.saturating_mul(planes);
    let mut argb = Vec::with_capacity(width.saturating_mul(height));
    for row in rows.chunks_exact(row_len.max(1)).take(height) {
        let mut held = colours.first().copied().unwrap_or(0);
        for x in 0..width {
            let mut value = 0u32;
            for p in 0..usize::from(depth) {
                let byte = row
                    .get(p.saturating_mul(row_bytes).saturating_add(x / 8))
                    .copied()
                    .unwrap_or(0);
                if byte & (0x80 >> (x % 8)) != 0 {
                    value |= 1 << p;
                }
            }
            let rgb = colour_of(value, &mut held, depth, (ham, ehb), colours);
            argb.push(0xFF00_0000 | rgb);
        }
    }
    argb
}

/// A `CMAP` as `0xRRGGBB` colours. A 4-bit palette written into the high
/// nibbles — every low nibble zero — is widened, `$F0` to `$FF`, as AROS
/// widens it.
pub(crate) fn colour_map(cmap: &[u8]) -> Vec<u32> {
    let mut colours: Vec<u32> = cmap
        .chunks_exact(3)
        .map(|rgb| match rgb {
            [r, g, b] => u32::from(*r) << 16 | u32::from(*g) << 8 | u32::from(*b),
            _ => 0,
        })
        .collect();
    if colours.iter().all(|&c| c & 0x0F_0F0F == 0) {
        for c in &mut colours {
            *c |= *c >> 4 & 0x0F_0F0F;
        }
    }
    colours
}

/// The chunks ADE does not draw from, by ID, in order.
fn other(chunks: &[Chunk<'_>]) -> Vec<String> {
    chunks
        .iter()
        .map(|(id, _)| id)
        .filter(|id| !matches!(*id, b"BMHD" | b"CMAP" | b"CAMG" | b"BODY" | b"ABIT"))
        .map(|id| id.iter().map(|&b| char::from(b)).collect())
        .collect()
}

/// The colour of a pixel whose planes make `value`, as `0xRRGGBB`. `held`
/// is the colour before it on the line, which hold-and-modify changes.
fn colour_of(
    value: u32,
    held: &mut u32,
    depth: u8,
    (ham, ehb): (bool, bool),
    colours: &[u32],
) -> u32 {
    let map = |i: u32| {
        colours
            .get(usize::try_from(i).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(0)
    };
    if depth == 24 {
        // Planes 0–7 red, 8–15 green, 16–23 blue, low bit first.
        (value & 0xFF) << 16 | (value >> 8 & 0xFF) << 8 | value >> 16 & 0xFF
    } else if ham {
        *held = hold_and_modify(*held, value, depth, colours);
        *held
    } else if ehb && value >= 32 {
        map(value.saturating_sub(32)) >> 1 & 0x7F_7F7F
    } else {
        map(value)
    }
}

/// One hold-and-modify pixel: the top two planes say what the rest are —
/// a colour from the map, or a new blue, red or green kept with the other
/// two from the pixel before.
fn hold_and_modify(held: u32, value: u32, depth: u8, colours: &[u32]) -> u32 {
    let bits = u32::from(depth).saturating_sub(2);
    let control = value >> bits & 3;
    let data = value & (1u32 << bits).saturating_sub(1);
    // The value becomes the component's top bits; HAM6's 4 bits are widened
    // to 8 by repeating them, HAM8's 6 keep the old low 2.
    let component = |old: u32| -> u32 {
        if bits == 4 {
            data << 4 | data
        } else {
            data << 2 | old & 3
        }
    };
    match control {
        0 => colours
            .get(usize::try_from(data).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(0),
        1 => held & 0xFF_FF00 | component(held & 0xFF),
        2 => held & 0x00_FFFF | component(held >> 16 & 0xFF) << 16,
        _ => held & 0xFF_00FF | component(held >> 8 & 0xFF) << 8,
    }
}

impl Ilbm {
    /// The display mode in words, from `CAMG`.
    #[must_use]
    pub fn mode_words(&self) -> String {
        let Some(m) = self.mode else {
            return "not recorded (no CAMG)".to_owned();
        };
        let mut words = vec![if m & SUPERHIRES != 0 {
            "super-high resolution"
        } else if m & HIRES != 0 {
            "high resolution"
        } else {
            "low resolution"
        }];
        if m & LACE != 0 {
            words.push("interlaced");
        }
        if m & HAM != 0 {
            words.push("hold-and-modify");
        }
        if m & EXTRA_HALFBRITE != 0 {
            words.push("extra-half-brite");
        }
        format!("{} (${m:08x})", words.join(", "))
    }

    /// Whether it is hold-and-modify, and whether extra-half-brite: what
    /// its `CAMG` mode says, as far as its depth allows.
    #[must_use]
    pub fn modes(&self) -> (bool, bool) {
        let ham = self.mode.is_some_and(|m| m & HAM != 0) && (self.depth == 6 || self.depth == 8);
        let ehb = self.mode.is_some_and(|m| m & EXTRA_HALFBRITE != 0) && !ham;
        (ham, ehb)
    }

    /// How a pixel is shaped, width to height, as the picture says: from
    /// its aspect, or failing that its display mode.
    #[must_use]
    pub fn pixel_shape(&self) -> (u8, u8) {
        let (xa, ya) = self.aspect;
        let ratio = if xa > 0 && ya > 0 {
            f64::from(ya) / f64::from(xa)
        } else {
            match self.mode {
                Some(m) if m & (HIRES | SUPERHIRES) != 0 && m & LACE == 0 => 2.0,
                Some(m) if m & (HIRES | SUPERHIRES) == 0 && m & LACE != 0 => 0.5,
                _ => 1.0,
            }
        };
        if ratio >= 1.5 {
            (1, 2)
        } else if ratio <= 1.0 / 1.5 {
            (2, 1)
        } else {
            (1, 1)
        }
    }

    /// What the picture says, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const P: &str = "Picture";
        let colours = match (self.depth, self.mode) {
            (24, _) => "true colour (24 bitplanes)".to_owned(),
            (d, Some(m)) if m & HAM != 0 && (d == 6 || d == 8) => format!(
                "hold-and-modify: {} colours from a map of {}",
                if d == 6 { "4,096" } else { "262,144" },
                self.colours.len()
            ),
            (d, Some(m)) if m & EXTRA_HALFBRITE != 0 => format!(
                "extra-half-brite: {} colours, the last 32 the first at half brightness",
                1u32 << d.min(8)
            ),
            (d, _) => format!(
                "{} colours, from a map of {}",
                1u32 << d.min(8),
                self.colours.len()
            ),
        };
        if self.argb.is_empty() {
            let mut out = vec![Field::new(
                P,
                "Holds",
                format!("a colour map of {} and no picture", self.colours.len()),
            )];
            out.extend(self.colour_fields());
            return out;
        }
        let mut out = vec![
            Field::new(P, "Size", format!("{} × {}", self.width, self.height)),
            Field::new(P, "Bitplanes", format!("{}: {colours}", self.depth)),
            Field::new(P, "Display mode", self.mode_words()),
            Field::new(
                P,
                "Compression",
                match (self.contiguous, self.compression) {
                    (true, 0) => "none: an ACBM, its planes stored whole",
                    (true, _) => "none: an ACBM, its planes stored whole, whatever its header says",
                    (false, 1) => "ByteRun1",
                    (false, _) => "none",
                },
            ),
            Field::new(
                P,
                "Masking",
                match self.masking {
                    0 => "none".to_owned(),
                    1 => "a mask plane, which ADE does not draw".to_owned(),
                    2 => format!(
                        "colour {} is see-through when pasted as a brush; drawn here as a colour",
                        self.transparent
                    ),
                    3 => "lasso".to_owned(),
                    m => format!("unknown ({m})"),
                },
            ),
        ];
        if self.aspect != (0, 0) {
            out.push(Field::new(
                P,
                "Pixel aspect",
                format!("{} : {}", self.aspect.0, self.aspect.1),
            ));
        }
        if self.page != (0, 0) {
            out.push(Field::new(
                P,
                "Drawn on a screen of",
                format!("{} × {}", self.page.0, self.page.1),
            ));
        }
        if !self.other_chunks.is_empty() {
            out.push(Field::new(P, "Other chunks", self.other_chunks.join(" ")));
        }
        if self.missing > 0 {
            out.push(Field::new(
                P,
                "Missing",
                format!(
                    "its ABIT is {} bytes short of its planes; the end of the last is drawn as 0",
                    self.missing
                ),
            ));
        }
        out.extend(self.colour_fields());
        out
    }

    fn colour_fields(&self) -> impl Iterator<Item = Field> + '_ {
        self.colours
            .iter()
            .enumerate()
            .take(256)
            .map(|(i, &c)| Field {
                group: "Colour map",
                label: i.to_string(),
                value: format!("${c:06x}"),
                colour: Some(c),
            })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::disallowed_methods,
    reason = "tests over bytes they build, by hand rather than with the helpers that read \
              them, so the two cannot agree about a field both have wrong (C-001's exception)"
)]
mod tests {
    use super::*;

    fn chunk(id: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut c = id.to_vec();
        c.extend((data.len() as u32).to_be_bytes());
        c.extend(data);
        if data.len() % 2 == 1 {
            c.push(0);
        }
        c
    }

    /// A FORM ILBM of `w` × `h`, depth `d`, from its parts.
    fn ilbm(
        w: u16,
        h: u16,
        d: u8,
        compression: u8,
        cmap: &[u8],
        camg: Option<u32>,
        body: &[u8],
    ) -> Vec<u8> {
        let mut bmhd = vec![0u8; 20];
        bmhd[0..2].copy_from_slice(&w.to_be_bytes());
        bmhd[2..4].copy_from_slice(&h.to_be_bytes());
        bmhd[8] = d;
        bmhd[10] = compression;
        let mut inner = b"ILBM".to_vec();
        inner.extend(chunk(*b"BMHD", &bmhd));
        inner.extend(chunk(*b"CMAP", cmap));
        if let Some(m) = camg {
            inner.extend(chunk(*b"CAMG", &m.to_be_bytes()));
        }
        inner.extend(chunk(*b"BODY", body));
        let mut out = b"FORM".to_vec();
        out.extend((inner.len() as u32).to_be_bytes());
        out.extend(inner);
        out
    }

    const GREY: [u8; 12] = [
        0, 0, 0, 0x55, 0x55, 0x55, 0xAA, 0xAA, 0xAA, 0xFF, 0xFF, 0xFF,
    ];

    /// The same picture as an ACBM: `abit` in place of the body, its planes
    /// stored whole.
    fn acbm(w: u16, h: u16, d: u8, compression: u8, abit: &[u8]) -> Vec<u8> {
        let mut out = ilbm(w, h, d, compression, &GREY, None, abit);
        out[8..12].copy_from_slice(b"ACBM");
        let at = out.windows(4).rposition(|c| c == b"BODY").unwrap();
        out[at..at + 4].copy_from_slice(b"ABIT");
        out
    }

    #[test]
    fn an_acbm_draws_its_whole_planes_as_an_ilbm_draws_its_rows() {
        // 16 × 2, 2 planes. Whole planes: plane 0 is rows $8000 then $2000,
        // plane 1 is $4000 then $1000. As rows: row 0's two planes, then row
        // 1's — a different order of the same bytes.
        let abit = [0x80, 0, 0x20, 0, 0x40, 0, 0x10, 0];
        let rows = [0x80, 0, 0x40, 0, 0x20, 0, 0x10, 0];
        let a = parse(&acbm(16, 2, 2, 0, &abit)).unwrap();
        let i = parse(&ilbm(16, 2, 2, 0, &GREY, None, &rows)).unwrap();
        assert!(a.contiguous && !i.contiguous);
        assert_eq!(a.argb, i.argb);
        assert_eq!(a.argb[0], 0xFF55_5555, "row 0, pixel 0: pen 1");
        assert_eq!(a.argb[1], 0xFFAA_AAAA, "row 0, pixel 1: pen 2");
        assert_eq!(a.argb[16 + 2], 0xFF55_5555, "row 1, pixel 2: pen 1");
        assert_eq!(a.argb[16 + 3], 0xFFAA_AAAA, "row 1, pixel 3: pen 2");
        assert_eq!(a.missing, 0);
        // Its header's compression is not believed: ACBM never compresses.
        assert_eq!(parse(&acbm(16, 2, 2, 1, &abit)).unwrap().argb, a.argb);
    }

    #[test]
    fn a_short_abit_is_drawn_when_only_the_last_plane_is_short() {
        let abit = [0x80, 0, 0x20, 0, 0x40, 0];
        let p = parse(&acbm(16, 2, 2, 0, &abit)).unwrap();
        assert_eq!(p.missing, 2);
        assert_eq!(p.argb[0], 0xFF55_5555);
        assert!(p.fields().iter().any(|f| f.label == "Missing"));
        // Short by more than the last plane: refused.
        assert!(parse(&acbm(16, 2, 2, 0, &abit[..2])).is_err());
    }

    #[test]
    fn a_chunk_is_read_to_its_own_length_when_the_form_understates() {
        // BUG-019: the FORM says two bytes less than its last chunk holds.
        let mut bytes = ilbm(16, 1, 2, 0, &GREY, None, &[0x80, 0, 0x40, 0]);
        let declared = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) - 2;
        bytes[4..8].copy_from_slice(&declared.to_be_bytes());
        let p = parse(&bytes).unwrap();
        assert_eq!(
            p.argb[1], 0xFFAA_AAAA,
            "pen 2, from the bytes past the FORM"
        );
    }

    #[test]
    fn planes_combine_into_pens_low_plane_first() {
        // 16 × 1, 2 planes: plane 0 = $8000, plane 1 = $4000.
        let p = parse(&ilbm(16, 1, 2, 0, &GREY, None, &[0x80, 0, 0x40, 0])).unwrap();
        assert_eq!(p.argb[0], 0xFF55_5555, "pen 1");
        assert_eq!(p.argb[1], 0xFFAA_AAAA, "pen 2");
        assert_eq!(p.argb[2], 0xFF00_0000, "pen 0");
        assert_eq!(p.body_left, 0);
    }

    #[test]
    fn byterun1_runs_and_literals_unpack_to_the_same_picture() {
        // Literals: n >= 0 copies the next n + 1 bytes.
        let plain = parse(&ilbm(16, 1, 2, 0, &GREY, None, &[0x80, 0, 0x40, 0])).unwrap();
        let literals = [0x03, 0x80, 0x00, 0x40, 0x00];
        assert_eq!(
            parse(&ilbm(16, 1, 2, 1, &GREY, None, &literals))
                .unwrap()
                .argb,
            plain.argb
        );
        // A run: n < 0 repeats the next byte 1 - n times; -128 is a no-op.
        let plain = parse(&ilbm(16, 1, 2, 0, &GREY, None, &[0x80, 0, 0, 0])).unwrap();
        let run = [0x00, 0x80, 0x80, 0xFE, 0x00];
        let p = parse(&ilbm(16, 1, 2, 1, &GREY, None, &run)).unwrap();
        assert_eq!(p.argb, plain.argb);
        assert_eq!(p.body_left, 0);
    }

    #[test]
    fn a_mask_plane_after_each_row_is_skipped() {
        // 16 × 2, one plane, masking 1: each row is its plane, then its mask.
        let mut bytes = ilbm(
            16,
            2,
            1,
            0,
            &GREY,
            None,
            &[0x80, 0, 0xFF, 0xFF, 0x40, 0, 0xFF, 0xFF],
        );
        let bmhd = bytes.windows(4).position(|w| w == b"BMHD").unwrap() + 8;
        bytes[bmhd + 9] = MASK_PLANE;
        let p = parse(&bytes).unwrap();
        assert_eq!((p.argb[0], p.argb[1]), (0xFF55_5555, 0xFF00_0000), "row 0");
        assert_eq!(
            (p.argb[16], p.argb[17]),
            (0xFF00_0000, 0xFF55_5555),
            "row 1"
        );
        assert_eq!(p.body_left, 0);
    }

    #[test]
    fn a_short_body_says_which_row() {
        // Big enough to be possible compressed, too short in fact.
        let e = parse(&ilbm(16, 2, 2, 1, &GREY, None, &[0x03, 0x80, 0, 0x40, 0])).unwrap_err();
        assert!(e.0.contains("row 1 of 2"), "{e}");
        // Too short to be possible at all: refused before allocating.
        let e = parse(&ilbm(4096, 4096, 2, 1, &GREY, None, &[0x80, 0])).unwrap_err();
        assert!(e.0.contains("cannot hold 4096 × 4096"), "{e}");
        assert!(parse(b"FORM\0\0\0\x04ILBM").unwrap_err().0.contains("BMHD"));
    }

    #[test]
    fn extra_half_brite_halves_the_first_thirty_two() {
        let mut cmap = vec![0u8; 96];
        cmap[3..6].copy_from_slice(&[0xFE, 0x80, 0x40]);
        // Pen 33 = planes 0 and 5.
        let mut body = vec![0u8; 12];
        body[0] = 0x80;
        body[10] = 0x80;
        let p = parse(&ilbm(16, 1, 6, 0, &cmap, Some(EXTRA_HALFBRITE), &body)).unwrap();
        assert_eq!(p.argb[0], 0xFF7F_4020);
    }

    #[test]
    fn hold_and_modify_keeps_two_components_and_changes_one() {
        let mut cmap = vec![0u8; 48];
        cmap[3..6].copy_from_slice(&[0x11, 0x22, 0x33]);
        // HAM6, three pixels: colour 1; modify red to $F; modify blue to $5.
        // Pixel values: 0b00_0001, 0b10_1111, 0b01_0101.
        let mut planes = [[0u8; 2]; 6];
        for (x, v) in [0b00_0001u8, 0b10_1111, 0b01_0101].into_iter().enumerate() {
            for (p, plane) in planes.iter_mut().enumerate() {
                if v & (1 << p) != 0 {
                    plane[0] |= 0x80 >> x;
                }
            }
        }
        let body: Vec<u8> = planes.concat();
        let p = parse(&ilbm(16, 1, 6, 0, &cmap, Some(HAM), &body)).unwrap();
        assert_eq!(&p.argb[0..3], &[0xFF11_2233, 0xFFFF_2233, 0xFFFF_2255]);
    }

    #[test]
    fn a_four_bit_colour_map_is_widened() {
        let p = parse(&ilbm(
            16,
            1,
            1,
            0,
            &[0, 0, 0, 0xF0, 0xA0, 0x50],
            None,
            &[0x80, 0],
        ))
        .unwrap();
        assert_eq!(p.colours[1], 0xFF_AA55);
    }

    #[test]
    fn a_colour_map_with_no_picture_is_a_palette() {
        let mut bytes = ilbm(0, 0, 0, 0, &GREY, None, &[]);
        let p = parse(&bytes).unwrap();
        assert_eq!(p.argb, Vec::<u32>::new());
        assert_eq!(p.colours.len(), 4);
        assert!(p.fields()[0].value.contains("no picture"));
        // No BODY at all, the same.
        let body = bytes.len() - 8;
        bytes.truncate(body);
        assert_eq!(parse(&bytes).unwrap().argb, Vec::<u32>::new());
    }

    #[test]
    fn a_picture_larger_than_any_screen_is_refused_before_it_is_allocated() {
        let e = parse(&ilbm(65535, 65535, 1, 1, &GREY, None, &[])).unwrap_err();
        assert!(e.0.contains("65535 × 65535"), "{e}");
    }
}
