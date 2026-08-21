//! Big-endian conversion — the single byte-order seam.
//!
//! **C-001.** All Amiga on-disk data is 68k big-endian; the host is
//! little-endian. Every conversion in ADE routes through this crate, and no
//! other crate may reach for a raw byte-order primitive: `clippy.toml`
//! disallows `u32::from_be_bytes` and its siblings workspace-wide, and this
//! crate is the sole exemption.
//!
//! Every accessor is bounds-checked and returns a typed error carrying the
//! offset that failed — there is no panicking path, per D-006 and F-001.

use core::fmt;

/// A read or write that fell outside the buffer.
///
/// Carries the offset and the width that was attempted so the caller can
/// report block/offset context rather than a bare failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfBounds {
    /// Byte offset the access started at.
    pub offset: usize,
    /// Number of bytes the access needed.
    pub needed: usize,
    /// Length of the buffer that was available.
    pub len: usize,
}

impl fmt::Display for OutOfBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "out-of-bounds access: {} byte(s) at offset {} in a {}-byte buffer",
            self.needed, self.offset, self.len
        )
    }
}

impl core::error::Error for OutOfBounds {}

/// Extract a fixed-width array, bounds-checked and overflow-safe.
fn bytes_at<const N: usize>(buf: &[u8], offset: usize) -> Result<[u8; N], OutOfBounds> {
    let oob = OutOfBounds {
        offset,
        needed: N,
        len: buf.len(),
    };
    // `checked_add` rather than `+`: a hostile offset near usize::MAX must not
    // wrap into a valid-looking range (AV-004).
    let end = offset.checked_add(N).ok_or(oob)?;
    let slice = buf.get(offset..end).ok_or(oob)?;
    slice.try_into().map_err(|_| oob)
}

/// Read an unsigned byte.
pub fn u8_at(buf: &[u8], offset: usize) -> Result<u8, OutOfBounds> {
    let [b] = bytes_at::<1>(buf, offset)?;
    Ok(b)
}

macro_rules! reader {
    ($name:ident, $ty:ty, $width:expr, $doc:literal) => {
        #[doc = $doc]
        #[allow(
            clippy::disallowed_methods,
            reason = "C-001: this crate is the single exemption"
        )]
        pub fn $name(buf: &[u8], offset: usize) -> Result<$ty, OutOfBounds> {
            Ok(<$ty>::from_be_bytes(bytes_at::<$width>(buf, offset)?))
        }
    };
}

reader!(u16_at, u16, 2, "Read a big-endian `u16`.");
reader!(u32_at, u32, 4, "Read a big-endian `u32`.");
reader!(u64_at, u64, 8, "Read a big-endian `u64`.");
reader!(i16_at, i16, 2, "Read a big-endian `i16`.");
reader!(i32_at, i32, 4, "Read a big-endian `i32`.");

macro_rules! writer {
    ($name:ident, $ty:ty, $width:expr, $doc:literal) => {
        #[doc = $doc]
        #[allow(
            clippy::disallowed_methods,
            reason = "C-001: this crate is the single exemption"
        )]
        pub fn $name(buf: &mut [u8], offset: usize, value: $ty) -> Result<(), OutOfBounds> {
            let oob = OutOfBounds {
                offset,
                needed: $width,
                len: buf.len(),
            };
            let end = offset.checked_add($width).ok_or(oob)?;
            let dst = buf.get_mut(offset..end).ok_or(oob)?;
            dst.copy_from_slice(&value.to_be_bytes());
            Ok(())
        }
    };
}

writer!(put_u16, u16, 2, "Write a big-endian `u16`.");
writer!(put_u32, u32, 4, "Write a big-endian `u32`.");

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    // A little-endian host must not read these back as-is; these vectors fail
    // loudly if the conversion is ever dropped.
    const BYTES: &[u8] = &[0x12, 0x34, 0x56, 0x78, 0x9A];

    #[test]
    fn reads_big_endian_not_host_order() {
        assert_eq!(u16_at(BYTES, 0).unwrap(), 0x1234);
        assert_eq!(u32_at(BYTES, 0).unwrap(), 0x1234_5678);
        assert_eq!(u8_at(BYTES, 4).unwrap(), 0x9A);
    }

    #[test]
    fn reads_at_offset() {
        assert_eq!(u16_at(BYTES, 1).unwrap(), 0x3456);
        assert_eq!(u32_at(BYTES, 1).unwrap(), 0x3456_789A);
    }

    #[test]
    fn signed_values_keep_their_sign() {
        assert_eq!(i32_at(&[0xFF, 0xFF, 0xFF, 0xFF], 0).unwrap(), -1);
        assert_eq!(i16_at(&[0x80, 0x00], 0).unwrap(), i16::MIN);
    }

    #[test]
    fn refuses_reads_past_the_end() {
        assert_eq!(
            u32_at(BYTES, 2),
            Err(OutOfBounds {
                offset: 2,
                needed: 4,
                len: 5
            })
        );
        assert!(u8_at(&[], 0).is_err());
    }

    #[test]
    fn hostile_offset_cannot_wrap() {
        // AV-004: offset + width must not overflow into a valid-looking range.
        assert!(u32_at(BYTES, usize::MAX).is_err());
        assert!(u32_at(BYTES, usize::MAX - 2).is_err());
    }

    #[test]
    fn round_trips_through_writers() {
        let mut buf = [0u8; 4];
        put_u32(&mut buf, 0, 0xDEAD_BEEF).unwrap();
        assert_eq!(buf, [0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(u32_at(&buf, 0).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn writers_are_bounds_checked() {
        let mut buf = [0u8; 3];
        assert!(put_u32(&mut buf, 0, 1).is_err());
        assert_eq!(buf, [0, 0, 0], "a rejected write must not partially apply");
    }
}
