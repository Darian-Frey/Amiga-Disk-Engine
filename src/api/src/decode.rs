//! The Decoded view's engine side (F-033).
//!
//! What a front end asks is "what does this file say, in words". Deciding
//! whether ADE can answer — which format a file is, and how to read it —
//! belongs here rather than in a front end, for the same reason as the
//! Assembly view's (F-032): the GUI and the CLI must give the same answer.
//!
//! Five formats: Intuition's Preferences, Workbench icons, IFF ILBM
//! pictures, bitmap fonts and IFF 8SVX sounds. All are the same shape —
//! labelled fields in groups, colours carried as colours, and a picture: a
//! font's is a specimen, a sound's its waveform.

pub use ade_object::decoded::Field;
use ade_object::icon::{self, Highlight, Icon};
use ade_object::ilbm;
use ade_object::prefs::{self, Pointer};
use ade_object::{font, svx};

/// What ADE decodes, for a file it does not.
const DECODES: &str = "It decodes Workbench icons, IFF ILBM pictures, IFF 8SVX sounds, \
                       bitmap fonts and Intuition's Preferences (devs/system-configuration).";

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
    /// The shape of a pixel as the Amiga showed it, width to height: (1, 1)
    /// square, (1, 2) a high-resolution non-interlaced screen's, twice as
    /// tall as wide, (2, 1) a low-resolution interlaced one's. Drawn square,
    /// a Workbench icon looks flattened.
    pub pixel_shape: (u8, u8),
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
    // By content, not by name: an icon is an icon whatever it is called, and
    // 180 corpus icons are not called `.info`.
    if icon::is_icon(bytes) {
        return icon::parse(bytes)
            .map(|i| icon_decoded(&i))
            .map_err(|e| NotDecoded(format!("a Workbench icon, but {e}")));
    }
    if ilbm::is_ilbm(bytes) {
        return ilbm::parse(bytes)
            .map(|p| ilbm_decoded(&p))
            .map_err(|e| NotDecoded(format!("an IFF ILBM picture, but {e}")));
    }
    if svx::is_svx(bytes) {
        return svx::parse(bytes)
            .map(|s| svx_decoded(&s))
            .map_err(|e| NotDecoded(format!("an IFF 8SVX sound, but {e}")));
    }
    if font::is_contents(name, bytes) {
        return font::contents(bytes)
            .map(|c| Decoded {
                title: "Font contents: the sizes of a font, each in a file of its own".to_owned(),
                fields: c.fields(),
                picture: None,
            })
            .map_err(|e| NotDecoded(format!("a font's contents file, but {e}")));
    }
    if font::is_font(bytes) {
        return font::parse(bytes)
            .map(|f| font_decoded(&f))
            .map_err(|e| NotDecoded(format!("a font, but {e}")));
    }
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
    Err(NotDecoded(format!(
        "not a format ADE decodes yet. {DECODES}"
    )))
}

/// The Workbench screen's first four colours, as `0xRRGGBB`: what an icon's
/// pens 0 to 3 were drawn in. Taken from the `devs/system-configuration` of
/// real Workbench disks rather than from memory — 1.2 and 1.3 both give
/// `$05a $fff $002 $f80`, 2.05 and 3.1 both `$aaa $000 $fff $68b`.
const WORKBENCH_1: [u32; 4] = [0x00_55AA, 0xFF_FFFF, 0x00_0022, 0xFF_8800];
const WORKBENCH_2: [u32; 4] = [0xAA_AAAA, 0x00_0000, 0xFF_FFFF, 0x66_88BB];

fn icon_decoded(i: &Icon) -> Decoded {
    let (palette, which) = if i.revision == 0 {
        (WORKBENCH_1, "Workbench 1.x")
    } else {
        (WORKBENCH_2, "Workbench 2.0")
    };
    let mut fields = i.fields();
    let deepest = i
        .image
        .iter()
        .chain(&i.selected)
        .map(|img| img.depth)
        .max()
        .unwrap_or(0);
    if deepest > 0 {
        for (pen, &rgb) in palette.iter().enumerate() {
            fields.push(Field {
                group: "Drawn with",
                label: format!("Pen {pen}"),
                value: format!(
                    "${:x}{:x}{:x}, {which}'s colour {pen}",
                    rgb >> 20 & 0xF,
                    rgb >> 12 & 0xF,
                    rgb >> 4 & 0xF
                ),
                colour: Some(rgb),
            });
        }
        if deepest > 2 {
            fields.push(Field::new(
                "Drawn with",
                "Pens 4 and up",
                "shades of grey: their colours depend on the screen, and the icon does not say",
            ));
        }
    }
    let picture = i.image.as_ref().map(|normal| {
        let selected = match i.highlight {
            Highlight::Image => i.selected.clone(),
            Highlight::Complement => Some(complement(normal)),
            Highlight::Box | Highlight::Nothing => None,
        };
        icon_picture(normal, selected.as_ref(), &palette, which)
    });
    Decoded {
        title: format!(
            "Workbench icon: {}",
            i.kind.describe().split(" (").next().unwrap_or_default()
        ),
        fields,
        picture,
    }
}

/// The image with every pen inverted, as Intuition draws a complemented
/// gadget.
fn complement(img: &icon::Image) -> icon::Image {
    let mask = u8::try_from((1u16 << img.depth.min(8)).saturating_sub(1)).unwrap_or(u8::MAX);
    icon::Image {
        pens: img.pens.iter().map(|&p| p ^ mask).collect(),
        ..img.clone()
    }
}

/// The icon, and beside it how it looks selected when that differs.
fn icon_picture(
    normal: &icon::Image,
    selected: Option<&icon::Image>,
    palette: &[u32; 4],
    which: &str,
) -> Picture {
    const GAP: usize = 4;
    let colour = |pen: u8| -> u32 {
        let rgb = palette.get(usize::from(pen)).copied().unwrap_or_else(|| {
            // A grey per pen, lightest first, distinct up to 256 pens.
            let level = 0xC0u32.saturating_sub(u32::from(pen).saturating_mul(3) & 0xBF);
            level << 16 | level << 8 | level
        });
        0xFF00_0000 | rgb
    };
    let width = normal
        .width
        .saturating_add(selected.map_or(0, |s| s.width.saturating_add(GAP)));
    let height = normal.height.max(selected.map_or(0, |s| s.height));
    let mut argb = vec![0u32; width.saturating_mul(height)];
    let mut draw = |img: &icon::Image, left: usize| {
        for y in 0..img.height {
            for x in 0..img.width {
                let pen = img
                    .pens
                    .get(y.saturating_mul(img.width).saturating_add(x))
                    .copied()
                    .unwrap_or(0);
                if let Some(px) = argb.get_mut(
                    y.saturating_mul(width)
                        .saturating_add(left.saturating_add(x)),
                ) {
                    *px = colour(pen);
                }
            }
        }
    };
    draw(normal, 0);
    if let Some(s) = selected {
        draw(s, normal.width.saturating_add(GAP));
    }
    Picture {
        caption: if selected.is_some() {
            format!("The icon, and selected, in {which}'s colours")
        } else {
            format!("The icon, in {which}'s colours")
        },
        width,
        height,
        argb,
        pixel_shape: (1, 2),
    }
}

fn font_decoded(f: &font::Font) -> Decoded {
    let picture = f
        .specimen(&font::SPECIMEN)
        .map(|(width, height, ink)| Picture {
            caption: "A specimen, set as Text() sets it".to_owned(),
            width,
            height,
            argb: ink
                .iter()
                .map(|&on| if on { 0xFF00_0000 } else { 0xFFFF_FFFF })
                .collect(),
            pixel_shape: f.pixel_shape(),
        });
    Decoded {
        title: format!("Font, {} pixels", f.height),
        fields: f.fields(),
        picture,
    }
}

fn svx_decoded(s: &svx::Sound) -> Decoded {
    const WIDTH: usize = 128;
    const HEIGHT: usize = 32;
    let picture = s.waveform(WIDTH, HEIGHT).map(|columns| {
        let mut argb = vec![0xFFFF_FFFF; WIDTH * HEIGHT];
        for x in 0..WIDTH {
            if let Some(px) = argb.get_mut((HEIGHT / 2 * WIDTH).saturating_add(x)) {
                *px = 0xFFCC_CCCC;
            }
        }
        for (x, (top, bottom, repeats)) in columns.into_iter().enumerate() {
            let ink = if repeats { 0xFFCC_6622 } else { 0xFF22_55AA };
            for y in top..=bottom {
                if let Some(px) = argb.get_mut(y.saturating_mul(WIDTH).saturating_add(x)) {
                    *px = ink;
                }
            }
        }
        Picture {
            caption: if s.repeat > 0 {
                "The waveform, the part that repeats in orange".to_owned()
            } else {
                "The waveform".to_owned()
            },
            width: WIDTH,
            height: HEIGHT,
            argb,
            pixel_shape: (1, 1),
        }
    });
    Decoded {
        title: "IFF 8SVX sound".to_owned(),
        fields: s.fields(),
        picture,
    }
}

fn ilbm_decoded(p: &ilbm::Ilbm) -> Decoded {
    let picture = (!p.argb.is_empty()).then(|| Picture {
        caption: "The picture".to_owned(),
        width: p.width,
        height: p.height,
        argb: p.argb.clone(),
        pixel_shape: p.pixel_shape(),
    });
    Decoded {
        title: if picture.is_some() {
            "IFF ILBM picture".to_owned()
        } else {
            "IFF ILBM colour map".to_owned()
        },
        fields: p.fields(),
        picture,
    }
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
        // A sprite is low-resolution: its pixels are as wide as they are
        // tall.
        pixel_shape: (1, 1),
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
                "{} ({} × {}; . is transparent, and each digit one colour):",
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
        assert!(e.0.contains("icons"), "{e}");
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
