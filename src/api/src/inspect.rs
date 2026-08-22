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

use ade_block::{Geometry, GeometryError, read_at};
use ade_container::{Detection, Kind, RawImage, sniff};
use ade_filesystem::{bootblock::Bootblock, rootblock::Rootblock};

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

    let Kind::Adf { cylinders, sectors } = detection.kind else {
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
        };
    };

    let geometry = match Geometry::new(cylinders, 2, sectors, 512, Geometry::FLOPPY_RESERVED) {
        Ok(g) => g,
        Err(e) => {
            return Inspection {
                detection,
                size,
                geometry: None,
                bootblock,
                volume: None,
                volume_absent: Some(e.to_string()),
            };
        }
    };

    let (volume, volume_absent) = read_volume(bytes, geometry);
    Inspection {
        detection,
        size,
        geometry: Some(geometry),
        bootblock,
        volume,
        volume_absent,
    }
}

fn read_volume(bytes: Vec<u8>, geometry: Geometry) -> (Option<VolumeInfo>, Option<String>) {
    let Ok(image) = RawImage::new(bytes, geometry) else {
        return (None, Some("image is shorter than its geometry".to_owned()));
    };
    // Computed, never read from the bootblock — that field says 880 even on HD
    // volumes whose rootblock is at 1760 (C-007).
    let at = geometry.root_block();
    let mut block = vec![0u8; geometry.block_size() as usize];
    if let Err(e) = read_at(&image, at, &mut block) {
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
