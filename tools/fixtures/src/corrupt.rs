//! Deliberate damage, applied to a built image.
//!
//! This module is the reason D-010 chose generation over committed binaries.
//! None of these structures occurs on a genuine disk: a hash chain cannot loop
//! by accident in any quantity, and a file header does not spontaneously point
//! past the end of the volume. AV-001 and AV-004 have no fixtures unless
//! something builds them, and a builder that says what it builds is worth more
//! than a blob that does not.
//!
//! Each function names the failure it induces and the register entry it
//! belongs to, so a test reading `corrupt::hash_chain_loop(&mut img, e)` states
//! its intent without a comment.

use crate::{BSIZE, Volume, get_u32, normal_checksum, put_u32};

/// Invalidate the bootblock checksum, leaving everything else intact.
///
/// Not itself an attack: only bootable disks need a valid bootblock checksum,
/// and 26% of a 4288-image survey lack one. The fixture exists to prove ADE
/// treats this as an observation rather than a rejection (C-008).
pub fn bootblock_checksum(img: &mut [u8]) {
    let current = get_u32(img, 4);
    put_u32(img, 4, current ^ 0xDEAD_BEEF);
}

/// Invalidate a block's normal checksum.
pub fn block_checksum(img: &mut [u8], block: u32) {
    let off = block as usize * BSIZE + 20;
    let current = get_u32(img, off);
    put_u32(img, off, current ^ 0xDEAD_BEEF);
}

/// Clear the bitmap-valid flag in the rootblock (AV-003).
///
/// The Linux AFFS documentation warns this happens after an unclean shutdown,
/// so it is a routine condition as much as a hostile one. ADE must treat the
/// flag as advisory and be able to rebuild the bitmap.
pub fn bitmap_flag_invalid(img: &mut [u8], root: u32) {
    let off = root as usize * BSIZE + (BSIZE - 200);
    put_u32(img, off, 0);
    rechecksum(img, root);
}

/// Point an entry's same-hash chain back at itself (AV-001).
///
/// The minimal cycle: one block whose `hash_chain` names itself. A traversal
/// without a visited-set will spin here forever.
pub fn hash_chain_loop(img: &mut [u8], entry: u32) {
    let off = entry as usize * BSIZE + (BSIZE - 16);
    put_u32(img, off, entry);
    rechecksum(img, entry);
}

/// Make two entries point at each other, forming a two-block cycle (AV-001).
///
/// Distinct from [`hash_chain_loop`] because a self-pointer is catchable by a
/// naive "next != self" check, while this is not. A depth limit cannot tell
/// this from a legitimately deep chain either; only a visited-set can.
pub fn hash_chain_cycle(img: &mut [u8], a: u32, b: u32) {
    put_u32(img, a as usize * BSIZE + (BSIZE - 16), b);
    put_u32(img, b as usize * BSIZE + (BSIZE - 16), a);
    rechecksum(img, a);
    rechecksum(img, b);
}

/// Point a file header's first data block past the end of the volume (AV-004).
///
/// The classic wild-read: dereferencing this without a bounds check reads
/// outside the image. In ADE this is structurally impossible, because
/// `BlockSource::read_block` takes a `ValidBlock` — the fixture proves it.
pub fn first_data_out_of_range(img: &mut [u8], header: u32) {
    let off = header as usize * BSIZE + 16;
    put_u32(img, off, 0xFFFF_FFFF);
    rechecksum(img, header);
}

/// Point a directory's hash slot at a block outside the volume (AV-004).
pub fn hash_slot_out_of_range(img: &mut [u8], dir: u32, slot: usize) {
    let off = dir as usize * BSIZE + 24 + slot * 4;
    put_u32(img, off, 0xFFFF_FFFF);
    rechecksum(img, dir);
}

/// Give the rootblock a wrong secondary type, so it no longer identifies as one.
///
/// 19% of `DOS`-magic images in the survey have no valid rootblock at the
/// expected location. This reproduces that condition deliberately.
pub fn rootblock_wrong_type(img: &mut [u8], root: u32) {
    put_u32(img, root as usize * BSIZE + (BSIZE - 4), 0x4242_4242);
    rechecksum(img, root);
}

/// Replace the `DOS` bootblock magic with a custom loader signature.
///
/// 7% of the survey corpus begins with something other than `DOS`, across 144
/// distinct leading words, and ten of those still mount. A fixture for the case
/// where format detection must not use the prefix as a gate (C-008).
pub fn non_dos_bootblock(img: &mut [u8], magic: &[u8; 4]) {
    img[..4].copy_from_slice(magic);
    let ck = crate::bootblock_checksum(&img[..BSIZE * 2]);
    put_u32(img, 4, ck);
}

/// Point a directory's first hash slot at another directory, forming a cycle
/// in the *tree* rather than in a hash chain (AV-001).
///
/// This is the shape a legitimate hard link to a directory produces. AmigaDOS
/// permits those, so a tree walk can revisit a directory on a structurally
/// valid disk — the reason traversal needs a visited set rather than a depth
/// limit.
pub fn directory_cycle(img: &mut [u8], dir: u32, target: u32) {
    put_u32(img, dir as usize * BSIZE + 24, target);
    rechecksum(img, dir);
}

/// Overwrite an OFS data block's primary type, so it stops identifying as one.
///
/// The shape found on `A500+A2000 Systest v9.1`, where a file's table points at
/// blocks holding raw audio: the header fields read as sample data (IMP-002).
pub fn data_block_type(img: &mut [u8], block: u32, block_type: u32) {
    put_u32(img, block as usize * BSIZE, block_type);
}

/// Make an OFS data block claim it belongs to a different file.
///
/// Cross-linked files are a classic filesystem corruption: two headers whose
/// tables name the same block.
pub fn data_block_owner(img: &mut [u8], block: u32, owner: u32) {
    put_u32(img, block as usize * BSIZE + 4, owner);
}

/// Give an OFS data block the wrong sequence number.
pub fn data_block_seq(img: &mut [u8], block: u32, seq: u32) {
    put_u32(img, block as usize * BSIZE + 8, seq);
}

/// Declare a payload longer than a block can hold.
pub fn data_block_oversized(img: &mut [u8], block: u32, declared: u32) {
    put_u32(img, block as usize * BSIZE + 12, declared);
}

/// Zero a block entirely.
///
/// 18 of one real file's 79 data-block pointers address blocks like this.
pub fn zero_block(img: &mut [u8], block: u32) {
    let o = block as usize * BSIZE;
    if let Some(slice) = img.get_mut(o..o + BSIZE) {
        slice.fill(0);
    }
}

/// Zero the volume's creation datestamp.
///
/// Day zero is treated as "unset" by Amiga software and is the commonest
/// cosmetic oddity in the wild: 567 of 4652 corpus images have one.
pub fn clear_created_date(img: &mut [u8], root: u32) {
    put_u32(img, root as usize * BSIZE + (BSIZE - 28), 0);
    rechecksum(img, root);
}

/// Point a file extension block's `extension` field back at itself (AV-001).
///
/// The extension chain loops as readily as a hash chain, and until the fixture
/// generator could build extension blocks at all (IMP-004) the visited set
/// guarding it had nothing to be tested against.
pub fn extension_chain_loop(img: &mut [u8], ext_block: u32) {
    put_u32(img, ext_block as usize * BSIZE + (BSIZE - 8), ext_block);
    rechecksum(img, ext_block);
}

/// Make two extension blocks point at each other (AV-001).
///
/// Distinct from the self-loop: a "next != self" check catches that one and
/// misses this.
pub fn extension_chain_cycle(img: &mut [u8], a: u32, b: u32) {
    put_u32(img, a as usize * BSIZE + (BSIZE - 8), b);
    put_u32(img, b as usize * BSIZE + (BSIZE - 8), a);
    rechecksum(img, a);
    rechecksum(img, b);
}

/// Truncate the image to `blocks` blocks.
///
/// One survey image is 90,112 bytes — eight cylinders of what should be eighty.
#[must_use]
pub fn truncated(img: &[u8], blocks: u32) -> Vec<u8> {
    img[..(blocks as usize * BSIZE).min(img.len())].to_vec()
}

/// Append trailing bytes that belong to no block.
///
/// One survey image is exactly 901,121 bytes: canonical plus one.
#[must_use]
pub fn with_trailing_junk(img: &[u8], extra: usize) -> Vec<u8> {
    let mut v = img.to_vec();
    v.extend(std::iter::repeat_n(0xA5, extra));
    v
}

/// A volume whose every block is zeroed except the bootblock.
///
/// 97 survey images begin with four zero bytes.
#[must_use]
pub fn zeroed_volume() -> Vec<u8> {
    vec![0u8; Volume::dd(0).total_blocks() as usize * BSIZE]
}

fn rechecksum(img: &mut [u8], block: u32) {
    let o = block as usize * BSIZE;
    let ck = normal_checksum(&img[o..o + BSIZE]);
    put_u32(img, o + 20, ck);
}
