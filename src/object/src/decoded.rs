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
