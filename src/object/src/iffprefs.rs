//! IFF preferences (F-033): the settings Workbench 2 and later keep in
//! `ENV:Sys` and `ENVARC:Sys`, and the `.country` files of `Locale:`.
//!
//! A `FORM PREF` holds a `PRHD` header and one chunk per setting, each a
//! fixed structure. The layouts are AROS's `prefs/*.h`, the constants its
//! `prefs/*.h`, `libraries/locale.h` and `graphics/modeid.h`, read locally
//! and not committed. Every structure is read only when its chunk is exactly
//! the size the structure is, so a chunk written by something else is said
//! to be unread rather than misread: across the corpus every chunk ADE reads
//! has that size.
//!
//! `WBCF`, Workbench's own `wbconfig.prefs`, is in no independent source,
//! and is reported as present and unread. So are a third party's chunks,
//! *ToolManager*'s among them.

use ade_endian::{i16_at, i32_at, u8_at, u16_at, u32_at};

use crate::decoded::Field;

/// A decoded preferences file: each chunk's fields, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    /// The chunks after `PRHD`, by ID, with what each says; `None` for a
    /// chunk ADE does not read, or one not the size of its structure.
    pub chunks: Vec<([u8; 4], Option<Vec<Field>>)>,
}

/// Why bytes are not preferences ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotPreferences(pub String);

impl core::fmt::Display for NotPreferences {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotPreferences {}

/// Whether `bytes` are an IFF preferences file.
#[must_use]
pub fn is_preferences(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"FORM") && bytes.get(8..12) == Some(b"PREF")
}

/// Read a preferences file.
///
/// # Errors
/// [`NotPreferences`] when it is not one.
pub fn parse(bytes: &[u8]) -> Result<Preferences, NotPreferences> {
    if !is_preferences(bytes) {
        return Err(NotPreferences("not a FORM PREF".to_owned()));
    }
    let declared = u32_at(bytes, 4).map_or(0, |v| v as usize);
    let end = declared.saturating_add(8).min(bytes.len());
    let mut at = 12usize;
    let mut chunks = Vec::new();
    while at.saturating_add(8) <= end {
        let mut id = [0u8; 4];
        id.copy_from_slice(bytes.get(at..at.saturating_add(4)).unwrap_or(&[0; 4]));
        let len = u32_at(bytes, at.saturating_add(4)).map_or(0, |v| v as usize);
        let start = at.saturating_add(8);
        let data = bytes
            .get(start..start.saturating_add(len).min(bytes.len()))
            .unwrap_or(&[]);
        if &id != b"PRHD" {
            chunks.push((id, chunk(id, data)));
        }
        at = start.saturating_add(len).saturating_add(len & 1);
    }
    Ok(Preferences { chunks })
}

/// One chunk's fields, when ADE reads it and it is its structure's size.
fn chunk(id: [u8; 4], d: &[u8]) -> Option<Vec<Field>> {
    let sized = |n: usize| (d.len() == n).then_some(());
    match &id {
        b"CTRY" => sized(504).map(|()| country("Country", d)),
        b"LCLE" => sized(860).map(|()| locale(d)),
        b"SCRM" => sized(28).map(|()| screen_mode(d)),
        b"SOND" => sized(284).map(|()| sound(d)),
        b"INPT" => (d.len() >= 44).then(|| input(d)),
        b"FONT" => sized(156).map(|()| font(d)),
        b"OSCN" => sized(36).map(|()| overscan(d)),
        b"SERL" => sized(34).map(|()| serial(d)),
        b"PNTR" => (d.len() >= 32).then(|| pointer(d)),
        b"PTXT" => sized(64).map(|()| printer_text(d)),
        b"PUNT" => sized(56).map(|()| printer_unit(d)),
        b"PGFX" => sized(38).map(|()| printer_graphics(d)),
        b"PSPD" => sized(124).map(|()| printer_postscript(d)),
        _ => None,
    }
}

/// Latin-1 text from a fixed field, to its first NUL.
fn text(d: &[u8], at: usize, len: usize) -> String {
    d.get(at..at.saturating_add(len))
        .unwrap_or(&[])
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect()
}

fn byte(d: &[u8], at: usize) -> u8 {
    u8_at(d, at).unwrap_or(0)
}

fn word(d: &[u8], at: usize) -> u16 {
    u16_at(d, at).unwrap_or(0)
}

fn long(d: &[u8], at: usize) -> u32 {
    u32_at(d, at).unwrap_or(0)
}

/// Shown as itself, or "none" when empty.
fn or_none(s: String) -> String {
    if s.is_empty() { "none".to_owned() } else { s }
}

/// A grouping table: digits per group, to the first zero.
fn grouping(d: &[u8], at: usize) -> String {
    let groups: Vec<String> = d
        .get(at..at.saturating_add(10))
        .unwrap_or(&[])
        .iter()
        .take_while(|&&b| b != 0)
        .map(u8::to_string)
        .collect();
    if groups.is_empty() {
        "none".to_owned()
    } else {
        format!("{} digits", groups.join(", "))
    }
}

/// `struct CountryPrefs` (a `RegionPrefs`), 504 bytes, under `group`.
fn country(group: &'static str, d: &[u8]) -> Vec<Field> {
    let code: String = d
        .get(16..20)
        .unwrap_or(&[])
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect();
    let measuring = match byte(d, 24) {
        0 => "metric (ISO)",
        1 => "American",
        2 => "imperial",
        3 => "British",
        _ => "unknown",
    };
    let week = match byte(d, 503) {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => "unknown",
    };
    let sign = |at: usize| -> String {
        let place = match byte(d, at.saturating_add(11)) {
            0 => "the amount in brackets",
            1 => "the sign before amount and symbol",
            2 => "the sign after amount and symbol",
            3 => "the sign just before the symbol",
            4 => "the sign just after the symbol",
            _ => "unknown",
        };
        let symbol = if byte(d, at.saturating_add(12)) == 0 {
            "before"
        } else {
            "after"
        };
        let space = if byte(d, at.saturating_add(10)) == 0 {
            "no space"
        } else {
            "a space"
        };
        format!(
            "sign \"{}\", {place}; symbol {symbol} the amount, {space}",
            text(d, at, 10)
        )
    };
    vec![
        Field::new(group, "Code", or_none(code)),
        Field::new(group, "Telephone code", format!("+{}", long(d, 20))),
        Field::new(group, "Measuring", measuring),
        Field::new(group, "Week starts on", week),
        Field::new(group, "Date and time", text(d, 25, 80)),
        Field::new(group, "Date", text(d, 105, 40)),
        Field::new(group, "Time", text(d, 145, 40)),
        Field::new(group, "Short date and time", text(d, 185, 80)),
        Field::new(group, "Short date", text(d, 265, 40)),
        Field::new(group, "Short time", text(d, 305, 40)),
        Field::new(group, "Decimal point", or_none(text(d, 345, 10))),
        Field::new(group, "Thousands separator", or_none(text(d, 355, 10))),
        Field::new(group, "Grouping", grouping(d, 375)),
        Field::new(group, "Currency", or_none(text(d, 447, 10))),
        Field::new(group, "Small currency", or_none(text(d, 457, 10))),
        Field::new(group, "International currency", or_none(text(d, 467, 10))),
        Field::new(group, "Money decimal point", or_none(text(d, 395, 10))),
        Field::new(
            group,
            "Money thousands separator",
            or_none(text(d, 405, 10)),
        ),
        Field::new(
            group,
            "Money digits",
            format!(
                "{} after the point, {} internationally",
                byte(d, 445),
                byte(d, 446)
            ),
        ),
        Field::new(group, "Positive amounts", sign(477)),
        Field::new(group, "Negative amounts", sign(490)),
    ]
}

/// `struct LocalePrefs`, 860 bytes: a country, the languages wanted in
/// order, and the time zone.
fn locale(d: &[u8]) -> Vec<Field> {
    const L: &str = "Locale";
    let languages: Vec<String> = (0..10)
        .map(|i: usize| text(d, 48usize.saturating_add(i.saturating_mul(30)), 30))
        .filter(|s| !s.is_empty())
        .collect();
    let minutes = i32_at(d, 348).unwrap_or(0);
    let mut out = vec![
        Field::new(L, "Country", or_none(text(d, 16, 32))),
        Field::new(L, "Languages", or_none(languages.join(", "))),
        // Minutes west of Greenwich, so east is negative.
        Field::new(
            L,
            "Time zone",
            format!(
                "GMT{}{}:{:02}",
                if minutes > 0 { '-' } else { '+' },
                minutes.unsigned_abs() / 60,
                minutes.unsigned_abs() % 60
            ),
        ),
    ];
    out.extend(country("Locale's country", d.get(356..).unwrap_or(&[])));
    out
}

/// A display mode ID in words, from `graphics/modeid.h`.
fn display_mode(id: u32) -> String {
    let monitor = match id & 0xFFFF_1000 {
        0x0000_0000 => "default monitor",
        0x0001_1000 => "NTSC",
        0x0002_1000 => "PAL",
        0x0003_1000 => "VGA",
        0x0004_1000 => "A2024",
        0x0005_1000 => "Proto",
        0x0006_1000 => "Euro72",
        0x0007_1000 => "Euro36",
        0x0008_1000 => "Super72",
        0x0009_1000 => "DblNTSC",
        0x000A_1000 => "DblPAL",
        _ => return format!("${id:08X}"),
    };
    // The resolution and colour bits are the same for the default, NTSC and
    // PAL monitors; other monitors give the low bits other meanings.
    if id & 0xFFFF_0000 > 0x0002_0000 {
        return format!("{monitor}, mode ${:04X}", id & 0xFFFF);
    }
    let resolution = match id & 0x8020 {
        0x8020 => "super-high resolution",
        0x8000 => "high resolution",
        _ => "low resolution",
    };
    let mut words = vec![monitor.to_owned(), resolution.to_owned()];
    for (bit, name) in [
        (0x4, "interlaced"),
        (0x8, "scan-doubled"),
        (0x80, "extra-half-brite"),
        (0x800, "hold-and-modify"),
        (0x400, "dual playfield"),
    ] {
        if id & bit != 0 {
            words.push(name.to_owned());
        }
    }
    format!("{} (${id:08X})", words.join(", "))
}

/// A width or height of `~0`, which means the default.
fn size(v: u16) -> String {
    if v == 0xFFFF {
        "the default".to_owned()
    } else {
        v.to_string()
    }
}

/// `struct ScreenModePrefs`, 28 bytes.
fn screen_mode(d: &[u8]) -> Vec<Field> {
    const S: &str = "Screen mode";
    vec![
        Field::new(S, "Display mode", display_mode(long(d, 16))),
        Field::new(S, "Width", size(word(d, 20))),
        Field::new(S, "Height", size(word(d, 22))),
        Field::new(
            S,
            "Colours",
            format!(
                "{} ({} bitplanes)",
                1u32 << word(d, 24).min(24),
                word(d, 24)
            ),
        ),
        Field::new(
            S,
            "Autoscroll",
            if word(d, 26) & 1 == 0 { "off" } else { "on" },
        ),
    ]
}

/// `struct SoundPrefs`, 284 bytes: what happens on `DisplayBeep()`.
fn sound(d: &[u8]) -> Vec<Field> {
    const S: &str = "Sound";
    let on = |at: usize| if word(d, at) == 0 { "no" } else { "yes" };
    let mut out = vec![
        Field::new(S, "Flash the screen", on(16)),
        Field::new(S, "Make a sound", on(18)),
    ];
    if word(d, 20) == 1 {
        out.push(Field::new(
            S,
            "Sound",
            format!("the sample {}", or_none(text(d, 28, 256))),
        ));
    } else {
        out.push(Field::new(
            S,
            "Sound",
            format!(
                "a beep: period {}, for {} — volume {} of 64",
                word(d, 24),
                word(d, 26),
                word(d, 22)
            ),
        ));
    }
    out
}

/// A `struct timeval` as seconds.
fn seconds(d: &[u8], at: usize) -> String {
    let micro = long(d, at.saturating_add(4));
    format!("{}.{:02} seconds", long(d, at), micro / 10_000)
}

/// `struct InputPrefs`: 44 bytes as Workbench writes it, more as AROS does.
fn input(d: &[u8]) -> Vec<Field> {
    const I: &str = "Input";
    vec![
        Field::new(I, "Keymap", or_none(text(d, 0, 16))),
        Field::new(I, "Pointer ticks", word(d, 16).to_string()),
        Field::new(I, "Double-click", seconds(d, 18)),
        Field::new(I, "Key repeat delay", seconds(d, 26)),
        Field::new(I, "Key repeat rate", seconds(d, 34)),
        Field::new(
            I,
            "Mouse acceleration",
            if i16_at(d, 42).unwrap_or(0) == 0 {
                "off"
            } else {
                "on"
            },
        ),
    ]
}

/// `struct FontPrefs`, 156 bytes: one of the three fonts Workbench uses.
fn font(d: &[u8]) -> Vec<Field> {
    const F: &str = "Font";
    let which = match word(d, 14) {
        0 => "the Workbench icon font",
        1 => "the system font",
        2 => "the screen font",
        _ => "unknown",
    };
    let style = byte(d, 26);
    let mut styles = Vec::new();
    for (bit, name) in [
        (1, "underlined"),
        (2, "bold"),
        (4, "italic"),
        (8, "extended"),
    ] {
        if style & bit != 0 {
            styles.push(name);
        }
    }
    vec![
        Field::new(F, "For", which),
        Field::new(
            F,
            "Font",
            format!("{}, {} pixels tall", or_none(text(d, 28, 128)), word(d, 24)),
        ),
        Field::new(
            F,
            "Style",
            if styles.is_empty() {
                "plain".to_owned()
            } else {
                styles.join(", ")
            },
        ),
        Field::new(
            F,
            "Pens",
            format!("text {}, background {}", byte(d, 16), byte(d, 17)),
        ),
    ]
}

/// `struct OverscanPrefs`, 36 bytes.
fn overscan(d: &[u8]) -> Vec<Field> {
    const O: &str = "Overscan";
    let point = |at: usize| {
        (
            i16_at(d, at).unwrap_or(0),
            i16_at(d, at.saturating_add(2)).unwrap_or(0),
        )
    };
    let (tw, th) = point(24);
    let (vx, vy) = point(20);
    let (x0, y0) = point(28);
    let (x1, y1) = point(32);
    vec![
        Field::new(O, "Display mode", display_mode(long(d, 16))),
        Field::new(O, "Text size", format!("{tw} × {th}")),
        Field::new(O, "Standard", format!("from ({x0}, {y0}) to ({x1}, {y1})")),
        Field::new(O, "View position", format!("({vx}, {vy})")),
        Field::new(
            O,
            "Check value",
            if long(d, 4) == 0xFEDC_BA89 {
                "$FEDCBA89, the value OSCAN_MAGIC names".to_owned()
            } else {
                format!("${:08X}, not the value OSCAN_MAGIC names", long(d, 4))
            },
        ),
    ]
}

/// `struct SerialPrefs`, 34 bytes.
fn serial(d: &[u8]) -> Vec<Field> {
    const S: &str = "Serial";
    let handshake = |at: usize| match byte(d, at) {
        0 => "XON/XOFF",
        1 => "RTS/CTS",
        2 => "none",
        _ => "unknown",
    };
    let parity = match byte(d, 30) {
        0 => "none",
        1 => "even",
        2 => "odd",
        3 => "mark",
        4 => "space",
        _ => "unknown",
    };
    vec![
        Field::new(S, "Baud rate", long(d, 16).to_string()),
        Field::new(
            S,
            "Format",
            format!(
                "{} data bits, parity {parity}, {} stop bits",
                byte(d, 31),
                byte(d, 32)
            ),
        ),
        Field::new(
            S,
            "Handshake",
            format!("{} in, {} out", handshake(28), handshake(29)),
        ),
        Field::new(
            S,
            "Buffers",
            format!("{} bytes in, {} out", long(d, 20), long(d, 24)),
        ),
    ]
}

/// `struct PointerPrefs`, its fields; the image that follows is not drawn.
fn pointer(d: &[u8]) -> Vec<Field> {
    const P: &str = "Pointer";
    vec![
        Field::new(
            P,
            "Which",
            if word(d, 16) == 1 {
                "the busy pointer"
            } else {
                "the normal pointer"
            },
        ),
        Field::new(
            P,
            "Image",
            format!(
                "{} × {}, {} bitplanes",
                word(d, 20),
                word(d, 22),
                word(d, 24)
            ),
        ),
        Field::new(
            P,
            "Hot spot",
            format!(
                "({}, {})",
                i16_at(d, 28).unwrap_or(0),
                i16_at(d, 30).unwrap_or(0)
            ),
        ),
    ]
}

/// `struct PrinterTxtPrefs`, 64 bytes.
fn printer_text(d: &[u8]) -> Vec<Field> {
    const P: &str = "Printer";
    let paper = match word(d, 50) {
        0 => "US letter".to_owned(),
        1 => "US legal".to_owned(),
        2 => "narrow tractor".to_owned(),
        3 => "wide tractor".to_owned(),
        4 => "custom".to_owned(),
        n @ 5..=13 => format!("A{}", n.saturating_sub(5)),
        _ => "unknown".to_owned(),
    };
    let pitch = match word(d, 54) {
        0 => "pica, 10 characters an inch",
        1 => "elite, 12 characters an inch",
        2 => "fine, 17.1 characters an inch",
        _ => "unknown",
    };
    vec![
        Field::new(P, "Driver", or_none(text(d, 16, 30))),
        Field::new(
            P,
            "Port",
            if byte(d, 46) == 1 {
                "serial"
            } else {
                "parallel"
            },
        ),
        Field::new(
            P,
            "Paper",
            format!(
                "{paper}, {}, {} lines a page",
                if word(d, 48) == 1 {
                    "single sheets"
                } else {
                    "fanfold"
                },
                word(d, 52)
            ),
        ),
        Field::new(P, "Pitch", pitch),
        Field::new(
            P,
            "Spacing",
            if word(d, 56) == 1 {
                "8 lines an inch"
            } else {
                "6 lines an inch"
            },
        ),
        Field::new(
            P,
            "Margins",
            format!("columns {} to {}", word(d, 58), word(d, 60)),
        ),
        Field::new(
            P,
            "Quality",
            if word(d, 62) == 1 { "letter" } else { "draft" },
        ),
    ]
}

/// `struct PrinterUnitPrefs`, 56 bytes.
fn printer_unit(d: &[u8]) -> Vec<Field> {
    const P: &str = "Printer unit";
    vec![
        Field::new(P, "Unit", i32_at(d, 16).unwrap_or(0).to_string()),
        Field::new(P, "Device", or_none(text(d, 24, 32))),
    ]
}

/// `struct PrinterGfxPrefs`, 38 bytes.
fn printer_graphics(d: &[u8]) -> Vec<Field> {
    const P: &str = "Printer graphics";
    let shade = match word(d, 18) {
        0 => "black and white",
        1 | 3 => "grey scale",
        2 => "colour",
        _ => "unknown",
    };
    let dither = match byte(d, 26) {
        0 => "ordered",
        1 => "halftone",
        2 => "Floyd-Steinberg",
        _ => "unknown",
    };
    vec![
        Field::new(
            P,
            "Orientation",
            if word(d, 16) == 1 {
                "landscape"
            } else {
                "portrait"
            },
        ),
        Field::new(P, "Shading", shade),
        Field::new(
            P,
            "Image",
            if word(d, 20) == 1 {
                "negative"
            } else {
                "positive"
            },
        ),
        Field::new(P, "Threshold", i16_at(d, 22).unwrap_or(0).to_string()),
        Field::new(P, "Dithering", dither),
        Field::new(P, "Density", byte(d, 30).to_string()),
        Field::new(
            P,
            "Largest",
            format!(
                "{}.{} × {}.{} inches",
                word(d, 32) / 10,
                word(d, 32) % 10,
                word(d, 34) / 10,
                word(d, 34) % 10
            ),
        ),
    ]
}

/// `struct PrinterPSPrefs`, 124 bytes.
fn printer_postscript(d: &[u8]) -> Vec<Field> {
    const P: &str = "PostScript printer";
    let paper = match byte(d, 17) {
        0 => "US letter",
        1 => "US legal",
        2 => "A4",
        3 => "custom",
        _ => "unknown",
    };
    let font = match byte(d, 40) {
        0 => "Courier",
        1 => "Times",
        2 => "Helvetica",
        3 => "Helvetica Narrow",
        4 => "Avant Garde",
        5 => "Bookman",
        6 => "New Century",
        7 => "Palatino",
        8 => "Zapf Chancery",
        _ => "unknown",
    };
    vec![
        Field::new(
            P,
            "Mode",
            if byte(d, 16) == 1 {
                "pass through"
            } else {
                "PostScript"
            },
        ),
        Field::new(P, "Paper", paper),
        Field::new(P, "Copies", i32_at(d, 20).unwrap_or(0).to_string()),
        Field::new(
            P,
            "Resolution",
            format!(
                "{} × {} dots an inch",
                i32_at(d, 32).unwrap_or(0),
                i32_at(d, 36).unwrap_or(0)
            ),
        ),
        Field::new(P, "Font", font),
        Field::new(
            P,
            "Orientation",
            if byte(d, 42) == 1 {
                "landscape"
            } else {
                "portrait"
            },
        ),
    ]
}

impl Preferences {
    /// What kind of file it is, in words.
    #[must_use]
    pub fn title(&self) -> String {
        match self.chunks.first().map(|(id, _)| id) {
            Some(b"CTRY") => "Country: how a country writes dates, numbers and money, \
                              for Workbench 2 and later's locale"
                .to_owned(),
            Some(b"LCLE") => "Locale preferences: the country, languages and time zone".to_owned(),
            _ => "IFF preferences: settings Workbench 2 and later keep".to_owned(),
        }
    }

    /// What the file sets, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        let mut out = Vec::new();
        for (id, fields) in &self.chunks {
            if let Some(f) = fields {
                out.extend(f.iter().cloned());
                continue;
            }
            let name: String = id.iter().map(|&b| char::from(b)).collect();
            let what = match id {
                b"WBCF" => "Workbench's own settings, in no source ADE can check a reading against",
                b"TMEX" | b"TMIM" | b"TMMO" | b"TMDO" => "ToolManager's, a third party's",
                _ => "not read, or not the size its structure is",
            };
            out.push(Field::new("Not read", name, what));
        }
        out
    }
}

#[cfg(test)]
#[allow(
    clippy::trivially_copy_pass_by_ref,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    reason = "tests over bytes they build, by hand rather than with the helpers that read \
              them, so the two cannot agree about a field both have wrong (C-001's exception)"
)]
mod tests {
    use super::*;

    fn form(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut inner = b"PREF".to_vec();
        inner.extend(b"PRHD");
        inner.extend(6u32.to_be_bytes());
        inner.extend([0u8; 6]);
        for (id, data) in chunks {
            inner.extend(*id);
            inner.extend((data.len() as u32).to_be_bytes());
            inner.extend(data);
            if data.len() % 2 == 1 {
                inner.push(0);
            }
        }
        let mut out = b"FORM".to_vec();
        out.extend((inner.len() as u32).to_be_bytes());
        out.extend(inner);
        out
    }

    fn value(fields: &[Field], label: &str) -> String {
        fields
            .iter()
            .find(|f| f.label == label)
            .map(|f| f.value.clone())
            .unwrap_or_default()
    }

    /// A country: Britain's code, telephone code and money, Monday first.
    fn britain() -> Vec<u8> {
        let mut c = vec![0u8; 504];
        c[16..18].copy_from_slice(b"GB");
        c[20..24].copy_from_slice(&44u32.to_be_bytes());
        c[24] = 3;
        c[105..116].copy_from_slice(b"%A %e %B %Y");
        c[345] = b'.';
        c[355] = b',';
        c[375] = 3;
        c[447] = 0xA3;
        c[490] = b'-';
        c[501] = 1;
        c[503] = 1;
        c
    }

    #[test]
    fn a_country_says_how_it_writes_dates_numbers_and_money() {
        let p = parse(&form(&[(b"CTRY", britain())])).unwrap();
        assert!(p.title().starts_with("Country"));
        let f = p.fields();
        assert_eq!(value(&f, "Code"), "GB");
        assert_eq!(value(&f, "Telephone code"), "+44");
        assert_eq!(value(&f, "Measuring"), "British");
        assert_eq!(value(&f, "Week starts on"), "Monday");
        assert_eq!(value(&f, "Date"), "%A %e %B %Y");
        assert_eq!(value(&f, "Grouping"), "3 digits");
        assert_eq!(value(&f, "Currency"), "£");
        assert!(value(&f, "Negative amounts").starts_with("sign \"-\", the sign before"));
    }

    #[test]
    fn a_locale_holds_its_country_and_a_time_zone_west_of_greenwich() {
        let mut l = vec![0u8; 860];
        l[16..23].copy_from_slice(b"danmark");
        l[48..53].copy_from_slice(b"dansk");
        // Minutes west: Denmark is an hour east.
        l[348..352].copy_from_slice(&(-60i32).to_be_bytes());
        l[356..].copy_from_slice(&britain());
        let f = parse(&form(&[(b"LCLE", l)])).unwrap().fields();
        assert_eq!(value(&f, "Country"), "danmark");
        assert_eq!(value(&f, "Languages"), "dansk");
        assert_eq!(value(&f, "Time zone"), "GMT+1:00");
        assert!(
            f.iter()
                .any(|x| x.group == "Locale's country" && x.value == "+44")
        );
    }

    #[test]
    fn a_screen_mode_names_its_monitor_and_resolution() {
        let mut s = vec![0u8; 28];
        s[16..20].copy_from_slice(&0x0002_9004u32.to_be_bytes());
        s[20..22].copy_from_slice(&0xFFFFu16.to_be_bytes());
        s[22..24].copy_from_slice(&512u16.to_be_bytes());
        s[24..26].copy_from_slice(&4u16.to_be_bytes());
        s[26..28].copy_from_slice(&1u16.to_be_bytes());
        let f = parse(&form(&[(b"SCRM", s)])).unwrap().fields();
        assert_eq!(
            value(&f, "Display mode"),
            "PAL, high resolution, interlaced ($00029004)"
        );
        assert_eq!(value(&f, "Width"), "the default");
        assert_eq!(value(&f, "Height"), "512");
        assert_eq!(value(&f, "Colours"), "16 (4 bitplanes)");
        assert_eq!(value(&f, "Autoscroll"), "on");
    }

    #[test]
    fn a_chunk_not_its_structure_s_size_is_not_read() {
        let p = parse(&form(&[(b"SCRM", vec![0; 30]), (b"WBCF", vec![0; 20])])).unwrap();
        let f = p.fields();
        assert!(f.iter().all(|x| x.group == "Not read"), "{f:?}");
        assert!(value(&f, "WBCF").contains("no source"));
        assert!(!is_preferences(b"FORM\0\0\0\x04ILBM"));
    }
}
