//! Intuition's Preferences, as `devs/system-configuration` holds them (F-033).
//!
//! On Kickstart 1.x the Preferences tool saved the user's settings as one
//! 232-byte `struct Preferences`, and Intuition loaded it from `DEVS:` at boot.
//! Most disks carry one because most were cut down from a Workbench disk: 110
//! of 466 sampled corpus disks, every one exactly 232 bytes.
//!
//! The layout is `intuition/preferences.h`, checked field by field against
//! AROS's transcription of the same header. It records how somebody liked
//! their Workbench — text size, colours, pointer, printer, serial port — and
//! **nothing about the hardware a disk needs**: that is `ade specs` (F-028).
//!
//! # The thirty bytes at offset 186
//!
//! Kickstart 1.x declared them as one 30-byte name; 2.0 split them into
//! twelve bytes of padding, a 16-byte printer device name and two unit
//! numbers. Real files show the 1.x text spilling across the 2.0 boundary
//! (`canon_pw1080` running into what 2.0 would call the device name), so the
//! area is read as 2.0 fields only when the padding is clean, and as the 1.x
//! name otherwise — never both.

use ade_endian::{i16_at, u8_at, u16_at, u32_at};

/// The size of `struct Preferences`.
pub const SIZE: usize = 232;

/// The name Intuition loads them from, in `DEVS:`.
pub const FILE_NAME: &str = "system-configuration";

pub use crate::decoded::Field;

/// The mouse pointer: a 16 × 16 sprite in three colours and transparent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pointer {
    /// One colour index per pixel, row by row: 0 is transparent, 1 to 3 are
    /// [`colours`](Self::colours).
    pub pixels: Vec<u8>,
    /// Sprite colours 17, 18 and 19, as `0xRRGGBB`.
    pub colours: [u32; 3],
    /// `XOffset` and `YOffset` as stored: where the image is drawn relative
    /// to the pointer's position. Not converted to a pixel of the image,
    /// because which pixels they count — the sprite's or the screen's — is
    /// not something this file says.
    pub offset: (i8, i8),
}

impl Pointer {
    /// Pixels across and down.
    pub const SIDE: usize = 16;
}

/// The settings, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    /// Every setting, in the order the Preferences tool shows them.
    pub fields: Vec<Field>,
    /// The pointer image.
    pub pointer: Pointer,
}

/// Whether a file is Preferences: named `system-configuration`, in any case,
/// and exactly [`SIZE`] bytes. The format has no magic number, so the name and
/// the size are all there is — and every corpus file satisfies both.
#[must_use]
pub fn is_preferences(name: &[u8], len: usize) -> bool {
    name.eq_ignore_ascii_case(FILE_NAME.as_bytes()) && len == SIZE
}

/// Decode the 232 bytes. `None` for anything that is not exactly that long.
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<Preferences> {
    if bytes.len() != SIZE {
        return None;
    }
    let mut out = Out::default();
    screen(bytes, &mut out);
    input(bytes, &mut out);
    let pointer = pointer(bytes, &mut out)?;
    workbench_colours(bytes, &mut out);
    printer(bytes, &mut out);
    graphics(bytes, &mut out);
    serial(bytes, &mut out);
    other(bytes, &mut out);
    Some(Preferences {
        fields: out.0,
        pointer,
    })
}

#[derive(Default)]
struct Out(Vec<Field>);

impl Out {
    fn add(&mut self, group: &'static str, label: &'static str, value: String) {
        self.0.push(Field {
            group,
            label: label.to_owned(),
            value,
            colour: None,
        });
    }

    fn colour(&mut self, group: &'static str, label: &'static str, amiga: u16) {
        self.0.push(Field {
            group,
            label: label.to_owned(),
            value: format!("${:03x}", amiga & 0x0FFF),
            colour: Some(rgb(amiga)),
        });
    }
}

// Field offsets, from `struct Preferences`.
const FONT_HEIGHT: usize = 0;
const PRINTER_PORT: usize = 1;
const BAUD_RATE: usize = 2;
const KEY_RPT_SPEED: usize = 4;
const KEY_RPT_DELAY: usize = 12;
const DOUBLE_CLICK: usize = 20;
const POINTER_MATRIX: usize = 28;
const X_OFFSET: usize = 100;
const Y_OFFSET: usize = 101;
const COLOR17: usize = 102;
const POINTER_TICKS: usize = 108;
const COLOR0: usize = 110;
const VIEW_X_OFFSET: usize = 118;
const VIEW_Y_OFFSET: usize = 119;
const VIEW_INIT_X: usize = 120;
const VIEW_INIT_Y: usize = 122;
const ENABLE_CLI: usize = 124;
const PRINTER_TYPE: usize = 126;
const PRINTER_FILENAME: usize = 128;
const PRINT_PITCH: usize = 158;
const PRINT_QUALITY: usize = 160;
const PRINT_SPACING: usize = 162;
const PRINT_LEFT_MARGIN: usize = 164;
const PRINT_RIGHT_MARGIN: usize = 166;
const PRINT_IMAGE: usize = 168;
const PRINT_ASPECT: usize = 170;
const PRINT_SHADE: usize = 172;
const PRINT_THRESHOLD: usize = 174;
const PAPER_SIZE: usize = 176;
const PAPER_LENGTH: usize = 178;
const PAPER_TYPE: usize = 180;
const SER_RW_BITS: usize = 182;
const SER_STOP_BUF: usize = 183;
const SER_PAR_SHK: usize = 184;
const LACE_WB: usize = 185;
const PAD: usize = 186;
const PRT_DEV_NAME: usize = 198;
const DEFAULT_PRT_UNIT: usize = 214;
const DEFAULT_SER_UNIT: usize = 215;
const ROW_SIZE_CHANGE: usize = 216;
const COLUMN_SIZE_CHANGE: usize = 217;
const PRINT_FLAGS: usize = 218;
const PRINT_MAX_WIDTH: usize = 220;
const PRINT_MAX_HEIGHT: usize = 222;
const PRINT_DENSITY: usize = 224;
const PRINT_X_OFFSET: usize = 225;
const WB_WIDTH: usize = 226;
const WB_HEIGHT: usize = 228;
const WB_DEPTH: usize = 230;
const EXT_SIZE: usize = 231;

fn byte(b: &[u8], at: usize) -> u8 {
    u8_at(b, at).unwrap_or(0)
}

fn signed(b: &[u8], at: usize) -> i8 {
    i8::from_ne_bytes([byte(b, at)])
}

fn word(b: &[u8], at: usize) -> u16 {
    u16_at(b, at).unwrap_or(0)
}

fn long(b: &[u8], at: usize) -> u32 {
    u32_at(b, at).unwrap_or(0)
}

/// An Amiga `$0RGB` colour as `0xRRGGBB`.
fn rgb(amiga: u16) -> u32 {
    let r = u32::from((amiga >> 8) & 0xF);
    let g = u32::from((amiga >> 4) & 0xF);
    let b = u32::from(amiga & 0xF);
    // Each nibble doubled into a byte: $f is $ff, $8 is $88.
    (r * 0x11) << 16 | (g * 0x11) << 8 | (b * 0x11)
}

/// A NUL-terminated name in a fixed field, as Latin-1.
fn name(b: &[u8], at: usize, len: usize) -> String {
    let field = at
        .checked_add(len)
        .and_then(|end| b.get(at..end))
        .unwrap_or_default();
    let text = field.split(|&c| c == 0).next().unwrap_or_default();
    text.iter().map(|&c| char::from(c)).collect()
}

/// A `struct timeval` — seconds, then microseconds — as a duration in words.
fn duration(b: &[u8], at: usize) -> String {
    let seconds = long(b, at);
    let micros = long(b, at.saturating_add(4));
    // Integer arithmetic throughout: milliseconds and their fraction.
    let millis = u64::from(seconds)
        .saturating_mul(1000)
        .saturating_add(u64::from(micros / 1000));
    let fraction = micros % 1000;
    if millis >= 1000 {
        let whole = millis / 1000;
        let hundredths = (millis % 1000) / 10;
        return format!("{whole}.{hundredths:02} s");
    }
    if fraction == 0 {
        return format!("{millis} ms");
    }
    let digits = format!("{fraction:03}");
    format!("{millis}.{} ms", digits.trim_end_matches('0'))
}

fn unknown(value: impl core::fmt::Display) -> String {
    format!("unknown ({value})")
}

fn on_off(on: bool) -> String {
    if on { "on" } else { "off" }.to_owned()
}

fn screen(b: &[u8], out: &mut Out) {
    const G: &str = "Screen";
    out.add(
        G,
        "Text size",
        match byte(b, FONT_HEIGHT) {
            8 => "80 columns (Topaz 8)".to_owned(),
            9 => "60 columns (Topaz 9)".to_owned(),
            n => unknown(format!("font height {n}")),
        },
    );
    out.add(G, "Interlaced Workbench", on_off(byte(b, LACE_WB) & 1 != 0));
    out.add(
        G,
        "Screen position",
        format!(
            "moved {:+} across, {:+} down from the default",
            signed(b, VIEW_X_OFFSET),
            signed(b, VIEW_Y_OFFSET)
        ),
    );
    out.add(
        G,
        "View origin",
        format!(
            "{}, {} (set by the system)",
            i16_at(b, VIEW_INIT_X).unwrap_or(0),
            i16_at(b, VIEW_INIT_Y).unwrap_or(0)
        ),
    );
    let rows = signed(b, ROW_SIZE_CHANGE);
    let columns = signed(b, COLUMN_SIZE_CHANGE);
    if rows != 0 || columns != 0 {
        out.add(
            G,
            "Text area",
            format!("{rows:+} rows, {columns:+} columns from the default"),
        );
    }
    let (width, height, depth) = (word(b, WB_WIDTH), word(b, WB_HEIGHT), byte(b, WB_DEPTH));
    if width != 0 || height != 0 || depth != 0 {
        out.add(
            G,
            "Workbench size",
            format!("{width} × {height}, {depth} bitplanes"),
        );
    }
}

fn input(b: &[u8], out: &mut Out) {
    const G: &str = "Keyboard and mouse";
    out.add(G, "Key repeat delay", duration(b, KEY_RPT_DELAY));
    out.add(
        G,
        "Key repeat rate",
        format!("every {}", duration(b, KEY_RPT_SPEED)),
    );
    out.add(
        G,
        "Double-click",
        format!("within {}", duration(b, DOUBLE_CLICK)),
    );
    out.add(
        G,
        "Mouse speed",
        match word(b, POINTER_TICKS) {
            1 => "1 (fastest)".to_owned(),
            n => format!("{n} (1 is fastest)"),
        },
    );
}

fn pointer(b: &[u8], out: &mut Out) -> Option<Pointer> {
    const G: &str = "Pointer";
    // POINTERSIZE words: a position/control pair, sixteen rows of two
    // bitplane words, and a terminating pair.
    let mut pixels = Vec::with_capacity(Pointer::SIDE * Pointer::SIDE);
    for row in 0..Pointer::SIDE {
        let at = POINTER_MATRIX
            .checked_add(4)?
            .checked_add(row.checked_mul(4)?)?;
        let plane0 = word(b, at);
        let plane1 = word(b, at.checked_add(2)?);
        for bit in (0..16).rev() {
            let low = u8::from(plane0 >> bit & 1 != 0);
            let high = u8::from(plane1 >> bit & 1 != 0);
            pixels.push(low | high << 1);
        }
    }
    let colours = [
        rgb(word(b, COLOR17)),
        rgb(word(b, COLOR17.checked_add(2)?)),
        rgb(word(b, COLOR17.checked_add(4)?)),
    ];
    let offset = (signed(b, X_OFFSET), signed(b, Y_OFFSET));
    out.add(
        G,
        "Hot spot offset",
        format!(
            "{}, {} (where the image sits from the pointer's position)",
            offset.0, offset.1
        ),
    );
    for (i, label) in ["Colour 17", "Colour 18", "Colour 19"]
        .into_iter()
        .enumerate()
    {
        let at = COLOR17.checked_add(i.checked_mul(2)?)?;
        out.colour(G, label, word(b, at));
    }
    Some(Pointer {
        pixels,
        colours,
        offset,
    })
}

fn workbench_colours(b: &[u8], out: &mut Out) {
    const G: &str = "Workbench colours";
    let labels = ["Colour 0 (background)", "Colour 1", "Colour 2", "Colour 3"];
    for (i, label) in labels.into_iter().enumerate() {
        let at = COLOR0.saturating_add(i.saturating_mul(2));
        out.colour(G, label, word(b, at));
    }
}

fn printer(b: &[u8], out: &mut Out) {
    const G: &str = "Printer";
    const TYPES: [&str; 13] = [
        "custom: the driver named below",
        "Alphacom Alphapro 101",
        "Brother HR-15XL",
        "Commodore MPS-1000",
        "Diablo 630",
        "Diablo Advantage D25",
        "Diablo C-150",
        "Epson",
        "Epson JX-80",
        "Okimate 20",
        "Qume LetterPro 20",
        "HP LaserJet",
        "HP LaserJet Plus",
    ];
    out.add(
        G,
        "Port",
        match byte(b, PRINTER_PORT) {
            0 => "parallel".to_owned(),
            1 => "serial".to_owned(),
            n => unknown(n),
        },
    );
    let kind = word(b, PRINTER_TYPE);
    out.add(
        G,
        "Printer",
        TYPES
            .get(usize::from(kind))
            .map_or_else(|| unknown(kind), |&t| t.to_owned()),
    );
    out.add(G, "Driver name", quoted(&name(b, PRINTER_FILENAME, 30)));
    out.add(
        G,
        "Pitch",
        match word(b, PRINT_PITCH) {
            0x000 => "pica (10 characters per inch)".to_owned(),
            0x400 => "elite (12 characters per inch)".to_owned(),
            0x800 => "fine (condensed)".to_owned(),
            n => unknown(format!("${n:x}")),
        },
    );
    out.add(
        G,
        "Quality",
        match word(b, PRINT_QUALITY) {
            0x000 => "draft".to_owned(),
            0x100 => "letter".to_owned(),
            n => unknown(format!("${n:x}")),
        },
    );
    out.add(
        G,
        "Line spacing",
        match word(b, PRINT_SPACING) {
            0x000 => "6 lines per inch".to_owned(),
            0x200 => "8 lines per inch".to_owned(),
            n => unknown(format!("${n:x}")),
        },
    );
    out.add(
        G,
        "Margins",
        format!(
            "columns {} to {}",
            word(b, PRINT_LEFT_MARGIN),
            word(b, PRINT_RIGHT_MARGIN)
        ),
    );
    out.add(G, "Paper", paper_size(word(b, PAPER_SIZE)));
    out.add(
        G,
        "Paper length",
        format!("{} lines", word(b, PAPER_LENGTH)),
    );
    out.add(
        G,
        "Paper feed",
        match word(b, PAPER_TYPE) {
            0x00 => "fanfold (continuous)".to_owned(),
            0x80 => "single sheets".to_owned(),
            n => unknown(format!("${n:x}")),
        },
    );
    printer_area(b, out);
}

fn quoted(text: &str) -> String {
    if text.is_empty() {
        "none".to_owned()
    } else {
        format!("\"{text}\"")
    }
}

fn paper_size(code: u16) -> String {
    match code {
        0x00 => "US letter (8.5 × 11 in)".to_owned(),
        0x10 => "US legal (8.5 × 14 in)".to_owned(),
        0x20 => "narrow tractor (9.5 × 11 in)".to_owned(),
        0x30 => "wide tractor (14.86 × 11 in)".to_owned(),
        0x40 => "custom (the maximum width and height below)".to_owned(),
        0x50 => "A0".to_owned(),
        0x60 => "A1".to_owned(),
        0x70 => "A2".to_owned(),
        0x80 => "A3".to_owned(),
        0x90 => "A4".to_owned(),
        0xA0 => "A5".to_owned(),
        0xB0 => "A6".to_owned(),
        0xC0 => "A7".to_owned(),
        0xD0 => "A8".to_owned(),
        n => unknown(format!("${n:x}")),
    }
}

/// Offset 186's thirty bytes: 2.0's device name and units when the padding
/// before them is clean, 1.x's single name when it is not.
fn printer_area(b: &[u8], out: &mut Out) {
    const G: &str = "Printer";
    let pad = PAD
        .checked_add(12)
        .and_then(|end| b.get(PAD..end))
        .unwrap_or_default();
    if pad.iter().all(|&c| c == 0) {
        let device = name(b, PRT_DEV_NAME, 16);
        let printable =
            !device.trim().is_empty() && device.chars().all(|c| (' '..='~').contains(&c));
        if printable {
            out.add(G, "Printer device (2.0)", format!("{device}.device"));
        }
        let (prt, ser) = (byte(b, DEFAULT_PRT_UNIT), byte(b, DEFAULT_SER_UNIT));
        if prt != 0 || ser != 0 {
            out.add(
                G,
                "Units (2.0)",
                format!("printer unit {prt}, serial unit {ser}"),
            );
        }
    } else {
        // Leftover bytes after an empty name say nothing worth a line.
        let work = name(b, PAD, 30);
        if !work.is_empty() {
            out.add(G, "Work file name (1.x)", quoted(&work));
        }
    }
}

fn graphics(b: &[u8], out: &mut Out) {
    const G: &str = "Graphics printing";
    out.add(
        G,
        "Image",
        match word(b, PRINT_IMAGE) {
            0 => "positive".to_owned(),
            1 => "negative".to_owned(),
            n => unknown(n),
        },
    );
    out.add(
        G,
        "Aspect",
        match word(b, PRINT_ASPECT) {
            0 => "horizontal".to_owned(),
            1 => "vertical".to_owned(),
            n => unknown(n),
        },
    );
    out.add(
        G,
        "Shade",
        match word(b, PRINT_SHADE) {
            0 => "black and white".to_owned(),
            1 => "grey scale".to_owned(),
            2 => "colour".to_owned(),
            n => unknown(n),
        },
    );
    out.add(
        G,
        "Threshold",
        i16_at(b, PRINT_THRESHOLD).unwrap_or(0).to_string(),
    );
    let flags = word(b, PRINT_FLAGS);
    out.add(G, "Options", print_flags(flags));
    out.add(
        G,
        "Maximum size",
        format!(
            "{} × {} in",
            tenths(word(b, PRINT_MAX_WIDTH)),
            tenths(word(b, PRINT_MAX_HEIGHT))
        ),
    );
    out.add(G, "Density", byte(b, PRINT_DENSITY).to_string());
    out.add(
        G,
        "Left offset",
        format!("{} in", tenths(u16::from(byte(b, PRINT_X_OFFSET)))),
    );
}

/// Tenths of an inch, as inches.
fn tenths(value: u16) -> String {
    format!("{}.{}", value / 10, value % 10)
}

fn print_flags(flags: u16) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let correct: Vec<&str> = [(0x1, "red"), (0x2, "green"), (0x4, "blue")]
        .into_iter()
        .filter(|&(bit, _)| flags & bit != 0)
        .map(|(_, name)| name)
        .collect();
    let correction;
    if !correct.is_empty() {
        correction = format!("correct {} shades", correct.join(", "));
        parts.push(&correction);
    }
    if flags & 0x0008 != 0 {
        parts.push("centre the image");
    }
    parts.push(match flags & 0x00F0 {
        0x0000 => "ignore the maximum size",
        0x0010 => "maximum size as a bound",
        0x0020 => "maximum size exactly",
        0x0040 => "maximum size in printer pixels",
        0x0080 => "maximum size as multipliers",
        _ => "several size modes at once",
    });
    if flags & 0x0100 != 0 {
        parts.push("integer scaling");
    }
    parts.push(match flags & 0x0600 {
        0x0000 => "ordered dithering",
        0x0200 => "halftone dithering",
        0x0400 => "Floyd-Steinberg dithering",
        _ => "two dithering modes at once",
    });
    if flags & 0x0800 != 0 {
        parts.push("anti-aliased");
    }
    if flags & 0x1000 != 0 {
        parts.push("grey scale 2 (for a high-resolution monitor)");
    }
    parts.join("; ")
}

fn serial(b: &[u8], out: &mut Out) {
    const G: &str = "Serial port";
    const BAUD: [&str; 8] = [
        "110",
        "300",
        "1200",
        "2400",
        "4800",
        "9600",
        "19200",
        "MIDI (31250)",
    ];
    const BUFFER: [&str; 6] = ["512", "1024", "2048", "4096", "8000", "16000"];
    const PARITY: [&str; 5] = ["none", "even", "odd", "mark", "space"];
    const HANDSHAKE: [&str; 3] = ["xON/xOFF", "RTS/CTS", "none"];
    let baud = word(b, BAUD_RATE);
    out.add(
        G,
        "Baud rate",
        BAUD.get(usize::from(baud))
            .map_or_else(|| unknown(baud), |&s| s.to_owned()),
    );
    // Each nibble holds 8 minus the number of bits.
    let rw = byte(b, SER_RW_BITS);
    let bits = |nibble: u8| {
        8u8.checked_sub(nibble)
            .map_or_else(|| unknown(nibble), |n| n.to_string())
    };
    out.add(
        G,
        "Data bits",
        format!("{} read, {} written", bits(rw >> 4), bits(rw & 0xF)),
    );
    let stop_buf = byte(b, SER_STOP_BUF);
    out.add(
        G,
        "Stop bits",
        (u16::from(stop_buf >> 4).saturating_add(1)).to_string(),
    );
    let buffer = stop_buf & 0xF;
    out.add(
        G,
        "Buffer",
        BUFFER
            .get(usize::from(buffer))
            .map_or_else(|| unknown(buffer), |&s| format!("{s} bytes")),
    );
    let par_shk = byte(b, SER_PAR_SHK);
    let (parity, handshake) = (par_shk >> 4, par_shk & 0xF);
    out.add(
        G,
        "Parity",
        PARITY
            .get(usize::from(parity))
            .map_or_else(|| unknown(parity), |&s| s.to_owned()),
    );
    out.add(
        G,
        "Handshake",
        HANDSHAKE
            .get(usize::from(handshake))
            .map_or_else(|| unknown(handshake), |&s| s.to_owned()),
    );
}

fn other(b: &[u8], out: &mut Out) {
    const G: &str = "Other";
    // A BOOL on 1.x; 2.0 keeps two flags in its top bits.
    let cli = word(b, ENABLE_CLI);
    out.add(G, "CLI", on_off(cli & 0x3FFF != 0));
    let mut flags = Vec::new();
    if cli & 0x4000 != 0 {
        flags.push("screen dragging");
    }
    if cli & 0x8000 != 0 {
        flags.push("mouse acceleration");
    }
    if !flags.is_empty() {
        out.add(G, "Also on (2.0)", flags.join(", "));
    }
    let ext = byte(b, EXT_SIZE);
    if ext != 0 {
        out.add(
            G,
            "Extension",
            format!("{ext} blocks of 64 bytes declared beyond the 232"),
        );
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "tests over fixed bytes"
)]
mod tests {
    use super::*;
    use ade_endian::put_u32;

    /// A file as the 1.3 Workbench disk ships it, near enough: Topaz 8, the
    /// blue/white/black/orange Workbench, default timings, Epson on parallel.
    fn workbench() -> Vec<u8> {
        let mut b = vec![0u8; SIZE];
        b[FONT_HEIGHT] = 8;
        b[BAUD_RATE + 1] = 5;
        put_u32(&mut b, KEY_RPT_SPEED + 4, 100_000).unwrap();
        put_u32(&mut b, KEY_RPT_DELAY + 4, 600_000).unwrap();
        put_u32(&mut b, DOUBLE_CLICK, 1).unwrap();
        put_u32(&mut b, DOUBLE_CLICK + 4, 200_000).unwrap();
        // Row 0: the top-left pixel in colour 1, the next in colour 2, the
        // next in colour 3.
        b[POINTER_MATRIX + 4] = 0b1010_0000;
        b[POINTER_MATRIX + 6] = 0b0110_0000;
        b[X_OFFSET] = 0xFE; // -2
        b[Y_OFFSET] = 0xFF; // -1
        b[COLOR17..COLOR17 + 6].copy_from_slice(&[0x0E, 0x44, 0x00, 0x00, 0x0E, 0xEC]);
        b[POINTER_TICKS + 1] = 1;
        b[COLOR0..COLOR0 + 8].copy_from_slice(&[0x00, 0x5A, 0x0F, 0xFF, 0x00, 0x02, 0x0F, 0x80]);
        b[VIEW_INIT_X + 1] = 129;
        b[VIEW_INIT_Y + 1] = 44;
        b[PRINTER_TYPE + 1] = 7;
        b[PRINTER_FILENAME..PRINTER_FILENAME + 7].copy_from_slice(b"generic");
        b[PAPER_SIZE + 1] = 0x20;
        b[SER_PAR_SHK] = 0x01;
        b[SER_STOP_BUF] = 0x05;
        b
    }

    fn value(p: &Preferences, label: &str) -> String {
        p.fields
            .iter()
            .find(|f| f.label == label)
            .unwrap_or_else(|| panic!("no field {label}"))
            .value
            .clone()
    }

    #[test]
    fn a_workbench_file_reads_as_its_settings() {
        let p = decode(&workbench()).unwrap();
        assert_eq!(value(&p, "Text size"), "80 columns (Topaz 8)");
        assert_eq!(value(&p, "Key repeat delay"), "600 ms");
        assert_eq!(value(&p, "Key repeat rate"), "every 100 ms");
        assert_eq!(value(&p, "Double-click"), "within 1.20 s");
        assert_eq!(value(&p, "Mouse speed"), "1 (fastest)");
        assert_eq!(value(&p, "Printer"), "Epson");
        assert_eq!(value(&p, "Driver name"), "\"generic\"");
        assert_eq!(value(&p, "Paper"), "narrow tractor (9.5 × 11 in)");
        assert_eq!(value(&p, "Baud rate"), "9600");
        assert_eq!(value(&p, "Data bits"), "8 read, 8 written");
        assert_eq!(value(&p, "Stop bits"), "1");
        assert_eq!(value(&p, "Buffer"), "16000 bytes");
        assert_eq!(value(&p, "Handshake"), "RTS/CTS");
        assert_eq!(value(&p, "View origin"), "129, 44 (set by the system)");
        assert_eq!(value(&p, "CLI"), "off");
    }

    #[test]
    fn colours_carry_their_rgb() {
        let p = decode(&workbench()).unwrap();
        let background = p
            .fields
            .iter()
            .find(|f| f.label == "Colour 0 (background)")
            .unwrap();
        assert_eq!(background.value, "$05a");
        assert_eq!(background.colour, Some(0x0000_55AA));
        let orange = p.fields.iter().find(|f| f.label == "Colour 3").unwrap();
        assert_eq!(orange.colour, Some(0xFF_8800));
    }

    #[test]
    fn the_pointer_is_its_two_bitplanes_and_its_offset() {
        let p = decode(&workbench()).unwrap();
        assert_eq!(p.pointer.pixels.len(), 256);
        assert_eq!(&p.pointer.pixels[..4], &[1, 2, 3, 0]);
        assert_eq!(p.pointer.offset, (-2, -1));
        assert_eq!(p.pointer.colours, [0xEE_4444, 0x00_0000, 0xEE_EECC]);
        assert_eq!(
            value(&p, "Hot spot offset"),
            "-2, -1 (where the image sits from the pointer's position)"
        );
    }

    #[test]
    fn short_durations_keep_their_precision() {
        // Seen in the corpus: a key repeat of 45 microseconds.
        let mut b = workbench();
        put_u32(&mut b, KEY_RPT_SPEED + 4, 45).unwrap();
        assert_eq!(
            value(&decode(&b).unwrap(), "Key repeat rate"),
            "every 0.045 ms"
        );
        put_u32(&mut b, KEY_RPT_SPEED + 4, 31_140).unwrap();
        assert_eq!(
            value(&decode(&b).unwrap(), "Key repeat rate"),
            "every 31.14 ms"
        );
    }

    #[test]
    fn the_printer_area_is_read_one_way_or_the_other() {
        // 1.x text spilling past the 2.0 padding: one name, not two fields.
        let mut b = workbench();
        b[PAD..PAD + 20].copy_from_slice(b"canon_pw1080l_LowRes");
        let p = decode(&b).unwrap();
        assert_eq!(
            value(&p, "Work file name (1.x)"),
            "\"canon_pw1080l_LowRes\""
        );
        assert!(p.fields.iter().all(|f| f.label != "Printer device (2.0)"));

        // Clean padding: 2.0's device and units.
        let mut b = workbench();
        b[PRT_DEV_NAME..PRT_DEV_NAME + 8].copy_from_slice(b"parallel");
        b[DEFAULT_SER_UNIT] = 1;
        let p = decode(&b).unwrap();
        assert_eq!(value(&p, "Printer device (2.0)"), "parallel.device");
        assert_eq!(value(&p, "Units (2.0)"), "printer unit 0, serial unit 1");
        assert!(p.fields.iter().all(|f| f.label != "Work file name (1.x)"));
    }

    #[test]
    fn the_2_0_flags_in_enable_cli_are_named() {
        let mut b = workbench();
        b[ENABLE_CLI] = 0x80;
        let p = decode(&b).unwrap();
        assert_eq!(value(&p, "CLI"), "off");
        assert_eq!(value(&p, "Also on (2.0)"), "mouse acceleration");
    }

    #[test]
    fn unknown_codes_say_so_rather_than_guess() {
        let mut b = workbench();
        b[FONT_HEIGHT] = 11;
        b[BAUD_RATE + 1] = 9;
        b[PRINTER_TYPE + 1] = 0x40;
        let p = decode(&b).unwrap();
        assert_eq!(value(&p, "Text size"), "unknown (font height 11)");
        assert_eq!(value(&p, "Baud rate"), "unknown (9)");
        assert_eq!(value(&p, "Printer"), "unknown (64)");
    }

    #[test]
    fn only_the_right_name_and_size_is_preferences() {
        assert!(is_preferences(b"system-configuration", 232));
        assert!(is_preferences(b"System-Configuration", 232));
        assert!(!is_preferences(b"system-configuration", 231));
        assert!(!is_preferences(b"system-configuration.bak", 232));
        assert!(decode(&[0; 231]).is_none());
        assert!(decode(&[0; 233]).is_none());
    }
}
