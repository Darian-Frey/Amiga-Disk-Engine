//! Making a blank disk (F-019, F-025).
//!
//! The *formatting* is [`ade_filesystem::format::blank`]. What lives here is
//! everything a front end would otherwise have to know for itself: which
//! filesystems ADE will write, what to call them, and what shape each disk is.
//!
//! It is here rather than in the CLI because the GUI needs the same answers,
//! and two front ends deciding separately which dostypes exist is two chances
//! to disagree with the engine — the same reasoning that moved MFM encoding
//! out of the CLI (IMP-007).

use std::path::Path;

use ade_block::Geometry;
use ade_filesystem::dostype::Dostype;
use ade_filesystem::format::{self, FormatError, Stamp};

/// A filesystem `ade create` will write.
#[derive(Debug, Clone, Copy)]
pub struct DiskType {
    /// What a caller names it: `ffs-intl`.
    pub name: &'static str,
    /// What a person reads: `FFS, international (DOS\3)`.
    pub label: &'static str,
    /// The dostype's flags byte.
    pub flags: u8,
}

/// The six AmigaDOS types ADE writes.
///
/// `DOS\6` and `DOS\7` are absent on purpose. LNFS is deferred by **D-013** on
/// verifiability rather than effort: no corpus image carries one and ADFlib
/// misreads them, so writing one means producing a format checkable only
/// against itself — which is what D-002 gave up ADFlib's accumulated knowledge
/// to avoid. Beyond AmigaDOS there are some forty other 4-byte tags in SPEC's
/// registry; none of them is ADE's to write.
pub const TYPES: [DiskType; 6] = [
    DiskType {
        name: "ofs",
        label: "OFS (DOS\\0)",
        flags: 0,
    },
    DiskType {
        name: "ffs",
        label: "FFS (DOS\\1)",
        flags: 1,
    },
    DiskType {
        name: "ofs-intl",
        label: "OFS, international (DOS\\2)",
        flags: 2,
    },
    DiskType {
        name: "ffs-intl",
        label: "FFS, international (DOS\\3)",
        flags: 3,
    },
    DiskType {
        name: "ofs-dc",
        label: "OFS, directory cache (DOS\\4)",
        flags: 4,
    },
    DiskType {
        name: "ffs-dc",
        label: "FFS, directory cache (DOS\\5)",
        flags: 5,
    },
];

/// The default, and why it is not plain `ffs`.
///
/// Everything since Workbench 2.0 writes the international variant, and a name
/// with an accent sorts wrongly without it (C-006).
pub const DEFAULT_TYPE: &str = "ffs-intl";

/// The dostype a type name means, or `None` if ADE will not write it.
#[must_use]
pub fn dostype(name: &str) -> Option<Dostype> {
    let wanted = name.to_ascii_lowercase();
    let found = TYPES.iter().find(|t| t.name == wanted)?;
    Dostype::from_raw(0x444F_5300 | u32::from(found.flags)).ok()
}

/// The shape of a disk to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// 3.5" double density: 880 KB, the norm.
    Dd,
    /// 3.5" high density: 1.76 MB.
    Hd,
    /// 5.25" double density: the A1020's 440 KB.
    ///
    /// No corpus image is one, and ADFlib refuses the size before it reaches
    /// any filesystem, so this rests on the rootblock formula being verified
    /// at other block counts rather than on an oracle.
    Dd525,
    /// An unpartitioned hard disk of this many megabytes.
    Hard(u32),
}

impl Shape {
    /// The geometry to format.
    ///
    /// # Errors
    /// A size that is not a disk, or one too large to express.
    pub fn geometry(self) -> Result<Geometry, String> {
        match self {
            // No option for the 81-, 82- and 83-cylinder images that occur in
            // the wild: those are not larger volumes but ordinary 80-cylinder
            // filesystems in files holding extra tracks, with the rootblock
            // still at 880 (BUG-009). Writing one would produce a file
            // matching no real disk.
            Self::Dd => Geometry::new(80, 2, 11, 512, 2),
            Self::Hd => Geometry::new(80, 2, 22, 512, 2),
            Self::Dd525 => Geometry::new(40, 2, 11, 512, 2),
            Self::Hard(megabytes) => {
                if megabytes == 0 {
                    return Err("0 MB is not a disk".to_owned());
                }
                // A hard disk is not a bigger floppy: it has no cylinders,
                // heads or sectors that anything reads, only a block count
                // (SPEC §Hardfiles). One head and 32 sectors is UAE's
                // convention and makes a megabyte exactly 64 cylinders.
                let cylinders = megabytes
                    .checked_mul(64)
                    .ok_or_else(|| format!("{megabytes} MB: too large"))?;
                Geometry::new(cylinders, 1, 32, 512, 2)
            }
        }
        .map_err(|e| format!("{e}"))
    }
}

/// Why a disk could not be made.
#[derive(Debug)]
pub enum CreateError {
    /// ADE will not write that filesystem.
    UnknownType(String),
    /// The shape is not a disk ADE can make.
    Shape(String),
    /// The formatter refused.
    Format(FormatError),
    /// Something already exists at that path.
    Exists,
    /// The file could not be written.
    Io(std::io::Error),
}

impl core::fmt::Display for CreateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownType(name) => {
                let names: Vec<&str> = TYPES.iter().map(|t| t.name).collect();
                write!(f, "{name}: expected one of {}", names.join(", "))
            }
            Self::Shape(why) => write!(f, "{why}"),
            Self::Format(e) => write!(f, "{e}"),
            Self::Exists => f.write_str("already exists, refusing to overwrite"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for CreateError {}

/// The current time, as AmigaDOS counts it.
///
/// "Created" means when the disk was made, and a tool that lies about that is
/// worse than one that omits it. Day zero is *illegal*: SPEC records that
/// Amiga software treats it as unset, and ADE's own health check agrees — the
/// first disk `ade create` ever produced reported three `datestamp-day-zero`
/// findings against itself.
///
/// Here rather than in a front end because every front end needs it and none
/// of them should be deciding what an Amiga epoch is. [`blank`] still takes an
/// explicit stamp, so tests stay deterministic.
///
/// Days are counted from 1978-01-01, which is 2,922 days after the Unix epoch
/// — eight years including the leap days of 1972 and 1976. A clock before 1978
/// gives day 1 rather than a negative: the field is unsigned, and a disk
/// stamped "the day after the Amiga's epoch" is odd where an underflowed one
/// is corrupt.
#[must_use]
pub fn now() -> Stamp {
    const AMIGA_EPOCH_IN_UNIX_DAYS: u64 = 2922;
    let Ok(since) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return Stamp {
            days: 1,
            mins: 0,
            ticks: 0,
        };
    };
    let secs = since.as_secs();
    let days = secs
        .checked_div(86_400)
        .unwrap_or(0)
        .saturating_sub(AMIGA_EPOCH_IN_UNIX_DAYS)
        .max(1);
    let in_day = secs.checked_rem(86_400).unwrap_or(0);
    Stamp {
        days: u32::try_from(days).unwrap_or(1),
        mins: u32::try_from(in_day.checked_div(60).unwrap_or(0)).unwrap_or(0),
        ticks: u32::try_from(in_day.checked_rem(60).unwrap_or(0).saturating_mul(50)).unwrap_or(0),
    }
}

/// Write a blank disk to `path`.
///
/// **Never overwrites.** A blank disk is the safest write there is precisely
/// because it makes a new file; writing over one somebody already has would
/// give that away for nothing.
///
/// # Errors
/// [`CreateError`], which distinguishes a filesystem ADE will not write from a
/// shape it cannot make from a file it could not put there.
pub fn blank(
    path: &Path,
    type_name: &str,
    volume_name: &str,
    shape: Shape,
    created: Stamp,
) -> Result<(), CreateError> {
    let dostype =
        dostype(type_name).ok_or_else(|| CreateError::UnknownType(type_name.to_owned()))?;
    let geometry = shape.geometry().map_err(CreateError::Shape)?;
    let bytes =
        format::blank(geometry, dostype, volume_name, created).map_err(CreateError::Format)?;

    if path.exists() {
        return Err(CreateError::Exists);
    }
    std::fs::write(path, &bytes).map_err(CreateError::Io)
}

/// What `ade create --format=json` reports (F-015, BUG-012).
///
/// The disk that was made, by the names a script would pass back in: the
/// type by its code (`ffs-intl`), not its label.
#[must_use]
pub fn created_json(path: &Path, type_name: &str, volume: &str, bytes: u64) -> crate::json::Value {
    use crate::json::Value;
    Value::Obj(vec![
        ("written", Value::str(path.to_string_lossy())),
        ("bytes", Value::Num(bytes)),
        ("type", Value::str(type_name.to_ascii_lowercase())),
        ("volume", Value::str(volume)),
    ])
}

/// How large a partition is to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionSize {
    /// This many megabytes.
    Megabytes(u32),
    /// Whatever the partitions before it leave: only the last may.
    Rest,
}

/// One partition of a disk to make: `NAME:SIZE[:TYPE[:VOLUME]]` from the
/// command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionSpec {
    /// Drive name: `DH0`.
    pub name: String,
    /// Its size.
    pub size: PartitionSize,
    /// Its filesystem, by the names [`TYPES`] gives; the disk's default when
    /// absent.
    pub type_name: Option<String>,
    /// Its volume name; the drive name when absent.
    pub volume: Option<String>,
}

impl PartitionSpec {
    /// Read `NAME:SIZE[:TYPE[:VOLUME]]`: `DH0:60`, `DH1:rest:ofs:Data`.
    /// SIZE is megabytes or `rest`.
    ///
    /// # Errors
    /// What is wrong with it, for a person to read.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut fields = text.splitn(4, ':');
        let name = fields.next().unwrap_or_default();
        let Some(size) = fields.next() else {
            return Err(format!(
                "{text:?}: a partition is NAME:SIZE[:TYPE[:VOLUME]], with SIZE in megabytes \
                 or rest"
            ));
        };
        let size = if size.eq_ignore_ascii_case("rest") {
            PartitionSize::Rest
        } else {
            match size.parse::<u32>() {
                Ok(0) | Err(_) => {
                    return Err(format!(
                        "{text:?}: the size is megabytes, at least 1, or rest"
                    ));
                }
                Ok(m) => PartitionSize::Megabytes(m),
            }
        };
        let nonempty = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_owned);
        Ok(Self {
            name: name.to_owned(),
            size,
            type_name: nonempty(fields.next()),
            volume: nonempty(fields.next()),
        })
    }
}

/// A partition as it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MadePartition {
    /// Drive name.
    pub name: String,
    /// Its filesystem's type name.
    pub type_name: String,
    /// Its volume name.
    pub volume: String,
    /// Its first block on the disk.
    pub first_block: u64,
    /// How many blocks it spans.
    pub blocks: u64,
    /// Whether it is marked bootable.
    pub bootable: bool,
}

/// Heads and sectors of a partitioned disk ADE makes: one head and 32
/// sectors, the shape of its unpartitioned hard disks, so a megabyte is
/// exactly 64 cylinders and a partition's size in megabytes is whole
/// cylinders.
const HEADS: u32 = 1;
const SECTORS: u32 = 32;
const CYLINDERS_PER_MB: u32 = 64;
/// Cylinders kept for the RDB, as HDToolBox keeps two: 64 blocks, room for
/// the `RDSK` block and 63 `PART` blocks.
const TABLE_CYLINDERS: u32 = 2;
/// The largest disk ADE partitions: 4 GB, past which an image needs the
/// 64-bit device commands C-002 has not settled.
const MAX_PARTITIONED_MB: u32 = 4096;

/// Write a partitioned hard disk of `megabytes` to `path`: an RDB with a
/// partition per spec, in order, each formatted with its own filesystem and
/// volume name (F-038, D-024).
///
/// No filesystem is written into the RDB (D-024 part 4): each partition
/// names its dostype and relies on the FFS in Kickstart's ROM, 2.0 and later,
/// which ADFlib does not accept and AmigaDOS does. The first partition is
/// bootable at priority 0, as HDToolBox makes it; the others are not. Space
/// the partitions do not use is left unpartitioned. **Never overwrites.**
///
/// # Errors
/// [`CreateError::Shape`] for a plan that is not a disk — names missing,
/// repeated or illegal, sizes that do not fit, `rest` anywhere but last —
/// [`CreateError::UnknownType`] for a filesystem ADE will not write,
/// [`CreateError::Format`] for a partition too small to hold a volume, and
/// [`CreateError::Exists`] or [`CreateError::Io`].
pub fn partitioned(
    path: &Path,
    megabytes: u32,
    specs: &[PartitionSpec],
    default_type: &str,
    created: Stamp,
) -> Result<Vec<MadePartition>, CreateError> {
    let shape = |why: String| CreateError::Shape(why);
    if megabytes == 0 || megabytes > MAX_PARTITIONED_MB {
        return Err(shape(format!(
            "a partitioned disk is 1 to {MAX_PARTITIONED_MB} MB, and {megabytes} MB is not"
        )));
    }
    let cylinders = megabytes.saturating_mul(CYLINDERS_PER_MB);
    let plan = plan_partitions(specs, cylinders, default_type)?;
    let table = ade_filesystem::rdb::NewTable {
        cylinders,
        heads: HEADS,
        sectors: SECTORS,
        lo_cylinder: TABLE_CYLINDERS,
        partitions: plan
            .iter()
            .map(|p| ade_filesystem::rdb::NewPartition {
                name: p.name.clone(),
                low_cylinder: p.low,
                high_cylinder: p.high,
                dostype: p.dostype.raw(),
                bootable: p.bootable,
                boot_priority: 0,
            })
            .collect(),
        drive: (b"ADE".to_vec(), b"HARDFILE".to_vec(), b"1.0".to_vec()),
    };
    let blocks = ade_filesystem::rdb::table_blocks(&table).map_err(shape)?;

    let cylinder_bytes = usize::try_from(HEADS.saturating_mul(SECTORS))
        .unwrap_or(0)
        .saturating_mul(512);
    let total = usize::try_from(cylinders)
        .unwrap_or(0)
        .saturating_mul(cylinder_bytes);
    let mut disk = vec![0u8; total];
    for (block, raw) in &blocks {
        let at = usize::try_from(*block).unwrap_or(0).saturating_mul(512);
        if let Some(there) = disk.get_mut(at..at.saturating_add(512)) {
            there.copy_from_slice(raw);
        }
    }
    let mut made = Vec::with_capacity(plan.len());
    for p in &plan {
        let count = p.high.saturating_sub(p.low).saturating_add(1);
        let geometry = Geometry::new(count, HEADS, SECTORS, 512, 2)
            .map_err(|e| shape(format!("{}: {e}", p.label)))?;
        let start = usize::try_from(p.low)
            .unwrap_or(0)
            .saturating_mul(cylinder_bytes);
        let end = start.saturating_add(usize::try_from(geometry.total_bytes()).unwrap_or(0));
        let Some(slice) = disk.get_mut(start..end) else {
            return Err(shape(format!("{}: past the end of the disk", p.label)));
        };
        format::blank_into(slice, geometry, p.dostype, &p.volume, created)
            .map_err(|e| CreateError::Shape(format!("{}: {e}", p.label)))?;
        made.push(MadePartition {
            name: p.label.clone(),
            type_name: p.type_name.clone(),
            volume: p.volume.iter().map(|&b| char::from(b)).collect(),
            first_block: geometry_start(p.low),
            blocks: geometry.total_blocks(),
            bootable: p.bootable,
        });
    }

    if path.exists() {
        return Err(CreateError::Exists);
    }
    std::fs::write(path, &disk).map_err(CreateError::Io)?;
    Ok(made)
}

fn geometry_start(cylinder: u32) -> u64 {
    u64::from(cylinder).saturating_mul(u64::from(HEADS.saturating_mul(SECTORS)))
}

/// A partition placed: its cylinders, and what it is to hold.
struct Planned {
    label: String,
    name: Vec<u8>,
    low: u32,
    high: u32,
    dostype: Dostype,
    type_name: String,
    volume: Vec<u8>,
    bootable: bool,
}

/// Place each spec in cylinders, in order, from the first after the table;
/// refuse anything that is not a disk.
fn plan_partitions(
    specs: &[PartitionSpec],
    cylinders: u32,
    default_type: &str,
) -> Result<Vec<Planned>, CreateError> {
    let shape = |why: String| CreateError::Shape(why);
    if specs.is_empty() {
        return Err(shape(
            "a partitioned disk needs at least one partition".to_owned(),
        ));
    }
    let most = TABLE_CYLINDERS
        .saturating_mul(HEADS.saturating_mul(SECTORS))
        .saturating_sub(1);
    if specs.len() > most as usize {
        return Err(shape(format!(
            "at most {most} partitions fit in the table, and {} were given",
            specs.len()
        )));
    }
    let mut next = TABLE_CYLINDERS;
    let mut out: Vec<Planned> = Vec::with_capacity(specs.len());
    for (i, spec) in specs.iter().enumerate() {
        let label = spec.name.clone();
        let name = drive_name(&spec.name)?;
        if out.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
            return Err(shape(format!("two partitions are named {label}")));
        }
        let type_name = spec
            .type_name
            .clone()
            .unwrap_or_else(|| default_type.to_owned());
        let dostype =
            dostype(&type_name).ok_or_else(|| CreateError::UnknownType(type_name.clone()))?;
        let left = cylinders.saturating_sub(next);
        let wanted = match spec.size {
            PartitionSize::Rest if i.saturating_add(1) != specs.len() => {
                return Err(shape(format!(
                    "{label}: only the last partition can take the rest of the disk"
                )));
            }
            PartitionSize::Rest => left,
            PartitionSize::Megabytes(m) => m.saturating_mul(CYLINDERS_PER_MB),
        };
        if wanted == 0 || wanted > left {
            return Err(shape(format!(
                "{label}: {} MB do not fit; {} MB are left after the partitions before it \
                 and the table",
                wanted / CYLINDERS_PER_MB,
                left / CYLINDERS_PER_MB
            )));
        }
        let volume_text = spec.volume.clone().unwrap_or_else(|| label.clone());
        let volume: Vec<u8> = volume_text
            .chars()
            .map(|ch| u8::try_from(u32::from(ch)))
            .collect::<Result<_, _>>()
            .map_err(|_| shape(format!("{volume_text:?}: an Amiga name is Latin-1")))?;
        out.push(Planned {
            label,
            name,
            low: next,
            high: next.saturating_add(wanted).saturating_sub(1),
            dostype,
            type_name: type_name.to_ascii_lowercase(),
            volume,
            bootable: i == 0,
        });
        next = next.saturating_add(wanted);
    }
    Ok(out)
}

/// A drive name as the table stores it: 1 to 31 printable Latin-1
/// characters, without the `:` and `/` that AmigaDOS reads as path syntax.
fn drive_name(text: &str) -> Result<Vec<u8>, CreateError> {
    let bytes: Vec<u8> = text
        .chars()
        .map(|ch| {
            u8::try_from(u32::from(ch))
                .ok()
                .filter(|b| *b > 0x20 && *b != 0x7f)
        })
        .collect::<Option<_>>()
        .ok_or_else(|| {
            CreateError::Shape(format!(
                "{text:?}: a drive name is printable Latin-1, without spaces"
            ))
        })?;
    if bytes.is_empty() || bytes.len() > 31 || bytes.iter().any(|&b| b == b':' || b == b'/') {
        return Err(CreateError::Shape(format!(
            "{text:?}: a drive name is 1 to 31 characters, without : or /"
        )));
    }
    Ok(bytes)
}

/// What `ade create --format=json` reports for a partitioned disk: as
/// [`created_json`], with each partition (schema 1.22).
#[must_use]
pub fn partitioned_json(path: &Path, bytes: u64, made: &[MadePartition]) -> crate::json::Value {
    use crate::json::Value;
    Value::Obj(vec![
        ("written", Value::str(path.to_string_lossy())),
        ("bytes", Value::Num(bytes)),
        (
            "partitions",
            Value::Arr(
                made.iter()
                    .map(|p| {
                        Value::Obj(vec![
                            ("name", Value::str(p.name.clone())),
                            ("type", Value::str(p.type_name.clone())),
                            ("volume", Value::str(p.volume.clone())),
                            ("first_block", Value::Num(p.first_block)),
                            ("blocks", Value::Num(p.blocks)),
                            ("bootable", Value::Bool(p.bootable)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}
