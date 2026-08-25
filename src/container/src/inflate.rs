//! DEFLATE decompression ([RFC 1951]) and the gzip wrapper ([RFC 1952]).
//!
//! ADZ and HDZ are gzip-wrapped ADF and HDF — nothing more. Reading them needs
//! an inflater, and with no external dependencies (the workspace has none) that
//! means writing one.
//!
//! # Why this is written defensively even by this project's standards
//!
//! A decompressor is **the** AV-005 surface. Every length it acts on comes from
//! the input, and a few bytes of it can ask for gigabytes of output — that is
//! the format working as designed, not a corruption. So:
//!
//! - Output is **capped up front** and the cap is checked before each write,
//!   never after. A limit tested afterwards has already allocated.
//! - Nothing is reserved from a declared size. gzip's `ISIZE` trailer states
//!   the original length, and believing it would be BUG-003 again with the
//!   attacker holding the pen; it is *verified* at the end, never trusted at
//!   the start.
//! - Every arithmetic operation that could overflow is checked, and an overflow
//!   is treated as corrupt input rather than wrapped silently.
//!
//! # Scope
//!
//! Decompression only. ADE does not write compressed images: D-004 defers write
//! paths to Phase 4, and a compressor is not needed to read a corpus.
//!
//! [RFC 1951]: https://www.rfc-editor.org/rfc/rfc1951
//! [RFC 1952]: https://www.rfc-editor.org/rfc/rfc1952

/// Longest Huffman code DEFLATE permits (RFC 1951 §3.2.7).
const MAX_BITS: usize = 15;

/// Symbols in the literal/length alphabet: 0–255 literals, 256 end-of-block,
/// 257–285 lengths, and two unused slots the format reserves.
const LITLEN_SYMBOLS: usize = 288;

/// Symbols in the distance alphabet.
const DIST_SYMBOLS: usize = 30;

/// Symbols in the code-length alphabet used by dynamic blocks.
const CODELEN_SYMBOLS: usize = 19;

/// The order code lengths are stored in for a dynamic block (RFC 1951 §3.2.7).
///
/// Not sorted: the lengths most likely to be zero come last, so a truncated
/// list still describes a usable alphabet.
const CODELEN_ORDER: [usize; CODELEN_SYMBOLS] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Base match length for each length symbol 257–285 (RFC 1951 §3.2.5).
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

/// Extra bits read after each length symbol.
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// Base match distance for each distance symbol (RFC 1951 §3.2.5).
const DIST_BASE: [u16; DIST_SYMBOLS] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

/// Extra bits read after each distance symbol.
const DIST_EXTRA: [u32; DIST_SYMBOLS] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Why decompression stopped.
///
/// Every variant is a statement about the *input*, not about ADE. A caller
/// showing one of these to a user is telling them something about their file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InflateError {
    /// The stream ended while more was expected.
    Truncated {
        /// How far in, in bytes.
        at: usize,
    },
    /// A block header named the reserved block type 3.
    ReservedBlockType {
        /// Where the block began, in bytes.
        at: usize,
    },
    /// A stored block's length and its one's-complement disagree — the format's
    /// own integrity check on the one block type that has no other.
    StoredLengthMismatch {
        /// The declared length.
        len: u16,
        /// The complement, which should have been `!len`.
        nlen: u16,
    },
    /// A Huffman code was not in the alphabet it was decoded against.
    BadCode {
        /// Which alphabet.
        alphabet: &'static str,
    },
    /// A code-length alphabet was incomplete or over-subscribed.
    BadCodeLengths {
        /// What was wrong.
        detail: &'static str,
    },
    /// A back-reference pointed before the start of the output.
    ///
    /// Legal DEFLATE never does this; it is the decompressor's equivalent of
    /// AV-004, and the check is the reason a crafted stream cannot read
    /// out-of-bounds memory here.
    DistanceTooFar {
        /// The distance asked for.
        distance: usize,
        /// How much output existed.
        available: usize,
    },
    /// The output would exceed the cap the caller set (AV-005).
    ///
    /// Not necessarily an attack: a legitimate 100 MB HDZ hits this if the cap
    /// was set for floppies. The cap is the caller's policy, and this says it
    /// was reached rather than guessing at intent.
    OutputTooLarge {
        /// The cap.
        limit: usize,
    },
    /// The gzip header is not a gzip header.
    NotGzip,
    /// The gzip header uses a compression method other than DEFLATE.
    UnsupportedMethod {
        /// The method byte.
        method: u8,
    },
    /// The trailer's CRC32 does not match the data that was decompressed.
    ChecksumMismatch {
        /// What the trailer claimed.
        expected: u32,
        /// What the output actually hashes to.
        actual: u32,
    },
    /// The trailer's length does not match the data that was decompressed.
    LengthMismatch {
        /// What the trailer claimed, modulo 2^32.
        expected: u32,
        /// How many bytes came out.
        actual: u64,
    },
}

impl core::fmt::Display for InflateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Truncated { at } => write!(f, "compressed stream ends early, at byte {at}"),
            Self::ReservedBlockType { at } => {
                write!(f, "reserved block type at byte {at} — not a DEFLATE stream")
            }
            Self::StoredLengthMismatch { len, nlen } => write!(
                f,
                "stored block length {len} does not match its complement {nlen}"
            ),
            Self::BadCode { alphabet } => write!(f, "invalid {alphabet} code"),
            Self::BadCodeLengths { detail } => write!(f, "invalid code lengths: {detail}"),
            Self::DistanceTooFar {
                distance,
                available,
            } => write!(
                f,
                "back-reference {distance} bytes back, with only {available} bytes decompressed"
            ),
            Self::OutputTooLarge { limit } => {
                write!(f, "decompressed output exceeds the {limit}-byte limit")
            }
            Self::NotGzip => f.write_str("not a gzip stream"),
            Self::UnsupportedMethod { method } => {
                write!(f, "gzip compression method {method} is not DEFLATE")
            }
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "checksum mismatch: the trailer says {expected:#010x}, the data hashes to \
                 {actual:#010x} — the image is corrupt"
            ),
            Self::LengthMismatch { expected, actual } => write!(
                f,
                "length mismatch: the trailer says {expected} bytes, {actual} came out"
            ),
        }
    }
}

impl core::error::Error for InflateError {}

/// Reads bits least-significant-first, as DEFLATE stores them.
///
/// Returns an error rather than zeros at the end of input: a truncated stream
/// that silently reads as zero bits decodes into plausible-looking garbage,
/// which is worse than failing.
struct BitReader<'a> {
    data: &'a [u8],
    /// Next byte to consume.
    pos: usize,
    /// Bits held but not yet consumed, LSB first.
    bits: u32,
    /// How many of `bits` are valid.
    count: u32,
}

impl<'a> BitReader<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bits: 0,
            count: 0,
        }
    }

    /// Read `n` bits (at most 24), LSB first.
    fn bits(&mut self, n: u32) -> Result<u32, InflateError> {
        while self.count < n {
            let byte = *self
                .data
                .get(self.pos)
                .ok_or(InflateError::Truncated { at: self.pos })?;
            self.pos = self.pos.saturating_add(1);
            self.bits |= u32::from(byte).checked_shl(self.count).unwrap_or(0);
            self.count = self.count.saturating_add(8);
        }
        let mask = 1u32.checked_shl(n).unwrap_or(0).wrapping_sub(1);
        let value = self.bits & mask;
        self.bits = self.bits.checked_shr(n).unwrap_or(0);
        self.count = self.count.saturating_sub(n);
        Ok(value)
    }

    /// Discard bits up to the next byte boundary, as a stored block requires.
    const fn align(&mut self) {
        self.bits = 0;
        self.count = 0;
    }

    /// How many whole bytes have been consumed, for error reporting.
    const fn byte_position(&self) -> usize {
        self.pos
    }

    /// Take `n` bytes directly, for a stored block.
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], InflateError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(InflateError::Truncated { at: self.pos })?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or(InflateError::Truncated { at: self.pos })?;
        self.pos = end;
        Ok(slice)
    }
}

/// A canonical Huffman alphabet, in the shape RFC 1951 §3.2.2 describes.
///
/// Codes are never materialised: the counts per length and the symbols in
/// canonical order are enough to decode, and building an explicit table would
/// mean allocating from a length that came off the input.
struct Huffman {
    /// How many codes have each length, indexed by length.
    counts: [u16; MAX_BITS + 1],
    /// Symbols ordered by code length, then by symbol value.
    symbols: Vec<u16>,
    /// Which alphabet this is, for error messages.
    name: &'static str,
}

impl Huffman {
    /// Build from a list of code lengths, one per symbol. Length 0 means the
    /// symbol is unused.
    fn new(lengths: &[u8], name: &'static str) -> Result<Self, InflateError> {
        let mut counts = [0u16; MAX_BITS + 1];
        for &length in lengths {
            let index = usize::from(length);
            let slot = counts.get_mut(index).ok_or(InflateError::BadCodeLengths {
                detail: "code longer than 15 bits",
            })?;
            *slot = slot.saturating_add(1);
        }
        // Length 0 is "absent", not a code.
        if let Some(zero) = counts.get_mut(0) {
            *zero = 0;
        }

        // Reject an over-subscribed alphabet: more codes of some length than
        // the tree can hold. An incomplete one is tolerated here because
        // DEFLATE permits a single-symbol distance alphabet.
        let mut left: i32 = 1;
        for length in 1..=MAX_BITS {
            left = left.checked_mul(2).ok_or(InflateError::BadCodeLengths {
                detail: "code length tree overflowed",
            })?;
            let count = i32::from(*counts.get(length).unwrap_or(&0));
            left = left
                .checked_sub(count)
                .ok_or(InflateError::BadCodeLengths {
                    detail: "over-subscribed code lengths",
                })?;
            if left < 0 {
                return Err(InflateError::BadCodeLengths {
                    detail: "over-subscribed code lengths",
                });
            }
        }

        // Canonical order: symbols sorted by code length, then by value.
        let mut offsets = [0u16; MAX_BITS + 2];
        for length in 1..=MAX_BITS {
            let previous = *offsets.get(length).unwrap_or(&0);
            let count = *counts.get(length).unwrap_or(&0);
            if let Some(next) = offsets.get_mut(length.saturating_add(1)) {
                *next = previous.saturating_add(count);
            }
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let index = usize::from(length);
            let Some(offset) = offsets.get_mut(index) else {
                continue;
            };
            let at = usize::from(*offset);
            *offset = offset.saturating_add(1);
            if let (Some(slot), Ok(value)) = (symbols.get_mut(at), u16::try_from(symbol)) {
                *slot = value;
            }
        }

        Ok(Self {
            counts,
            symbols,
            name,
        })
    }

    /// Decode one symbol, consuming bits until a code matches.
    ///
    /// Walks lengths shortest-first, which is what makes the canonical form
    /// decodable without a table: at each length, the codes of that length
    /// occupy a contiguous numeric range.
    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, InflateError> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;

        for length in 1..=MAX_BITS {
            let bit = i32::try_from(reader.bits(1)?).unwrap_or(0);
            code |= bit;
            let count = i32::from(*self.counts.get(length).unwrap_or(&0));
            let offset = code.checked_sub(first).unwrap_or(i32::MAX);
            if offset < count {
                let at = index
                    .checked_add(offset)
                    .and_then(|i| usize::try_from(i).ok());
                return at.and_then(|i| self.symbols.get(i)).copied().ok_or(
                    InflateError::BadCode {
                        alphabet: self.name,
                    },
                );
            }
            index = index.saturating_add(count);
            first = first.saturating_add(count).saturating_mul(2);
            code = code.saturating_mul(2);
        }
        Err(InflateError::BadCode {
            alphabet: self.name,
        })
    }
}

/// The fixed literal/length alphabet every DEFLATE stream may use (§3.2.6).
fn fixed_litlen() -> Result<Huffman, InflateError> {
    let mut lengths = [0u8; LITLEN_SYMBOLS];
    for (symbol, slot) in lengths.iter_mut().enumerate() {
        // 0..=143 and 280..=287 both take 8 bits, which is why RFC 1951 gives
        // this as a table rather than a formula: the alphabet is not ordered
        // by code length.
        *slot = match symbol {
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Huffman::new(&lengths, "literal/length")
}

/// The fixed distance alphabet: 32 codes of 5 bits each.
fn fixed_dist() -> Result<Huffman, InflateError> {
    Huffman::new(&[5u8; 32], "distance")
}

/// Decompress a raw DEFLATE stream.
///
/// `limit` caps the output. It is checked before every write, so the cap
/// bounds memory rather than merely reporting afterwards that it was exceeded
/// (AV-005).
///
/// # Errors
/// Any [`InflateError`]. Every one describes the input, not a failure in ADE.
pub fn inflate(input: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    let mut reader = BitReader::new(input);
    // Deliberately not reserved from any declared size: the only sizes
    // available come from the input (BUG-003).
    let mut out: Vec<u8> = Vec::new();

    loop {
        let final_block = reader.bits(1)? == 1;
        let block_type = reader.bits(2)?;
        match block_type {
            0 => stored_block(&mut reader, &mut out, limit)?,
            1 => {
                let litlen = fixed_litlen()?;
                let dist = fixed_dist()?;
                compressed_block(&mut reader, &mut out, limit, &litlen, &dist)?;
            }
            2 => {
                let (litlen, dist) = dynamic_tables(&mut reader)?;
                compressed_block(&mut reader, &mut out, limit, &litlen, &dist)?;
            }
            _ => {
                return Err(InflateError::ReservedBlockType {
                    at: reader.byte_position(),
                });
            }
        }
        if final_block {
            return Ok(out);
        }
    }
}

/// An uncompressed block: byte-aligned, length-prefixed, complement-checked.
fn stored_block(
    reader: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    limit: usize,
) -> Result<(), InflateError> {
    reader.align();
    let header = reader.bytes(4)?;
    let len = u16::from(*header.first().unwrap_or(&0))
        | u16::from(*header.get(1).unwrap_or(&0)).wrapping_shl(8);
    let nlen = u16::from(*header.get(2).unwrap_or(&0))
        | u16::from(*header.get(3).unwrap_or(&0)).wrapping_shl(8);
    if len != !nlen {
        return Err(InflateError::StoredLengthMismatch { len, nlen });
    }
    let wanted = usize::from(len);
    if out.len().saturating_add(wanted) > limit {
        return Err(InflateError::OutputTooLarge { limit });
    }
    let data = reader.bytes(wanted)?;
    out.extend_from_slice(data);
    Ok(())
}

/// Read the code-length alphabets a dynamic block defines for itself (§3.2.7).
fn dynamic_tables(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), InflateError> {
    let hlit = usize::try_from(reader.bits(5)?)
        .unwrap_or(0)
        .saturating_add(257);
    let hdist = usize::try_from(reader.bits(5)?)
        .unwrap_or(0)
        .saturating_add(1);
    let hclen = usize::try_from(reader.bits(4)?)
        .unwrap_or(0)
        .saturating_add(4);

    if hlit > LITLEN_SYMBOLS || hdist > DIST_SYMBOLS.saturating_add(2) {
        return Err(InflateError::BadCodeLengths {
            detail: "alphabet larger than the format allows",
        });
    }

    // The code-length alphabet, which describes the other two.
    let mut code_lengths = [0u8; CODELEN_SYMBOLS];
    for &position in CODELEN_ORDER.iter().take(hclen) {
        let value = u8::try_from(reader.bits(3)?).unwrap_or(0);
        if let Some(slot) = code_lengths.get_mut(position) {
            *slot = value;
        }
    }
    let code_length_table = Huffman::new(&code_lengths, "code length")?;

    // Both alphabets are encoded as one run, literals first.
    let total = hlit.saturating_add(hdist);
    let mut lengths = vec![0u8; total];
    let mut written = 0usize;
    while written < total {
        let symbol = code_length_table.decode(reader)?;
        match symbol {
            0..=15 => {
                if let Some(slot) = lengths.get_mut(written) {
                    *slot = u8::try_from(symbol).unwrap_or(0);
                }
                written = written.saturating_add(1);
            }
            // 16: repeat the previous length 3–6 times.
            16 => {
                let previous = written
                    .checked_sub(1)
                    .and_then(|i| lengths.get(i))
                    .copied()
                    .ok_or(InflateError::BadCodeLengths {
                        detail: "repeat with no previous length",
                    })?;
                let times = usize::try_from(reader.bits(2)?)
                    .unwrap_or(0)
                    .saturating_add(3);
                written = fill(&mut lengths, written, times, previous, total)?;
            }
            // 17 and 18: runs of zero, short and long.
            17 => {
                let times = usize::try_from(reader.bits(3)?)
                    .unwrap_or(0)
                    .saturating_add(3);
                written = fill(&mut lengths, written, times, 0, total)?;
            }
            18 => {
                let times = usize::try_from(reader.bits(7)?)
                    .unwrap_or(0)
                    .saturating_add(11);
                written = fill(&mut lengths, written, times, 0, total)?;
            }
            _ => {
                return Err(InflateError::BadCodeLengths {
                    detail: "code-length symbol out of range",
                });
            }
        }
    }

    let (litlen_lengths, dist_lengths) =
        lengths
            .split_at_checked(hlit)
            .ok_or(InflateError::BadCodeLengths {
                detail: "literal and distance lengths do not add up",
            })?;

    // A block with no matches at all encodes one unused distance code; that is
    // legal, and rejecting it would fail on legitimate streams.
    Ok((
        Huffman::new(litlen_lengths, "literal/length")?,
        Huffman::new(dist_lengths, "distance")?,
    ))
}

/// Write `times` copies of `value`, refusing to run past the declared total.
fn fill(
    lengths: &mut [u8],
    at: usize,
    times: usize,
    value: u8,
    total: usize,
) -> Result<usize, InflateError> {
    let end = at.checked_add(times).ok_or(InflateError::BadCodeLengths {
        detail: "run overflows",
    })?;
    if end > total {
        return Err(InflateError::BadCodeLengths {
            detail: "run extends past the declared alphabet",
        });
    }
    for slot in lengths.get_mut(at..end).unwrap_or(&mut []) {
        *slot = value;
    }
    Ok(end)
}

/// Decode one Huffman-coded block, fixed or dynamic — they differ only in
/// where the alphabets came from.
fn compressed_block(
    reader: &mut BitReader<'_>,
    out: &mut Vec<u8>,
    limit: usize,
    litlen: &Huffman,
    dist: &Huffman,
) -> Result<(), InflateError> {
    loop {
        let symbol = litlen.decode(reader)?;
        match symbol {
            0..=255 => {
                if out.len() >= limit {
                    return Err(InflateError::OutputTooLarge { limit });
                }
                out.push(u8::try_from(symbol).unwrap_or(0));
            }
            256 => return Ok(()),
            _ => {
                let index = usize::from(symbol).saturating_sub(257);
                let base = *LENGTH_BASE
                    .get(index)
                    .ok_or(InflateError::BadCode { alphabet: "length" })?;
                let extra = *LENGTH_EXTRA.get(index).unwrap_or(&0);
                let length = usize::from(base)
                    .saturating_add(usize::try_from(reader.bits(extra)?).unwrap_or(0));

                let dist_symbol = dist.decode(reader)?;
                let dist_index = usize::from(dist_symbol);
                let dist_base = *DIST_BASE.get(dist_index).ok_or(InflateError::BadCode {
                    alphabet: "distance",
                })?;
                let dist_extra = *DIST_EXTRA.get(dist_index).unwrap_or(&0);
                let distance = usize::from(dist_base)
                    .saturating_add(usize::try_from(reader.bits(dist_extra)?).unwrap_or(0));

                copy_back(out, distance, length, limit)?;
            }
        }
    }
}

/// Copy `length` bytes from `distance` back in the output.
///
/// Byte at a time on purpose: DEFLATE allows the run to overlap itself — a
/// distance of 1 repeating the last byte is how it encodes runs — so the
/// source grows as the copy proceeds and a bulk copy would be wrong.
fn copy_back(
    out: &mut Vec<u8>,
    distance: usize,
    length: usize,
    limit: usize,
) -> Result<(), InflateError> {
    if distance == 0 || distance > out.len() {
        return Err(InflateError::DistanceTooFar {
            distance,
            available: out.len(),
        });
    }
    if out.len().saturating_add(length) > limit {
        return Err(InflateError::OutputTooLarge { limit });
    }
    let mut from = out.len().saturating_sub(distance);
    for _ in 0..length {
        let byte = *out.get(from).ok_or(InflateError::DistanceTooFar {
            distance,
            available: out.len(),
        })?;
        out.push(byte);
        from = from.saturating_add(1);
    }
    Ok(())
}

/// The CRC32 table for the reflected polynomial `0xEDB88320`, built at compile
/// time.
///
/// A byte at a time rather than a bit at a time. The bitwise form is eight
/// times the work, which is invisible in a release build and very visible in a
/// debug one — where the tests run, and where a corpus round-trip over real
/// 900 KB images went from minutes to seconds.
#[allow(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "const-evaluated: an out-of-range index or a truncating cast here               is a compile error, not a runtime one, and `i` is bounded by 256"
)]
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// CRC32 as gzip uses it: the reflected polynomial `0xEDB88320`.
///
/// This is what makes a decompressed image trustworthy rather than merely
/// plausible — see [`gunzip`], which verifies it against the trailer.
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        // Taking the low byte is the algorithm, not a lossy conversion.
        let index = usize::try_from((crc ^ u32::from(byte)) & 0xFF).unwrap_or(0);
        let entry = CRC32_TABLE.get(index).copied().unwrap_or(0);
        crc = entry ^ (crc >> 8);
    }
    !crc
}

/// gzip's fixed header length, before any optional fields (RFC 1952 §2.3).
const GZIP_HEADER: usize = 10;

/// gzip's trailer: CRC32 then ISIZE, both little-endian.
const GZIP_TRAILER: usize = 8;

/// Decompress a gzip stream — an ADZ or HDZ.
///
/// The trailer's CRC32 and length are **verified after decompressing**, never
/// used to size anything beforehand. A caller gets either bytes that provably
/// match what was compressed, or an error.
///
/// # Errors
/// [`InflateError::NotGzip`] if the magic is wrong, [`InflateError::ChecksumMismatch`]
/// if the data does not match its trailer, or any decompression error.
pub fn gunzip(input: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    let header = input.get(..GZIP_HEADER).ok_or(InflateError::NotGzip)?;
    if header.first() != Some(&0x1F) || header.get(1) != Some(&0x8B) {
        return Err(InflateError::NotGzip);
    }
    let method = *header.get(2).unwrap_or(&0);
    if method != 8 {
        return Err(InflateError::UnsupportedMethod { method });
    }
    let flags = *header.get(3).unwrap_or(&0);

    let mut at = GZIP_HEADER;
    // FEXTRA: a length-prefixed blob.
    if flags & 0b0000_0100 != 0 {
        let len = input
            .get(at..at.saturating_add(2))
            .map(|b| {
                usize::from(*b.first().unwrap_or(&0))
                    | usize::from(*b.get(1).unwrap_or(&0)).wrapping_shl(8)
            })
            .ok_or(InflateError::Truncated { at })?;
        at = at
            .checked_add(2)
            .and_then(|a| a.checked_add(len))
            .ok_or(InflateError::Truncated { at })?;
    }
    // FNAME and FCOMMENT: NUL-terminated strings.
    for flag in [0b0000_1000u8, 0b0001_0000] {
        if flags & flag != 0 {
            at = skip_cstring(input, at)?;
        }
    }
    // FHCRC: a 16-bit header check, which we skip rather than verify — the
    // data CRC below covers everything that matters.
    if flags & 0b0000_0010 != 0 {
        at = at.checked_add(2).ok_or(InflateError::Truncated { at })?;
    }

    let end = input
        .len()
        .checked_sub(GZIP_TRAILER)
        .ok_or(InflateError::Truncated { at: input.len() })?;
    let body = input.get(at..end).ok_or(InflateError::Truncated { at })?;
    let trailer = input
        .get(end..)
        .ok_or(InflateError::Truncated { at: end })?;

    let out = inflate(body, limit)?;

    let expected_crc = le_u32(trailer, 0);
    let actual_crc = crc32(&out);
    if expected_crc != actual_crc {
        return Err(InflateError::ChecksumMismatch {
            expected: expected_crc,
            actual: actual_crc,
        });
    }
    // ISIZE is the original length modulo 2^32, so it is compared modulo 2^32.
    let expected_len = le_u32(trailer, 4);
    let actual_len = out.len() as u64;
    if expected_len != (actual_len & 0xFFFF_FFFF) as u32 {
        return Err(InflateError::LengthMismatch {
            expected: expected_len,
            actual: actual_len,
        });
    }

    Ok(out)
}

/// Advance past a NUL-terminated string, or fail if it never terminates.
fn skip_cstring(input: &[u8], at: usize) -> Result<usize, InflateError> {
    let rest = input.get(at..).ok_or(InflateError::Truncated { at })?;
    let nul = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or(InflateError::Truncated { at })?;
    at.checked_add(nul)
        .and_then(|a| a.checked_add(1))
        .ok_or(InflateError::Truncated { at })
}

/// Read a little-endian `u32` — gzip's own framing is little-endian, unlike
/// everything on an Amiga disk, which is why this does not go through
/// `ade-endian` (C-001 governs *disk* data).
fn le_u32(buf: &[u8], at: usize) -> u32 {
    let byte = |i: usize| u32::from(*buf.get(at.saturating_add(i)).unwrap_or(&0));
    byte(0) | byte(1).wrapping_shl(8) | byte(2).wrapping_shl(16) | byte(3).wrapping_shl(24)
}
