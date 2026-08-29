//! Searching an image for bytes (F-021).
//!
//! # Text or hex, decided from the input
//!
//! A person looking for `Copylock` types the word; a person looking for a
//! `BRA` instruction types `60 1A`. Requiring a flag to say which is asking
//! them to describe what they have already written, so the pattern is
//! classified: anything that is entirely hex digits and separators, and has an
//! even number of digits, is read as bytes — otherwise it is text.
//!
//! That rule has one deliberate consequence worth knowing. `dead` is a word
//! and also four hex digits, and it is read as **hex**, because the reverse
//! mistake is worse: someone searching for `60 1A` and silently getting the
//! ASCII of "60 1A" would find nothing and conclude the disk is clean.
//! `--text` forces the other reading.
//!
//! # Latin-1, not UTF-8
//!
//! Text is matched byte for byte in ISO 8859-1, which is what an Amiga wrote.
//! A pattern with a character above U+00FF cannot occur on a disk and matches
//! nothing rather than being lossily re-encoded into something that might.

/// What to look for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    /// The bytes to match.
    pub bytes: Vec<u8>,
    /// Whether the pattern was read as hex rather than text.
    pub is_hex: bool,
    /// Whether ASCII letters match either case.
    pub ignore_case: bool,
}

/// Why a pattern could not be understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// The pattern was empty.
    Empty,
    /// Hex digits did not pair up into bytes.
    OddHexDigits {
        /// How many digits there were.
        digits: usize,
    },
    /// A character that cannot appear on an Amiga disk.
    NotLatin1 {
        /// The offending character.
        found: char,
    },
}

impl core::fmt::Display for PatternError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str("nothing to search for"),
            Self::OddHexDigits { digits } => write!(
                f,
                "{digits} hex digits do not make whole bytes — pairs are needed"
            ),
            Self::NotLatin1 { found } => write!(
                f,
                "'{found}' cannot appear on an Amiga disk, which is ISO 8859-1"
            ),
        }
    }
}

impl core::error::Error for PatternError {}

impl Pattern {
    /// Read a pattern, deciding hex or text from its shape.
    ///
    /// # Errors
    /// [`PatternError`] if it is empty, is hex with an odd number of digits,
    /// or holds a character no Amiga disk could.
    pub fn parse(input: &str, force_text: bool, ignore_case: bool) -> Result<Self, PatternError> {
        if input.is_empty() {
            return Err(PatternError::Empty);
        }
        if !force_text && looks_like_hex(input) {
            let digits: String = input
                .trim_start_matches("0x")
                .trim_start_matches("0X")
                .chars()
                .filter(|c| !c.is_whitespace() && *c != ',')
                .collect();
            if digits.len() % 2 != 0 {
                return Err(PatternError::OddHexDigits {
                    digits: digits.len(),
                });
            }
            let mut bytes = Vec::with_capacity(digits.len() / 2);
            let chars: Vec<char> = digits.chars().collect();
            for pair in chars.chunks(2) {
                let hi = pair.first().and_then(|c| c.to_digit(16)).unwrap_or(0);
                let lo = pair.get(1).and_then(|c| c.to_digit(16)).unwrap_or(0);
                bytes.push(u8::try_from(hi.saturating_mul(16).saturating_add(lo)).unwrap_or(0));
            }
            return Ok(Self {
                bytes,
                is_hex: true,
                ignore_case: false,
            });
        }

        let mut bytes = Vec::with_capacity(input.len());
        for ch in input.chars() {
            let code = ch as u32;
            if code > 0xFF {
                return Err(PatternError::NotLatin1 { found: ch });
            }
            bytes.push(u8::try_from(code).unwrap_or(b'?'));
        }
        Ok(Self {
            bytes,
            is_hex: false,
            ignore_case,
        })
    }
}

/// Whether an input should be read as hex rather than text.
///
/// An explicit `0x` says so outright. Otherwise every character must be a hex
/// digit, a space, or a comma, and the digits must pair up into whole bytes.
/// So `60 1A`, `deadbeef` and `ff,00` are hex; `Copylock`, `dos` and `beef!`
/// are text.
///
/// The rule catches a few English words — `dead`, `face`, `added` — and reads
/// them as bytes. That is deliberate rather than tolerated: the opposite
/// mistake is worse. Someone searching for `60 1A` and silently getting the
/// ASCII of "60 1A" finds nothing and concludes the disk is clean, whereas
/// someone searching for the word `dead` sees `hex: true` in the output and
/// has `--text` to say otherwise. A wrong answer that announces itself beats a
/// wrong answer that looks like a clean result.
fn looks_like_hex(input: &str) -> bool {
    if input.starts_with("0x") || input.starts_with("0X") {
        return true;
    }
    let mut digits = 0usize;
    for ch in input.chars() {
        if ch.is_ascii_hexdigit() {
            digits = digits.saturating_add(1);
        } else if !ch.is_whitespace() && ch != ',' {
            return false;
        }
    }
    digits >= 2 && digits % 2 == 0
}

/// One place a pattern was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Byte offset into the image.
    pub offset: u64,
    /// The block it falls in.
    pub block: u64,
}

/// Every place `pattern` occurs, in offset order.
///
/// Overlapping occurrences are all reported: searching `aa` in `aaa` gives two
/// matches, because the second one is genuinely there and a caller counting
/// occurrences of a repeating byte sequence would otherwise be told a number
/// that is quietly wrong.
#[must_use]
pub fn search(bytes: &[u8], pattern: &Pattern, block_size: u32) -> Vec<Match> {
    let needle = &pattern.bytes;
    if needle.is_empty() || needle.len() > bytes.len() {
        return Vec::new();
    }
    let block_size = u64::from(block_size.max(1));
    let mut out = Vec::new();
    let last = bytes.len().saturating_sub(needle.len());

    for at in 0..=last {
        let Some(window) = bytes.get(at..at.saturating_add(needle.len())) else {
            break;
        };
        let hit = if pattern.ignore_case {
            window
                .iter()
                .zip(needle.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
        } else {
            window == needle.as_slice()
        };
        if hit {
            let offset = at as u64;
            out.push(Match {
                offset,
                block: offset.checked_div(block_size).unwrap_or(0),
            });
        }
    }
    out
}
