//! MFM track codec — decoding raw tracks into Amiga sectors.
//!
//! A plain ADF holds sectors that were already decoded by the drive. A raw
//! track holds what the drive actually read: MFM, with its clock bits, sync
//! marks and gaps intact. Decoding it is what turns a protected disk's capture
//! back into data — and, just as importantly, what shows where it *cannot* be
//! turned back, because that is where the protection lives.
//!
//! # The decode is self-evidencing
//!
//! Every Amiga sector carries two checksums of its own: one over the header and
//! one over the data. So unlike most of this project, MFM decoding needs no
//! oracle and no corpus comparison — a correct decode produces sectors whose
//! own checksums agree, and an incorrect one does not.
//!
//! That property settled a question the sources disagree on. Descriptions of
//! the odd/even split differ over which half comes first; rather than pick a
//! source, both orders were tried against a real track and only one produced
//! matching checksums. **Odd half first.** See SPEC §MFM.
//!
//! # Layout
//!
//! One sector is 1088 MFM bytes, and eleven of them plus a gap is the 12668
//! bytes a raw DD track occupies — which is exactly the allocation observed in
//! the corpus, and a useful confirmation that the structure below is right.

use ade_endian::u32_at;

/// The Amiga sync word. Deliberately not producible by ordinary data: its
/// encoding omits a clock bit, so it is legal MFM but an illegal data+clock
/// combination, which is what makes it findable by scanning.
pub const SYNC: u16 = 0x4489;

/// Two sync words, the pattern that actually marks a sector.
const SYNC_PAIR: u32 = 0x4489_4489;

/// MFM bytes in one sector, sync words included.
pub const SECTOR_MFM_BYTES: usize = 1088;

/// Decoded bytes in one sector.
pub const SECTOR_BYTES: usize = 512;

/// Sectors on a standard double-density track.
pub const DD_SECTORS: usize = 11;

/// The format byte an AmigaDOS sector header carries.
pub const FORMAT_AMIGADOS: u8 = 0xFF;

/// Offsets within a sector, measured from the end of the two sync words.
#[allow(
    unreachable_pub,
    reason = "a private module's constants, grouped for documentation"
)]
mod field {
    /// Header info: format, track, sector, sectors-to-gap.
    pub const INFO: usize = 0;
    /// Sector label — 16 bytes, in practice always zero.
    pub const LABEL: usize = 8;
    /// Checksum over the info and label.
    pub const HEADER_CHECKSUM: usize = 40;
    /// Checksum over the data.
    pub const DATA_CHECKSUM: usize = 48;
    /// The data itself.
    pub const DATA: usize = 56;
    /// Total, from the end of the sync words.
    pub const END: usize = 1080;
}

/// One decoded sector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sector {
    /// Where the sync word sat in the raw track.
    pub offset: usize,
    /// Format byte — [`FORMAT_AMIGADOS`] on an ordinary disk.
    pub format: u8,
    /// The track this sector says it belongs to.
    pub track: u8,
    /// The sector number this sector says it is.
    pub sector: u8,
    /// How many sectors follow before the gap.
    pub sectors_to_gap: u8,
    /// The 16-byte sector label, as decoded.
    pub label: [u8; 16],
    /// Whether the header checksum agrees.
    pub header_checksum_valid: bool,
    /// Whether the data checksum agrees.
    pub data_checksum_valid: bool,
    /// Clock bits in the header and data that break the MFM encoding rule.
    ///
    /// Zero on an ordinary sector — measured at exactly zero across 4095 data
    /// bits and 1279 header bits of a real one. A non-zero count means the
    /// bytes were never written by a standard drive, which is either damage or
    /// deliberate: illegal MFM is a protection technique in its own right.
    pub clock_violations: usize,
    /// The 512 decoded bytes.
    pub data: Vec<u8>,
}

impl Sector {
    /// Whether both checksums agree and the format byte is AmigaDOS.
    ///
    /// This is the whole verification story: a sector that says this is a
    /// sector that decoded correctly, because nothing else produces two
    /// agreeing checksums.
    #[must_use]
    pub const fn is_sound(&self) -> bool {
        self.header_checksum_valid && self.data_checksum_valid && self.format == FORMAT_AMIGADOS
    }

    /// Whether the sector's bytes are legal MFM as well as correct data.
    ///
    /// A separate question from [`Self::is_sound`]: a sector can checksum
    /// perfectly and still be encoded in a way no drive would produce.
    #[must_use]
    pub const fn clock_valid(&self) -> bool {
        self.clock_violations == 0
    }
}

/// What a raw track yielded.
#[derive(Debug, Clone)]
pub struct TrackDecode {
    /// Sectors found, in the order they appear on the track.
    pub sectors: Vec<Sector>,
    /// Every sync word on the track, not only those that begin a sector.
    ///
    /// Counted by scanning the whole track at its clock phase, because a sync
    /// in a gap is just as illegal as one before a sector and contributes the
    /// same violation. Counting only sector-leading syncs undercounts the
    /// baseline and makes ordinary tracks look like they carry illegal MFM —
    /// which is exactly the wrong way round.
    pub sync_words: usize,
    /// Clock-bit violations across the whole track.
    ///
    /// Measured from the first sync word, because that is the only place the
    /// clock/data phase is known: a track's byte boundaries say nothing about
    /// where its bit pairs begin. `None` when the track has no sync at all and
    /// the phase is therefore unknowable.
    ///
    /// # This is an observation, not a protection score
    ///
    /// It is tempting to subtract [`Self::sync_words`] and call the remainder
    /// illegal MFM, since a sync word is deliberately illegal. **That does not
    /// work**, and it was tried: a sync word does not contribute exactly one
    /// violation, because the transitions into and out of a sync region
    /// contribute their own. A known-good Terrorpods track has 22 sync words
    /// and 27 violations.
    ///
    /// Counting the baseline two different ways produced two different and
    /// contradictory pictures of which disks were protected — the first made
    /// the heavily-protected ones look clean. The raw counts are reported
    /// because they are facts; the difference is not, and no score is derived
    /// from it. Isolating deliberate illegal MFM needs the sync boundaries
    /// modelled properly, which is not done.
    pub clock_violations: Option<usize>,
    /// Sync marks that lead to no sector at all.
    ///
    /// **This is a protection signature, not an error.** A sync mark written
    /// into the gap with nothing behind it is how a custom loader finds its own
    /// data: the hardware can sync to it, and a standard reader finds no
    /// sector. Every raw track in `Realm of the Trolls` and `Wings of Death`
    /// is like this — three sync words followed by gap.
    pub stray_syncs: usize,
}

impl TrackDecode {
    /// Sectors whose own checksums agree.
    #[must_use]
    pub fn sound(&self) -> usize {
        self.sectors.iter().filter(|s| s.is_sound()).count()
    }

    /// Whether this looks like an ordinary track: eleven sound sectors,
    /// numbered 0 to 10 exactly once each.
    ///
    /// A track that is *not* standard is the interesting case — that is where
    /// copy protection lives — so this distinguishes "decoded fine" from
    /// "decoded fine and unremarkable".
    #[must_use]
    pub fn is_standard(&self) -> bool {
        let sound: Vec<&Sector> = self.sectors.iter().filter(|s| s.is_sound()).collect();
        if sound.len() != DD_SECTORS {
            return false;
        }
        let mut seen = [false; DD_SECTORS];
        for sector in sound {
            let Some(slot) = seen.get_mut(usize::from(sector.sector)) else {
                return false;
            };
            if *slot {
                return false;
            }
            *slot = true;
        }
        true
    }
}

/// Count clock bits that break the MFM encoding rule.
///
/// In MFM each data bit is preceded by a clock bit, and the clock is set
/// **only when both the previous and the current data bit are zero** — its
/// whole job is to keep a run of zeros from losing its timing. In the Amiga's
/// byte layout the clock bits are the odd positions (`0xAA`) and the data bits
/// the even ones (`0x55`).
///
/// The first pair is skipped: its clock depends on the data bit before the
/// slice, which is not in the slice.
///
/// # What a violation means
///
/// Legal MFM is what a drive can write. A violation is therefore either damage
/// or intent, and the format's own sync word is the proof that intent happens:
/// `0x4489` is deliberately illegal — measured here at exactly **2 violations
/// across two sync words** — which is precisely what makes it findable in a
/// stream where ordinary data cannot produce it.
#[must_use]
pub fn clock_violations(mfm: &[u8]) -> usize {
    let mut violations = 0usize;
    let mut previous: Option<u8> = None;

    for (index, byte) in mfm.iter().enumerate() {
        let _ = index;
        // Four (clock, data) pairs per byte, most significant first.
        for pair in 0..4u32 {
            let shift = 7u32.saturating_sub(pair.saturating_mul(2));
            let clock = byte.checked_shr(shift).unwrap_or(0) & 1;
            let data = byte.checked_shr(shift.saturating_sub(1)).unwrap_or(0) & 1;
            if let Some(prior) = previous {
                let expected = u8::from(prior == 0 && data == 0);
                if clock != expected {
                    violations = violations.saturating_add(1);
                }
            }
            previous = Some(data);
        }
    }
    violations
}

/// Decode the odd/even halves of an MFM field.
///
/// Each MFM byte carries four data bits in its even positions, so the mask is
/// `0x55`; the odd half supplies the high bit of each pair and the even half
/// the low one. **Odd half first** — determined by checksum agreement on real
/// tracks, not by choosing between sources that disagree.
fn decode_halves(odd: &[u8], even: &[u8]) -> Vec<u8> {
    odd.iter()
        .zip(even.iter())
        .map(|(o, e)| ((o & 0x55) << 1) | (e & 0x55))
        .collect()
}

/// Decode a field that occupies `2 * len` MFM bytes at `at`.
fn decode_field(mfm: &[u8], at: usize, len: usize) -> Option<Vec<u8>> {
    let odd = mfm.get(at..at.checked_add(len)?)?;
    let even_at = at.checked_add(len)?;
    let even = mfm.get(even_at..even_at.checked_add(len)?)?;
    Some(decode_halves(odd, even))
}

/// The Amiga checksum: the XOR of the MFM longs, keeping only the data bits.
///
/// Computed over the **encoded** bytes, not the decoded ones — the clock bits
/// are excluded by the mask rather than by decoding first.
fn checksum(mfm: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    let mut at = 0usize;
    while let Ok(word) = u32_at(mfm, at) {
        sum ^= word;
        at = at.saturating_add(4);
    }
    sum & 0x5555_5555
}

/// Read a stored checksum, which is itself odd/even encoded.
fn stored_checksum(mfm: &[u8], at: usize) -> Option<u32> {
    let bytes = decode_field(mfm, at, 4)?;
    u32_at(&bytes, 0).ok()
}

/// Decode one sector whose sync words end at bit offset `body`.
fn decode_sector_at(track: &[u8], sync_bit: usize, body: usize) -> Option<Sector> {
    let mfm = bytes_at_bit(track, body, field::END)?;
    let mfm = mfm.as_slice();
    let sync_at = sync_bit / 8;

    let info = decode_field(mfm, field::INFO, 4)?;
    let label_bytes = decode_field(mfm, field::LABEL, 16)?;
    let mut label = [0u8; 16];
    label.copy_from_slice(label_bytes.get(..16)?);

    // The header checksum covers the info and label as they are stored: the
    // 40 MFM bytes between the sync words and the checksum itself.
    let header_calculated = checksum(mfm.get(field::INFO..field::HEADER_CHECKSUM)?);
    let header_stored = stored_checksum(mfm, field::HEADER_CHECKSUM)?;

    let data_mfm = mfm.get(field::DATA..field::END)?;
    let data_calculated = checksum(data_mfm);
    let data_stored = stored_checksum(mfm, field::DATA_CHECKSUM)?;

    Some(Sector {
        offset: sync_at,
        format: *info.first()?,
        track: *info.get(1)?,
        sector: *info.get(2)?,
        sectors_to_gap: *info.get(3)?,
        label,
        header_checksum_valid: header_stored == header_calculated,
        data_checksum_valid: data_stored == data_calculated,
        // The checksummed regions only: the gap between them is not the
        // sector's to be legal about.
        clock_violations: clock_violations(mfm.get(field::INFO..field::HEADER_CHECKSUM)?)
            .saturating_add(clock_violations(data_mfm)),
        data: decode_field(mfm, field::DATA, SECTOR_BYTES)?,
    })
}

/// Read `len` bytes starting at an arbitrary **bit** offset.
///
/// This is the whole reason the decoder is bit-addressed. See [`decode_track`].
fn bytes_at_bit(track: &[u8], bit: usize, len: usize) -> Option<Vec<u8>> {
    let start_byte = bit / 8;
    let shift = bit % 8;
    if shift == 0 {
        return track
            .get(start_byte..start_byte.checked_add(len)?)
            .map(<[u8]>::to_vec);
    }
    // Each output byte straddles two input bytes.
    let end = start_byte.checked_add(len)?.checked_add(1)?;
    let window = track.get(start_byte..end)?;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let high = u16::from(*window.get(i)?);
        let low = u16::from(*window.get(i.checked_add(1)?)?);
        let pair = (high << 8) | low;
        let taken = pair
            .checked_shr(u32::try_from(8usize.saturating_sub(shift)).unwrap_or(0))
            .unwrap_or(0);
        out.push(u8::try_from(taken & 0xFF).unwrap_or(0));
    }
    Some(out)
}

/// Decode every sector a raw MFM track contains.
///
/// # Why this scans bits, not bytes
///
/// A raw track is a **bit** stream. The Amiga writes it continuously and there
/// is no reason for a sector to begin on a byte boundary of the file that
/// happens to hold it — measured across the corpus, most do not. Every sync in
/// one `Realm of the Trolls` track sits at bit offset ≡ 7 (mod 8), so a
/// byte-aligned scan finds nothing there at all. A first implementation that
/// scanned bytes decoded 8% of sectors and looked merely disappointing rather
/// than wrong.
///
/// Sectors are found by their sync mark rather than by assumed positions,
/// because a protected track has no reliable positions — that is frequently
/// the protection itself. A sync that yields no sector is counted rather than
/// discarded: on a protected track, that count is the finding.
#[must_use]
pub fn decode_track(track: &[u8]) -> TrackDecode {
    let mut sectors = Vec::new();
    let mut stray_syncs = 0usize;
    let mut first_sync: Option<usize> = None;

    let total_bits = track.len().saturating_mul(8);
    // A rolling 32-bit window over the bit stream.
    let mut window: u32 = 0;
    for prime in 0..32.min(total_bits) {
        window = (window << 1) | u32::from(bit_at(track, prime));
    }
    let mut position = 32usize;

    while position <= total_bits {
        if window == SYNC_PAIR {
            // The sync pattern begins 32 bits back.
            let bit = position.saturating_sub(32);
            // Sectors carry two sync words, but some tracks carry more; the
            // body starts after the last of them.
            if first_sync.is_none() {
                first_sync = Some(bit);
            }
            let mut body = bit.saturating_add(32);
            while sync_at(track, body) {
                body = body.saturating_add(16);
            }
            // A sync mark does not imply a sector. The format byte is the
            // only reliable marker: a checksum test is not enough, because a
            // run of gap decodes to all-zero and so satisfies its own checksum
            // trivially — zero XORs to zero. Every sync in `Wings of Death`
            // would otherwise be reported as a sector full of gap.
            match decode_sector_at(track, bit, body).filter(|s| s.format == FORMAT_AMIGADOS) {
                Some(sector) => {
                    sectors.push(sector);
                    // Skip what was just read: a sync cannot occur inside a
                    // sector, so there is nothing to find in between.
                    position = body
                        .saturating_add(field::END.saturating_mul(8))
                        .saturating_add(32);
                    window = 0;
                    for i in position.saturating_sub(32)..position {
                        window = (window << 1) | u32::from(bit_at(track, i));
                    }
                    continue;
                }
                None => stray_syncs = stray_syncs.saturating_add(1),
            }
        }
        if position == total_bits {
            break;
        }
        window = (window << 1) | u32::from(bit_at(track, position));
        position = position.saturating_add(1);
    }

    // The clock/data phase is only knowable from a sync word, so a track with
    // no sync cannot be checked at all — its byte boundaries say nothing about
    // where its bit pairs begin.
    let clock_violations = first_sync.map(|at| clock_violations_from(track, at));
    let sync_words = first_sync.map_or(0, |at| count_sync_words(track, at));

    TrackDecode {
        sectors,
        sync_words,
        clock_violations,
        stray_syncs,
    }
}

/// Count every sync word on the track, at the track's clock phase.
///
/// Phase-aligned because a sync word is eight (clock, data) pairs: one that
/// straddled the phase would not be a sync word as the hardware sees it.
fn count_sync_words(track: &[u8], phase: usize) -> usize {
    let total_bits = track.len().saturating_mul(8);
    let mut count = 0usize;
    let mut at = phase;
    while at.saturating_add(16) <= total_bits {
        if sync_at(track, at) {
            count = count.saturating_add(1);
            at = at.saturating_add(16);
        } else {
            at = at.saturating_add(2);
        }
    }
    count
}

/// Count clock violations across a track, starting from a known clock bit.
fn clock_violations_from(track: &[u8], start_bit: usize) -> usize {
    let total_bits = track.len().saturating_mul(8);
    let mut violations = 0usize;
    let mut previous: Option<u8> = None;
    let mut at = start_bit;

    while at.saturating_add(1) < total_bits {
        let clock = bit_at(track, at);
        let data = bit_at(track, at.saturating_add(1));
        if let Some(prior) = previous {
            let expected = u8::from(prior == 0 && data == 0);
            if clock != expected {
                violations = violations.saturating_add(1);
            }
        }
        previous = Some(data);
        at = at.saturating_add(2);
    }
    violations
}

/// One bit of the track, zero past the end.
fn bit_at(track: &[u8], bit: usize) -> u8 {
    let Some(byte) = track.get(bit / 8) else {
        return 0;
    };
    let within = bit.checked_rem(8).unwrap_or(0);
    let shift = u32::try_from(7usize.saturating_sub(within)).unwrap_or(0);
    byte.checked_shr(shift).unwrap_or(0) & 1
}

/// Whether a single sync word sits at this bit offset.
fn sync_at(track: &[u8], bit: usize) -> bool {
    bytes_at_bit(track, bit, 2)
        .and_then(|b| Some(u16::from(*b.first()?) << 8 | u16::from(*b.get(1)?)))
        .is_some_and(|word| word == SYNC)
}

/// Bytes of gap written before each sector.
///
/// The Amiga's own format leaves roughly this much between sectors; what
/// matters for a reader is that it is non-zero and decodes to nothing, so the
/// sync mark stands alone.
const GAP_BYTES: usize = 32;

/// Encode one sector as MFM, sync words included.
///
/// The inverse of [`decode_track`], and verified against it: a sector encoded
/// here decodes back to the same bytes with both checksums agreeing and no
/// clock violations.
///
/// # Clock bits are computed, not left clear
///
/// This is the part a test helper can skip and a real encoder cannot. A clock
/// bit depends on the data bit *before* it, so the body is written as one
/// continuous stream and the clock bits are filled in afterwards — they cannot
/// be computed field by field. The sync words are deliberately excluded: their
/// illegality is what makes them findable (SPEC §Clock bits and the encoding
/// rule).
///
/// # Panics
/// Never. A `data` slice that is not [`SECTOR_BYTES`] long yields `None`.
#[must_use]
pub fn encode_sector(track: u8, sector: u8, sectors_to_gap: u8, data: &[u8]) -> Option<Vec<u8>> {
    if data.len() != SECTOR_BYTES {
        return None;
    }

    // Fields first with data bits only; clock bits go on at the end, once the
    // whole stream exists to compute them from.
    let mut body = Vec::with_capacity(field::END);
    body.extend(encode_halves(&[
        FORMAT_AMIGADOS,
        track,
        sector,
        sectors_to_gap,
    ]));
    body.extend(encode_halves(&[0u8; 16]));

    // The checksums mask clock bits off, so they can be computed now and are
    // unaffected by the clock pass below.
    let header_sum = checksum(body.get(..field::HEADER_CHECKSUM)?);
    body.extend(encode_halves(&be_bytes(header_sum)));

    let encoded_data = encode_halves(data);
    let data_sum = checksum(&encoded_data);
    body.extend(encode_halves(&be_bytes(data_sum)));
    body.extend(encoded_data);

    if body.len() != field::END {
        return None;
    }
    // The last data bit of the sync word seeds the first clock bit: 0x4489
    // ends in 1, so the body's opening clock bit is clear.
    apply_clock_bits(&mut body, 1);

    // 1088 bytes: four of lead-in, four of sync, 1080 of body. The lead-in is
    // part of the sector as SPEC counts it, not part of the gap.
    let mut out = Vec::with_capacity(SECTOR_MFM_BYTES);
    out.extend_from_slice(&[0xAA; 4]);
    out.extend_from_slice(&be_bytes_u16(SYNC));
    out.extend_from_slice(&be_bytes_u16(SYNC));
    out.extend_from_slice(&body);
    debug_assert_eq!(out.len(), SECTOR_MFM_BYTES);
    Some(out)
}

/// Encode a whole track: eleven sectors, each preceded by a gap.
///
/// `sectors` are taken in order and numbered from zero, as a standard track
/// numbers them.
///
/// # Panics
/// Never. `None` if any sector is not [`SECTOR_BYTES`] long.
#[must_use]
pub fn encode_track(track: u8, sectors: &[&[u8]]) -> Option<Vec<u8>> {
    let count = u8::try_from(sectors.len()).ok()?;
    let mut out = Vec::new();
    for (index, data) in sectors.iter().enumerate() {
        let number = u8::try_from(index).ok()?;
        // Gap is 0xAA — clock set, data clear — which is legal MFM and decodes
        // to nothing.
        out.extend(core::iter::repeat_n(0xAAu8, GAP_BYTES));
        out.extend(encode_sector(
            track,
            number,
            count.saturating_sub(number),
            data,
        )?);
    }
    out.extend(core::iter::repeat_n(0xAAu8, GAP_BYTES));
    Some(out)
}

/// Split a field into odd and even MFM halves, clock bits left clear.
///
/// The inverse of [`decode_halves`]: the odd half carries each byte's odd data
/// bits and the even half its even ones, and **the odd half comes first**.
fn encode_halves(data: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = data.iter().map(|b| (b >> 1) & 0x55).collect();
    out.extend(data.iter().map(|b| b & 0x55));
    out
}

/// A `u32` as big-endian bytes, through the C-001 seam.
fn be_bytes(value: u32) -> [u8; 4] {
    let mut out = [0u8; 4];
    let _ = ade_endian::put_u32(&mut out, 0, value);
    out
}

/// A `u16` as big-endian bytes, through the C-001 seam.
fn be_bytes_u16(value: u16) -> [u8; 2] {
    let mut out = [0u8; 2];
    let _ = ade_endian::put_u16(&mut out, 0, value);
    out
}

/// Fill in the clock bits of an already-written data stream.
///
/// `previous_data` is the data bit immediately before the stream, which the
/// first clock bit depends on.
fn apply_clock_bits(stream: &mut [u8], previous_data: u8) {
    let mut previous = previous_data;
    for byte in stream.iter_mut() {
        let mut result = *byte;
        for pair in 0..4u32 {
            let data_shift = 6u32.saturating_sub(pair.saturating_mul(2));
            let clock_shift = data_shift.saturating_add(1);
            let data = byte.checked_shr(data_shift).unwrap_or(0) & 1;
            if previous == 0 && data == 0 {
                result |= 1u8.checked_shl(clock_shift).unwrap_or(0);
            } else {
                result &= !(1u8.checked_shl(clock_shift).unwrap_or(0));
            }
            previous = data;
        }
        *byte = result;
    }
}
