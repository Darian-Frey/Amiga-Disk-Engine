//! ProTracker modules (F-033): music, as the Amiga's trackers wrote it.
//!
//! A module is a header, patterns, then samples. The header is a 20-byte
//! title; 31 sample headers of 30 bytes each — a 22-byte name, then the
//! length, finetune, volume, loop start and loop length, the lengths in
//! words; the song's length in positions; a byte trackers use for
//! restarting; the 128-entry order list of patterns; and at byte 1080 a
//! four-byte tag that says how many channels the patterns hold — `M.K.`,
//! `M!K!`, `FLT4` and `4CHN` four, `6CHN` six, `8CHN` eight. Patterns are 64
//! rows of four bytes per channel, as many as the highest pattern the order
//! list names, and the samples' signed 8-bit data follow them in order.
//!
//! The 15-sample Soundtracker modules that came before carry no tag and are
//! not recognised: nothing in such a file says what it is. `FLT8` is not
//! either: it lays eight channels out as pairs of four-channel patterns, and
//! no corpus module uses it.
//!
//! How long a module plays is worked out by playing its patterns without
//! sound: speed and tempo changes (`Fxx`), position jumps (`Bxx`), pattern
//! breaks (`Dxy`), pattern loops (`E6x`) and pattern delays (`EEx`), until
//! the song ends or comes back to a row it has played. That is the
//! measurement libopenmpt makes, and the corpus test compares the two:
//! `F00` is passed over as libopenmpt passes it over, where some players
//! stopped the song.

use core::fmt::Write as _;

use ade_endian::{u8_at, u16_at};

use crate::decoded::Field;

/// Where the order list and the channel tag sit.
const SONG_LENGTH_AT: usize = 950;
const ORDERS_AT: usize = 952;
const TAG_AT: usize = 1080;
const PATTERNS_AT: usize = 1084;

/// The 31 sample headers, from byte 20, 30 bytes each.
const SAMPLES: usize = 31;
const SAMPLE_AT: usize = 20;
const SAMPLE_LEN: usize = 30;

/// Rows in a pattern, and the most positions a song has.
const ROWS: usize = 64;
const POSITIONS: usize = 128;

/// More rows than any song plays before it ends or repeats: the walk stops
/// here rather than trust a file to end.
const MAX_ROWS: usize = 128 * 64 * 16;

/// How many of each sample's bytes are kept, for drawing.
const KEPT: usize = 4096;

/// One sample: an instrument's sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// Its name, 22 bytes, which composers often used for messages.
    pub name: String,
    /// Its length in bytes.
    pub length: usize,
    /// Fine tuning, in eighths of a semitone, -8 to 7.
    pub finetune: i8,
    /// Its volume, 0 to 64.
    pub volume: u8,
    /// Where its loop starts, and how long it is, in bytes. A loop of two
    /// bytes or less is no loop.
    pub repeat: (usize, usize),
    /// The start of its data, to `KEPT` bytes, for drawing.
    pub data: Vec<i8>,
}

impl Sample {
    /// Whether the sample loops when held.
    #[must_use]
    pub const fn loops(&self) -> bool {
        self.repeat.1 > 2
    }
}

/// A decoded module.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// Its title.
    pub title: String,
    /// The channel tag at byte 1080.
    pub tag: [u8; 4],
    /// How many channels its patterns hold.
    pub channels: usize,
    /// How many positions the song plays.
    pub song_length: usize,
    /// The patterns, in the order the song plays them.
    pub orders: Vec<u8>,
    /// How many patterns the file holds.
    pub patterns: usize,
    /// All 31 samples, empty ones included, in order.
    pub samples: Vec<Sample>,
    /// How long the song plays, in seconds, until it ends or repeats.
    pub seconds: f64,
    /// Whether it ends by coming back to a row it has played, rather than
    /// by running out of positions.
    pub loops: bool,
    /// How many bytes of pattern and sample data the file is short of.
    pub short_by: usize,
}

/// Why bytes are not a module ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAModule(pub String);

impl core::fmt::Display for NotAModule {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotAModule {}

/// How many channels a tag says, if it is one ADE reads.
fn channels(tag: &[u8]) -> Option<usize> {
    match tag {
        b"M.K." | b"M!K!" | b"FLT4" | b"4CHN" => Some(4),
        b"6CHN" => Some(6),
        b"8CHN" => Some(8),
        _ => None,
    }
}

/// Whether `bytes` are a module.
#[must_use]
pub fn is_module(bytes: &[u8]) -> bool {
    bytes.len() > PATTERNS_AT && bytes.get(TAG_AT..PATTERNS_AT).and_then(channels).is_some()
}

/// Latin-1 text, to the first NUL, with control codes left out.
fn text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .filter(|&&b| b >= 0x20 && b != 0x7F)
        .map(|&b| char::from(b))
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Read a module.
///
/// # Errors
/// [`NotAModule`] when it is not one.
pub fn parse(bytes: &[u8]) -> Result<Module, NotAModule> {
    let tag_bytes = bytes
        .get(TAG_AT..PATTERNS_AT)
        .ok_or_else(|| NotAModule("shorter than a module's header".to_owned()))?;
    let channels = channels(tag_bytes)
        .ok_or_else(|| NotAModule("no channel tag ADE reads at byte 1080".to_owned()))?;
    let mut tag = [0u8; 4];
    tag.copy_from_slice(tag_bytes);

    let song_length = usize::from(u8_at(bytes, SONG_LENGTH_AT).unwrap_or(0)).min(POSITIONS);
    let all_orders = bytes
        .get(ORDERS_AT..ORDERS_AT.saturating_add(POSITIONS))
        .unwrap_or(&[]);
    // As ProTracker counts them: every entry of the list, played or not.
    let patterns = all_orders
        .iter()
        .copied()
        .max()
        .map_or(0, |m| usize::from(m).saturating_add(1));
    let orders = all_orders.get(..song_length).unwrap_or(&[]).to_vec();
    let row_bytes = channels.saturating_mul(4);
    let pattern_bytes = ROWS.saturating_mul(row_bytes);

    let mut data_at = PATTERNS_AT.saturating_add(patterns.saturating_mul(pattern_bytes));
    let mut short_by = data_at.saturating_sub(bytes.len());
    let mut samples = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let at = SAMPLE_AT.saturating_add(i.saturating_mul(SAMPLE_LEN));
        let word = |off: usize| {
            usize::from(u16_at(bytes, at.saturating_add(off)).unwrap_or(0)).saturating_mul(2)
        };
        let length = word(22);
        let nibble = u8_at(bytes, at.saturating_add(24)).unwrap_or(0) & 0x0F;
        // A signed nibble: 8 to 15 are -8 to -1.
        let finetune = i8::try_from(nibble)
            .unwrap_or(0)
            .wrapping_sub(if nibble > 7 { 16 } else { 0 });
        let data_end = data_at.saturating_add(length);
        let present = bytes
            .get(data_at.min(bytes.len())..data_end.min(bytes.len()))
            .unwrap_or(&[]);
        short_by = short_by.saturating_add(length.saturating_sub(present.len()));
        samples.push(Sample {
            name: text(bytes.get(at..at.saturating_add(22)).unwrap_or(&[])),
            length,
            finetune,
            volume: u8_at(bytes, at.saturating_add(25)).unwrap_or(0).min(64),
            repeat: (word(26), word(28)),
            data: present
                .iter()
                .take(KEPT)
                .map(|&b| i8::from_ne_bytes([b]))
                .collect(),
        });
        data_at = data_end;
    }
    let pattern_data = bytes.get(PATTERNS_AT..).unwrap_or(&[]);
    let (seconds, loops) = play(pattern_data, &orders, channels);
    Ok(Module {
        title: text(bytes.get(..20).unwrap_or(&[])),
        tag,
        channels,
        song_length,
        orders,
        patterns,
        samples,
        seconds,
        loops,
        short_by,
    })
}

/// Play the song without sound: how many seconds until it ends or comes back
/// to a row it has played, and which of the two.
fn play(patterns: &[u8], orders: &[u8], channels: usize) -> (f64, bool) {
    let row_bytes = channels.saturating_mul(4);
    let (mut speed, mut tempo) = (6u32, 125u32);
    let (mut position, mut row) = (0usize, 0usize);
    let mut seconds = 0.0f64;
    let mut seen = std::collections::HashSet::new();
    let mut loop_row = vec![0usize; channels];
    let mut loop_count = vec![0u8; channels];
    for _ in 0..MAX_ROWS {
        let Some(&pattern) = orders.get(position) else {
            return (seconds, false);
        };
        // A row met again is the song repeating — unless a pattern loop is
        // counting, which plays rows again on purpose.
        if loop_count.iter().all(|&c| c == 0) && !seen.insert((position, row)) {
            return (seconds, true);
        }
        let at = usize::from(pattern)
            .saturating_mul(ROWS)
            .saturating_add(row)
            .saturating_mul(row_bytes);
        let cells = patterns
            .get(at..at.saturating_add(row_bytes))
            .unwrap_or(&[]);
        let (mut jump, mut breaks, mut delay) = (None, None, 0u32);
        let mut loop_to = None;
        for (ch, cell) in cells.chunks_exact(4).enumerate() {
            let effect = u8_at(cell, 2).unwrap_or(0) & 0x0F;
            let param = u8_at(cell, 3).unwrap_or(0);
            match effect {
                0x0B => jump = Some(usize::from(param)),
                0x0D => {
                    // Two decimal digits, not a byte.
                    let target = usize::from(param >> 4)
                        .saturating_mul(10)
                        .saturating_add(usize::from(param & 0x0F));
                    breaks = Some(if target < ROWS { target } else { 0 });
                }
                0x0E => match param >> 4 {
                    0x6 => {
                        let times = param & 0x0F;
                        if times == 0 {
                            if let Some(r) = loop_row.get_mut(ch) {
                                *r = row;
                            }
                        } else if let (Some(count), Some(&from)) =
                            (loop_count.get_mut(ch), loop_row.get(ch))
                        {
                            if *count == 0 {
                                *count = times;
                                loop_to = Some(from);
                            } else {
                                *count = count.saturating_sub(1);
                                if *count > 0 {
                                    loop_to = Some(from);
                                }
                            }
                        }
                    }
                    0xE => delay = u32::from(param & 0x0F),
                    _ => {}
                },
                0x0F if param > 0 => {
                    if param < 0x20 {
                        speed = u32::from(param);
                    } else {
                        tempo = u32::from(param);
                    }
                }
                // `F00` sets no speed, and is passed over as libopenmpt
                // passes it over: *Home Alone*'s silent module has it on
                // every row of a pattern, which plays for 7.68 seconds.
                _ => {}
            }
        }
        // A row lasts `speed` ticks of 2.5 / tempo seconds each, and a
        // pattern delay plays it that many times more.
        seconds +=
            f64::from(speed.saturating_mul(delay.saturating_add(1))) * 2.5 / f64::from(tempo);

        if let Some(r) = loop_to {
            row = r;
            continue;
        }
        match (jump, breaks) {
            (Some(p), b) => {
                position = p;
                row = b.unwrap_or(0);
            }
            (None, Some(b)) => {
                position = position.saturating_add(1);
                row = b;
            }
            (None, None) => {
                row = row.saturating_add(1);
                if row == ROWS {
                    row = 0;
                    position = position.saturating_add(1);
                }
            }
        }
    }
    (seconds, true)
}

impl Module {
    /// The samples with something in them, by number from 1.
    pub fn used(&self) -> impl Iterator<Item = (usize, &Sample)> {
        self.samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.length > 2)
            .map(|(i, s)| (i.saturating_add(1), s))
    }

    /// What the module says, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const M: &str = "Module";
        let minutes = (self.seconds / 60.0).floor();
        let mut out = vec![
            Field::new(M, "Title", self.title.clone()),
            Field::new(
                M,
                "Format",
                format!(
                    "{} channels, tagged \"{}\"",
                    self.channels,
                    self.tag.iter().map(|&b| char::from(b)).collect::<String>()
                ),
            ),
            Field::new(
                M,
                "Plays for",
                format!(
                    "{}:{:04.1}{}",
                    minutes,
                    self.seconds - minutes * 60.0,
                    if self.loops { ", then repeats" } else { "" }
                ),
            ),
            Field::new(M, "Song", format!("{} positions", self.song_length)),
            Field::new(M, "Patterns", self.patterns.to_string()),
            Field::new(M, "Samples", self.used().count().to_string()),
        ];
        if self.short_by > 0 {
            out.push(Field::new(
                M,
                "Missing",
                format!("the file is {} bytes short of its samples", self.short_by),
            ));
        }
        // Names first, because composers used them to write: greetings,
        // credits, messages, often across several otherwise empty samples.
        for (i, s) in self.samples.iter().enumerate() {
            if s.name.is_empty() && s.length <= 2 {
                continue;
            }
            let what = if s.length > 2 {
                let mut v = format!("{} bytes, volume {}", s.length, s.volume);
                if s.finetune != 0 {
                    let _ = write!(v, ", finetune {:+}", s.finetune);
                }
                if s.loops() {
                    let _ = write!(v, ", loops {} bytes from {}", s.repeat.1, s.repeat.0);
                }
                v
            } else {
                "no sound".to_owned()
            };
            let label = format!("{:2}", i.saturating_add(1));
            let value = if s.name.is_empty() {
                what
            } else {
                format!("{} — {what}", s.name)
            };
            out.push(Field::new("Samples", label, value));
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
    clippy::float_cmp,
    reason = "tests over bytes they build, by hand rather than with the helpers that read \
              them, so the two cannot agree about a field both have wrong (C-001's exception)"
)]
mod tests {
    use super::*;

    /// A four-channel module titled "tune" whose song is `orders`, with the
    /// patterns it names, whose effects are `effects` — (pattern, row, channel,
    /// effect, parameter) — and one sample, "beep", of 8 bytes looping its
    /// last 4. `cut` bytes are taken off the end.
    fn module(orders: &[u8], effects: &[(usize, usize, usize, u8, u8)], cut: usize) -> Vec<u8> {
        let mut m = vec![0u8; 1084];
        m[..4].copy_from_slice(b"tune");
        let s = 20;
        m[s..s + 4].copy_from_slice(b"beep");
        m[s + 22..s + 24].copy_from_slice(&4u16.to_be_bytes()); // 8 bytes
        m[s + 24] = 0x0F; // finetune -1
        m[s + 25] = 48;
        m[s + 26..s + 28].copy_from_slice(&2u16.to_be_bytes()); // loop from 4
        m[s + 28..s + 30].copy_from_slice(&2u16.to_be_bytes()); // 4 bytes
        m[950] = orders.len() as u8;
        m[952..952 + orders.len()].copy_from_slice(orders);
        m[1080..1084].copy_from_slice(b"M.K.");
        let count = usize::from(orders.iter().copied().max().unwrap_or(0)) + 1;
        let mut patterns = vec![0u8; count * 1024];
        for &(p, row, ch, effect, param) in effects {
            let at = p * 1024 + row * 16 + ch * 4;
            patterns[at + 2] = effect;
            patterns[at + 3] = param;
        }
        m.extend(patterns);
        m.extend([0, 40, 80, 120, 127, 64, 0, 0xC0]);
        m.truncate(m.len() - cut);
        m
    }

    /// One row at speed 6 and tempo 125.
    const ROW: f64 = 6.0 * 2.5 / 125.0;

    #[test]
    fn a_module_reads_its_header_and_samples() {
        let bytes = module(&[0, 1], &[], 0);
        assert!(is_module(&bytes));
        let m = parse(&bytes).unwrap();
        assert_eq!(
            (m.title.as_str(), m.channels, m.song_length),
            ("tune", 4, 2)
        );
        assert_eq!((m.patterns, m.short_by), (2, 0));
        let s = &m.samples[0];
        assert_eq!(
            (s.name.as_str(), s.length, s.finetune, s.volume),
            ("beep", 8, -1, 48)
        );
        assert_eq!(s.repeat, (4, 4));
        assert!(s.loops());
        assert_eq!(s.data, vec![0, 40, 80, 120, 127, 64, 0, -64]);
        assert_eq!(m.used().count(), 1);
        // Two patterns of 64 rows, played through once.
        assert!((m.seconds - 128.0 * ROW).abs() < 1e-9);
        assert!(!m.loops);
    }

    #[test]
    fn speed_tempo_breaks_jumps_loops_and_delays_set_how_long_it_plays() {
        // Speed 3 from the first row, a break to row 60 of the next
        // position: 1 row at speed 3, then 4 rows.
        let m = parse(&module(
            &[0, 1],
            &[(0, 0, 0, 0xF, 3), (0, 0, 1, 0xD, 0x60)],
            0,
        ))
        .unwrap();
        let fast = 3.0 * 2.5 / 125.0;
        assert!((m.seconds - 5.0 * fast).abs() < 1e-9, "{}", m.seconds);

        // Tempo 250 halves every row.
        let m = parse(&module(&[0], &[(0, 0, 0, 0xF, 250)], 0)).unwrap();
        assert!((m.seconds - 64.0 * ROW / 2.0).abs() < 1e-9);

        // A jump back to the start on the last row: the song repeats.
        let m = parse(&module(&[0, 1], &[(1, 63, 0, 0xB, 0)], 0)).unwrap();
        assert!((m.seconds - 128.0 * ROW).abs() < 1e-9);
        assert!(m.loops);

        // Rows 0–3 looped twice more, by E60 and E62: 64 + 8 rows.
        let m = parse(&module(
            &[0],
            &[(0, 0, 0, 0xE, 0x60), (0, 3, 0, 0xE, 0x62)],
            0,
        ))
        .unwrap();
        assert!((m.seconds - 72.0 * ROW).abs() < 1e-9, "{}", m.seconds);

        // A pattern delay of 3 plays its row four times.
        let m = parse(&module(&[0], &[(0, 10, 2, 0xE, 0xE3)], 0)).unwrap();
        assert!((m.seconds - 67.0 * ROW).abs() < 1e-9);

        // F00 is passed over.
        let m = parse(&module(&[0], &[(0, 0, 0, 0xF, 0)], 0)).unwrap();
        assert!((m.seconds - 64.0 * ROW).abs() < 1e-9);
    }

    #[test]
    fn a_short_module_says_how_short_and_an_untagged_file_is_not_one() {
        let m = parse(&module(&[0], &[], 3)).unwrap();
        assert_eq!(m.short_by, 3);
        assert_eq!(m.samples[0].data.len(), 5);
        let mut untagged = module(&[0], &[], 0);
        untagged[1080..1084].copy_from_slice(b"FLT8");
        assert!(!is_module(&untagged));
        assert!(parse(&untagged).is_err());
        assert!(!is_module(&untagged[..1000]));
    }

    #[test]
    fn a_module_is_said_with_its_samples_names_first() {
        let m = parse(&module(&[0], &[], 0)).unwrap();
        let fields = m.fields();
        let value = |label: &str| {
            fields
                .iter()
                .find(|f| f.label == label)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(value("Title"), "tune");
        assert_eq!(value("Format"), "4 channels, tagged \"M.K.\"");
        assert_eq!(value("Plays for"), "0:07.7");
        assert_eq!(
            value(" 1"),
            "beep — 8 bytes, volume 48, finetune -1, loops 4 bytes from 4"
        );
    }
}
