//! Datatype descriptors (F-033): how `DEVS:DataTypes` tells Workbench 3
//! what kind of file a file is.
//!
//! A `FORM DTYP`: `NAME`, the type's name; `FVER`, a `$VER:` string; and
//! `DTHD`, a `DataTypeHeader` whose name, base name, pattern and mask are
//! offsets into the chunk. The mask is the bytes a file of this type starts
//! with, a word each, a negative word matching any byte. The layout and
//! constants are AROS's `datatypes/datatypes.h`, the mask's reading its
//! `ObtainDataTypeA()`, read locally and not committed.

use ade_endian::{i16_at, u16_at, u32_at};

use crate::decoded::Field;

/// A decoded descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    /// The `NAME` chunk.
    pub name: String,
    /// The `$VER` string, without `$VER: `.
    pub version: String,
    /// `dth_BaseName`: the class library it opens, without `.datatype`.
    pub base: String,
    /// `dth_Pattern`: the file names it matches.
    pub pattern: String,
    /// `dth_Mask`: each byte the file starts with, `None` for any byte.
    pub mask: Vec<Option<u16>>,
    /// `dth_GroupID` and `dth_ID`, as four characters each.
    pub group: [u8; 4],
    /// `dth_ID`.
    pub id: [u8; 4],
    /// `dth_Flags`.
    pub flags: u16,
    /// `dth_Priority`.
    pub priority: u16,
}

/// Why bytes are not a descriptor ADE can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotADescriptor(pub String);

impl core::fmt::Display for NotADescriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for NotADescriptor {}

/// Whether `bytes` are a datatype descriptor.
#[must_use]
pub fn is_descriptor(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"FORM") && bytes.get(8..12) == Some(b"DTYP")
}

fn text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(256)
        .take_while(|&&b| b != 0)
        .map(|&b| char::from(b))
        .collect()
}

/// The text at the offset the long at `at` holds, within `dthd`; empty for
/// an offset of zero or one outside.
fn string_at(dthd: &[u8], at: usize) -> String {
    match u32_at(dthd, at).map(|v| v as usize) {
        Ok(0) | Err(_) => String::new(),
        Ok(off) => text(dthd.get(off..).unwrap_or(&[])),
    }
}

/// Read a descriptor.
///
/// # Errors
/// [`NotADescriptor`] when it is not one, or its `DTHD` is cut short.
pub fn parse(bytes: &[u8]) -> Result<Descriptor, NotADescriptor> {
    if !is_descriptor(bytes) {
        return Err(NotADescriptor("not a FORM DTYP".to_owned()));
    }
    let declared = u32_at(bytes, 4).map_or(0, |v| v as usize);
    let end = declared.saturating_add(8).min(bytes.len());
    let (mut name, mut version, mut header) = (String::new(), String::new(), None);
    let mut at = 12usize;
    while at.saturating_add(8) <= end {
        let id = bytes.get(at..at.saturating_add(4)).unwrap_or(&[]);
        let len = u32_at(bytes, at.saturating_add(4)).map_or(0, |v| v as usize);
        let start = at.saturating_add(8);
        let data = bytes
            .get(start..start.saturating_add(len).min(bytes.len()))
            .unwrap_or(&[]);
        match id {
            b"NAME" => name = text(data),
            b"FVER" => {
                let v = text(data);
                v.strip_prefix("$VER: ")
                    .unwrap_or(&v)
                    .trim()
                    .clone_into(&mut version);
            }
            b"DTHD" => header = Some(data),
            _ => {}
        }
        at = start.saturating_add(len).saturating_add(len & 1);
    }
    let dthd = header.ok_or_else(|| NotADescriptor("it has no DTHD".to_owned()))?;
    if dthd.len() < 32 {
        return Err(NotADescriptor("its DTHD is cut short".to_owned()));
    }
    let four = |at: usize| {
        let mut id = [0u8; 4];
        id.copy_from_slice(dthd.get(at..at.saturating_add(4)).unwrap_or(&[0; 4]));
        id
    };
    let mask_len = usize::try_from(i16_at(dthd, 24).unwrap_or(0).max(0)).unwrap_or(0);
    let mask_at = u32_at(dthd, 12).map_or(0, |v| v as usize);
    let mask = (0..mask_len)
        .map_while(|i| i16_at(dthd, mask_at.saturating_add(i.saturating_mul(2))).ok())
        .map(|w| u16::try_from(w).ok())
        .collect();
    Ok(Descriptor {
        name,
        version,
        base: string_at(dthd, 4),
        pattern: string_at(dthd, 8),
        mask,
        group: four(16),
        id: four(20),
        flags: u16_at(dthd, 28).unwrap_or(0),
        priority: u16_at(dthd, 30).unwrap_or(0),
    })
}

impl Descriptor {
    /// What the descriptor says, in fields for the Decoded view.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        const D: &str = "Datatype";
        let group = match &self.group {
            b"syst" => "system: programs, directories, libraries, fonts".to_owned(),
            b"text" => "text".to_owned(),
            b"docu" => "document: text with pictures".to_owned(),
            b"soun" => "sound".to_owned(),
            b"inst" => "musical instrument".to_owned(),
            b"musi" => "music score".to_owned(),
            b"pict" => "picture".to_owned(),
            b"anim" => "animation".to_owned(),
            b"movi" => "film: animation with sound".to_owned(),
            other => other.iter().map(|&b| char::from(b)).collect(),
        };
        let kind = match self.flags & 0x0F {
            0 => "binary",
            1 => "text",
            2 => "IFF",
            _ => "recognised by code of its own",
        };
        let mask: String = self
            .mask
            .iter()
            .map(|m| match m {
                None => "?".to_owned(),
                Some(c) if (0x20..0x7F).contains(c) => {
                    char::from(u8::try_from(*c).unwrap_or(b'?')).to_string()
                }
                Some(c) => format!("<${c:02X}>"),
            })
            .collect();
        let mut out = vec![
            Field::new(D, "Name", self.name.clone()),
            Field::new(D, "Group", group),
            Field::new(
                D,
                "ID",
                self.id
                    .iter()
                    .filter(|&&b| b != 0)
                    .map(|&b| char::from(b))
                    .collect::<String>(),
            ),
            Field::new(
                D,
                "Files",
                format!(
                    "{kind}, named like {}",
                    if self.pattern.is_empty() {
                        "anything"
                    } else {
                        &self.pattern
                    }
                ),
            ),
        ];
        if !self.mask.is_empty() {
            out.push(Field::new(
                D,
                "Starting with",
                format!(
                    "{mask}{}",
                    if self.flags & 0x10 != 0 {
                        ""
                    } else {
                        ", in either case"
                    }
                ),
            ));
        }
        if !self.base.is_empty() {
            out.push(Field::new(D, "Read by", format!("{}.datatype", self.base)));
        }
        if !self.version.is_empty() {
            out.push(Field::new(D, "Version", self.version.clone()));
        }
        out.push(Field::new(D, "Priority", self.priority.to_string()));
        out
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::cast_possible_truncation,
    reason = "tests over bytes they build, by hand rather than with the helpers that read \
              them, so the two cannot agree about a field both have wrong (C-001's exception)"
)]
mod tests {
    use super::*;

    /// AmigaGuide's descriptor as Workbench 3.0 ships it, byte for byte in
    /// its layout: a mask of `@database`, any case.
    fn amigaguide() -> Vec<u8> {
        let mut dthd = Vec::new();
        for v in [0x32u32, 0x3D, 0x48, 0x20] {
            dthd.extend(v.to_be_bytes());
        }
        dthd.extend(b"docuamig");
        dthd.extend(9u16.to_be_bytes());
        dthd.extend(0u16.to_be_bytes());
        dthd.extend(1u16.to_be_bytes());
        dthd.extend(0u16.to_be_bytes());
        for c in b"@database" {
            dthd.extend(u16::from(*c).to_be_bytes());
        }
        dthd.extend(b"AmigaGuide\0amigaguide\0#?\0");
        let mut inner = b"DTYP".to_vec();
        inner.extend(b"NAME");
        inner.extend(10u32.to_be_bytes());
        inner.extend(b"AmigaGuide");
        inner.extend(b"DTHD");
        inner.extend((dthd.len() as u32).to_be_bytes());
        inner.extend(&dthd);
        if dthd.len() % 2 == 1 {
            inner.push(0);
        }
        let mut out = b"FORM".to_vec();
        out.extend((inner.len() as u32).to_be_bytes());
        out.extend(inner);
        out
    }

    #[test]
    fn a_descriptor_says_its_group_mask_and_reader() {
        let d = parse(&amigaguide()).unwrap();
        assert_eq!(
            (d.name.as_str(), d.base.as_str(), d.pattern.as_str()),
            ("AmigaGuide", "amigaguide", "#?")
        );
        assert_eq!((&d.group, &d.id), (b"docu", b"amig"));
        assert_eq!(d.mask.len(), 9);
        let f = d.fields();
        let value = |l: &str| f.iter().find(|x| x.label == l).unwrap().value.clone();
        assert_eq!(value("Starting with"), "@database, in either case");
        assert_eq!(value("Read by"), "amigaguide.datatype");
        assert_eq!(value("Files"), "text, named like #?");
    }

    #[test]
    fn a_negative_mask_word_matches_any_byte() {
        let mut bytes = amigaguide();
        let at = bytes.windows(2).position(|w| w == [0, b'@']).unwrap();
        bytes[at..at + 2].copy_from_slice(&0xFFFFu16.to_be_bytes());
        let d = parse(&bytes).unwrap();
        assert_eq!(d.mask[0], None);
        assert!(d.fields().iter().any(|x| x.value.starts_with("?database")));
    }
}
