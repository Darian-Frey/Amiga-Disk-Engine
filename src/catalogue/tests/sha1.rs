//! SHA-1 against the specification's own vectors and against `sha1sum`.
//!
//! Two independent checks, and they answer different questions. [RFC 3174]'s
//! vectors say the algorithm is right; `sha1sum` says it stays right on inputs
//! nobody chose — including every length around the block and padding
//! boundaries, which is where a hash implementation actually breaks.
//!
//! [RFC 3174]: https://www.rfc-editor.org/rfc/rfc3174

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use std::process::{Command, Stdio};

use ade_catalogue::sha1::{hex, sha1};

fn digest(bytes: &[u8]) -> String {
    hex(&sha1(bytes))
}

#[test]
fn the_rfc_3174_test_vectors() {
    // §7.3, verbatim.
    assert_eq!(digest(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(
        digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
    assert_eq!(
        digest(&b"a".repeat(1_000_000)),
        "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
    );
    assert_eq!(
        digest(&b"0123456701234567012345670123456701234567012345670123456701234567".repeat(10)),
        "dea356a2cddd90c7a7ecedc5ebb563934f460452"
    );
}

#[test]
fn the_empty_input() {
    // Not in the RFC's list, and the one every implementation must get right
    // for free: the padding block is the whole message.
    assert_eq!(digest(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
}

/// `sha1sum` over the same bytes, or `None` when it is not installed.
fn oracle(bytes: &[u8]) -> Option<String> {
    use std::io::Write as _;
    let mut child = Command::new("sha1sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    // A writer thread, because a large input fills the pipe buffer and the
    // child cannot drain it while we are still writing — the deadlock the gzip
    // oracle test already ran into once.
    let mut stdin = child.stdin.take()?;
    let owned = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&owned);
    });
    let out = child.wait_with_output().ok()?;
    let _ = writer.join();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.split_whitespace().next().map(str::to_owned)
}

#[test]
fn every_length_across_the_block_and_padding_boundaries() {
    if oracle(b"probe").is_none() {
        eprintln!("skipping: sha1sum not installed");
        return;
    }
    // 0..200 covers three block boundaries and both padding cases — a
    // remainder that leaves room for the 8-byte length, and one that does not
    // and needs a second block. The second is where implementations break.
    for len in 0..200usize {
        let bytes: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(31) % 251) as u8).collect();
        let theirs = oracle(&bytes).expect("sha1sum ran once already");
        assert_eq!(digest(&bytes), theirs, "length {len}");
    }
}

#[test]
fn a_disk_sized_input_agrees_with_the_oracle() {
    if oracle(b"probe").is_none() {
        eprintln!("skipping: sha1sum not installed");
        return;
    }
    // 880 KB, the size this will actually be used on.
    let bytes: Vec<u8> = (0..901_120usize)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    assert_eq!(digest(&bytes), oracle(&bytes).unwrap());
}
