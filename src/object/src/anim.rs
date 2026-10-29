//! IFF ANIM animations (F-033): a picture, then the changes to it.
//!
//! A `FORM ANIM` holds `FORM ILBM`s in order. The first is a whole picture;
//! each after it holds an `ANHD` — which compression its changes use, how
//! long to show the frame in sixtieths of a second, and which frame the
//! changes apply to — and a `DLTA` of the changes. Players keep two frames
//! and alternate between them, so by default a change applies to the frame
//! two back; an `ANHD` interleave of 1 makes it the one before.
//!
//! ADE reads compression 5, byte vertical delta, which is what every corpus
//! animation uses: a `DLTA` opens with sixteen offsets, one per plane, zero
//! for a plane that does not change, and each plane's changes run down its
//! byte-wide columns, a count of operations and then the operations — a
//! byte below `$80` skips that many rows, `$80` and above copies that many
//! less `$80` bytes down the column, and zero repeats the byte after the
//! count. The reading is the ANIM document's, as the AmigaOS wiki publishes
//! it, with its appendix on DPaint III's Anim Brushes: those change the
//! frame before rather than two back, and XOR their changes in, so that a
//! brush plays backwards too. AROS has no ANIM decoder. ffmpeg has one, and
//! the corpus test compares every frame of an ordinary animation with
//! ffmpeg's; it does not read Anim Brushes, which the document says no
//! player of its day did either, so a brush is checked as the document
//! describes it instead: its last change brings back its first frame.

use ade_endian::{u8_at, u32_at};

use crate::decoded::Field;
use crate::ilbm;

/// More frames than any animation holds; a file claiming more is damage.
const MAX_FRAMES: usize = 4096;

/// An animation's header and timing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anim {
    /// The first frame, as a picture: its size, depth, colours and mode.
    pub first: ilbm::Ilbm,
    /// How many frames, the first included.
    pub frames: usize,
    /// How long each frame after the first is shown, in sixtieths of a
    /// second.
    pub times: Vec<u32>,
    /// Compressions the frames use, by `ANHD` operation, with a count each.
    pub operations: Vec<(u8, usize)>,
    /// Frames whose changes ADE could not apply: another compression, or a
    /// `DLTA` that points outside itself.
    pub unapplied: usize,
    /// Frames that bring colours of their own.
    pub palettes: usize,
    /// Whether it is a DPaint Anim Brush: changes XORed into the frame
    /// before, not stored into the frame two back.
    pub brush: bool,
}

/// Why bytes are not an animation ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAnAnim(pub String);

impl core::fmt::Display for NotAnAnim {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotAnAnim {}

/// Whether `bytes` are an animation.
#[must_use]
pub fn is_anim(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"FORM") && bytes.get(8..12) == Some(b"ANIM")
}

/// The `FORM`s inside an animation, each as a whole `FORM` of its own.
fn forms(bytes: &[u8]) -> Vec<&[u8]> {
    let declared = u32_at(bytes, 4).map_or(0, |v| v as usize);
    let end = declared.saturating_add(8).min(bytes.len());
    let mut at = 12usize;
    let mut out = Vec::new();
    while at.saturating_add(8) <= end && out.len() < MAX_FRAMES {
        let len = u32_at(bytes, at.saturating_add(4)).map_or(0, |v| v as usize);
        let stop = at.saturating_add(8).saturating_add(len);
        if bytes.get(at..at.saturating_add(4)) == Some(b"FORM") {
            if let Some(form) = bytes.get(at..stop.min(bytes.len())) {
                out.push(form);
            }
        }
        at = stop.saturating_add(len & 1);
    }
    out
}

/// A frame's `ANHD` and `DLTA`.
fn header_and_delta(form: &[u8]) -> (Option<&[u8]>, Option<&[u8]>) {
    let chunks = ilbm::chunks(form).unwrap_or_default();
    let find = |id: &[u8; 4]| chunks.iter().find(|(c, _)| c == id).map(|(_, d)| *d);
    (find(b"ANHD"), find(b"DLTA"))
}

/// Apply a byte vertical delta to `rows`. Returns whether it could be.
fn byte_vertical(
    rows: &mut [u8],
    delta: &[u8],
    (row_bytes, row_len, depth): (usize, usize, usize),
    xor: bool,
) -> bool {
    let height = rows.len().checked_div(row_len).unwrap_or(0);
    // A plane whose changes run out stops there; the others are still
    // applied, as each plane's changes stand on their own.
    let mut whole = true;
    'plane: for plane in 0..depth.min(16) {
        let start = u32_at(delta, plane.saturating_mul(4)).map_or(0, |v| v as usize);
        if start == 0 {
            continue;
        }
        let Some(mut ops) = delta.get(start..) else {
            whole = false;
            continue 'plane;
        };
        for column in 0..row_bytes {
            let Some((&count, rest)) = ops.split_first() else {
                whole = false;
                continue 'plane;
            };
            ops = rest;
            let mut y = 0usize;
            let at = |y: usize| {
                y.saturating_mul(row_len)
                    .saturating_add(plane.saturating_mul(row_bytes))
                    .saturating_add(column)
            };
            for _ in 0..count {
                let Some((&op, rest)) = ops.split_first() else {
                    whole = false;
                    continue 'plane;
                };
                ops = rest;
                if op == 0 {
                    // The same byte, this many times.
                    let (Some(&n), Some(&value)) = (ops.first(), ops.get(1)) else {
                        whole = false;
                        continue 'plane;
                    };
                    ops = ops.get(2..).unwrap_or(&[]);
                    for _ in 0..n {
                        if y < height {
                            if let Some(b) = rows.get_mut(at(y)) {
                                *b = if xor { *b ^ value } else { value };
                            }
                        }
                        y = y.saturating_add(1);
                    }
                } else if op & 0x80 != 0 {
                    // This many bytes, one a row.
                    let n = usize::from(op & 0x7F);
                    let Some(literal) = ops.get(..n) else {
                        whole = false;
                        continue 'plane;
                    };
                    ops = ops.get(n..).unwrap_or(&[]);
                    for &value in literal {
                        if y < height {
                            if let Some(b) = rows.get_mut(at(y)) {
                                *b = if xor { *b ^ value } else { value };
                            }
                        }
                        y = y.saturating_add(1);
                    }
                } else {
                    y = y.saturating_add(usize::from(op));
                }
            }
        }
    }
    whole
}

/// Play the animation: `frame` is called with each frame's number from 0
/// and its pixels, in order, until it returns `false`.
///
/// # Errors
/// [`NotAnAnim`] when it is not one, or its first frame is no picture.
pub fn play(bytes: &[u8], mut frame: impl FnMut(usize, &[u32]) -> bool) -> Result<Anim, NotAnAnim> {
    if !is_anim(bytes) {
        return Err(NotAnAnim("not a FORM ANIM".to_owned()));
    }
    let forms = forms(bytes);
    let first_form = forms
        .first()
        .ok_or_else(|| NotAnAnim("it holds no frames".to_owned()))?;
    let first = ilbm::parse(first_form)
        .map_err(|e| NotAnAnim(format!("its first frame is not a picture: {e}")))?;
    if first.argb.is_empty() {
        return Err(NotAnAnim("its first frame has no pixels".to_owned()));
    }
    let chunks = ilbm::chunks(first_form).unwrap_or_default();
    let body = chunks
        .iter()
        .find(|(c, _)| c == b"BODY")
        .map_or(&[][..], |(_, d)| *d);
    let dims = (first.width, first.height, first.depth);
    let mask_plane = first.masking == ilbm::MASK_PLANE;
    let (rows, _) = ilbm::unpack(body, dims, mask_plane, first.compression == 1)
        .map_err(|e| NotAnAnim(format!("its first frame: {e}")))?;
    let modes = first.modes();
    let row_bytes = first.width.div_ceil(16).saturating_mul(2);
    let row_len =
        row_bytes.saturating_mul(usize::from(first.depth).saturating_add(usize::from(mask_plane)));

    // The two frames a player keeps — the one before and the one two back —
    // both the first to begin with.
    let mut before = rows.clone();
    let mut two_back = rows;
    let mut anim = Anim {
        first: first.clone(),
        frames: forms.len(),
        times: Vec::new(),
        operations: Vec::new(),
        unapplied: 0,
        palettes: 0,
        brush: false,
    };
    // The colours as they stand: a frame may bring a CMAP of its own, which
    // holds from then on — Personal Paint's animations cycle colours so.
    let mut colours = first.colours.clone();
    let mut going = frame(0, &first.argb);
    for (n, form) in forms.iter().enumerate().skip(1) {
        let (header, delta) = header_and_delta(form);
        if let Some(cmap) = ilbm::chunks(form)
            .unwrap_or_default()
            .iter()
            .find(|(id, _)| id == b"CMAP")
        {
            colours = ilbm::colour_map(cmap.1);
            anim.palettes = anim.palettes.saturating_add(1);
        }
        let operation = header.and_then(|h| u8_at(h, 0).ok()).unwrap_or(0);
        // `ANHD`: operation, mask, width, height, x, y, absolute time, the
        // time since the last frame, then the interleave.
        let interleave = header.and_then(|h| u8_at(h, 18).ok()).unwrap_or(0);
        let bits = header.and_then(|h| u32_at(h, 20).ok()).unwrap_or(0);
        // A DPaint Anim Brush: changes from the frame before, XORed so the
        // brush can play backwards — interleave 1, and the XOR bit set where
        // the ANIM document puts it (bit 1) or where DPaint does (bit 2).
        let xor = interleave == 1 || bits & 0b110 != 0;
        anim.brush |= xor;
        anim.times
            .push(header.and_then(|h| u32_at(h, 14).ok()).unwrap_or(0));
        match anim.operations.iter_mut().find(|(op, _)| *op == operation) {
            Some((_, count)) => *count = count.saturating_add(1),
            None => anim.operations.push((operation, 1)),
        }
        // Frame n is built on the frame two back, the one the hidden buffer
        // holds; with an interleave of 1, on the frame before.
        let mut next = if interleave == 1 {
            before.clone()
        } else {
            core::mem::take(&mut two_back)
        };
        // Operation 0 is a whole picture again, in a BODY of its own, as
        // Personal Paint writes one into its animations.
        let whole = (operation == 0)
            .then(|| ilbm::chunks(form).unwrap_or_default())
            .and_then(|c| c.iter().find(|(id, _)| id == b"BODY").map(|(_, d)| *d))
            .and_then(|body| ilbm::unpack(body, dims, mask_plane, first.compression == 1).ok())
            .filter(|(rows, _)| rows.len() == next.len());
        let applied = if let Some((rows, _)) = whole {
            next = rows;
            true
        } else {
            operation == 5
                && delta.is_some_and(|d| {
                    byte_vertical(
                        &mut next,
                        d,
                        (row_bytes, row_len, usize::from(first.depth)),
                        xor,
                    )
                })
        };
        if !applied {
            anim.unapplied = anim.unapplied.saturating_add(1);
        }
        if going {
            let argb = ilbm::render(&next, dims, mask_plane, modes, &colours);
            going = frame(n, &argb);
        }
        two_back = core::mem::replace(&mut before, next);
    }
    Ok(anim)
}

impl Anim {
    /// How long it plays, in seconds, the first frame not counted.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        f64::from(self.times.iter().fold(0u32, |a, &t| a.saturating_add(t))) / 60.0
    }

    /// What the animation is, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const A: &str = "Animation";
        let operations: Vec<String> = self
            .operations
            .iter()
            .map(|(op, n)| {
                let name = match op {
                    0 => "a whole picture".to_owned(),
                    5 => "byte vertical delta".to_owned(),
                    other => format!("compression {other}"),
                };
                format!("{name} ({n})")
            })
            .collect();
        let mut out = vec![
            Field::new(
                A,
                "Size",
                format!("{} × {}", self.first.width, self.first.height),
            ),
            Field::new(A, "Frames", self.frames.to_string()),
            Field::new(
                A,
                "Kind",
                if self.brush {
                    "a DPaint Anim Brush: each frame XORed into the one before, so it plays \
                     backwards too"
                } else {
                    "an animation: each frame changes the one two before, for double \
                     buffering"
                },
            ),
            Field::new(A, "Plays for", format!("{:.2} seconds", self.seconds())),
            Field::new(
                A,
                "Changes",
                if operations.is_empty() {
                    "none".to_owned()
                } else {
                    operations.join(", ")
                },
            ),
        ];
        if self.palettes > 0 {
            out.push(Field::new(
                A,
                "Colours",
                format!("{} frames change the colours as they play", self.palettes),
            ));
        }
        if self.unapplied > 0 {
            out.push(Field::new(
                A,
                "Not played",
                format!(
                    "{} frames use a compression ADE does not read, or changes that do not \
                     fit the picture; they are shown unchanged",
                    self.unapplied
                ),
            ));
        }
        out.extend(self.first.fields().into_iter().map(|mut f| {
            f.group = "First frame";
            f
        }));
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

    fn form(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
        let inner: Vec<u8> = kind.iter().copied().chain(chunks.concat()).collect();
        let mut out = b"FORM".to_vec();
        out.extend((inner.len() as u32).to_be_bytes());
        out.extend(inner);
        out
    }

    /// 16 × 4, one plane, black and white: column 0 of `rows`.
    fn first(rows: [u8; 4]) -> Vec<u8> {
        let mut bmhd = vec![0u8; 20];
        bmhd[0..2].copy_from_slice(&16u16.to_be_bytes());
        bmhd[2..4].copy_from_slice(&4u16.to_be_bytes());
        bmhd[8] = 1;
        let body: Vec<u8> = rows.iter().flat_map(|&r| [r, 0]).collect();
        form(
            b"ILBM",
            &[
                chunk(b"BMHD", &bmhd),
                chunk(b"CMAP", &[0, 0, 0, 255, 255, 255]),
                chunk(b"BODY", &body),
            ],
        )
    }

    /// A frame changing plane 0's column 0 with `count` operations, `op`.
    fn frame(interleave: u8, count: u8, op: &[u8], extra: Option<Vec<u8>>) -> Vec<u8> {
        let mut anhd = vec![0u8; 40];
        anhd[0] = 5;
        anhd[17] = 1; // one jiffy
        anhd[18] = interleave;
        let mut dlta = vec![0u8; 64];
        dlta[0..4].copy_from_slice(&64u32.to_be_bytes());
        dlta.push(count);
        dlta.extend(op);
        dlta.push(0); // column 1: no change
        let mut chunks = vec![chunk(b"ANHD", &anhd), chunk(b"DLTA", &dlta)];
        chunks.extend(extra);
        form(b"ILBM", &chunks)
    }

    fn anim(frames: &[Vec<u8>]) -> Vec<u8> {
        form(b"ANIM", frames)
    }

    /// Column 0 of each frame, as the top bit of rows 0 to 3.
    fn columns(bytes: &[u8]) -> (Anim, Vec<[bool; 4]>) {
        let mut out = Vec::new();
        let a = play(bytes, |_, argb| {
            out.push([0, 1, 2, 3].map(|y| argb[y * 16] == 0xFFFF_FFFF));
            true
        })
        .unwrap();
        (a, out)
    }

    #[test]
    fn an_animation_stores_each_change_into_the_frame_two_back() {
        // Frame 1 sets row 0; frame 2, built on frame 0, sets row 1 — so
        // row 0 is off again.
        let bytes = anim(&[
            first([0; 4]),
            frame(0, 1, &[0x81, 0x80], None),
            frame(0, 2, &[1, 0x81, 0x80], None),
        ]);
        let (a, frames) = columns(&bytes);
        assert!(!a.brush);
        assert_eq!(a.frames, 3);
        assert_eq!(frames[1], [true, false, false, false]);
        assert_eq!(frames[2], [false, true, false, false]);
        assert_eq!(a.times, vec![1, 1]);
    }

    #[test]
    fn an_anim_brush_xors_each_change_into_the_frame_before() {
        // Frame 1 sets row 0; frame 2 XORs row 0 back off and row 1 on, a
        // run of two; frame 3 XORs row 1 off, ending on the first frame.
        let bytes = anim(&[
            first([0; 4]),
            frame(1, 1, &[0x81, 0x80], None),
            frame(1, 1, &[0, 2, 0x80], None),
            frame(1, 2, &[1, 0x81, 0x80], None),
        ]);
        let (a, frames) = columns(&bytes);
        assert!(a.brush);
        assert_eq!(frames[1], [true, false, false, false]);
        assert_eq!(frames[2], [false, true, false, false]);
        assert_eq!(frames[3], frames[0]);
    }

    #[test]
    fn a_frame_may_bring_its_own_colours() {
        let bytes = anim(&[
            first([0x80, 0, 0, 0]),
            frame(0, 1, &[3], Some(chunk(b"CMAP", &[0, 0, 0, 255, 0, 0]))),
        ]);
        let mut seen = Vec::new();
        let a = play(&bytes, |_, argb| {
            seen.push(argb[0]);
            true
        })
        .unwrap();
        assert_eq!(seen, vec![0xFFFF_FFFF, 0xFFFF_0000]);
        assert_eq!(a.palettes, 1);
    }

    #[test]
    fn a_change_that_runs_out_stops_its_plane_and_is_counted() {
        let bytes = anim(&[first([0; 4]), frame(0, 1, &[0x84, 0x80], None)]);
        let (a, _) = columns(&bytes);
        assert_eq!(a.unapplied, 1);
        assert!(!is_anim(&first([0; 4])));
    }
}
