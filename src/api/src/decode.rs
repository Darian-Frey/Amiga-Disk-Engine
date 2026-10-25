//! The Decoded view's engine side (F-033).
//!
//! What a front end asks is "what does this file say, in words". Deciding
//! whether ADE can answer — which format a file is, and how to read it —
//! belongs here rather than in a front end, for the same reason as the
//! Assembly view's (F-032): the GUI and the CLI must give the same answer.
//!
//! One format so far, Intuition's Preferences. The shape — labelled fields in
//! groups, colours carried as colours, and a picture — is meant for the next
//! one too: an icon is fields and a picture, a font is fields and a picture.

pub use ade_object::prefs::Field;
use ade_object::prefs::{self, Pointer};

/// A decoded file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// What the file is: "Preferences (devs/system-configuration)".
    pub title: String,
    /// What it says, in the order a person would read it.
    pub fields: Vec<Field>,
    /// An image the file holds, if it holds one.
    pub picture: Option<Picture>,
}

/// An image a decoded file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// What it is a picture of.
    pub caption: String,
    /// Pixels across.
    pub width: usize,
    /// Pixels down.
    pub height: usize,
    /// One `0xAARRGGBB` per pixel, row by row; alpha 0 is transparent.
    pub argb: Vec<u32>,
}

/// Why a file was not decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotDecoded(pub String);

impl core::fmt::Display for NotDecoded {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotDecoded {}

/// Decode a file, given its name and contents.
///
/// # Errors
/// [`NotDecoded`] for a file ADE does not recognise, saying what it does
/// decode — the view shows that rather than nothing.
pub fn file(name: &[u8], bytes: &[u8]) -> Result<Decoded, NotDecoded> {
    if prefs::is_preferences(name, bytes.len())
        && let Some(p) = prefs::decode(bytes)
    {
        return Ok(Decoded {
            title: "Preferences (devs/system-configuration): the settings Kickstart 1.x \
                    loads at boot"
                .to_owned(),
            fields: p.fields,
            picture: Some(pointer_picture(&p.pointer)),
        });
    }
    if name.eq_ignore_ascii_case(prefs::FILE_NAME.as_bytes()) {
        return Err(NotDecoded(format!(
            "named {}, but {} bytes where Preferences are {}",
            prefs::FILE_NAME,
            bytes.len(),
            prefs::SIZE
        )));
    }
    Err(NotDecoded(
        "not a format ADE decodes yet. It decodes Intuition's Preferences \
         (devs/system-configuration)."
            .to_owned(),
    ))
}

fn pointer_picture(p: &Pointer) -> Picture {
    let argb = p
        .pixels
        .iter()
        .map(|&i| match i {
            1..=3 => {
                0xFF00_0000
                    | p.colours
                        .get(usize::from(i).saturating_sub(1))
                        .copied()
                        .unwrap_or(0)
            }
            _ => 0,
        })
        .collect();
    Picture {
        caption: "The mouse pointer".to_owned(),
        width: Pointer::SIDE,
        height: Pointer::SIDE,
        argb,
    }
}

impl Decoded {
    /// The decoded file as text: the title, then each group's fields with
    /// their labels aligned, then the picture drawn in characters.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![self.title.clone()];
        let width = self
            .fields
            .iter()
            .map(|f| f.label.chars().count())
            .max()
            .unwrap_or(0);
        let mut group = "";
        for f in &self.fields {
            if f.group != group {
                group = f.group;
                out.push(String::new());
                out.push(group.to_owned());
            }
            out.push(format!("  {:<width$}  {}", f.label, f.value));
        }
        if let Some(p) = &self.picture {
            out.push(String::new());
            out.push(format!(
                "{} ({} × {}; . is transparent, 1 to 3 are its colours):",
                p.caption, p.width, p.height
            ));
            out.extend(p.ascii());
        }
        out
    }
}

impl Picture {
    /// The picture in characters: `.` for transparent, and a digit for each
    /// distinct colour in order of first use.
    #[must_use]
    pub fn ascii(&self) -> Vec<String> {
        let mut seen: Vec<u32> = Vec::new();
        let mut rows = Vec::new();
        for row in self.argb.chunks(self.width.max(1)) {
            let line: String = row
                .iter()
                .map(|&px| {
                    if px >> 24 == 0 {
                        return '.';
                    }
                    let index = seen.iter().position(|&c| c == px).unwrap_or_else(|| {
                        seen.push(px);
                        seen.len().saturating_sub(1)
                    });
                    char::from_digit(
                        u32::try_from(index.saturating_add(1)).unwrap_or(9).min(9),
                        10,
                    )
                    .unwrap_or('#')
                })
                .collect();
            rows.push(format!("  {line}"));
        }
        rows
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn preferences_are_decoded_by_name_and_size() {
        let mut bytes = vec![0u8; prefs::SIZE];
        bytes[0] = 8;
        let d = file(b"system-configuration", &bytes).unwrap();
        assert!(d.title.starts_with("Preferences"));
        assert!(d.fields.iter().any(|f| f.value == "80 columns (Topaz 8)"));
        let p = d.picture.unwrap();
        assert_eq!((p.width, p.height, p.argb.len()), (16, 16, 256));
        assert!(
            p.argb.iter().all(|&px| px == 0),
            "an empty pointer is transparent"
        );
    }

    #[test]
    fn anything_else_says_what_is_decoded() {
        let e = file(b"startup-sequence", b"echo hello").unwrap_err();
        assert!(e.0.contains("system-configuration"), "{e}");
        let e = file(b"system-configuration", &[0; 100]).unwrap_err();
        assert!(e.0.contains("100 bytes"), "{e}");
    }

    #[test]
    fn the_text_form_aligns_its_labels_and_draws_the_picture() {
        let mut bytes = vec![0u8; prefs::SIZE];
        bytes[0] = 8;
        // The pointer's top-left pixel in colour 1.
        bytes[32] = 0x80;
        let lines = file(b"system-configuration", &bytes).unwrap().lines();
        assert!(lines.iter().any(|l| l == "Screen"));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Text size") && l.ends_with("80 columns (Topaz 8)"))
        );
        let art = lines
            .iter()
            .position(|l| l.starts_with("The mouse pointer"))
            .unwrap();
        assert_eq!(lines[art + 1], "  1...............");
    }
}
