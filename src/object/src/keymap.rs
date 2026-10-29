//! Keymaps (F-033): what each key types, from `DEVS:Keymaps`.
//!
//! A keymap file is a loadable executable whose one hunk begins with a
//! `KeyMapNode`: an exec `Node` naming the keymap, then a `KeyMap` of eight
//! pointers — the types, mappings, capsable and repeatable bits for the low
//! keys (`$00`–`$3F`) and for the high ones (`$40`–`$77`). Each pointer is an
//! offset into the hunk that the loader relocates, and a key's mapping is
//! itself a pointer when the key types a string or takes part in a dead-key
//! sequence. So a keymap is recognised by its relocations: the name and all
//! eight tables, relocated, at the offsets a `KeyMapNode` puts them.
//!
//! The layout is AROS's `devices/keymap.h`; how a mapping is read is AROS's
//! `MapRawKey()` and `MapANSI()` (`rom/keymap`), read locally and not
//! committed. A plain key holds up to four characters in its long, lowest
//! byte first: unqualified, then each qualifier combination its type names,
//! in the order `MapANSI()`'s tables give — Shift, Alt, Shift+Alt for a key
//! that takes both. A string or dead key points at descriptor pairs, up to
//! eight, in the same order with Control's combinations after.

use ade_endian::{u8_at, u32_at};

use crate::decoded::Field;
use crate::hunk;

/// Where the `KeyMapNode`'s fields sit: `ln_Name`, then `kn_KeyMap`.
const NAME_AT: usize = 10;
const KEYMAP_AT: usize = 14;

/// Keys in each half: `$00`–`$3F`, and `$40`–`$77`.
const LO_KEYS: usize = 64;
const HI_KEYS: usize = 56;

/// `KCF_*` key types.
const KCF_SHIFT: u8 = 1;
const KCF_ALT: u8 = 2;
const KCF_CONTROL: u8 = 4;
const KCF_DEAD: u8 = 0x20;
const KCF_STRING: u8 = 0x40;
const KCF_NOP: u8 = 0x80;

/// Dead-key descriptor types.
const DPF_MOD: u8 = 1;
const DPF_DEAD: u8 = 8;

/// Larger than any keymap; a hunk claiming more is not one.
const MAX_HUNK: usize = 64 * 1024;

/// The qualifier combinations a key's entries stand for, by the key's type
/// bits (Shift 1, Alt 2, Control 4) — `MapANSI()`'s `Qualifiers2`. A plain
/// key holds the first four at most.
const QUALIFIERS: [&[Qualifier]; 8] = {
    use Qualifier::{
        Alt, AltShift, Control, ControlAlt, ControlAltShift, ControlShift, None, Shift,
    };
    [
        &[None],
        &[None, Shift],
        &[None, Alt],
        &[None, Shift, Alt, AltShift],
        &[None, Control],
        &[None, Shift, Control, ControlShift],
        &[None, Alt, Control, ControlAlt],
        &[
            None,
            Shift,
            Alt,
            AltShift,
            Control,
            ControlShift,
            ControlAlt,
            ControlAltShift,
        ],
    ]
};

/// Which qualifier keys are held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Qualifier {
    /// None.
    None,
    /// Shift.
    Shift,
    /// Alt.
    Alt,
    /// Alt and Shift.
    AltShift,
    /// Control.
    Control,
    /// Control and Shift.
    ControlShift,
    /// Control and Alt.
    ControlAlt,
    /// Control, Alt and Shift.
    ControlAltShift,
}

/// What one key does with one combination of qualifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// One character.
    Char(u8),
    /// Nothing: the keymap maps it to no character.
    Nothing,
    /// A string of characters.
    String(Vec<u8>),
    /// A dead key: it types nothing itself, and changes the next key. The
    /// index, from 1, says which accent it is.
    Dead(u8),
    /// A key a dead key changes: this character on its own, and after each
    /// dead key, by index from 1, the character in the list.
    Accentable(u8, Vec<u8>),
}

/// One key of a keymap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// Its raw key code.
    pub code: u8,
    /// What it types, by qualifier; empty for a key that does nothing.
    pub outputs: Vec<(Qualifier, Output)>,
    /// Whether Caps Lock shifts it.
    pub capsable: bool,
    /// Whether it repeats when held.
    pub repeatable: bool,
}

/// A decoded keymap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    /// Its name, from the node: `gb`, `d`, `usa0`.
    pub name: String,
    /// Every key with a mapping, low then high.
    pub keys: Vec<Key>,
    /// Descriptors pointing outside the hunk, which were left unread.
    pub out_of_range: usize,
}

/// Why bytes are not a keymap ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAKeymap(pub String);

impl core::fmt::Display for NotAKeymap {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotAKeymap {}

fn fail(why: impl Into<String>) -> NotAKeymap {
    NotAKeymap(why.into())
}

/// The keymap's hunk, as it would sit in memory: the file's bytes, then zero
/// to the hunk's size. `None` unless the name and all eight tables are
/// relocated into this hunk at the offsets a `KeyMapNode` puts them.
fn keymap_hunk(bytes: &[u8]) -> Option<Vec<u8>> {
    let exe = hunk::parse(bytes)?;
    let first = exe.hunks.first()?;
    let size = usize::try_from(first.size).ok()?;
    if size > MAX_HUNK || size < KEYMAP_AT.saturating_add(32) {
        return None;
    }
    let relocated = |at: usize| {
        first
            .relocs
            .iter()
            .any(|r| r.offset as usize == at && r.target == 0)
    };
    let fields = core::iter::once(NAME_AT)
        .chain((0..8usize).map(|i| KEYMAP_AT.saturating_add(i.saturating_mul(4))));
    if !fields.into_iter().all(relocated) {
        return None;
    }
    let file = bytes.get(first.file_offset..first.file_offset.checked_add(first.file_len)?)?;
    let mut data = file.get(..file.len().min(size))?.to_vec();
    data.resize(size, 0);
    Some(data)
}

/// Whether `bytes` are a keymap.
#[must_use]
pub fn is_keymap(bytes: &[u8]) -> bool {
    hunk::is_executable(bytes) && keymap_hunk(bytes).is_some()
}

/// Read a keymap.
///
/// # Errors
/// [`NotAKeymap`] when it is not one, or a table lies outside the hunk.
pub fn parse(bytes: &[u8]) -> Result<Keymap, NotAKeymap> {
    let data = keymap_hunk(bytes).ok_or_else(|| fail("not a keymap: no KeyMapNode"))?;
    let long = |at: usize| u32_at(&data, at).map_or(usize::MAX, |v| v as usize);
    let table = |field: usize, len: usize, what: &str| -> Result<&[u8], NotAKeymap> {
        let at = long(KEYMAP_AT.saturating_add(field.saturating_mul(4)));
        data.get(at..at.saturating_add(len))
            .ok_or_else(|| fail(format!("its {what} table lies outside the file")))
    };
    let name_at = long(NAME_AT);
    let name: String = data
        .get(name_at..)
        .unwrap_or(&[])
        .iter()
        .take(64)
        .take_while(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect();

    let halves = [
        (0u8, LO_KEYS, [0usize, 1, 2, 3]),
        (0x40u8, HI_KEYS, [4usize, 5, 6, 7]),
    ];
    let mut keys = Vec::new();
    let mut out_of_range = 0usize;
    for (base, count, [types_f, map_f, caps_f, rep_f]) in halves {
        let types = table(types_f, count, "key type")?;
        let map = table(map_f, count.saturating_mul(4), "key mapping")?;
        let caps = table(caps_f, count.div_ceil(8), "capsable")?;
        let rep = table(rep_f, count.div_ceil(8), "repeatable")?;
        for i in 0..count {
            let kind = u8_at(types, i).unwrap_or(KCF_NOP);
            let mapping = u32_at(map, i.saturating_mul(4)).unwrap_or(0);
            let bit = |bits: &[u8]| u8_at(bits, i / 8).is_ok_and(|b| b & (1 << (i % 8)) != 0);
            let outputs = outputs(&data, kind, mapping, &mut out_of_range);
            if outputs.is_empty() {
                continue;
            }
            keys.push(Key {
                code: base.saturating_add(u8::try_from(i).unwrap_or(0)),
                outputs,
                capsable: bit(caps),
                repeatable: bit(rep),
            });
        }
    }
    // An accentable key's table runs one entry per dead key the keymap has;
    // what follows it is the next table, not more accents.
    let deads = keys
        .iter()
        .flat_map(|k| &k.outputs)
        .filter_map(|(_, o)| match o {
            Output::Dead(i) => Some(usize::from(*i)),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    for key in &mut keys {
        for (_, output) in &mut key.outputs {
            if let Output::Accentable(_, after) = output {
                after.truncate(deads);
            }
        }
    }
    Ok(Keymap {
        name,
        keys,
        out_of_range,
    })
}

/// What a key of type `kind` with mapping `mapping` types, by qualifier.
fn outputs(
    data: &[u8],
    kind: u8,
    mapping: u32,
    out_of_range: &mut usize,
) -> Vec<(Qualifier, Output)> {
    if kind & KCF_NOP != 0 {
        return Vec::new();
    }
    let qualifiers = QUALIFIERS
        .get(usize::from(kind & (KCF_SHIFT | KCF_ALT | KCF_CONTROL)))
        .copied()
        .unwrap_or(&[]);
    if kind & (KCF_STRING | KCF_DEAD) == 0 {
        // Four characters at most, lowest byte first; zero types nothing.
        return qualifiers
            .iter()
            .take(4)
            .enumerate()
            .map(|(k, &q)| {
                let c = u8::try_from(
                    mapping
                        .checked_shr(u32::try_from(k).unwrap_or(4).saturating_mul(8))
                        .unwrap_or(0)
                        & 0xFF,
                )
                .unwrap_or(0);
                (
                    q,
                    if c == 0 {
                        Output::Nothing
                    } else {
                        Output::Char(c)
                    },
                )
            })
            .collect();
    }
    let at = mapping as usize;
    let Some(descriptors) = data.get(at..at.saturating_add(qualifiers.len().saturating_mul(2)))
    else {
        *out_of_range = out_of_range.saturating_add(1);
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, &q) in qualifiers.iter().enumerate() {
        let first = u8_at(descriptors, k.saturating_mul(2)).unwrap_or(0);
        let second = u8_at(descriptors, k.saturating_mul(2).saturating_add(1)).unwrap_or(0);
        let output = if kind & KCF_STRING != 0 {
            // Length, then offset from the descriptors.
            let from = at.saturating_add(usize::from(second));
            Output::String(
                data.get(from..from.saturating_add(usize::from(first)))
                    .unwrap_or(&[])
                    .to_vec(),
            )
        } else {
            match first {
                0 => Output::Char(second),
                DPF_DEAD => Output::Dead(second & 0x0F),
                DPF_MOD => {
                    // The table: the key on its own, then after each dead
                    // key. How many dead keys there are is the keymap's, so
                    // the table is read to fifteen and trimmed by the caller.
                    let from = at.saturating_add(usize::from(second));
                    // Near the hunk's end there may be fewer than sixteen.
                    let rest = data.get(from..).unwrap_or(&[]);
                    let table = rest.get(..rest.len().min(16)).unwrap_or(&[]);
                    let own = table.first().copied().unwrap_or(0);
                    Output::Accentable(own, table.get(1..).unwrap_or(&[]).to_vec())
                }
                _ => Output::Nothing,
            }
        };
        out.push((q, output));
    }
    out
}

/// Where a key sits, named for its place on a US keyboard, as AROS's
/// `rawkeycodes.txt` draws the Amiga's: the key a keymap gives `ü` is still
/// the key to the right of `P`.
#[must_use]
pub fn key_name(code: u8) -> String {
    const NUMBER: &[u8] = b"`1234567890-=\\";
    const TOP: &[u8] = b"QWERTYUIOP[]";
    const HOME: &[u8] = b"ASDFGHJKL;'";
    const BOTTOM: &[u8] = b"ZXCVBNM,./";
    let at = |row: &[u8], i: u8| row.get(usize::from(i)).map(|&b| char::from(b).to_string());
    let named = match code {
        0x00..=0x0D => at(NUMBER, code),
        0x10..=0x1B => at(TOP, code.saturating_sub(0x10)),
        0x20..=0x2A => at(HOME, code.saturating_sub(0x20)),
        0x2B => Some("the key beside Return".to_owned()),
        0x30 => Some("the key beside left Shift".to_owned()),
        0x31..=0x3A => at(BOTTOM, code.saturating_sub(0x31)),
        0x0F => Some("keypad 0".to_owned()),
        0x1D..=0x1F => Some(format!("keypad {}", code.saturating_sub(0x1C))),
        0x2D..=0x2F => Some(format!("keypad {}", code.saturating_sub(0x29))),
        0x3D..=0x3F => Some(format!("keypad {}", code.saturating_sub(0x36))),
        0x3C => Some("keypad .".to_owned()),
        0x40 => Some("Space".to_owned()),
        0x41 => Some("Backspace".to_owned()),
        0x42 => Some("Tab".to_owned()),
        0x43 => Some("keypad Enter".to_owned()),
        0x44 => Some("Return".to_owned()),
        0x45 => Some("Esc".to_owned()),
        0x46 => Some("Del".to_owned()),
        0x4A => Some("keypad -".to_owned()),
        0x4C => Some("cursor up".to_owned()),
        0x4D => Some("cursor down".to_owned()),
        0x4E => Some("cursor right".to_owned()),
        0x4F => Some("cursor left".to_owned()),
        0x50..=0x59 => Some(format!("F{}", code.saturating_sub(0x4F))),
        0x5A => Some("keypad (".to_owned()),
        0x5B => Some("keypad )".to_owned()),
        0x5C => Some("keypad /".to_owned()),
        0x5D => Some("keypad *".to_owned()),
        0x5E => Some("keypad +".to_owned()),
        0x5F => Some("Help".to_owned()),
        _ => None,
    };
    named.unwrap_or_else(|| format!("key ${code:02X}"))
}

/// One character as a person reads it: Latin-1 printed, control codes named.
fn shown(c: u8) -> String {
    match c {
        0x08 => "Backspace".to_owned(),
        0x09 => "Tab".to_owned(),
        0x0A => "Line feed".to_owned(),
        0x0D => "Return".to_owned(),
        0x1B => "Esc".to_owned(),
        0x20 => "Space".to_owned(),
        0x7F => "Del".to_owned(),
        0x9B => "CSI".to_owned(),
        0xA0 => "NBSP".to_owned(),
        0xAD => "SHY".to_owned(),
        0x00..=0x1F => format!("^{}", char::from(c.saturating_add(0x40))),
        0x80..=0x9F => format!("${c:02X}"),
        _ => char::from(c).to_string(),
    }
}

/// A string a key types: its printable runs as they are, everything else
/// named, a space between each.
fn shown_string(s: &[u8]) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut run = String::new();
    for &c in s {
        if c.is_ascii_graphic() {
            run.push(char::from(c));
        } else {
            if !run.is_empty() {
                words.push(core::mem::take(&mut run));
            }
            words.push(shown(c));
        }
    }
    if !run.is_empty() {
        words.push(run);
    }
    words.join(" ")
}

impl Qualifier {
    /// As it is written before a key: "Shift+", or nothing.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Shift => "Shift+",
            Self::Alt => "Alt+",
            Self::AltShift => "Shift+Alt+",
            Self::Control => "Ctrl+",
            Self::ControlShift => "Ctrl+Shift+",
            Self::ControlAlt => "Ctrl+Alt+",
            Self::ControlAltShift => "Ctrl+Shift+Alt+",
        }
    }
}

/// The four rows of a typewriter keyboard, by raw code.
const ROWS: [(&str, &[u8]); 4] = [
    (
        "Number row",
        &[
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
        ],
    ),
    (
        "Top row",
        &[
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B,
        ],
    ),
    (
        "Home row",
        &[
            0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B,
        ],
    ),
    (
        "Bottom row",
        &[
            0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A,
        ],
    ),
];

impl Keymap {
    fn key(&self, code: u8) -> Option<&Key> {
        self.keys.iter().find(|k| k.code == code)
    }

    /// What `key` types with `q` held, in a row: one character, `◌` for a
    /// dead key, `·` where it has no meaning of its own.
    fn cell(key: Option<&Key>, q: Qualifier) -> String {
        let Some((_, output)) = key.and_then(|k| k.outputs.iter().find(|(kq, _)| *kq == q)) else {
            return "·".to_owned();
        };
        match output {
            Output::Char(c) | Output::Accentable(c, _) => shown(*c),
            Output::Nothing => "·".to_owned(),
            Output::String(s) => shown_string(s),
            Output::Dead(_) => "◌".to_owned(),
        }
    }

    /// Each dead key, and what it does to the keys it changes: once, with
    /// every way of typing it.
    fn dead_key_fields(&self, dead: &[(u8, Qualifier, u8)], indices: &[u8]) -> Vec<Field> {
        let mut out = Vec::new();
        for index in indices {
            let typed: Vec<String> = dead
                .iter()
                .filter(|(i, _, _)| i == index)
                .map(|(_, q, code)| format!("{}{}", q.prefix(), key_name(*code)))
                .collect();
            let changed: Vec<String> = self
                .keys
                .iter()
                .flat_map(|k| &k.outputs)
                .filter_map(|(_, o)| match o {
                    Output::Accentable(own, after) => after
                        .get(usize::from(*index).saturating_sub(1))
                        .filter(|&&c| c != *own && c != 0)
                        .map(|&c| format!("{}→{}", shown(*own), shown(c))),
                    _ => None,
                })
                .take(12)
                .collect();
            out.push(Field::new(
                "Dead keys",
                typed.join(", "),
                if changed.is_empty() {
                    "changes no key".to_owned()
                } else {
                    changed.join(" ")
                },
            ));
        }
        out
    }

    /// The keymap in words.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        let mut out = vec![
            Field::new("Keymap", "Name", self.name.clone()),
            Field::new("Keymap", "Keys mapped", self.keys.len().to_string()),
        ];
        let dead: Vec<(u8, Qualifier, u8)> = self
            .keys
            .iter()
            .flat_map(|k| {
                k.outputs.iter().filter_map(move |(q, o)| match o {
                    Output::Dead(i) => Some((*i, *q, k.code)),
                    _ => None,
                })
            })
            .collect();
        let mut indices: Vec<u8> = dead.iter().map(|(i, _, _)| *i).collect();
        indices.sort_unstable();
        indices.dedup();
        if !indices.is_empty() {
            out.push(Field::new("Keymap", "Dead keys", indices.len().to_string()));
        }
        if self.out_of_range > 0 {
            out.push(Field::new(
                "Keymap",
                "Unread",
                format!(
                    "{} keys point outside the file, and are not shown",
                    self.out_of_range
                ),
            ));
        }

        // The typewriter rows, one line per qualifier, so a layout reads as
        // it would on the keys: `◌` is a dead key, `·` no meaning of its own.
        for (row, codes) in ROWS {
            for q in [
                Qualifier::None,
                Qualifier::Shift,
                Qualifier::Alt,
                Qualifier::AltShift,
            ] {
                let cells: Vec<String> =
                    codes.iter().map(|&c| Self::cell(self.key(c), q)).collect();
                if q != Qualifier::None && cells.iter().all(|c| c == "·") {
                    continue;
                }
                let label = match q {
                    Qualifier::None => row.to_owned(),
                    _ => format!("{row}, {}", q.prefix().trim_end_matches('+')),
                };
                out.push(Field::new("Typing", label, cells.join(" ")));
            }
        }

        out.extend(self.dead_key_fields(&dead, &indices));

        // Everything off the typewriter rows.
        let on_rows: Vec<u8> = ROWS.iter().flat_map(|(_, c)| c.iter().copied()).collect();
        for key in self.keys.iter().filter(|k| !on_rows.contains(&k.code)) {
            let said: Vec<String> = key
                .outputs
                .iter()
                .filter(|(_, o)| *o != Output::Nothing)
                .map(|(q, _)| format!("{}{}", q.prefix(), Self::cell(Some(key), *q)))
                .collect();
            if said.is_empty() {
                continue;
            }
            // A key typing the same with every qualifier is said once.
            let first = Self::cell(Some(key), Qualifier::None);
            let value = if key
                .outputs
                .iter()
                .all(|(q, _)| Self::cell(Some(key), *q) == first)
            {
                first
            } else {
                said.join(", ")
            };
            out.push(Field::new("Other keys", key_name(key.code), value));
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

    /// A keymap named "test" as a one-hunk executable: `Q` with all four
    /// characters, `1` shifted only, cursor up typing strings, `F` a dead
    /// key under Alt, `E` changed by it. Every other key does nothing.
    /// `relocate` drops the name's relocation when false; `wild` points the
    /// dead key's descriptors outside the hunk.
    fn keymap(relocate: bool, wild: bool) -> Vec<u8> {
        let mut h = vec![0u8; 46];
        let mut relocs: Vec<u32> = (0..8).map(|i| 14 + i * 4).collect();
        if relocate {
            relocs.push(10);
        }
        let set = |h: &mut Vec<u8>, at: usize, v: u32| {
            h[at..at + 4].copy_from_slice(&v.to_be_bytes());
        };
        let table = |h: &mut Vec<u8>, field: usize, len: usize, fill: u8| -> usize {
            let at = h.len();
            h.resize(at + len, fill);
            set(h, 14 + field * 4, at as u32);
            at
        };
        let lo_types = table(&mut h, 0, 64, KCF_NOP);
        let lo_map = table(&mut h, 1, 256, 0);
        table(&mut h, 2, 8, 0xFF);
        table(&mut h, 3, 8, 0xFF);
        let hi_types = table(&mut h, 4, 56, KCF_NOP);
        let hi_map = table(&mut h, 5, 224, 0);
        table(&mut h, 6, 7, 0);
        table(&mut h, 7, 7, 0);
        let name = h.len();
        h.extend(b"test\0");
        set(&mut h, 10, name as u32);

        h[lo_types + 0x10] = 7;
        set(
            &mut h,
            lo_map + 0x10 * 4,
            u32::from_le_bytes([b'q', b'Q', 0xE5, 0xC5]),
        );
        h[lo_types + 0x01] = KCF_SHIFT;
        set(&mut h, lo_map + 4, u32::from_le_bytes([b'1', b'!', 0, 0]));

        // Cursor up: a string, and another with Shift.
        let up = h.len();
        h.extend([2, 4, 2, 6, 0x9B, b'A', 0x9B, b'T']);
        h[hi_types + 0x0C] = KCF_STRING | KCF_SHIFT;
        set(&mut h, hi_map + 0x0C * 4, up as u32);
        relocs.push((hi_map + 0x0C * 4) as u32);

        // F: f, F, then dead key 1 under Alt and Shift+Alt.
        let f = h.len();
        h.extend([
            0, b'f', 0, b'F', DPF_DEAD, 1, DPF_DEAD, 1, 0, 6, 0, 6, 0, 0, 0, 0,
        ]);
        h[lo_types + 0x23] = KCF_DEAD | 7;
        set(
            &mut h,
            lo_map + 0x23 * 4,
            if wild { 0xFFFF } else { f as u32 },
        );
        relocs.push((lo_map + 0x23 * 4) as u32);

        // E: e alone, è after dead key 1; E shifted.
        let e = h.len();
        h.extend([DPF_MOD, 4, 0, b'E', b'e', 0xE8]);
        h[lo_types + 0x12] = KCF_DEAD | KCF_SHIFT;
        set(&mut h, lo_map + 0x12 * 4, e as u32);
        relocs.push((lo_map + 0x12 * 4) as u32);

        while h.len() % 4 != 0 {
            h.push(0);
        }
        let mut exe = Vec::new();
        let longs = (h.len() / 4) as u32;
        for v in [0x3F3u32, 0, 1, 0, 0, longs, 0x3E9, longs] {
            exe.extend(v.to_be_bytes());
        }
        exe.extend(h);
        exe.extend(0x3ECu32.to_be_bytes());
        exe.extend((relocs.len() as u32).to_be_bytes());
        exe.extend(0u32.to_be_bytes());
        for r in relocs {
            exe.extend(r.to_be_bytes());
        }
        exe.extend(0u32.to_be_bytes());
        exe.extend(0x3F2u32.to_be_bytes());
        exe
    }

    fn outputs_of(k: &Keymap, code: u8) -> Vec<(Qualifier, Output)> {
        k.keys
            .iter()
            .find(|key| key.code == code)
            .unwrap()
            .outputs
            .clone()
    }

    #[test]
    fn a_keymap_reads_every_kind_of_key() {
        let bytes = keymap(true, false);
        assert!(is_keymap(&bytes));
        let k = parse(&bytes).unwrap();
        assert_eq!(k.name, "test");
        assert_eq!(k.keys.len(), 5);
        assert_eq!(
            outputs_of(&k, 0x10),
            vec![
                (Qualifier::None, Output::Char(b'q')),
                (Qualifier::Shift, Output::Char(b'Q')),
                (Qualifier::Alt, Output::Char(0xE5)),
                (Qualifier::AltShift, Output::Char(0xC5)),
            ]
        );
        assert_eq!(
            outputs_of(&k, 0x01),
            vec![
                (Qualifier::None, Output::Char(b'1')),
                (Qualifier::Shift, Output::Char(b'!')),
            ]
        );
        assert_eq!(
            outputs_of(&k, 0x4C),
            vec![
                (Qualifier::None, Output::String(vec![0x9B, b'A'])),
                (Qualifier::Shift, Output::String(vec![0x9B, b'T'])),
            ]
        );
        let f = outputs_of(&k, 0x23);
        assert_eq!(f[2], (Qualifier::Alt, Output::Dead(1)));
        assert_eq!(f[4], (Qualifier::Control, Output::Char(6)));
        // The accent table is trimmed to the one dead key the keymap has.
        assert_eq!(
            outputs_of(&k, 0x12)[0],
            (Qualifier::None, Output::Accentable(b'e', vec![0xE8]))
        );
    }

    #[test]
    fn a_keymap_is_said_as_rows_dead_keys_and_the_rest() {
        let k = parse(&keymap(true, false)).unwrap();
        let fields = k.fields();
        let find = |label: &str| {
            fields
                .iter()
                .find(|f| f.label == label)
                .map(|f| f.value.clone())
                .unwrap_or_default()
        };
        assert_eq!(find("Name"), "test");
        assert_eq!(find("Dead keys"), "1");
        assert_eq!(find("Top row"), "q · e · · · · · · · · ·");
        assert_eq!(find("Top row, Shift"), "Q · E · · · · · · · · ·");
        assert_eq!(find("Home row, Alt"), "· · · ◌ · · · · · · · ·");
        assert_eq!(find("Alt+F, Shift+Alt+F"), "e→è");
        assert_eq!(find("cursor up"), "CSI A, Shift+CSI T");
    }

    #[test]
    fn a_keymap_without_its_relocations_is_not_one() {
        assert!(!is_keymap(&keymap(false, false)));
        assert!(parse(&keymap(false, false)).is_err());
    }

    #[test]
    fn descriptors_outside_the_hunk_are_counted_not_read() {
        let k = parse(&keymap(true, true)).unwrap();
        assert_eq!(k.out_of_range, 1);
        assert!(k.keys.iter().all(|key| key.code != 0x23));
    }
}
