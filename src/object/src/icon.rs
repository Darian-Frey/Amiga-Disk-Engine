//! Workbench icons: the `.info` file beside a program, a project, a drawer or
//! a disk (F-033).
//!
//! An icon is a `struct DiskObject` written out by `icon.library`, with what
//! its pointers pointed at following it in a fixed order. The order is not in
//! any header: it is taken from AROS's `icon.library` reader
//! (`workbench/libs/icon/diskobjio.c`, read locally, not committed), and the
//! structures from `workbench/workbench.h` and `intuition/intuition.h`.
//! SPEC §Workbench icons has the table.
//!
//! What follows the 78-byte `DiskObject`, each part only when the structure
//! says it is there:
//!
//! 1. the drawer's window, 56 bytes, when `do_DrawerData` is set;
//! 2. the image, a 20-byte `struct Image` and its bitplanes, when
//!    `GadgetRender` is set or the gadget is flagged `GFLG_GADGIMAGE`;
//! 3. the selected image, likewise, when `SelectRender` is set or the
//!    highlight mode is `GFLG_GADGHIMAGE`;
//! 4. the default tool, then the tool types, then the tool window: each
//!    string a length (counting its NUL) and the bytes;
//! 5. six more bytes of drawer settings, on a revision-1 icon with a drawer —
//!    when the file has them, which not all do.
//!
//! A pointer stored in the file is only ever a flag — non-zero means "there is
//! one, it follows" — since the address it held belonged to the Amiga that
//! saved it.
//!
//! # What an icon does not say
//!
//! Its colours. The image holds pen numbers; what colour pen 1 is depends on
//! the screen it is shown on. [`Icon::revision`] says which Workbench saved it,
//! which is the best evidence of the palette it was drawn for.

use ade_endian::{i16_at, i32_at, u8_at, u16_at, u32_at};

use crate::decoded::Field;

/// `WB_DISKMAGIC`: the first two bytes of every icon.
pub const MAGIC: u16 = 0xE310;

/// `NO_ICON_POSITION`: Workbench places the icon itself.
const NO_ICON_POSITION: i32 = i32::MIN;

/// Bytes of `struct DiskObject` as stored.
const DISK_OBJECT: usize = 78;
/// Bytes of `struct OldDrawerData` as stored: a `NewWindow` and two longs.
const OLD_DRAWER_DATA: usize = 56;
/// Bytes of `struct Image` as stored.
const IMAGE_HEADER: usize = 20;

/// The widest or tallest image ADE will draw. Bounds what a header can make
/// it allocate (D-006): an image of depth 0 stores no bitplanes, so without
/// this a 20-byte header could ask for four billion pixels. The corpus's
/// largest icon is 330 pixels across.
const MAX_SIDE: u16 = 1024;

/// The deepest image ADE will draw. Workbench screens went to 8 planes; a
/// larger number is not an image but the bytes after one.
const MAX_DEPTH: u16 = 8;

/// What an icon is the icon of: `do_Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `WBDISK`.
    Disk,
    /// `WBDRAWER`.
    Drawer,
    /// `WBTOOL`: a program.
    Tool,
    /// `WBPROJECT`: a document, opened with its default tool.
    Project,
    /// `WBGARBAGE`: the trashcan.
    Trashcan,
    /// `WBDEVICE`.
    Device,
    /// `WBKICK`: a Kickstart disk.
    Kick,
    /// `WBAPPICON`.
    AppIcon,
    /// Anything else, as stored.
    Other(u8),
}

impl Kind {
    fn from_byte(b: u8) -> Self {
        match b {
            1 => Self::Disk,
            2 => Self::Drawer,
            3 => Self::Tool,
            4 => Self::Project,
            5 => Self::Trashcan,
            6 => Self::Device,
            7 => Self::Kick,
            8 => Self::AppIcon,
            other => Self::Other(other),
        }
    }

    /// The kind in words.
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Self::Disk => "Disk".to_owned(),
            Self::Drawer => "Drawer".to_owned(),
            Self::Tool => "Tool (a program)".to_owned(),
            Self::Project => "Project (a document, opened with its default tool)".to_owned(),
            Self::Trashcan => "Trashcan".to_owned(),
            Self::Device => "Device".to_owned(),
            Self::Kick => "Kickstart disk".to_owned(),
            Self::AppIcon => "AppIcon".to_owned(),
            Self::Other(b) => format!("unknown ({b})"),
        }
    }
}

/// How the icon shows that it is selected: the gadget's highlight bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Highlight {
    /// `GFLG_GADGHCOMP`: the image's colours are complemented.
    Complement,
    /// `GFLG_GADGHBOX`: a box is drawn round it.
    Box,
    /// `GFLG_GADGHIMAGE`: the second image is shown.
    Image,
    /// `GFLG_GADGHNONE`: nothing changes.
    Nothing,
}

/// One of an icon's images: pen numbers, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Pixels across.
    pub width: usize,
    /// Pixels down.
    pub height: usize,
    /// Bitplanes stored.
    pub depth: u16,
    /// `PlanePick` as stored: which of the screen's planes the stored planes
    /// go to. ADE draws stored plane *n* as bit *n* of the pen, which is
    /// what it means whenever the low `depth` bits are set — every corpus
    /// icon but seven.
    pub plane_pick: u8,
    /// One pen number per pixel.
    pub pens: Vec<u8>,
}

/// A drawer's window, as the icon of a disk, drawer or trashcan keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawer {
    /// Where the window opens: left, top.
    pub position: (i16, i16),
    /// Its size: width, height.
    pub size: (i16, i16),
    /// How far its contents are scrolled: x, y.
    pub scrolled: (i32, i32),
    /// `dd_Flags`, on a revision-1 icon: 1 shows only icons, 2 all files.
    pub flags: Option<u32>,
    /// `dd_ViewModes`, on a revision-1 icon: 1 icons, 2–5 a list by name,
    /// date, size or type.
    pub view: Option<u16>,
}

/// A decoded icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    /// What it is the icon of.
    pub kind: Kind,
    /// `do_Gadget.UserData & 0xFF`: 0 for an icon saved by Workbench 1.x, 1
    /// for one saved by 2.0 or later (`WB_DISKREVISION`).
    pub revision: u8,
    /// The gadget's size: width, height.
    pub size: (i16, i16),
    /// Where it sits in its window, or `None` when Workbench places it.
    pub position: Option<(i32, i32)>,
    /// How it shows that it is selected.
    pub highlight: Highlight,
    /// `do_StackSize`: the stack a tool asks for, in bytes.
    pub stack: i32,
    /// The program a project opens with, or the one a tool's icon names.
    pub default_tool: Option<Vec<u8>>,
    /// The tool types, in order, without the NewIcons image lines.
    pub tool_types: Vec<Vec<u8>>,
    /// How many tool-type lines carry a NewIcons image, which ADE does not
    /// draw.
    pub newicons_lines: usize,
    /// `do_ToolWindow`: the window a tool's output goes to.
    pub tool_window: Option<Vec<u8>>,
    /// The drawer's window, for an icon that has one.
    pub drawer: Option<Drawer>,
    /// The image.
    pub image: Option<Image>,
    /// The image shown when selected, for an icon that has a second one.
    pub selected: Option<Image>,
    /// Bytes after the icon that are not zero, which ADE does not read.
    pub trailing: usize,
}

/// Why bytes are not an icon ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAnIcon(pub String);

impl core::fmt::Display for NotAnIcon {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `bytes` start like an icon.
#[must_use]
pub fn is_icon(bytes: &[u8]) -> bool {
    u16_at(bytes, 0) == Ok(MAGIC)
}

/// A cursor over the bytes after the `DiskObject`.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn short(&self, what: &str) -> NotAnIcon {
        NotAnIcon(format!(
            "the icon ends at byte {} in the middle of its {what}",
            self.bytes.len()
        ))
    }

    fn take(&mut self, n: usize, what: &str) -> Result<&[u8], NotAnIcon> {
        let end = self.at.checked_add(n).ok_or_else(|| self.short(what))?;
        let out = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| self.short(what))?;
        self.at = end;
        Ok(out)
    }

    fn long(&mut self, what: &str) -> Result<u32, NotAnIcon> {
        let b = self.take(4, what)?;
        u32_at(b, 0).map_err(|_| NotAnIcon(format!("no room for the {what}")))
    }

    /// A string as `icon.library` writes one: a length counting the NUL,
    /// then the bytes. Returned without the NUL.
    fn string(&mut self, what: &str) -> Result<Vec<u8>, NotAnIcon> {
        let len = self.long(what)? as usize;
        let bytes = self.take(len, what)?;
        let text = bytes.split(|&b| b == 0).next().unwrap_or(&[]);
        Ok(text.to_vec())
    }

    fn image(&mut self, what: &str) -> Result<Image, NotAnIcon> {
        let header = self.take(IMAGE_HEADER, what)?;
        let field = |at| u16_at(header, at).unwrap_or(0);
        let (width, height, depth) = (field(4), field(6), field(8));
        let plane_pick = u8_at(header, 14).unwrap_or(0);
        if depth > MAX_DEPTH {
            return Err(NotAnIcon(format!(
                "its {what} claims {depth} bitplanes; a Workbench image has at most {MAX_DEPTH}"
            )));
        }
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(NotAnIcon(format!(
                "its {what} claims {width} × {height} pixels, larger than any Workbench screen"
            )));
        }
        let (width, height) = (usize::from(width), usize::from(height));
        // Rows are padded to whole 16-bit words, plane after plane.
        let row = width.div_ceil(16).saturating_mul(2);
        let plane = row.saturating_mul(height);
        let data = self.take(plane.saturating_mul(usize::from(depth)), what)?;
        let mut pens = vec![0u8; width.saturating_mul(height)];
        for p in 0..usize::from(depth) {
            let bits = data
                .get(p.saturating_mul(plane)..p.saturating_add(1).saturating_mul(plane))
                .unwrap_or(&[]);
            for y in 0..height {
                for x in 0..width {
                    let byte = bits
                        .get(y.saturating_mul(row).saturating_add(x / 8))
                        .copied()
                        .unwrap_or(0);
                    if byte & (0x80 >> (x % 8)) != 0
                        && let Some(pen) = pens.get_mut(y.saturating_mul(width).saturating_add(x))
                    {
                        *pen |= 1 << p;
                    }
                }
            }
        }
        Ok(Image {
            width,
            height,
            depth,
            plane_pick,
            pens,
        })
    }
}

/// The marker NewIcons writes before the tool types that hold its image.
const NEWICONS_MARKER: &[u8] = b"*** DON'T EDIT THE FOLLOWING LINES!! ***";

/// Read an icon.
///
/// # Errors
/// [`NotAnIcon`] when the magic number is wrong, a part the structure says
/// is there runs off the end, or an image is deeper than any screen.
pub fn parse(bytes: &[u8]) -> Result<Icon, NotAnIcon> {
    if !is_icon(bytes) {
        return Err(NotAnIcon("an icon starts with $E310".to_owned()));
    }
    if bytes.len() < DISK_OBJECT {
        return Err(NotAnIcon(format!(
            "{} bytes, and an icon's header alone is {DISK_OBJECT}",
            bytes.len()
        )));
    }
    let word = |at| u16_at(bytes, at).unwrap_or(0);
    let long = |at| u32_at(bytes, at).unwrap_or(0);
    let signed_word = |at| i16_at(bytes, at).unwrap_or(0);
    let signed_long = |at| i32_at(bytes, at).unwrap_or(0);

    // The gadget starts at 4; its fields from intuition.h.
    let flags = word(16);
    let (gadget_render, select_render) = (long(22), long(26));
    let revision = u8::try_from(long(44) & 0xFF).unwrap_or(0);
    let kind = Kind::from_byte(u8_at(bytes, 48).unwrap_or(0));
    let (default_tool, tool_types) = (long(50), long(54));
    let (x, y) = (signed_long(58), signed_long(62));
    let (drawer_data, tool_window) = (long(66), long(70));

    let highlight = match flags & 3 {
        0 => Highlight::Complement,
        1 => Highlight::Box,
        2 => Highlight::Image,
        _ => Highlight::Nothing,
    };

    let mut r = Reader {
        bytes,
        at: DISK_OBJECT,
    };
    let mut drawer = if drawer_data == 0 {
        None
    } else {
        let d = r.take(OLD_DRAWER_DATA, "drawer window")?;
        Some(Drawer {
            position: (i16_at(d, 0).unwrap_or(0), i16_at(d, 2).unwrap_or(0)),
            size: (i16_at(d, 4).unwrap_or(0), i16_at(d, 6).unwrap_or(0)),
            scrolled: (i32_at(d, 48).unwrap_or(0), i32_at(d, 52).unwrap_or(0)),
            flags: None,
            view: None,
        })
    };
    // GFLG_GADGIMAGE, and the highlight mode that means a second image.
    let image = (gadget_render != 0 || flags & 4 != 0)
        .then(|| r.image("image"))
        .transpose()?;
    let selected = (select_render != 0 || highlight == Highlight::Image)
        .then(|| r.image("selected image"))
        .transpose()?;
    let default_tool = (default_tool != 0)
        .then(|| r.string("default tool"))
        .transpose()?;

    let mut types = Vec::new();
    let mut newicons_lines = 0usize;
    if tool_types != 0 {
        // A count of the pointer array, terminator included.
        let count = (r.long("tool types")? / 4).saturating_sub(1);
        let mut in_newicons = false;
        for _ in 0..count {
            let t = r.string("tool types")?;
            in_newicons |= t == NEWICONS_MARKER;
            if in_newicons || t.starts_with(b"IM1=") || t.starts_with(b"IM2=") {
                newicons_lines = newicons_lines.saturating_add(1);
            } else {
                types.push(t);
            }
        }
    }
    let tool_window = (tool_window != 0)
        .then(|| r.string("tool window"))
        .transpose()?;
    // Optional in practice: 62 corpus icons of revision 1 with a drawer end
    // exactly where these six bytes would start, and were saved by
    // Workbench, which read them back. Absent is not damaged.
    if let Some(d) = drawer.as_mut()
        && revision == 1
        && let Ok(more) = r.take(6, "drawer settings")
    {
        d.flags = u32_at(more, 0).ok();
        d.view = u16_at(more, 4).ok();
    }
    let trailing = bytes
        .get(r.at..)
        .map_or(0, |rest| rest.iter().filter(|&&b| b != 0).count());

    Ok(Icon {
        kind,
        revision,
        size: (signed_word(12), signed_word(14)),
        position: (x != NO_ICON_POSITION).then_some((x, y)),
        highlight,
        stack: signed_long(74),
        default_tool,
        tool_types: types,
        newicons_lines,
        tool_window,
        drawer,
        image,
        selected,
        trailing,
    })
}

/// Bytes as Latin-1 text, which is what an Amiga string is.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

impl Icon {
    /// What the icon says, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const ICON: &str = "Icon";
        let mut out = vec![
            Field::new(ICON, "Kind", self.kind.describe()),
            Field::new(
                ICON,
                "Saved by",
                match self.revision {
                    0 => "Workbench 1.x (revision 0)".to_owned(),
                    1 => "Workbench 2.0 or later (revision 1)".to_owned(),
                    r => format!("unknown (revision {r})"),
                },
            ),
        ];
        if let Some(img) = &self.image {
            out.push(Field::new(
                ICON,
                "Image",
                format!(
                    "{} × {}, {} {}",
                    img.width,
                    img.height,
                    img.depth,
                    if img.depth == 1 {
                        "bitplane"
                    } else {
                        "bitplanes"
                    }
                ),
            ));
        }
        out.push(Field::new(
            ICON,
            "When selected",
            match self.highlight {
                Highlight::Complement => "its colours are inverted",
                Highlight::Box => "a box is drawn round it",
                Highlight::Image if self.selected.is_some() => "the second image is shown",
                Highlight::Image => "a second image, which the icon lacks",
                Highlight::Nothing => "nothing changes",
            },
        ));
        out.push(Field::new(
            ICON,
            "Position",
            self.position.map_or_else(
                || "not set; Workbench places it".to_owned(),
                |(x, y)| format!("{x}, {y}"),
            ),
        ));
        if self.stack != 0 {
            out.push(Field::new(ICON, "Stack", format!("{} bytes", self.stack)));
        }
        if let Some(t) = &self.default_tool {
            out.push(Field::new(ICON, "Default tool", latin1(t)));
        }
        if let Some(t) = &self.tool_window {
            out.push(Field::new(ICON, "Tool window", latin1(t)));
        }
        if self.trailing > 0 {
            out.push(Field::new(
                ICON,
                "After the icon",
                format!("{} bytes ADE does not read", self.trailing),
            ));
        }

        for (n, t) in self.tool_types.iter().enumerate() {
            out.push(Field::new(
                "Tool types",
                n.saturating_add(1).to_string(),
                latin1(t),
            ));
        }
        if self.newicons_lines > 0 {
            out.push(Field::new(
                "Tool types",
                "NewIcons",
                format!(
                    "{} lines hold a NewIcons image, which ADE does not draw",
                    self.newicons_lines
                ),
            ));
        }
        if let Some(d) = &self.drawer {
            out.extend(d.fields());
        }
        out
    }
}

impl Drawer {
    /// The drawer's window, in fields.
    fn fields(&self) -> Vec<Field> {
        const DRAWER: &str = "Drawer window";
        let d = self;
        let mut out = Vec::new();
        out.push(Field::new(
            DRAWER,
            "Opens at",
            format!("{}, {}", d.position.0, d.position.1),
        ));
        out.push(Field::new(
            DRAWER,
            "Size",
            format!("{} × {}", d.size.0, d.size.1),
        ));
        if d.scrolled != (0, 0) {
            out.push(Field::new(
                DRAWER,
                "Scrolled to",
                format!("{}, {}", d.scrolled.0, d.scrolled.1),
            ));
        }
        if let Some(f) = d.flags {
            out.push(Field::new(
                DRAWER,
                "Shows",
                match f {
                    0 => "the default".to_owned(),
                    1 => "only files with icons".to_owned(),
                    2 => "all files".to_owned(),
                    f => format!("unknown ({f})"),
                },
            ));
        }
        if let Some(v) = d.view {
            out.push(Field::new(
                DRAWER,
                "View",
                match v {
                    0 => "the default".to_owned(),
                    1 => "as icons".to_owned(),
                    2 => "as a list, by name".to_owned(),
                    3 => "as a list, by date".to_owned(),
                    4 => "as a list, by size".to_owned(),
                    5 => "as a list, by type".to_owned(),
                    v => format!("unknown ({v})"),
                },
            ));
        }
        out
    }
}

#[cfg(test)]
#[allow(
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

    /// A minimal icon: a 16 × 2 image of 2 planes, a tool type, and
    /// whatever `edit` changes in the header.
    fn build(edit: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut b = vec![0u8; DISK_OBJECT];
        b[0..2].copy_from_slice(&MAGIC.to_be_bytes());
        b[3] = 1; // do_Version
        b[17] = 4; // GFLG_GADGIMAGE; highlight bits 0, complement
        b[22..26].copy_from_slice(&1u32.to_be_bytes()); // GadgetRender
        b[48] = 3; // WBTOOL
        b[54..58].copy_from_slice(&1u32.to_be_bytes()); // ToolTypes
        b[58..62].copy_from_slice(&0x8000_0000u32.to_be_bytes()); // no position
        edit(&mut b);
        // Image: 16 × 2, depth 2. Plane 0 row 0 = $8000, plane 1 row 1 = $0001.
        let mut img = vec![0u8; IMAGE_HEADER];
        img[4..6].copy_from_slice(&16u16.to_be_bytes());
        img[6..8].copy_from_slice(&2u16.to_be_bytes());
        img[8..10].copy_from_slice(&2u16.to_be_bytes());
        img[14] = 3;
        b.extend_from_slice(&img);
        b.extend_from_slice(&[0x80, 0, 0, 0, 0, 0, 0, 1]);
        // Tool types: one entry plus the terminator, then the string.
        b.extend_from_slice(&8u32.to_be_bytes());
        b.extend_from_slice(&10u32.to_be_bytes());
        b.extend_from_slice(b"DONOTWAIT\0");
        b
    }

    #[test]
    fn an_icon_reads_its_parts_in_order() {
        let icon = parse(&build(|_| {})).unwrap();
        assert_eq!(icon.kind, Kind::Tool);
        assert_eq!(icon.position, None);
        assert_eq!(icon.highlight, Highlight::Complement);
        assert_eq!(icon.tool_types, vec![b"DONOTWAIT".to_vec()]);
        let img = icon.image.unwrap();
        assert_eq!((img.width, img.height, img.depth), (16, 2, 2));
        assert_eq!(img.pens[0], 1, "plane 0, top left");
        assert_eq!(img.pens[31], 2, "plane 1, bottom right");
        assert_eq!(img.pens.iter().filter(|&&p| p != 0).count(), 2);
        assert_eq!(icon.trailing, 0);
    }

    #[test]
    fn the_flags_alone_say_an_image_follows_as_aros_reads_them() {
        // No corpus icon sets a flag without the pointer, so this is AROS's
        // rule pinned rather than one the corpus shows: GFLG_GADGIMAGE means
        // an image, GFLG_GADGHIMAGE a second one.
        let mut b = build(|b| {
            b[22..26].fill(0); // no GadgetRender pointer
            b[17] = 4 | 2; // GFLG_GADGIMAGE, GFLG_GADGHIMAGE
        });
        // The second image: a copy of the first, inserted after it.
        let first = DISK_OBJECT..DISK_OBJECT + IMAGE_HEADER + 8;
        let copy = b[first.clone()].to_vec();
        b.splice(first.end..first.end, copy);
        let icon = parse(&b).unwrap();
        assert!(icon.image.is_some());
        assert_eq!(icon.selected, icon.image);
        assert_eq!(icon.tool_types, vec![b"DONOTWAIT".to_vec()]);
    }

    #[test]
    fn a_short_icon_says_where_it_ends() {
        let mut b = build(|_| {});
        b.truncate(b.len() - 3);
        let e = parse(&b).unwrap_err();
        assert!(e.0.contains("tool types"), "{e}");
        assert!(parse(&b[..40]).unwrap_err().0.contains("header"));
        assert!(parse(b"FORM").is_err());
    }

    #[test]
    fn an_image_deeper_than_any_screen_is_refused() {
        let mut b = build(|_| {});
        b[DISK_OBJECT + 9] = 20;
        assert!(parse(&b).unwrap_err().0.contains("20 bitplanes"));
    }

    #[test]
    fn an_image_larger_than_any_screen_is_refused_before_it_is_allocated() {
        // Depth 0 stores no planes, so only the bound stands between a
        // 20-byte header and a 4 GB allocation (D-006).
        let mut b = build(|_| {});
        b[DISK_OBJECT + 4..DISK_OBJECT + 8].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        b[DISK_OBJECT + 9] = 0;
        assert!(parse(&b).unwrap_err().0.contains("65535 × 65535"));
    }

    #[test]
    fn newicons_lines_are_counted_and_kept_out_of_the_tool_types() {
        let mut b = build(|_| {});
        let at = b.len() - 4 - 10 - 4;
        b.truncate(at);
        let lines: [&[u8]; 3] = [b"DONOTWAIT", NEWICONS_MARKER, b"IM1=abc"];
        b.extend_from_slice(&16u32.to_be_bytes());
        for l in lines {
            b.extend_from_slice(&(l.len() as u32 + 1).to_be_bytes());
            b.extend_from_slice(l);
            b.push(0);
        }
        let icon = parse(&b).unwrap();
        assert_eq!(icon.tool_types, vec![b"DONOTWAIT".to_vec()]);
        assert_eq!(icon.newicons_lines, 2);
    }
}
