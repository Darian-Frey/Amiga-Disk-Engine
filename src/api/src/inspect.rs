//! Read-only inspection of an image: what it is, and what state it is in.
//!
//! The first vertical slice through the pipeline — container, block, endian and
//! filesystem all participate — and the model for how ADE reports.
//!
//! # Two independent facts (C-008)
//!
//! An inspection never collapses "what container is this" and "does it hold a
//! mountable volume" into one verdict. Measured over 4288 real images, those
//! answers disagree often enough that collapsing them loses the interesting
//! cases: 19% of `DOS`-prefixed images have no rootblock, and ten images with
//! a foreign prefix mount perfectly. So [`Inspection`] carries the container
//! detection *with its evidence*, the bootblock *with its faults*, and the
//! volume *if there is one* — separately.

use std::{fs, io, path::Path};

use ade_block::{BlockError, BlockSource as _};
use ade_block::{Geometry, GeometryError, read_at};
use ade_container::{Detection, Kind, RawImage, Window, sniff};
use ade_filesystem::{
    bootblock::Bootblock,
    datestamp::DateFault,
    dostype::FileSystem,
    entry::Entry,
    rdb::{self, Partition, RigidDiskBlock},
    rootblock::Rootblock,
    volume::{FsError, Volume},
};

use crate::json::Value;

/// Bytes of an image that sniffing needs — the first two blocks, plus enough
/// to scan sixteen blocks for an RDB.
const HEAD_BYTES: usize = 512 * 16;

/// Everything an inspection could determine.
#[derive(Debug, Clone)]
pub struct Inspection {
    /// What the container appears to be, and the evidence for it.
    pub detection: Detection,
    /// Total bytes in the image.
    pub size: u64,
    /// The geometry used, where one could be established.
    pub geometry: Option<Geometry>,
    /// The bootblock, where the image was long enough to hold one.
    pub bootblock: Option<Bootblock>,
    /// The volume, if a rootblock was found where one should be.
    pub volume: Option<VolumeInfo>,
    /// Why no volume was found, when none was.
    pub volume_absent: Option<String>,
    /// The Rigid Disk Block, where the device has one.
    pub rdb: Option<RdbInfo>,
    /// The partitions the device declares, empty for an unpartitioned image.
    pub partitions: Vec<PartitionInfo>,
    /// Faults found walking the partition chain.
    pub partition_faults: Vec<String>,
}

/// A device's Rigid Disk Block, as `ade info` reports it.
///
/// The geometry here is what the **drive** declares, which is not the geometry
/// used to address blocks: block numbers are linear from the start of the
/// image. It is reported because partitions are cut in cylinders, so the
/// cylinder size is what makes their extents make sense.
#[derive(Debug, Clone)]
pub struct RdbInfo {
    /// The block the `RDSK` structure occupies — 0 on almost every device.
    pub block: u32,
    /// Whether its checksum verifies.
    pub checksum_valid: bool,
    /// Device block size in bytes.
    pub block_size: u32,
    /// Physical cylinders.
    pub cylinders: u32,
    /// Heads.
    pub heads: u32,
    /// Sectors per track.
    pub sectors: u32,
    /// Highest block used by the reserved area.
    pub high_rdsk_block: u32,
    /// Drive vendor, as stored.
    pub vendor: String,
    /// Drive product, as stored.
    pub product: String,
    /// Drive revision, as stored.
    pub revision: String,
}

/// One partition, as `ade info` reports it.
///
/// The dostype here is what the **partition table** claims, which is advisory:
/// the partition's own bootblock is authoritative (ADF FAQ §6.3). Where the two
/// disagree, `volume_name` came from mounting, so it reflects the bootblock.
#[derive(Debug, Clone)]
pub struct PartitionInfo {
    /// Drive name — `DH0` and the like.
    pub name: String,
    /// First cylinder, inclusive.
    pub low_cylinder: u32,
    /// Last cylinder, inclusive.
    pub high_cylinder: u32,
    /// First block on the device.
    pub first_block: u64,
    /// How many blocks the partition spans.
    pub blocks: u64,
    /// Block size in bytes.
    pub block_size: u32,
    /// Reserved blocks at the partition's start — usually 2. The rootblock is
    /// computed from it, so a partition that differs is still placed correctly
    /// (C-007).
    pub reserved: u32,
    /// The dostype the partition table claims.
    pub dostype: u32,
    /// Whether the partition is marked bootable.
    pub bootable: bool,
    /// Whether its `PART` checksum verifies.
    pub checksum_valid: bool,
    /// The volume label, where the partition mounted.
    pub volume_name: Option<String>,
    /// Why it did not mount, where it did not.
    pub mount_error: Option<String>,
}

/// A mounted-enough volume: what the rootblock says about itself.
#[derive(Debug, Clone)]
pub struct VolumeInfo {
    /// Where the rootblock was found — computed, not read (C-007).
    pub rootblock_at: u64,
    /// The rootblock as parsed.
    pub rootblock: Rootblock,
}

/// Why an inspection could not be performed at all.
#[derive(Debug)]
pub enum InspectionError {
    /// The file could not be read.
    Io(io::Error),
    /// The bytes do not cover a usable geometry.
    Geometry(GeometryError),
}

impl std::fmt::Display for InspectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Geometry(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for InspectionError {}

impl From<io::Error> for InspectionError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Inspect an image file.
///
/// # Errors
/// [`InspectionError::Io`] if the file cannot be read. Everything else — an
/// unrecognised container, a corrupt bootblock, a missing rootblock — is
/// reported in the [`Inspection`], because those are findings about the image
/// rather than failures of the tool.
pub fn inspect_path(path: &Path) -> Result<Inspection, InspectionError> {
    // Floppies are under two megabytes, so reading whole is simplest and
    // fastest. Whole-disk HDF images reach gigabytes and will want a
    // positional-read source; `BlockSource` is shaped to allow that in Phase 2
    // without disturbing anything above it.
    Ok(inspect_bytes(fs::read(path)?))
}

/// Inspect an image already in memory.
#[must_use]
pub fn inspect_bytes(bytes: Vec<u8>) -> Inspection {
    let size = bytes.len() as u64;
    let head = bytes.get(..HEAD_BYTES.min(bytes.len())).unwrap_or(&[]);
    let detection = sniff(head, size);

    let bootblock = Bootblock::parse(&bytes).ok();

    let Some(geometry) = geometry_for(detection.kind, size) else {
        // Every other container is a later phase. Report what was identified
        // and stop, rather than guessing at a geometry.
        let reason = match detection.kind {
            Kind::Unknown => "reading an unrecognised container is not implemented yet".to_owned(),
            other => format!("reading {other} is not implemented yet"),
        };
        return Inspection {
            detection,
            size,
            geometry: None,
            bootblock,
            volume: None,
            volume_absent: Some(reason),
            rdb: None,
            partitions: Vec::new(),
            partition_faults: Vec::new(),
        };
    };

    let geometry = match geometry {
        Ok(g) => g,
        Err(e) => {
            return Inspection {
                detection,
                size,
                geometry: None,
                bootblock,
                volume: None,
                volume_absent: Some(e.to_string()),
                rdb: None,
                partitions: Vec::new(),
                partition_faults: Vec::new(),
            };
        }
    };

    let Ok(image) = RawImage::new(bytes, geometry) else {
        return Inspection {
            detection,
            size,
            geometry: Some(geometry),
            bootblock,
            volume: None,
            volume_absent: Some("image is shorter than its geometry".to_owned()),
            rdb: None,
            partitions: Vec::new(),
            partition_faults: Vec::new(),
        };
    };
    let (volume, volume_absent) = read_volume(&image, geometry);
    let (rdb, partitions, partition_faults) = read_partition_table(&image);
    Inspection {
        detection,
        size,
        geometry: Some(geometry),
        // A device whose block 0 is an RDB has no bootblock; parsing one out of
        // the RDSK bytes yields a confident-sounding report about nothing.
        bootblock: if rdb.is_some() { None } else { bootblock },
        volume,
        volume_absent,
        rdb,
        partitions,
        partition_faults,
    }
}

/// Read the partition table, mounting each partition far enough to learn its
/// label.
///
/// A device with no Rigid Disk Block yields no partitions and no faults: a
/// floppy having no partition table is not a defect.
fn read_partition_table(image: &RawImage) -> (Option<RdbInfo>, Vec<PartitionInfo>, Vec<String>) {
    let geometry = *image.geometry();
    let Ok(Some(rdb)) = RigidDiskBlock::find(image, &geometry) else {
        return (None, Vec::new(), Vec::new());
    };
    let latin1 = |v: &[u8]| -> String { v.iter().map(|&b| char::from(b)).collect() };
    let info = RdbInfo {
        block: rdb.block,
        checksum_valid: rdb.checksum_valid,
        block_size: rdb.block_size,
        cylinders: rdb.cylinders,
        heads: rdb.heads,
        sectors: rdb.sectors,
        high_rdsk_block: rdb.high_rdsk_block,
        vendor: latin1(&rdb.vendor).trim().to_owned(),
        product: latin1(&rdb.product).trim().to_owned(),
        revision: latin1(&rdb.revision).trim().to_owned(),
    };
    let (parts, faults) = rdb::read_partitions(image, &geometry, &rdb);
    let infos = parts
        .iter()
        .map(|p| {
            let blocks = u32::try_from(p.block_count()).unwrap_or(u32::MAX);
            let block_size = if p.block_size == 0 { 512 } else { p.block_size };
            let window = Window::new(image, p.first_block(), blocks, block_size, p.reserved);
            let (volume_name, mount_error) = match window {
                Ok(w) => match Volume::mount(&w) {
                    Ok(v) => (Some(v.rootblock().name_lossy()), None),
                    Err(e) => (None, Some(e.to_string())),
                },
                Err(e) => (None, Some(e.to_string())),
            };
            PartitionInfo {
                name: p.name_lossy(),
                low_cylinder: p.low_cylinder,
                high_cylinder: p.high_cylinder,
                first_block: p.first_block(),
                blocks: p.block_count(),
                block_size,
                reserved: p.reserved,
                dostype: p.dostype,
                bootable: p.bootable,
                checksum_valid: p.checksum_valid,
                volume_name,
                mount_error,
            }
        })
        .collect();
    (
        Some(info),
        infos,
        faults.iter().map(ToString::to_string).collect(),
    )
}

fn read_volume(image: &RawImage, geometry: Geometry) -> (Option<VolumeInfo>, Option<String>) {
    // Computed, never read from the bootblock — that field says 880 even on HD
    // volumes whose rootblock is at 1760 (C-007).
    let at = geometry.root_block();
    let mut block = vec![0u8; geometry.block_size() as usize];
    if let Err(e) = read_at(image, at, &mut block) {
        return (None, Some(format!("cannot read {at}: {e}")));
    }
    match Rootblock::parse(&block) {
        Ok(r) if r.looks_like_a_rootblock() => (
            Some(VolumeInfo {
                rootblock_at: at.0,
                rootblock: r,
            }),
            None,
        ),
        Ok(r) => (
            None,
            Some(format!(
                "no rootblock at {at}: type {:#x}, secondary type {:#x}",
                r.block_type, r.secondary_type
            )),
        ),
        Err(e) => (None, Some(format!("cannot parse {at}: {e}"))),
    }
}

/// A problem found in an image.
///
/// Carries a **stable code** as well as a message. The message is for people
/// and may be reworded; the code is part of the scriptable surface (F-015) and
/// is not to be changed once released, because batch runs will match on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// Stable machine-readable identifier, kebab-case.
    pub code: &'static str,
    /// Human-readable description.
    pub message: String,
}

impl Fault {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Inspection {
    /// Everything wrong with this image.
    ///
    /// Computed here rather than in a front-end, so the human and JSON outputs
    /// cannot drift apart and a future GUI inherits the same list.
    ///
    /// Deliberately **not** included: an invalid bootblock checksum, a foreign
    /// bootblock prefix, and the absence of a volume. Each is normal on real
    /// disks — 26% of a 4288-image survey fail the checksum, 7% have a foreign
    /// prefix, and 25% hold no AmigaDOS volume — so treating them as faults
    /// would drown the real findings (C-008).
    #[must_use]
    pub fn faults(&self) -> Vec<Fault> {
        let mut faults = Vec::new();

        if let Some(bb) = &self.bootblock
            && let Ok(d) = &bb.dostype
            && d.unrecognised_flags() != 0
        {
            faults.push(Fault::new(
                "dostype-unknown-bits",
                format!(
                    "dostype carries undocumented bits {:#04x}",
                    d.unrecognised_flags()
                ),
            ));
        }

        let Some(v) = &self.volume else {
            return faults;
        };
        let r = &v.rootblock;
        if !r.checksum_valid {
            faults.push(Fault::new(
                "rootblock-checksum",
                "rootblock checksum does not match",
            ));
        }
        if !r.bitmap_flag_valid() {
            faults.push(Fault::new(
                "bitmap-flag-clear",
                "bitmap-valid flag is clear — the bitmap may be stale",
            ));
        }
        if r.name_length_overflows() {
            faults.push(Fault::new(
                "name-length-overflow",
                format!(
                    "volume name length claims {} bytes in a 30-byte field",
                    r.declared_name_len
                ),
            ));
        }
        for (label, stamp) in [
            ("created", r.created),
            ("modified", r.volume_altered),
            ("root-altered", r.root_altered),
        ] {
            for fault in stamp.faults() {
                let code = match fault {
                    DateFault::DayZero => "datestamp-day-zero",
                    DateFault::MinutesOutOfRange => "datestamp-minutes-range",
                    DateFault::TicksOutOfRange => "datestamp-ticks-range",
                };
                faults.push(Fault::new(code, format!("{label} datestamp: {fault}")));
            }
        }
        faults
    }

    /// The whole inspection as a JSON value (F-015).
    #[must_use]
    pub fn to_json(&self) -> Value {
        let geometry = self.geometry.map(|g| {
            Value::Obj(vec![
                ("cylinders", Value::Num(u64::from(g.cylinders()))),
                ("heads", Value::Num(u64::from(g.heads()))),
                ("sectors", Value::Num(u64::from(g.sectors()))),
                ("block_size", Value::Num(u64::from(g.block_size()))),
                ("total_blocks", Value::Num(g.total_blocks())),
            ])
        });

        let bootblock = self.bootblock.as_ref().map(bootblock_json);
        let volume = self.volume.as_ref().map(volume_json);
        let rdb = self.rdb.as_ref().map(RdbInfo::to_json);
        let partitions = Value::Arr(self.partitions.iter().map(PartitionInfo::to_json).collect());

        Value::Obj(vec![
            ("container", Value::str(self.detection.kind.to_string())),
            ("size", Value::Num(self.size)),
            (
                "evidence",
                Value::Arr(
                    self.detection
                        .evidence
                        .iter()
                        .map(|e| Value::str(e.to_string()))
                        .collect(),
                ),
            ),
            ("geometry", Value::opt(geometry, |g| g)),
            ("bootblock", Value::opt(bootblock, |b| b)),
            ("volume", Value::opt(volume, |v| v)),
            (
                "volume_absent",
                Value::opt(self.volume_absent.as_ref(), Value::str),
            ),
            ("rdb", Value::opt(rdb, |r| r)),
            ("partitions", partitions),
            (
                "partition_faults",
                Value::Arr(
                    self.partition_faults
                        .iter()
                        .map(|f| Value::str(f.clone()))
                        .collect(),
                ),
            ),
            (
                "faults",
                Value::Arr(
                    self.faults()
                        .into_iter()
                        .map(|f| {
                            Value::Obj(vec![
                                ("code", Value::str(f.code)),
                                ("message", Value::str(f.message)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// A directory entry as a JSON value (F-015).
#[must_use]
pub fn entry_to_json(entry: &Entry, path: &[Vec<u8>]) -> Value {
    Value::Obj(vec![
        ("name", Value::latin1(&entry.name)),
        ("path", Value::latin1(&path.join(&b'/'))),
        ("kind", Value::str(entry.kind.to_string())),
        (
            "size",
            if entry.kind.is_file() {
                Value::Num(u64::from(entry.byte_size))
            } else {
                Value::Null
            },
        ),
        ("block", Value::Num(u64::from(entry.block))),
        ("parent", Value::Num(u64::from(entry.parent))),
        (
            "protection",
            Value::str(entry.protection.to_amigados_string()),
        ),
        ("protection_bits", Value::Num(u64::from(entry.protection.0))),
        ("altered", Value::str(entry.altered.to_string())),
        (
            "comment",
            if entry.comment.is_empty() {
                Value::Null
            } else {
                Value::latin1(&entry.comment)
            },
        ),
        ("checksum_valid", Value::Bool(entry.checksum_valid)),
    ])
}

/// The geometry to mount a detected container with, if ADE can mount it.
///
/// A raw volume — a hardfile — carries no geometry of its own. Heads and
/// sectors are a convention of whatever created it, not a property of the
/// bytes, and nothing above the block layer depends on them: what matters is
/// the block count, which fixes where the rootblock sits (C-007). ADFlib takes
/// the same view, reporting an 8 MB hardfile as "Cylinders = 16384, Heads = 1,
/// Sectors = 1" — a shape it invented to reach the right total.
fn geometry_for(kind: Kind, size: u64) -> Option<Result<Geometry, GeometryError>> {
    match kind {
        Kind::Adf { cylinders, sectors } => Some(Geometry::new(
            cylinders,
            2,
            sectors,
            512,
            Geometry::FLOPPY_RESERVED,
        )),
        // Both present the device as a flat run of blocks. For an RDB device
        // that run is the *device*, not a volume: partitions are windows onto
        // it, each with its own reserved count from its DOSEnvVec.
        Kind::Hardfile | Kind::RigidDisk => {
            let blocks = u32::try_from(size / 512).ok()?;
            // Below a floppy's worth there is nothing worth mounting, and the
            // rootblock arithmetic stops being meaningful.
            (blocks >= 64).then(|| Geometry::new(blocks, 1, 1, 512, Geometry::FLOPPY_RESERVED))
        }
        _ => None,
    }
}

/// An image opened for browsing.
///
/// Owns the bytes so a [`Volume`] can borrow from it; the two-step open keeps
/// the borrow explicit rather than hiding it behind a self-referential type.
pub struct Image {
    raw: RawImage,
}

impl Image {
    /// Open an image file for browsing.
    ///
    /// # Errors
    /// [`InspectionError::Io`] if the file cannot be read, or
    /// [`InspectionError::Geometry`] if its size matches no usable geometry.
    pub fn open(path: &Path) -> Result<Self, InspectionError> {
        Self::from_bytes(fs::read(path)?)
    }

    /// Open bytes already in memory.
    ///
    /// # Errors
    /// [`InspectionError::Geometry`] if the size matches no usable geometry.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, InspectionError> {
        let size = bytes.len() as u64;
        let head = bytes.get(..HEAD_BYTES.min(bytes.len())).unwrap_or(&[]);
        let geometry = geometry_for(sniff(head, size).kind, size)
            .ok_or(InspectionError::Geometry(GeometryError::ZeroDimension))?
            .map_err(InspectionError::Geometry)?;
        let raw = RawImage::new(bytes, geometry)
            .map_err(|_| InspectionError::Geometry(GeometryError::ReservedExceedsVolume))?;
        Ok(Self { raw })
    }

    /// The device's partition table, if it has one.
    ///
    /// `Ok(None)` for an image with no Rigid Disk Block, which is most of them
    /// — a floppy has no partition table and that is not a fault.
    ///
    /// # Errors
    /// A read error on the reserved area.
    pub fn rdb(&self) -> Result<Option<RigidDiskBlock>, FsError> {
        RigidDiskBlock::find(&self.raw, self.raw.geometry())
    }

    /// Every partition the device declares, with any faults found walking the
    /// chain.
    ///
    /// A broken chain stops the walk and is reported rather than discarding the
    /// partitions found before it: half a partition table is still worth having.
    ///
    /// # Errors
    /// A read error on the reserved area.
    pub fn partitions(&self) -> Result<(Vec<Partition>, Vec<FsError>), FsError> {
        let Some(rdb) = self.rdb()? else {
            return Ok((Vec::new(), Vec::new()));
        };
        Ok(rdb::read_partitions(&self.raw, self.raw.geometry(), &rdb))
    }

    /// A window onto one partition, which can then be mounted like any volume.
    ///
    /// The partition's own `reserved` count comes from its DOSEnvVec and feeds
    /// the rootblock computation, so a partition differing from the usual two
    /// is still placed correctly (C-007).
    ///
    /// # Errors
    /// [`BlockError`] if the partition's extent falls outside the device.
    pub fn partition_window(&self, p: &Partition) -> Result<Window<'_>, BlockError> {
        let blocks = u32::try_from(p.block_count()).unwrap_or(u32::MAX);
        let block_size = if p.block_size == 0 { 512 } else { p.block_size };
        Window::new(&self.raw, p.first_block(), blocks, block_size, p.reserved)
    }

    /// Mount the volume this image holds.
    ///
    /// # Errors
    /// Whatever [`Volume::mount`] reports — most often that there is no
    /// rootblock where one should be.
    pub fn volume(&self) -> Result<Volume<'_>, FsError> {
        Volume::mount(&self.raw)
    }
}

/// One bootblock as JSON, split out to keep `Inspection::to_json` readable.
fn bootblock_json(bb: &Bootblock) -> Value {
    Value::Obj(vec![
        ("prefix", Value::str(bb.prefix_display())),
        ("is_dos", Value::Bool(bb.is_dos())),
        (
            "dostype",
            bb.dostype.as_ref().map_or(Value::Null, |d| {
                Value::Obj(vec![
                    ("raw", Value::Num(u64::from(d.raw()))),
                    ("flags", Value::Num(u64::from(d.flags()))),
                    ("label", Value::str(d.to_string())),
                    (
                        "filesystem",
                        Value::str(match d.filesystem() {
                            FileSystem::Ofs => "ofs",
                            FileSystem::Ffs => "ffs",
                        }),
                    ),
                    ("international", Value::Bool(d.is_international())),
                    ("dircache", Value::Bool(d.has_dircache())),
                    (
                        "unrecognised_flags",
                        Value::Num(u64::from(d.unrecognised_flags())),
                    ),
                ])
            }),
        ),
        ("checksum_valid", Value::Bool(bb.checksum_valid)),
        ("has_boot_code", Value::Bool(bb.has_boot_code)),
        (
            "stored_rootblock",
            Value::Num(u64::from(bb.stored_rootblock)),
        ),
    ])
}

/// One volume as JSON, split out to keep `Inspection::to_json` readable.
fn volume_json(v: &VolumeInfo) -> Value {
    let r = &v.rootblock;
    Value::Obj(vec![
        ("name", Value::latin1(&r.name)),
        ("rootblock", Value::Num(v.rootblock_at)),
        ("checksum_valid", Value::Bool(r.checksum_valid)),
        ("bitmap_flag_valid", Value::Bool(r.bitmap_flag_valid())),
        ("hash_table_size", Value::Num(u64::from(r.hash_table_size))),
        ("created", Value::str(r.created.to_string())),
        ("modified", Value::str(r.volume_altered.to_string())),
        ("root_altered", Value::str(r.root_altered.to_string())),
    ])
}

impl RdbInfo {
    /// This device's Rigid Disk Block as JSON (F-015).
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("block", Value::Num(u64::from(self.block))),
            ("checksum_valid", Value::Bool(self.checksum_valid)),
            ("block_size", Value::Num(u64::from(self.block_size))),
            ("cylinders", Value::Num(u64::from(self.cylinders))),
            ("heads", Value::Num(u64::from(self.heads))),
            ("sectors", Value::Num(u64::from(self.sectors))),
            (
                "high_rdsk_block",
                Value::Num(u64::from(self.high_rdsk_block)),
            ),
            ("vendor", Value::str(self.vendor.clone())),
            ("product", Value::str(self.product.clone())),
            ("revision", Value::str(self.revision.clone())),
        ])
    }
}

impl PartitionInfo {
    /// This partition as JSON (F-015).
    ///
    /// `dostype` is the **table's** claim; `volume_name` came from mounting, so
    /// it reflects the partition's own bootblock where the two disagree.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("name", Value::str(self.name.clone())),
            ("low_cylinder", Value::Num(u64::from(self.low_cylinder))),
            ("high_cylinder", Value::Num(u64::from(self.high_cylinder))),
            ("first_block", Value::Num(self.first_block)),
            ("blocks", Value::Num(self.blocks)),
            ("block_size", Value::Num(u64::from(self.block_size))),
            ("reserved", Value::Num(u64::from(self.reserved))),
            ("dostype", Value::Num(u64::from(self.dostype))),
            ("bootable", Value::Bool(self.bootable)),
            ("checksum_valid", Value::Bool(self.checksum_valid)),
            (
                "volume_name",
                Value::opt(self.volume_name.as_ref(), Value::str),
            ),
            (
                "mount_error",
                Value::opt(self.mount_error.as_ref(), Value::str),
            ),
        ])
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "tests build their own buffers"
)]
mod tests {
    use super::*;

    // The engine is tested against fixtures built by an independent statement
    // of the format, never against images committed to the repository (D-010).
    use ade_fixtures::{Volume, corrupt};

    #[test]
    fn identifies_and_mounts_a_clean_ofs_floppy() {
        let img = Volume::dd(0).named("Workbench").build();
        let i = inspect_bytes(img);
        assert_eq!(
            i.detection.kind,
            Kind::Adf {
                cylinders: 80,
                sectors: 11
            }
        );
        let bb = i.bootblock.expect("bootblock");
        assert!(bb.is_dos());
        assert!(bb.checksum_valid);
        let v = i.volume.expect("volume");
        assert_eq!(v.rootblock_at, 880);
        assert_eq!(v.rootblock.name_lossy(), "Workbench");
        assert!(v.rootblock.checksum_valid);
    }

    #[test]
    fn finds_the_hd_rootblock_at_1760_despite_the_bootblock_claim() {
        let img = Volume::hd(1).named("Big").build();
        let i = inspect_bytes(img);
        let v = i.volume.expect("volume");
        assert_eq!(v.rootblock_at, 1760, "computed, not read (C-007)");
        assert_eq!(
            i.bootblock.expect("bootblock").stored_rootblock,
            880,
            "...while the stored pointer still says 880"
        );
    }

    #[test]
    fn a_bad_bootblock_checksum_does_not_prevent_mounting() {
        // 26% of real images have one. It must not gate anything (C-008).
        let mut img = Volume::dd(0).named("NotBootable").build();
        corrupt::bootblock_checksum(&mut img);
        let i = inspect_bytes(img);
        assert!(!i.bootblock.expect("bootblock").checksum_valid);
        assert_eq!(
            i.volume.expect("volume").rootblock.name_lossy(),
            "NotBootable"
        );
    }

    #[test]
    fn a_foreign_bootblock_does_not_prevent_mounting() {
        // Ten of 300 non-DOS images in the survey mount perfectly.
        let mut img = Volume::dd(1).named("QUARTEX").build();
        corrupt::non_dos_bootblock(&mut img, b"ATN!");
        let i = inspect_bytes(img);
        let bb = i.bootblock.expect("bootblock");
        assert!(!bb.is_dos());
        assert_eq!(bb.prefix_display(), "ATN!");
        assert_eq!(i.volume.expect("volume").rootblock.name_lossy(), "QUARTEX");
    }

    #[test]
    fn a_missing_rootblock_is_reported_precisely() {
        // 19% of real DOS images are like this.
        let v = Volume::dd(0);
        let root = v.root();
        let mut img = v.build();
        corrupt::rootblock_wrong_type(&mut img, root);
        let i = inspect_bytes(img);
        assert!(
            i.bootblock.expect("bootblock").is_dos(),
            "the bootblock is fine"
        );
        assert!(i.volume.is_none());
        let why = i.volume_absent.expect("a reason");
        assert!(why.contains("no rootblock at block 880"), "got: {why}");
    }

    #[test]
    fn a_damaged_rootblock_still_mounts_but_says_so() {
        let v = Volume::dd(0).named("Damaged");
        let root = v.root();
        let mut img = v.build();
        corrupt::block_checksum(&mut img, root);
        let i = inspect_bytes(img);
        let vol = i.volume.expect("still identifies as a rootblock");
        assert!(!vol.rootblock.checksum_valid, "...but is damaged");
    }

    #[test]
    fn extra_cylinder_images_mount() {
        for cyl in [81u32, 82, 83] {
            let img = ade_fixtures::Volume::new(cyl, 2, 11, 1)
                .named("Extra")
                .build();
            let i = inspect_bytes(img);
            assert_eq!(
                i.detection.kind,
                Kind::Adf {
                    cylinders: cyl,
                    sectors: 11
                }
            );
            assert!(i.volume.is_some(), "{cyl} cylinders should mount");
        }
    }

    #[test]
    fn later_phase_containers_are_named_not_mangled() {
        let mut img = vec![0u8; 4096];
        img[..4].copy_from_slice(b"DMS!");
        let i = inspect_bytes(img);
        assert_eq!(i.detection.kind, Kind::Dms);
        assert!(i.volume_absent.expect("reason").contains("not implemented"));
    }

    #[test]
    fn hostile_and_degenerate_inputs_do_not_panic() {
        for bytes in [
            vec![],
            vec![0u8; 1],
            vec![0u8; 511],
            vec![0u8; 1024],
            corrupt::zeroed_volume(),
            corrupt::truncated(&Volume::dd(0).build(), 176),
            corrupt::with_trailing_junk(&Volume::dd(0).build(), 1),
        ] {
            let _ = inspect_bytes(bytes);
        }
    }
}
