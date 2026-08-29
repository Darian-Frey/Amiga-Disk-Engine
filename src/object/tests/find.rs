//! Reading a search pattern, and finding it (F-021).
//!
//! # What these pin
//!
//! Nearly all of it is the hex-or-text decision. The search itself is a byte
//! comparison and hard to get subtly wrong; deciding what the user *meant* by
//! `dead` is where a search tool silently returns the wrong answer.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test scaffolding: a failure to set up is a test failure"
)]

use ade_object::find::{Pattern, PatternError, search};

const BLOCK: u32 = 512;

fn parse(input: &str) -> Pattern {
    Pattern::parse(input, false, false).unwrap()
}

#[test]
fn a_word_is_read_as_text() {
    let p = parse("Copylock");
    assert!(!p.is_hex);
    assert_eq!(p.bytes, b"Copylock");
}

#[test]
fn spaced_hex_pairs_are_read_as_bytes() {
    // The `BRA` a cracker's patch leaves behind. Read as text this finds
    // nothing, and nothing found reads as a clean disk.
    let p = parse("60 1A");
    assert!(p.is_hex);
    assert_eq!(p.bytes, vec![0x60, 0x1A]);
}

#[test]
fn run_together_hex_is_read_as_bytes() {
    assert_eq!(parse("deadbeef").bytes, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(parse("ff,00").bytes, vec![0xFF, 0x00]);
}

#[test]
fn an_ambiguous_word_is_read_as_hex_and_text_overrides() {
    // `dead` is a word and four hex digits. It goes to hex, because the
    // failure mode of the other choice is silent: `60 1A` read as ASCII finds
    // nothing and looks like a clean result. This choice is visible — the
    // output says `hex: true` — and `--text` reverses it.
    let guessed = parse("dead");
    assert!(guessed.is_hex);
    assert_eq!(guessed.bytes, vec![0xDE, 0xAD]);

    let forced = Pattern::parse("dead", true, false).unwrap();
    assert!(!forced.is_hex);
    assert_eq!(forced.bytes, b"dead");
}

#[test]
fn a_hex_prefix_says_so_outright() {
    let p = parse("0xC0DE");
    assert!(p.is_hex);
    assert_eq!(p.bytes, vec![0xC0, 0xDE]);
}

#[test]
fn a_word_that_is_nearly_hex_is_still_text() {
    // One non-hex character is enough. `beef!` and `dos` are words.
    assert!(!parse("beef!").is_hex);
    assert!(!parse("dos").is_hex, "three digits do not pair up");
}

#[test]
fn odd_hex_digits_are_refused_rather_than_padded() {
    // Padding would have to guess an end: is `60 1` the byte 0x60 followed by
    // a nibble, or 0x06 0x01? Refusing is the only honest answer.
    assert_eq!(
        Pattern::parse("0x601", false, false),
        Err(PatternError::OddHexDigits { digits: 3 })
    );
}

#[test]
fn an_empty_pattern_is_refused() {
    assert_eq!(Pattern::parse("", false, false), Err(PatternError::Empty));
}

#[test]
fn a_character_no_amiga_disk_could_hold_is_refused() {
    // Not lossily re-encoded into something that might match: an Amiga wrote
    // ISO 8859-1, and U+2014 is not in it.
    assert_eq!(
        Pattern::parse("em—dash", false, false),
        Err(PatternError::NotLatin1 { found: '—' })
    );
}

#[test]
fn latin1_above_ascii_is_matched_byte_for_byte() {
    let p = parse("café");
    assert_eq!(p.bytes, vec![b'c', b'a', b'f', 0xE9]);
    let mut bytes = vec![0u8; 512];
    bytes[10..14].copy_from_slice(&p.bytes);
    assert_eq!(search(&bytes, &p, BLOCK).len(), 1);
}

#[test]
fn matches_report_the_block_they_fall_in() {
    let mut bytes = vec![0u8; 4 * 512];
    bytes[1030..1034].copy_from_slice(b"DOS\0");
    let hits = search(&bytes, &parse("DOS\0"), BLOCK);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].offset, 1030);
    assert_eq!(hits[0].block, 2);
}

#[test]
fn overlapping_occurrences_are_all_reported() {
    // `aa` in `aaa` is twice. A caller counting occurrences of a repeating
    // sequence — which is what the xDMS failure filler is — would otherwise
    // be told a number that is quietly low.
    //
    // Forced to text because `aa` is also two hex digits, and this test
    // originally failed on exactly that: the rule bites on real input, which
    // is the argument for `hex: true` being in the output.
    let text = Pattern::parse("aa", true, false).unwrap();
    let bytes = b"aaa".to_vec();
    assert_eq!(search(&bytes, &text, BLOCK).len(), 2);
}

#[test]
fn case_is_significant_unless_it_is_waived() {
    let bytes = b"Workbench".to_vec();
    let exact = Pattern::parse("workbench", false, false).unwrap();
    assert!(search(&bytes, &exact, BLOCK).is_empty());

    let loose = Pattern::parse("workbench", false, true).unwrap();
    assert_eq!(search(&bytes, &loose, BLOCK).len(), 1);
}

#[test]
fn a_pattern_longer_than_the_image_matches_nothing() {
    // Rather than panicking on the window arithmetic (D-006).
    let bytes = vec![0u8; 3];
    assert!(search(&bytes, &parse("a much longer pattern"), BLOCK).is_empty());
}

#[test]
fn an_empty_image_matches_nothing() {
    assert!(search(&[], &parse("anything"), BLOCK).is_empty());
}

#[test]
fn a_zero_block_size_does_not_divide_by_zero() {
    // Nothing in ADE passes zero, but `search` takes a `u32` and D-006 says a
    // parse path does not crash. The block number is then meaningless, which
    // is fine — the offset is still right.
    let bytes = b"xx".to_vec();
    assert_eq!(search(&bytes, &parse("xx"), 0).len(), 1);
}
