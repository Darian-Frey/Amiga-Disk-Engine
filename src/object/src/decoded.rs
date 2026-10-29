//! What a decoded file says, shared by every format the Decoded view reads
//! (F-033): labelled fields in groups, a colour where a field names one.

/// One thing a file says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The part of the file it belongs to: "Screen", "Tool types".
    pub group: &'static str,
    /// What it is.
    pub label: String,
    /// What it is set to, in words.
    pub value: String,
    /// The colour it names, as `0xRRGGBB`, for a colour setting.
    pub colour: Option<u32>,
}

impl Field {
    /// A field with no colour.
    #[must_use]
    pub fn new(group: &'static str, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            group,
            label: label.into(),
            value: value.into(),
            colour: None,
        }
    }
}

/// Signed 8-bit samples drawn into `width` columns of `height` rows: each
/// column the highest and lowest sample it covers, as rows from the top, and
/// whether it lies at or past `repeats_from`, the part a held note repeats.
/// `None` when there is nothing to draw.
#[must_use]
pub fn waveform(
    samples: &[i8],
    repeats_from: Option<usize>,
    width: usize,
    height: usize,
) -> Option<Vec<(usize, usize, bool)>> {
    if samples.is_empty() || width == 0 || height < 2 {
        return None;
    }
    let n = samples.len();
    let mid = height / 2;
    Some(
        (0..width)
            .map(|col| {
                let from = col.saturating_mul(n).checked_div(width).unwrap_or(0);
                let to = (col
                    .saturating_add(1)
                    .saturating_mul(n)
                    .checked_div(width)
                    .unwrap_or(0))
                .max(from.saturating_add(1))
                .min(n);
                let span = samples.get(from..to).unwrap_or(&[]);
                let hi = span.iter().copied().max().unwrap_or(0);
                let lo = span.iter().copied().min().unwrap_or(0);
                // 128 levels each way onto half the height.
                let scale = |v: i8| -> usize {
                    let off = i32::from(v).saturating_mul(i32::try_from(mid).unwrap_or(0)) / 128;
                    usize::try_from(i32::try_from(mid).unwrap_or(0).saturating_sub(off))
                        .unwrap_or(0)
                        .min(height.saturating_sub(1))
                };
                (
                    scale(hi),
                    scale(lo),
                    repeats_from.is_some_and(|r| from >= r),
                )
            })
            .collect(),
    )
}
