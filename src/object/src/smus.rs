//! IFF SMUS scores (F-033): music as notes, the way Deluxe Music and games'
//! theme songs kept it.
//!
//! A `FORM SMUS`: `SHDR` — the tempo in 128ths of a quarter note a minute,
//! the volume and how many tracks — then the text chunks `NAME`, `AUTH`,
//! `(c) ` and `ANNO`, an `INS1` per instrument register naming an instrument
//! or a MIDI channel and preset, and a `TRAK` per track. A track is two-byte
//! events in time order: a note, its pitch the MIDI tone number and its
//! duration a division of a whole note, dotted or in a triplet, quintuplet or
//! septuplet, and chorded with the next or tied to it; a rest; and settings
//! that hold from then on — instrument, time and key signature, dynamics.
//! The reading is the SMUS document's, as the AmigaOS wiki publishes it.

use ade_endian::{u8_at, u16_at, u32_at};

use crate::decoded::Field;

/// A quarter note, in the units ADE counts time in: 2⁷ × 2 × 3 × 5 × 7, so
/// every division down to a 128th, dotted or in any tuplet, is a whole
/// number of them.
pub const QUARTER: u64 = 26_880;

/// One track's summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Track {
    /// Notes, chorded ones included.
    pub notes: usize,
    /// Rests.
    pub rests: usize,
    /// How long it plays, in [`QUARTER`]s.
    pub length: u64,
    /// Its lowest and highest MIDI tone.
    pub range: Option<(u8, u8)>,
    /// The instrument registers it plays, in the order it first uses them;
    /// a track starts on the register of its own number.
    pub instruments: Vec<u8>,
    /// Its first time signature, as (beats, beat), where it gives one.
    pub time: Option<(u8, u8)>,
    /// Its first key signature, the document's 0 to 14, where it gives one.
    pub key: Option<u8>,
    /// Events of a kind ADE does not count, by ID.
    pub other: usize,
}

/// An instrument register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instrument {
    /// The register's number.
    pub register: u8,
    /// Its name.
    pub name: String,
    /// The MIDI channel and preset it asks for, where it does.
    pub midi: Option<(u8, u8)>,
}

/// A decoded score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Score {
    /// `SHDR`'s tempo, in 128ths of a quarter note a minute.
    pub tempo: u16,
    /// `SHDR`'s volume, 0 to 127.
    pub volume: u8,
    /// How many tracks `SHDR` says.
    pub tracks_said: u8,
    /// The text chunks, by ID, in order.
    pub text: Vec<(String, String)>,
    /// The instrument registers.
    pub instruments: Vec<Instrument>,
    /// The tracks.
    pub tracks: Vec<Track>,
}

/// Why bytes are not a score ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAScore(pub String);

impl core::fmt::Display for NotAScore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotAScore {}

/// Whether `bytes` are a score.
#[must_use]
pub fn is_score(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"FORM") && bytes.get(8..12) == Some(b"SMUS")
}

fn text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// A note or rest's duration, in [`QUARTER`]s: a whole note's 2^-division,
/// half as long again when dotted, 2/3, 4/5 or 6/7 in a tuplet.
#[must_use]
pub fn duration(data: u8) -> u64 {
    let whole = QUARTER.saturating_mul(4);
    let mut d = whole >> (data & 7);
    if data & 0x08 != 0 {
        d = d.saturating_mul(3) / 2;
    }
    match (data >> 4) & 3 {
        1 => d.saturating_mul(2) / 3,
        2 => d.saturating_mul(4) / 5,
        3 => d.saturating_mul(6) / 7,
        _ => d,
    }
}

/// Read a score.
///
/// # Errors
/// [`NotAScore`] when it is not one, or has no `SHDR`.
pub fn parse(bytes: &[u8]) -> Result<Score, NotAScore> {
    if !is_score(bytes) {
        return Err(NotAScore("not a FORM SMUS".to_owned()));
    }
    let declared = u32_at(bytes, 4).map_or(0, |v| v as usize);
    let end = declared.saturating_add(8).min(bytes.len());
    let mut score: Option<Score> = None;
    let mut text_chunks = Vec::new();
    let mut instruments = Vec::new();
    let mut tracks = Vec::new();
    let mut at = 12usize;
    while at.saturating_add(8) <= end {
        let id = bytes.get(at..at.saturating_add(4)).unwrap_or(&[]);
        let len = u32_at(bytes, at.saturating_add(4)).map_or(0, |v| v as usize);
        let start = at.saturating_add(8);
        let data = bytes
            .get(start..start.saturating_add(len).min(bytes.len()))
            .unwrap_or(&[]);
        match id {
            b"SHDR" if data.len() >= 4 => {
                score = Some(Score {
                    tempo: u16_at(data, 0).unwrap_or(0),
                    volume: u8_at(data, 2).unwrap_or(0),
                    tracks_said: u8_at(data, 3).unwrap_or(0),
                    text: Vec::new(),
                    instruments: Vec::new(),
                    tracks: Vec::new(),
                });
            }
            b"NAME" | b"AUTH" | b"(c) " | b"ANNO" => {
                let label = match id {
                    b"NAME" => "Name",
                    b"AUTH" => "Author",
                    b"(c) " => "Copyright",
                    _ => "Note",
                };
                text_chunks.push((label.to_owned(), text(data)));
            }
            b"INS1" if data.len() >= 4 => instruments.push(Instrument {
                register: u8_at(data, 0).unwrap_or(0),
                name: text(data.get(4..).unwrap_or(&[])),
                midi: (u8_at(data, 1).unwrap_or(0) == 1)
                    .then(|| (u8_at(data, 2).unwrap_or(0), u8_at(data, 3).unwrap_or(0))),
            }),
            b"TRAK" => {
                let number = u8::try_from(tracks.len().saturating_add(1)).unwrap_or(u8::MAX);
                tracks.push(track(data, number));
            }
            _ => {}
        }
        at = start.saturating_add(len).saturating_add(len & 1);
    }
    let mut score = score.ok_or_else(|| NotAScore("it has no SHDR, so no tempo".to_owned()))?;
    score.text = text_chunks;
    score.instruments = instruments;
    score.tracks = tracks;
    Ok(score)
}

/// One `TRAK`'s events, track `number` from 1.
fn track(data: &[u8], number: u8) -> Track {
    let mut t = Track {
        instruments: vec![number],
        ..Track::default()
    };
    for event in data.chunks_exact(2) {
        let (id, value) = (
            event.first().copied().unwrap_or(0),
            event.get(1).copied().unwrap_or(0),
        );
        match id {
            0..=127 => {
                t.notes = t.notes.saturating_add(1);
                t.range = Some(
                    t.range
                        .map_or((id, id), |(lo, hi)| (lo.min(id), hi.max(id))),
                );
                // A chorded note sounds with the next: only the last of a
                // chord moves time on.
                if value & 0x80 == 0 {
                    t.length = t.length.saturating_add(duration(value));
                }
            }
            128 => {
                t.rests = t.rests.saturating_add(1);
                t.length = t.length.saturating_add(duration(value));
            }
            129 => {
                if !t.instruments.contains(&value) {
                    t.instruments.push(value);
                }
            }
            130 => {
                t.time
                    .get_or_insert(((value >> 3).saturating_add(1), 1u8 << (value & 7)));
            }
            131 => {
                t.key.get_or_insert(value);
            }
            132..=136 | 144..=159 => {}
            _ => t.other = t.other.saturating_add(1),
        }
    }
    t
}

/// A MIDI tone number as a note name: 60 is middle C, C4.
fn note_name(tone: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B",
    ];
    let name = NAMES.get(usize::from(tone % 12)).copied().unwrap_or("?");
    format!("{name}{}", i16::from(tone / 12).saturating_sub(1))
}

/// A key signature, the document's 0 to 14.
fn key_name(key: u8) -> String {
    const KEYS: [&str; 15] = [
        "C major",
        "G major",
        "D major",
        "A major",
        "E major",
        "B major",
        "F♯ major",
        "C♯ major",
        "F major",
        "B♭ major",
        "E♭ major",
        "A♭ major",
        "D♭ major",
        "G♭ major",
        "C♭ major",
    ];
    KEYS.get(usize::from(key))
        .map_or_else(|| format!("key {key}"), |k| (*k).to_owned())
}

/// [`QUARTER`] units as quarter notes, for display: exact below some
/// 159,000 quarter notes, which no score approaches.
fn as_quarters(units: u64) -> f64 {
    f64::from(u32::try_from(units).unwrap_or(u32::MAX))
        / f64::from(u32::try_from(QUARTER).unwrap_or(1))
}

/// Quarters as a count, with a fraction where there is one.
fn quarters(units: u64) -> String {
    let whole = units / QUARTER;
    let rest = units % QUARTER;
    if rest == 0 {
        whole.to_string()
    } else {
        format!("{:.2}", as_quarters(units))
    }
}

impl Score {
    /// How long the longest track plays, in [`QUARTER`]s.
    #[must_use]
    pub fn length(&self) -> u64 {
        self.tracks.iter().map(|t| t.length).max().unwrap_or(0)
    }

    /// How long it plays at its tempo, in seconds.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        if self.tempo == 0 {
            return 0.0;
        }
        as_quarters(self.length()) * 60.0 * 128.0 / f64::from(self.tempo)
    }

    /// What the score is, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const S: &str = "Score";
        let mut out: Vec<Field> = self
            .text
            .iter()
            .map(|(label, words)| Field::new(S, label.clone(), words.clone()))
            .collect();
        let seconds = self.seconds();
        let minutes = (seconds / 60.0).floor();
        out.extend([
            Field::new(
                S,
                "Tempo",
                format!(
                    "{} quarter notes a minute",
                    quarters(u64::from(self.tempo) * QUARTER / 128)
                ),
            ),
            Field::new(
                S,
                "Plays for",
                format!(
                    "{minutes}:{:04.1}, {} quarter notes",
                    seconds - minutes * 60.0,
                    quarters(self.length())
                ),
            ),
            Field::new(S, "Volume", format!("{} of 127", self.volume)),
            Field::new(S, "Tracks", self.tracks.len().to_string()),
        ]);
        for i in &self.instruments {
            let midi = i.midi.map_or_else(String::new, |(c, p)| {
                format!(", MIDI channel {c}, preset {p}")
            });
            out.push(Field::new(
                "Instruments",
                i.register.to_string(),
                format!("{}{midi}", i.name),
            ));
        }
        for (n, t) in self.tracks.iter().enumerate() {
            let mut words = vec![format!("{} notes, {} rests", t.notes, t.rests)];
            if let Some((lo, hi)) = t.range {
                words.push(format!("{} to {}", note_name(lo), note_name(hi)));
            }
            words.push(format!("{} quarter notes long", quarters(t.length)));
            if let Some((beats, beat)) = t.time {
                words.push(format!("{beats}/{beat} time"));
            }
            if let Some(k) = t.key {
                words.push(key_name(k));
            }
            let registers: Vec<String> = t.instruments.iter().map(u8::to_string).collect();
            words.push(format!("instrument {}", registers.join(", ")));
            out.push(Field::new(
                "Tracks",
                (n.saturating_add(1)).to_string(),
                words.join("; "),
            ));
        }
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

    fn score(tracks: &[&[u8]]) -> Vec<u8> {
        let mut inner = b"SMUS".to_vec();
        // 120 quarter notes a minute, volume 100.
        inner.extend(chunk(
            b"SHDR",
            &[
                ((120 * 128) >> 8) as u8,
                ((120 * 128) & 0xFF) as u8,
                100,
                tracks.len() as u8,
            ],
        ));
        inner.extend(chunk(b"NAME", b"Tune"));
        inner.extend(chunk(b"INS1", b"\x01\x01\x02\x05Piano"));
        for t in tracks {
            inner.extend(chunk(b"TRAK", t));
        }
        let mut out = b"FORM".to_vec();
        out.extend((inner.len() as u32).to_be_bytes());
        out.extend(inner);
        out
    }

    #[test]
    fn durations_are_whole_numbers_of_the_unit_for_every_division_dot_and_tuplet() {
        assert_eq!(duration(2), QUARTER, "a quarter note");
        assert_eq!(duration(0), 4 * QUARTER, "a whole note");
        assert_eq!(duration(2 | 0x08), QUARTER * 3 / 2, "a dotted quarter");
        assert_eq!(duration(3 | 0x10), QUARTER / 3, "a triplet eighth");
        assert_eq!(duration(2 | 0x20), QUARTER * 4 / 5, "a quintuplet quarter");
        assert_eq!(
            duration(7 | 0x08 | 0x30),
            QUARTER / 32 * 3 / 2 * 6 / 7,
            "a dotted septuplet 128th"
        );
        for data in 0..=0x3F {
            let d = duration(data);
            assert!(d > 0);
        }
    }

    #[test]
    fn a_score_counts_its_tracks_notes_and_time() {
        // Track 1: a C major chord of quarter notes (C, E chorded onto G),
        // then a quarter rest — two quarters. Track 2: in 3/4, D major, a
        // dotted half note on instrument 1 — three quarters.
        let one: &[u8] = &[60, 0x82, 64, 0x82, 67, 2, 128, 2];
        let two: &[u8] = &[130, (2 << 3) | 2, 131, 2, 129, 1, 62, 1 | 0x08];
        let s = parse(&score(&[one, two])).unwrap();
        assert_eq!((s.tempo, s.volume, s.tracks.len()), (120 * 128, 100, 2));
        assert_eq!(s.text, vec![("Name".to_owned(), "Tune".to_owned())]);
        assert_eq!(
            s.instruments,
            vec![Instrument {
                register: 1,
                name: "Piano".to_owned(),
                midi: Some((2, 5))
            }]
        );
        let (a, b) = (&s.tracks[0], &s.tracks[1]);
        assert_eq!(
            (a.notes, a.rests, a.length, a.range),
            (3, 1, 2 * QUARTER, Some((60, 67)))
        );
        assert_eq!(
            (b.notes, b.length, b.time, b.key),
            (1, 3 * QUARTER, Some((3, 4)), Some(2))
        );
        assert_eq!(b.instruments, vec![2, 1]);
        assert_eq!(s.length(), 3 * QUARTER);
        assert!((s.seconds() - 1.5).abs() < 1e-9);
        let f = s.fields();
        let value = |l: &str| f.iter().find(|x| x.label == l).unwrap().value.clone();
        assert_eq!(value("Tempo"), "120 quarter notes a minute");
        assert_eq!(value("1"), "Piano, MIDI channel 2, preset 5");
        assert!(
            f.iter()
                .any(|x| x.group == "Tracks" && x.value.contains("C4 to G4"))
        );
        assert!(
            f.iter()
                .any(|x| x.group == "Tracks" && x.value.contains("3/4 time; D major"))
        );
    }

    #[test]
    fn a_score_without_a_header_is_not_read() {
        let mut bytes = score(&[&[60, 2]]);
        let at = bytes.windows(4).position(|w| w == b"SHDR").unwrap();
        bytes[at..at + 4].copy_from_slice(b"XXXX");
        assert!(parse(&bytes).is_err());
    }
}
