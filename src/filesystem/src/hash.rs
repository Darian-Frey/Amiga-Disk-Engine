//! Directory name hashing.
//!
//! ```text
//! hash = strlen(name)
//! for each character c:
//!     hash = hash * 13
//!     hash = hash + toupper(c)
//!     hash = hash & 0x7ff
//! return hash % ((BSIZE / 4) - 56)
//! ```
//!
//! ADF FAQ §4.2.1. The `& 0x7ff` inside the loop is load-bearing, not an
//! optimisation: without it the intermediate grows and the result differs.
//!
//! # `toupper` is the whole difference (C-006)
//!
//! International and non-international volumes differ in **nothing else**.
//! Choosing the wrong one does not error — it produces a hash that misses, so
//! a lookup reports "not found" on a structurally perfect disk. That is why
//! [`crate::dostype::Dostype::is_international`] must be consulted rather than
//! testing the INTL bit, and why `DOS\4`/`DOS\5` are the dangerous case.

/// AmigaDOS `toupper`, ASCII only.
#[must_use]
pub const fn toupper(c: u8) -> u8 {
    if c.is_ascii_lowercase() {
        c.wrapping_sub(b'a' - b'A')
    } else {
        c
    }
}

/// AmigaDOS `toupper` in international mode.
///
/// Also folds the Latin-1 accented range. Codes 224–254 are lowercase accented
/// forms of 192–222; 247 is the division sign and is excluded, as 215 (its
/// multiplication counterpart) is already outside the lowercase range.
#[must_use]
pub const fn intl_toupper(c: u8) -> u8 {
    if c.is_ascii_lowercase() || (c >= 224 && c <= 254 && c != 247) {
        c.wrapping_sub(b'a' - b'A')
    } else {
        c
    }
}

/// Hash a name into a directory hash table of `ht_size` slots.
///
/// Returns 0 for an empty table rather than dividing by zero.
#[must_use]
pub fn hash_name(name: &[u8], ht_size: u32, international: bool) -> u32 {
    let mut hash = u32::try_from(name.len()).unwrap_or(u32::MAX);
    for &c in name {
        hash = hash.wrapping_mul(13);
        hash = hash.wrapping_add(u32::from(if international {
            intl_toupper(c)
        } else {
            toupper(c)
        }));
        hash &= 0x7ff;
    }
    hash.checked_rem(ht_size).unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    const HT: u32 = 72;

    #[test]
    fn folding_is_case_insensitive() {
        assert_eq!(
            hash_name(b"README", HT, false),
            hash_name(b"readme", HT, false)
        );
        assert_eq!(
            hash_name(b"MiXeD", HT, false),
            hash_name(b"mixed", HT, false)
        );
    }

    #[test]
    fn accents_fold_only_in_international_mode() {
        // 'ä' (0xE4) folds to 'Ä' (0xC4) internationally, and not otherwise.
        assert_eq!(intl_toupper(0xE4), 0xC4);
        assert_eq!(toupper(0xE4), 0xE4);
        assert_ne!(
            hash_name(b"\xe4pfel", HT, true),
            hash_name(b"\xe4pfel", HT, false),
            "the same name must hash differently under the two modes (C-006)"
        );
    }

    #[test]
    fn the_division_sign_is_not_a_letter() {
        // 247 sits inside the accented range but is arithmetic, not a letter.
        assert_eq!(intl_toupper(247), 247);
        assert_eq!(intl_toupper(246), 214);
        assert_eq!(intl_toupper(254), 222);
        assert_eq!(intl_toupper(255), 255, "y-diaeresis has no uppercase here");
        assert_eq!(intl_toupper(223), 223, "sharp s is below the folded range");
    }

    #[test]
    fn results_stay_inside_the_table() {
        for name in [
            &b"a"[..],
            b"zzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            b"",
            b"\xff\xff\xff",
        ] {
            for intl in [false, true] {
                assert!(hash_name(name, HT, intl) < HT, "{name:?} intl={intl}");
            }
        }
    }

    #[test]
    fn the_mask_inside_the_loop_matters() {
        // Recomputing without the & 0x7ff gives a different answer for a name
        // long enough to overflow eleven bits — this pins the algorithm rather
        // than merely exercising it.
        let name = b"averylongfilenameindeed";
        let mut unmasked = u32::try_from(name.len()).unwrap_or(u32::MAX);
        for &c in name {
            unmasked = unmasked
                .wrapping_mul(13)
                .wrapping_add(u32::from(toupper(c)));
        }
        assert_ne!(hash_name(name, HT, false), unmasked % HT);
    }

    #[test]
    fn an_empty_table_does_not_divide_by_zero() {
        assert_eq!(hash_name(b"anything", 0, false), 0);
    }
}
