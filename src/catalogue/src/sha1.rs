//! SHA-1 ([RFC 3174]), for separating content hashes that collide.
//!
//! # Why a second hash at all
//!
//! A CRC32 bucket can hold more than one entry, and until 2026-08-29 ADE
//! reported that as an ambiguity and declined to choose. Measuring it found
//! something better: all 77 such groups in the TOSEC Amiga set are **duplicate
//! content under different names**, every member carrying the same SHA-1 and
//! the same MD5. Zero are collisions.
//!
//! So SHA-1 is not here to break ties — it is here to say *which kind of
//! several* a match is. Duplicate names are a property of the catalogue and
//! every name is right; different content sharing a CRC32 would be a reason to
//! distrust the match entirely. Telling them apart needs a second hash, and
//! every one of the 88,921 entries already carries one.
//!
//! # Why it is written here rather than pulled in
//!
//! The workspace has no dependencies, and this is 60 lines of arithmetic from
//! a specification that has not changed since 2001. More to the point it is
//! *checkable*: RFC 3174 publishes test vectors, `sha1sum` is on every machine
//! as an independent implementation, and the corpus supplies 4.2 GB of further
//! cases. That is the same footing gzip stood on, and the reason ADE could
//! write a DEFLATE decoder without an oracle problem.
//!
//! # SHA-1 is broken, and that does not matter here
//!
//! Collisions can be *constructed* against SHA-1 — deliberately, at
//! considerable cost. This is not a security boundary: it is a lookup key for
//! naming a disk against a published dataset, and an attacker who can make ADE
//! print the wrong game title has achieved nothing. What matters is that
//! accidental collisions do not happen, and at 160 bits they do not.
//!
//! [RFC 3174]: https://www.rfc-editor.org/rfc/rfc3174

//! # Byte order goes through the seam, even here
//!
//! SHA-1 is defined in big-endian words, and C-001 admits no exception for
//! "internal to an algorithm" — that is the shape every erosion of an
//! invariant takes. The message schedule reads through `ade_endian::u32_at`
//! and the length and state are written with its writers. The rule found a
//! real slip while it was being applied: the first draft assembled the
//! schedule words by hand with shifts, which is a big-endian read the tripwire
//! cannot see.

use ade_endian::{put_u32, put_u64, u32_at};

/// A SHA-1 digest, as 20 bytes.
pub type Digest = [u8; 20];

/// The five initial state words ([RFC 3174] §6.1).
const INIT: [u32; 5] = [
    0x6745_2301,
    0xEFCD_AB89,
    0x98BA_DCFE,
    0x1032_5476,
    0xC3D2_E1F0,
];

/// The four round constants, one per twenty rounds.
const K: [u32; 4] = [0x5A82_7999, 0x6ED9_EBA1, 0x8F1B_BCDC, 0xCA62_C1D6];

/// SHA-1 of a byte slice.
#[must_use]
pub fn sha1(bytes: &[u8]) -> Digest {
    let mut state = INIT;
    let mut chunks = bytes.chunks_exact(64);
    for chunk in &mut chunks {
        compress(&mut state, chunk);
    }

    // The tail, padded: a `0x80` byte, zeros, then the length in **bits** as a
    // big-endian u64. Two blocks are needed when the remainder leaves no room
    // for the length, which is the case every naive implementation gets wrong.
    let rest = chunks.remainder();
    let mut tail = [0u8; 128];
    let len = rest.len();
    if let Some(slot) = tail.get_mut(..len) {
        slot.copy_from_slice(rest);
    }
    if let Some(byte) = tail.get_mut(len) {
        *byte = 0x80;
    }
    let blocks: usize = if len >= 56 { 2 } else { 1 };
    let bits = (bytes.len() as u64).wrapping_mul(8);
    let end = blocks.saturating_mul(64);
    let _ = put_u64(&mut tail, end.saturating_sub(8), bits);
    for block in 0..blocks {
        let at = block.saturating_mul(64);
        if let Some(chunk) = tail.get(at..at.saturating_add(64)) {
            compress(&mut state, chunk);
        }
    }

    let mut out = [0u8; 20];
    for (i, word) in state.iter().enumerate() {
        let _ = put_u32(&mut out, i.saturating_mul(4), *word);
    }
    out
}

/// One 64-byte block ([RFC 3174] §6.1). Wrapping arithmetic throughout: the
/// algorithm is defined modulo 2^32, so overflow is the specification rather
/// than a mistake.
#[allow(
    clippy::many_single_char_names,
    reason = "a, b, c, d, e are the specification's own names for the state"
)]
fn compress(state: &mut [u32; 5], block: &[u8]) {
    let mut w = [0u32; 80];
    for i in 0usize..16 {
        let Ok(value) = u32_at(block, i.saturating_mul(4)) else {
            return;
        };
        if let Some(slot) = w.get_mut(i) {
            *slot = value;
        }
    }
    for i in 16usize..80 {
        let mixed = w.get(i.saturating_sub(3)).copied().unwrap_or(0)
            ^ w.get(i.saturating_sub(8)).copied().unwrap_or(0)
            ^ w.get(i.saturating_sub(14)).copied().unwrap_or(0)
            ^ w.get(i.saturating_sub(16)).copied().unwrap_or(0);
        if let Some(slot) = w.get_mut(i) {
            *slot = mixed.rotate_left(1);
        }
    }

    let [mut a, mut b, mut c, mut d, mut e] = *state;
    for (i, word) in w.iter().enumerate() {
        let (f, k) = match i / 20 {
            0 => ((b & c) | ((!b) & d), K[0]),
            1 => (b ^ c ^ d, K[1]),
            2 => ((b & c) | (b & d) | (c & d), K[2]),
            _ => (b ^ c ^ d, K[3]),
        };
        let temp = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(k)
            .wrapping_add(*word);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = temp;
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
}

/// A digest as lowercase hex, the form every datfile writes.
#[must_use]
pub fn hex(digest: &Digest) -> String {
    let mut out = String::with_capacity(40);
    for byte in digest {
        out.push(nibble(byte >> 4));
        out.push(nibble(byte & 0x0F));
    }
    out
}

fn nibble(value: u8) -> char {
    // A lookup rather than arithmetic on a byte: the workspace denies
    // `arithmetic_side_effects`, and `b'a' + value - 10` is the sort of
    // expression that is obviously fine until the input is not what you
    // assumed.
    const HEX: [char; 16] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
    ];
    match HEX.get(value as usize) {
        Some(c) => *c,
        None => '?',
    }
}
