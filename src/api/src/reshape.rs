//! Converting between a partition and a standalone hardfile (F-038).
//!
//! The volume is copied byte for byte, both ways. A hardfile *is* a
//! partition's blocks with nothing in front of them, so nothing in the volume
//! need change: its rootblock is in the middle of its blocks in either
//! (C-007), provided there are exactly as many. They always are here: where
//! a volume is not a whole number of the disk's cylinders, its partition is
//! given its own cylinders — one head, as many sectors as divide both — since
//! an environment vector carries its own geometry and AmigaDOS mounts from
//! that. Growing the volume to fit instead would move its rootblock onto
//! whatever is next to it, which is usually its own bitmap.
//!
//! What a partition has that a hardfile cannot hold is refused when it would
//! change how the volume reads — blocks that are not 512 bytes, a reserved
//! count other than 2 — and said when it would not: the drive name and boot
//! priority are the table's.

use std::io::Write;
use std::path::Path;

use ade_block::BlockSource as _;
use ade_container::Kind;
use ade_container::bytes::{ByteSource, Flat};
use ade_filesystem::dostype::Dostype;
use ade_filesystem::rdb::{self, END_OF_LIST, NewPartition, NewTable, Partition};
use ade_filesystem::volume::Volume;

use crate::create::{HEADS, MAX_PARTITIONED_MB, SECTORS, TABLE_CYLINDERS};
use crate::inspect::inspect_in_place;
use crate::{Image, Input};

/// Why a conversion was not made.
#[derive(Debug)]
pub enum ReshapeError {
    /// The image cannot be converted so, and why.
    Refused(String),
    /// Something already exists at the output path.
    Exists,
    /// The input could not be read or the output written.
    Io(std::io::Error),
}

impl core::fmt::Display for ReshapeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused(why) => f.write_str(why),
            Self::Exists => f.write_str("already exists, refusing to overwrite"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for ReshapeError {}

impl From<std::io::Error> for ReshapeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

fn refuse(why: impl Into<String>) -> ReshapeError {
    ReshapeError::Refused(why.into())
}

/// A conversion made.
#[derive(Debug, Clone)]
pub struct Reshaped {
    /// What was read: [`Kind::RigidDisk`] or [`Kind::Hardfile`] or an ADF.
    pub from: Kind,
    /// What was written; [`Kind::Gzip`] for an HDZ.
    pub to: Kind,
    /// The partition's drive name, on whichever side has one.
    pub partition: String,
    /// The volume's name, where ADE can mount it.
    pub volume: Option<String>,
    /// Bytes written.
    pub bytes: u64,
    /// What a reader of the result should know.
    pub notes: Vec<String>,
}

impl Reshaped {
    /// As `ade convert --format=json` reports it: the fields of any
    /// conversion, and the partition and volume (schema 1.23).
    #[must_use]
    pub fn to_json(&self, written: &Path) -> crate::json::Value {
        use crate::json::Value;
        let json = crate::convert::converted_json(self.from, self.to, false, written, self.bytes);
        let Value::Obj(mut fields) = json else {
            return json;
        };
        fields.push(("partition", Value::str(self.partition.clone())));
        fields.push(("volume", Value::opt(self.volume.as_ref(), Value::str)));
        fields.push((
            "notes",
            Value::Arr(self.notes.iter().map(Value::str).collect()),
        ));
        Value::Obj(fields)
    }
}

/// Convert `input` between shapes, naming the partition: out of it, if the
/// image has a partition table; into a new disk with just it, if not.
///
/// `output` is written as a hardfile, or with `compressed` as an HDZ. Never
/// over an existing file, and nothing is left there if it fails.
///
/// # Errors
/// [`ReshapeError`].
pub fn convert(
    input: &Path,
    partition: &str,
    output: &Path,
    compressed: bool,
) -> Result<Reshaped, ReshapeError> {
    let image = open(input)?;
    let (parts, faults) = image
        .partitions()
        .map_err(|e| refuse(format!("the partition table cannot be read: {e}")))?;
    if let Some(fault) = faults.first() {
        return Err(refuse(format!(
            "the partition table cannot be read: {fault}"
        )));
    }
    let has_table = image.rdb().ok().flatten().is_some();
    let mut made = if has_table {
        let index = crate::find_partition(&parts, partition).map_err(refuse)?;
        let part = parts
            .get(index)
            .ok_or_else(|| refuse("no such partition"))?;
        out_of(&image, part, output, compressed)?
    } else {
        into_new(&image, partition, output, compressed)?
    };
    if compressed {
        made.to = Kind::Gzip;
    }
    Ok(made)
}

fn open(path: &Path) -> Result<Image, ReshapeError> {
    let input = Input::open(path)?;
    let inspection = input.inspect();
    let kind = inspection.detection.kind;
    if !matches!(kind, Kind::Adf { .. } | Kind::Hardfile | Kind::RigidDisk) {
        return Err(refuse(format!(
            "{kind} is not a volume or a partitioned disk; convert it to a sector image first"
        )));
    }
    let size = inspection.size;
    let image = input.into_image().map_err(|e| match e {
        crate::InspectionError::Io(e) => ReshapeError::Io(e),
        crate::InspectionError::Geometry(_) => refuse(e.to_string()),
    })?;
    let blocks = image.geometry().total_bytes();
    if kind != Kind::RigidDisk && blocks != size {
        return Err(refuse(format!(
            "the volume is {blocks} bytes of the image's {size}, and the rest would be lost"
        )));
    }
    Ok(image)
}

/// A volume ADE can put in a partition: its name and dostype.
struct Held {
    name: String,
    dostype: Dostype,
}

fn held(image: &Image) -> Result<Held, ReshapeError> {
    let volume = image
        .volume()
        .map_err(|e| refuse(format!("it holds no volume ADE can mount: {e}")))?;
    let dostype = volume.dostype().ok_or_else(|| {
        refuse("its bootblock names no AmigaDOS filesystem, so a partition could not name it")
    })?;
    Ok(Held {
        name: volume.rootblock().name_lossy(),
        dostype,
    })
}

/// A partition's blocks, as a hardfile.
fn out_of(
    image: &Image,
    part: &Partition,
    output: &Path,
    compressed: bool,
) -> Result<Reshaped, ReshapeError> {
    let name = part.name_lossy();
    if part.block_size != 512 || part.reserved != 2 {
        return Err(refuse(format!(
            "{name} has {}-byte blocks and {} reserved; a hardfile has nowhere to say so and \
             is read as 512 and 2",
            part.block_size, part.reserved
        )));
    }
    let window = image
        .partition_window(part)
        .map_err(|e| refuse(format!("{name} does not fit the disk: {e}")))?;
    let bytes = Flat(&window);
    // Read as the file it will be: it must be the same volume, or — where ADE
    // can mount none — at least not be taken for some other container.
    let mounted = Volume::mount(&window).ok();
    let alone = inspect_in_place(&bytes);
    let kind = alone.detection.kind;
    let same = if mounted.is_some() {
        matches!(kind, Kind::Adf { .. } | Kind::Hardfile)
            && alone.geometry.map(|g| g.total_blocks()) == Some(window.geometry().total_blocks())
    } else {
        crate::inspect::reads_in_place(kind) && kind != Kind::RigidDisk
    };
    if !same {
        return Err(refuse(format!(
            "{name} on its own would be read as something else ({kind})"
        )));
    }
    let volume = mounted.as_ref().map(|v| v.rootblock().name_lossy());
    let mut notes = vec![format!(
        "the drive name ({name}) and boot settings are the partition table's and are not in \
         a hardfile"
    )];
    let own = mounted.as_ref().and_then(Volume::dostype);
    if mounted.is_none() {
        notes.push(
            "it holds no volume ADE can mount, and was copied as it is; a filesystem loaded \
             from the partition table is not in a hardfile"
                .to_owned(),
        );
    } else if own.map(Dostype::raw) != Some(part.dostype) {
        notes.push(format!(
            "the table called it {:#010x} and its bootblock says {}; a hardfile has only the \
             bootblock",
            part.dostype,
            own.map_or_else(|| "nothing".to_owned(), |d| d.to_string())
        ));
    }
    let size = bytes.size();
    let written = write_out(output, compressed, size, &[], &bytes, 0)?;
    Ok(Reshaped {
        from: Kind::RigidDisk,
        to: Kind::Hardfile,
        partition: name,
        volume,
        bytes: written,
        notes,
    })
}

/// How a partition of exactly `blocks`, starting at cylinder `first` of a
/// disk with `heads` × `sectors`, is described: its own heads and sectors,
/// and its cylinders in those.
pub(crate) struct Fit {
    pub(crate) heads: u32,
    pub(crate) sectors: u32,
    pub(crate) low: u32,
    pub(crate) high: u32,
}

impl Fit {
    /// The disk's own cylinders when they divide the volume; otherwise one
    /// head and the most sectors that divide both. `None` if nothing does.
    pub(crate) fn of(blocks: u32, heads: u32, sectors: u32, first: u32) -> Option<Self> {
        let per = heads.checked_mul(sectors)?;
        let start = first.checked_mul(per)?;
        let (heads, sectors) = if blocks.checked_rem(per)? == 0 {
            (heads, sectors)
        } else {
            (1, gcd(per, blocks))
        };
        let own = heads.checked_mul(sectors)?;
        let low = start.checked_div(own)?;
        let high = low.checked_add(blocks.checked_div(own)?)?.checked_sub(1)?;
        Some(Self {
            heads,
            sectors,
            low,
            high,
        })
    }

    /// What to say of it, when it is not the disk's.
    pub(crate) fn note(&self, heads: u32, sectors: u32) -> Option<String> {
        ((self.heads, self.sectors) != (heads, sectors)).then(|| {
            format!(
                "the volume is not a whole number of the disk's cylinders, so the partition \
                 has its own, of {} blocks; AmigaDOS mounts it, and a partitioning tool may \
                 show it oddly",
                self.heads.saturating_mul(self.sectors)
            )
        })
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    match a.checked_rem(b) {
        None | Some(0) => b.max(1),
        Some(r) => gcd(b, r),
    }
}

/// A volume as the one partition of a new disk, bootable, as
/// [`crate::create::partitioned`] makes its first.
fn into_new(
    image: &Image,
    name: &str,
    output: &Path,
    compressed: bool,
) -> Result<Reshaped, ReshapeError> {
    let drive = crate::create::drive_name(name).map_err(|e| refuse(e.to_string()))?;
    let held = held(image)?;
    let from = inspect_in_place(&Flat(image.source())).detection.kind;
    let blocks = u32::try_from(image.geometry().total_blocks())
        .map_err(|_| refuse("the volume is too large for a partition"))?;
    let per = HEADS.saturating_mul(SECTORS);
    let too_large = || {
        refuse(format!(
            "a partitioned disk is at most {MAX_PARTITIONED_MB} MB"
        ))
    };
    let fit = Fit::of(blocks, HEADS, SECTORS, TABLE_CYLINDERS).ok_or_else(too_large)?;
    let cylinders = TABLE_CYLINDERS.saturating_add(blocks.div_ceil(per));
    let size = u64::from(cylinders)
        .saturating_mul(u64::from(per))
        .saturating_mul(512);
    if size > u64::from(MAX_PARTITIONED_MB).saturating_mul(1024 * 1024) {
        return Err(too_large());
    }
    let part = NewPartition {
        name: drive,
        low_cylinder: fit.low,
        high_cylinder: fit.high,
        dostype: held.dostype.raw(),
        bootable: true,
        boot_priority: 0,
    };
    let table = NewTable {
        cylinders,
        heads: HEADS,
        sectors: SECTORS,
        lo_cylinder: TABLE_CYLINDERS,
        partitions: vec![part.clone()],
        drive: crate::create::drive_strings(),
    };
    let mut table_bytes = Vec::new();
    for (_, raw) in rdb::table_blocks(&table).map_err(refuse)? {
        table_bytes.extend_from_slice(&raw);
    }
    // Block 1, the partition, in its own cylinders.
    let block = rdb::part_block(fit.heads, fit.sectors, &part, END_OF_LIST);
    if let Some(place) = table_bytes.get_mut(512..1024) {
        place.copy_from_slice(&block);
    }
    let start = u64::from(TABLE_CYLINDERS.saturating_mul(per)).saturating_mul(512);
    let bytes = Flat(image.source());
    let written = write_out(output, compressed, size, &table_bytes, &bytes, start)?;
    let mut notes: Vec<String> = fit.note(HEADS, SECTORS).into_iter().collect();
    notes.push(format!(
        "{name} is bootable at priority 0; the filesystem is the one in Kickstart's ROM, 2.0 \
         and later, since ADE writes none into the table (D-024)"
    ));
    Ok(Reshaped {
        from,
        to: Kind::RigidDisk,
        partition: name.to_owned(),
        volume: Some(held.name),
        bytes: written,
        notes,
    })
}

/// Write `size` bytes to a new file: `head`, then `body` at `start`, zeros
/// between and after. Removed again if it fails. Returns the file's length.
fn write_out(
    output: &Path,
    compressed: bool,
    size: u64,
    head: &[u8],
    body: &dyn ByteSource,
    start: u64,
) -> Result<u64, ReshapeError> {
    if compressed && size > crate::MAX_DECOMPRESSED as u64 {
        return Err(refuse(format!(
            "an HDZ ADE could read back holds at most {} MB",
            crate::MAX_DECOMPRESSED / (1024 * 1024)
        )));
    }
    let mut file = match std::fs::File::create_new(output) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ReshapeError::Exists);
        }
        Err(e) => return Err(ReshapeError::Io(e)),
    };
    let written = if compressed {
        let mut whole = std::io::Cursor::new(Vec::new());
        lay_out(&mut whole, size, head, body, start).and_then(|()| {
            let mut whole = whole.into_inner();
            whole.resize(usize::try_from(size).unwrap_or(usize::MAX), 0);
            let packed =
                ade_container::deflate::gzip(&whole).ok_or(std::io::ErrorKind::InvalidInput)?;
            file.write_all(&packed)
        })
    } else {
        lay_out(&mut file, size, head, body, start).and_then(|()| file.set_len(size))
    };
    let length = written
        .and_then(|()| file.sync_all())
        .and_then(|()| file.metadata());
    match length {
        Ok(about) => Ok(about.len()),
        Err(e) => {
            // A half-written disk would be trusted; it is removed.
            drop(file);
            let _ = std::fs::remove_file(output);
            Err(ReshapeError::Io(e))
        }
    }
}

fn lay_out(
    out: &mut (impl Write + std::io::Seek),
    size: u64,
    head: &[u8],
    body: &dyn ByteSource,
    start: u64,
) -> std::io::Result<()> {
    if start.saturating_add(body.size()) > size {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    out.write_all(head)?;
    out.seek(std::io::SeekFrom::Start(start))?;
    let mut failed = Ok(());
    let read = crate::input::pieces(body, 512, 0, |piece, _, _| {
        failed = out.write_all(piece);
        failed.is_ok()
    });
    failed?;
    if read < body.size() {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    Ok(())
}

/// A volume to go into a partition of an existing disk: sound, whole, and
/// every byte of it ([`crate::edit::Session::import_partition`]).
pub(crate) struct Incoming {
    pub(crate) image: Image,
    pub(crate) dostype: Dostype,
    pub(crate) blocks: u32,
    pub(crate) volume: String,
}

pub(crate) fn incoming(path: &Path) -> Result<Incoming, ReshapeError> {
    let image = open(path)?;
    if image.rdb().ok().flatten().is_some() {
        return Err(refuse(
            "it is a partitioned disk, not a volume; convert one of its partitions to a \
             hardfile first",
        ));
    }
    let held = held(&image)?;
    let report = Input::open(path)?.examine(None);
    let faults: Vec<String> = report
        .findings
        .iter()
        .filter(|f| {
            f.severity >= crate::health::Severity::Warning && f.code != "free-blocks-hold-data"
        })
        .map(|f| format!("{}: {}", f.code, f.message))
        .collect();
    if !faults.is_empty() {
        return Err(refuse(format!(
            "its volume is not sound; repair it first ({})",
            faults.join("; ")
        )));
    }
    let blocks = u32::try_from(image.geometry().total_blocks())
        .map_err(|_| refuse("the volume is too large for a partition"))?;
    Ok(Incoming {
        image,
        dostype: held.dostype,
        blocks,
        volume: held.name,
    })
}
