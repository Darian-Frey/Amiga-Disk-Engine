//! IFF 8SVX sounds (F-033): sampled sound, 8 bits signed, from instruments
//! to speech.
//!
//! A `FORM 8SVX` of chunks. `VHDR` says how many samples play once and how
//! many then repeat, the rate, how many octaves the body holds and whether
//! it is compressed; `BODY` holds the samples, the first octave then each
//! next one twice as long; `CHAN` says which side a sound is for, and a
//! stereo sound holds the left channel's samples and then the right's. The
//! text chunks — `NAME`, `AUTH`, `ANNO`, `(c) ` — say what it is.
//!
//! The layout is `datatypes/soundclass.h`; the reading follows AROS's
//! `8svx.datatype` (`workbench/classes/datatypes/8svx/8svxclass.c`), read
//! locally and not committed. Like it, ADE does not decompress a
//! Fibonacci-delta body: there is no second statement of that encoding here
//! to check a decoder against, and six corpus files use it.

use ade_endian::{u8_at, u16_at, u32_at};

use crate::decoded::Field;

/// `CHAN` values: which side.
const LEFT: u32 = 2;
const RIGHT: u32 = 4;
const STEREO: u32 = 6;

/// A decoded sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sound {
    /// `vh_OneShotHiSamples`: samples in the highest octave that play once.
    pub one_shot: u32,
    /// `vh_RepeatHiSamples`: samples that then repeat, for a held note.
    pub repeat: u32,
    /// `vh_SamplesPerHiCycle`: samples in one cycle of the highest octave's
    /// note, for a musical instrument; zero otherwise.
    pub per_cycle: u32,
    /// `vh_SamplesPerSec`.
    pub rate: u16,
    /// `vh_Octaves`.
    pub octaves: u8,
    /// `vh_Compression`: 0 none, 1 Fibonacci-delta.
    pub compression: u8,
    /// `vh_Volume`, 16.16 fixed point: `$10000` is full.
    pub volume: u32,
    /// The `CHAN` chunk: 2 left, 4 right, 6 stereo.
    pub channel: Option<u32>,
    /// The text chunks, by ID, in order.
    pub text: Vec<(String, String)>,
    /// The first octave's samples, of the first channel; empty when the body
    /// is compressed.
    pub samples: Vec<i8>,
    /// Bytes of `BODY`, and how many the header says it should hold.
    pub body: (usize, usize),
}

/// Why bytes are not a sound ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotASound(pub String);

impl core::fmt::Display for NotASound {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `bytes` start like an IFF 8SVX.
#[must_use]
pub fn is_svx(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"FORM") && bytes.get(8..12) == Some(b"8SVX")
}

/// Read a sound.
///
/// # Errors
/// [`NotASound`] when it has no `VHDR`.
pub fn parse(bytes: &[u8]) -> Result<Sound, NotASound> {
    if !is_svx(bytes) {
        return Err(NotASound(
            "an 8SVX starts with FORM, a length, and 8SVX".to_owned(),
        ));
    }
    let declared = u32_at(bytes, 4).unwrap_or(0) as usize;
    let end = declared.saturating_add(8).min(bytes.len());
    let (mut vhdr, mut body, mut channel) = (None, None, None);
    let mut text = Vec::new();
    let mut at = 12usize;
    while at.saturating_add(8) <= end {
        let id = bytes.get(at..at.saturating_add(4)).unwrap_or(&[]);
        let len = u32_at(bytes, at.saturating_add(4)).unwrap_or(0) as usize;
        let start = at.saturating_add(8);
        let data = bytes
            .get(start..start.saturating_add(len).min(end))
            .unwrap_or(&[]);
        match id {
            b"VHDR" => vhdr = Some(data),
            b"BODY" => body = Some(data),
            b"CHAN" => channel = u32_at(data, 0).ok(),
            b"NAME" | b"AUTH" | b"ANNO" | b"(c) " => {
                let words: String = data
                    .iter()
                    .take_while(|&&b| b != 0)
                    .map(|&b| char::from(b))
                    .collect();
                let label = match id {
                    b"NAME" => "Name",
                    b"AUTH" => "Author",
                    b"ANNO" => "Note",
                    _ => "Copyright",
                };
                if !words.trim().is_empty() {
                    text.push((label.to_owned(), words.trim().to_owned()));
                }
            }
            _ => {}
        }
        at = start.saturating_add(len).saturating_add(len & 1);
    }
    let vhdr =
        vhdr.ok_or_else(|| NotASound("it has no VHDR, so nothing about it is known".to_owned()))?;
    let one_shot = u32_at(vhdr, 0).unwrap_or(0);
    let repeat = u32_at(vhdr, 4).unwrap_or(0);
    let octaves = u8_at(vhdr, 14).unwrap_or(0);
    let compression = u8_at(vhdr, 15).unwrap_or(0);
    let body = body.unwrap_or(&[]);
    let first = one_shot.saturating_add(repeat) as usize;
    let sides = if channel == Some(STEREO) { 2 } else { 1 };
    // Each octave twice the one before: first × (2^octaves − 1) per side.
    let expected = first
        .saturating_mul((1usize << octaves.min(16)).saturating_sub(1).max(1))
        .saturating_mul(sides);
    let samples = if compression == 0 {
        body.iter()
            .take(first)
            .map(|&b| i8::from_ne_bytes([b]))
            .collect()
    } else {
        Vec::new()
    };
    Ok(Sound {
        one_shot,
        repeat,
        per_cycle: u32_at(vhdr, 8).unwrap_or(0),
        rate: u16_at(vhdr, 12).unwrap_or(0),
        octaves,
        compression,
        volume: u32_at(vhdr, 16).unwrap_or(0),
        channel,
        text,
        samples,
        body: (body.len(), expected),
    })
}

impl Sound {
    /// The waveform of the first octave, `width` columns of `height`: in
    /// each column, how far the samples it covers reach above and below
    /// silence, and whether the column is in the repeating part.
    #[must_use]
    pub fn waveform(&self, width: usize, height: usize) -> Option<Vec<(usize, usize, bool)>> {
        let repeats_from = (self.repeat > 0).then_some(self.one_shot as usize);
        crate::decoded::waveform(&self.samples, repeats_from, width, height)
    }

    /// What the sound says, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const S: &str = "Sound";
        let mut out: Vec<Field> = self
            .text
            .iter()
            .map(|(label, words)| Field::new(S, label.clone(), words.clone()))
            .collect();
        let samples = self.one_shot.saturating_add(self.repeat);
        let seconds = if self.rate > 0 {
            format!(", {:.2} seconds", f64::from(samples) / f64::from(self.rate))
        } else {
            String::new()
        };
        out.push(Field::new(
            S,
            "Length",
            format!("{samples} samples{seconds}"),
        ));
        if self.repeat > 0 {
            out.push(Field::new(
                S,
                "Plays",
                format!(
                    "{} once, then repeats the last {} while held",
                    self.one_shot, self.repeat
                ),
            ));
        }
        out.push(Field::new(
            S,
            "Rate",
            if self.rate == 0 {
                "not given".to_owned()
            } else {
                format!("{} samples a second", self.rate)
            },
        ));
        if self.octaves > 1 {
            out.push(Field::new(
                S,
                "Octaves",
                format!(
                    "{}: an instrument, each octave down twice as long",
                    self.octaves
                ),
            ));
        }
        out.push(Field::new(
            S,
            "Volume",
            format!("{}%", u64::from(self.volume).saturating_mul(100) / 0x10000),
        ));
        out.push(Field::new(
            S,
            "Channel",
            match self.channel {
                None => "one (no CHAN chunk)".to_owned(),
                Some(LEFT) => "left".to_owned(),
                Some(RIGHT) => "right".to_owned(),
                Some(STEREO) => "stereo: the left channel's samples, then the right's".to_owned(),
                Some(c) => format!("unknown ({c})"),
            },
        ));
        out.push(Field::new(
            S,
            "Compression",
            match self.compression {
                0 => "none".to_owned(),
                1 => "Fibonacci-delta, which ADE does not decompress".to_owned(),
                c => format!("unknown ({c})"),
            },
        ));
        if self.compression == 0 && self.body.0 != self.body.1 {
            out.push(Field::new(
                S,
                "Sample data",
                format!(
                    "{} bytes, where the header describes {}",
                    self.body.0, self.body.1
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
    clippy::arithmetic_side_effects,
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
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

    fn sound(
        one: u32,
        rep: u32,
        octaves: u8,
        compression: u8,
        chan: Option<u32>,
        body: &[u8],
    ) -> Vec<u8> {
        let mut vhdr = Vec::new();
        vhdr.extend(one.to_be_bytes());
        vhdr.extend(rep.to_be_bytes());
        vhdr.extend(0u32.to_be_bytes());
        vhdr.extend(8000u16.to_be_bytes());
        vhdr.push(octaves);
        vhdr.push(compression);
        vhdr.extend(0x8000u32.to_be_bytes());
        let mut inner = b"8SVX".to_vec();
        inner.extend(chunk(*b"VHDR", &vhdr));
        inner.extend(chunk(*b"NAME", b"beep"));
        if let Some(c) = chan {
            inner.extend(chunk(*b"CHAN", &c.to_be_bytes()));
        }
        inner.extend(chunk(*b"BODY", body));
        let mut out = b"FORM".to_vec();
        out.extend((inner.len() as u32).to_be_bytes());
        out.extend(inner);
        out
    }

    #[test]
    fn a_sound_reads_its_header_text_and_first_octave() {
        let body: Vec<u8> = [0i8, 100, -100, 0, 50, -50, 0, 127]
            .iter()
            .map(|&s| s.to_ne_bytes()[0])
            .collect();
        let s = parse(&sound(4, 4, 1, 0, None, &body)).unwrap();
        assert_eq!((s.one_shot, s.repeat, s.rate, s.octaves), (4, 4, 8000, 1));
        assert_eq!(s.samples, vec![0, 100, -100, 0, 50, -50, 0, 127]);
        assert_eq!(s.body, (8, 8));
        assert_eq!(s.text, vec![("Name".to_owned(), "beep".to_owned())]);
        let fields = s.fields();
        assert!(fields.iter().any(|f| f.value == "8 samples, 0.00 seconds"));
        assert!(
            fields
                .iter()
                .any(|f| f.label == "Volume" && f.value == "50%")
        );
        // The repeating half is marked as such.
        let w = s.waveform(8, 16).unwrap();
        assert_eq!(
            w.iter().map(|c| c.2).collect::<Vec<_>>(),
            [false, false, false, false, true, true, true, true]
        );
        // 127 is 127/128 of full scale: a row short of the top. Silence
        // sits at the middle.
        assert_eq!(w[7].0, 1);
        assert_eq!((w[0].0, w[0].1), (8, 8));
    }

    #[test]
    fn octaves_and_stereo_say_how_long_the_body_should_be() {
        // Three octaves of 4: 4 + 8 + 16 = 28 samples, both sides 56.
        let s = parse(&sound(4, 0, 3, 0, Some(STEREO), &[0; 56])).unwrap();
        assert_eq!(s.body, (56, 56));
        assert_eq!(s.samples.len(), 4, "the first octave of the first side");
        let short = parse(&sound(4, 0, 3, 0, Some(STEREO), &[0; 30])).unwrap();
        assert!(
            short
                .fields()
                .iter()
                .any(|f| f.value == "30 bytes, where the header describes 56")
        );
    }

    #[test]
    fn a_compressed_body_is_described_not_decoded() {
        let s = parse(&sound(4, 0, 1, 1, None, &[1, 2, 3])).unwrap();
        assert_eq!(s.samples, Vec::<i8>::new());
        assert!(s.waveform(8, 16).is_none());
        assert!(
            s.fields()
                .iter()
                .any(|f| f.value.contains("Fibonacci-delta"))
        );
        assert!(parse(b"FORM\0\0\0\x048SVX").unwrap_err().0.contains("VHDR"));
    }
}
