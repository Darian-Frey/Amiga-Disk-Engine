//! Gzip compression ([RFC 1951], [RFC 1952]), for writing ADZ and HDZ.
//!
//! One block of fixed Huffman codes over hash-chained matches, or stored
//! blocks where that would be larger. Simpler than what `gzip` does and less
//! tight, which on disk images — mostly runs, or data already packed — costs
//! little; what matters is that any inflater gives the bytes back exactly.
//! ADE's own and `gzip` itself are both asked to in the tests.
//!
//! [RFC 1951]: https://www.rfc-editor.org/rfc/rfc1951
//! [RFC 1952]: https://www.rfc-editor.org/rfc/rfc1952

use crate::inflate::{DIST_BASE, DIST_EXTRA, LENGTH_BASE, LENGTH_EXTRA, crc32};

/// How far back a match may reach.
const WINDOW: usize = 32 * 1024;
/// The longest match the format can express, and the shortest.
const LONGEST: usize = 258;
const SHORTEST: usize = 3;
/// How many earlier places with the same three bytes are tried.
const TRIES: usize = 32;
const HASH_SIZE: usize = 1 << 15;
/// The most a stored block holds.
const STORED: usize = 0xFFFF;
/// No earlier place.
const NONE: u32 = u32::MAX;

/// Compress `data` as a gzip file.
///
/// `None` past 4 GB, which gzip's length field cannot hold.
#[must_use]
pub fn gzip(data: &[u8]) -> Option<Vec<u8>> {
    let size = u32::try_from(data.len()).ok()?;
    // No name and no time: the same bytes always give the same file. 255: an
    // unknown operating system.
    let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 255];
    let packed = fixed(data);
    let blocks = data.len().div_ceil(STORED).max(1);
    if packed.len() > data.len().saturating_add(blocks.saturating_mul(5)) {
        stored(data, &mut out);
    } else {
        out.extend_from_slice(&packed);
    }
    little(&mut out, crc32(data), 4);
    little(&mut out, size, 4);
    Some(out)
}

/// The low `bytes` of `value`, least significant first: gzip's order, where
/// everything Amiga is the other way and goes through `ade-endian` (C-001).
fn little(out: &mut Vec<u8>, value: u32, bytes: u32) {
    for i in 0..bytes {
        let byte = value.wrapping_shr(i.saturating_mul(8)) & 0xFF;
        out.push(u8::try_from(byte).unwrap_or(0));
    }
}

/// `data` in stored blocks.
fn stored(data: &[u8], out: &mut Vec<u8>) {
    let mut rest = data;
    loop {
        let (piece, after) = rest.split_at(rest.len().min(STORED));
        out.push(u8::from(after.is_empty()));
        let len = u32::try_from(piece.len()).unwrap_or(0);
        little(out, len, 2);
        little(out, !len, 2);
        out.extend_from_slice(piece);
        if after.is_empty() {
            return;
        }
        rest = after;
    }
}

/// Bits, least significant first, as the format packs them.
struct Bits {
    out: Vec<u8>,
    held: u64,
    count: u32,
}

impl Bits {
    /// The low `n` bits of `value`, `n` at most 16.
    fn put(&mut self, value: u32, n: u32) {
        self.held |= u64::from(value).wrapping_shl(self.count);
        self.count = self.count.saturating_add(n);
        while self.count >= 8 {
            self.out.push(u8::try_from(self.held & 0xFF).unwrap_or(0));
            self.held >>= 8;
            self.count = self.count.saturating_sub(8);
        }
    }

    /// A Huffman code, which goes most significant bit first.
    fn code(&mut self, code: u32, n: u32) {
        let reversed = code.reverse_bits().wrapping_shr(32u32.saturating_sub(n));
        self.put(reversed, n);
    }

    /// A literal or length symbol in the fixed code (RFC 1951 §3.2.6).
    fn symbol(&mut self, s: u32) {
        match s {
            0..=143 => self.code(s.saturating_add(0x30), 8),
            144..=255 => self.code(s.saturating_add(0x190 - 144), 9),
            256..=279 => self.code(s.saturating_sub(256), 7),
            _ => self.code(s.saturating_add(0xC0).saturating_sub(280), 8),
        }
    }

    /// A match of `length` bytes from `distance` back.
    fn pair(&mut self, length: usize, distance: usize) {
        let (i, over) = slot(&LENGTH_BASE, length);
        self.symbol(u32::try_from(i).unwrap_or(0).saturating_add(257));
        self.put(over, LENGTH_EXTRA.get(i).copied().unwrap_or(0));
        let (i, over) = slot(&DIST_BASE, distance);
        self.code(u32::try_from(i).unwrap_or(0), 5);
        self.put(over, DIST_EXTRA.get(i).copied().unwrap_or(0));
    }
}

/// The last entry of `bases` not above `value`, and how far above it that is.
fn slot(bases: &[u16], value: usize) -> (usize, u32) {
    let i = bases
        .partition_point(|&b| usize::from(b) <= value)
        .saturating_sub(1);
    let base = bases.get(i).copied().map_or(0, usize::from);
    (i, u32::try_from(value.saturating_sub(base)).unwrap_or(0))
}

/// The bucket of the three bytes at `at`, if there are three.
fn hash(data: &[u8], at: usize) -> Option<usize> {
    let &[a, b, c] = data.get(at..at.checked_add(SHORTEST)?)? else {
        return None;
    };
    let key = u32::from(a) | u32::from(b) << 8 | u32::from(c) << 16;
    usize::try_from(key.wrapping_mul(0x9E37_79B1) >> 17).ok()
}

/// Where each three bytes were last seen, and where before that.
struct Seen {
    head: Vec<u32>,
    before: Vec<u32>,
}

impl Seen {
    fn add(&mut self, data: &[u8], at: usize) {
        if let Some(h) = hash(data, at)
            && let Some(head) = self.head.get_mut(h)
            && let Some(before) = self.before.get_mut(at % WINDOW)
        {
            *before = *head;
            *head = u32::try_from(at).unwrap_or(NONE);
        }
    }

    /// The longest match for `at` among the places tried: length, distance.
    fn best(&self, data: &[u8], at: usize) -> Option<(usize, usize)> {
        let here = data.get(at..)?;
        let here = here.get(..here.len().min(LONGEST))?;
        let mut place = self.head.get(hash(data, at)?).copied()?;
        let mut best: Option<(usize, usize)> = None;
        for _ in 0..TRIES {
            let from = usize::try_from(place).ok()?;
            let distance = at.saturating_sub(from);
            if place == NONE || distance == 0 || distance > WINDOW {
                break;
            }
            let there = data.get(from..)?;
            let length = here.iter().zip(there).take_while(|(a, b)| a == b).count();
            if length >= SHORTEST && best.is_none_or(|(l, _)| length > l) {
                best = Some((length, distance));
                if length == here.len() {
                    break;
                }
            }
            let earlier = self.before.get(from % WINDOW).copied()?;
            // A slot written over since points forward, not back.
            if earlier >= place {
                break;
            }
            place = earlier;
        }
        best
    }
}

/// `data` as one block of the fixed code.
fn fixed(data: &[u8]) -> Vec<u8> {
    let mut bits = Bits {
        out: Vec::new(),
        held: 0,
        count: 0,
    };
    bits.put(1, 1); // the last block
    bits.put(1, 2); // fixed codes
    let mut seen = Seen {
        head: vec![NONE; HASH_SIZE],
        before: vec![NONE; WINDOW],
    };
    let mut at = 0usize;
    while let Some(&byte) = data.get(at) {
        let length = if let Some((length, distance)) = seen.best(data, at) {
            bits.pair(length, distance);
            length
        } else {
            bits.symbol(u32::from(byte));
            1
        };
        for i in 0..length {
            seen.add(data, at.saturating_add(i));
        }
        at = at.saturating_add(length);
    }
    bits.symbol(256);
    bits.put(0, 7); // to the end of the byte
    bits.out
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    reason = "tests may unwrap and count"
)]
mod tests {
    use super::*;
    use crate::inflate::gunzip;

    fn noise(len: usize) -> Vec<u8> {
        let mut x = 1u32;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                u8::try_from(x & 0xFF).unwrap()
            })
            .collect()
    }

    fn cases() -> Vec<Vec<u8>> {
        let mut out = vec![Vec::new(), b"a".to_vec(), b"ab".to_vec(), noise(200_000)];
        // Runs of every length about the longest match, and every byte.
        out.push((0..=255u8).cycle().take(5000).collect());
        for n in [
            3, 4, 257, 258, 259, 260, 516, 517, 65_535, 65_536, 1_000_000,
        ] {
            out.push(vec![0u8; n]);
        }
        // A repeat at, just inside and just past the window.
        for gap in [WINDOW - 600, WINDOW - 500, WINDOW - 1, WINDOW, WINDOW + 1] {
            let mut d = noise(500);
            d.extend(vec![7u8; gap]);
            d.extend(noise(500));
            out.push(d);
        }
        // Every distance class, by text repeated at growing spacings.
        let mut d = Vec::new();
        for gap in (1..60).map(|i| i * i * 9) {
            d.extend_from_slice(b"the same words");
            d.extend(noise(gap));
        }
        out.push(d);
        out
    }

    #[test]
    fn what_is_compressed_inflates_to_itself() {
        for d in cases() {
            let z = gzip(&d).unwrap();
            assert!(gunzip(&z, 2_000_000).unwrap() == d, "{} bytes", d.len());
        }
    }

    #[test]
    fn runs_shrink_and_noise_hardly_grows() {
        assert!(gzip(&vec![0u8; 1_000_000]).unwrap().len() < 10_000);
        assert!(gzip(&noise(1_000_000)).unwrap().len() < 1_000_200);
    }

    #[test]
    fn gzip_itself_reads_it() {
        use std::io::Write as _;
        use std::process::{Command, Stdio};
        for d in cases() {
            let Ok(mut child) = Command::new("gzip")
                .arg("-dc")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
            else {
                return;
            };
            let z = gzip(&d).unwrap();
            let mut to = child.stdin.take().unwrap();
            let feeder = std::thread::spawn(move || to.write_all(&z));
            let got = child.wait_with_output().unwrap();
            let _ = feeder.join();
            assert!(got.status.success() && got.stdout == d, "{} bytes", d.len());
        }
    }
}
