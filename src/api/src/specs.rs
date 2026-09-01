//! What a disk needs to run, and what it cannot tell you (F-028).
//!
//! # What this is careful about
//!
//! An ADF is a dump of 880 KB of magnetic media. It does not carry a manifest,
//! so most of what somebody means by "the hardware this needs" — how much RAM,
//! which processor, OCS against AGA, PAL against NTSC — **is not in the bytes**
//! and cannot be recovered from them without executing or disassembling the
//! code, which ADE does not do (AV-002 is structural: there is no execution
//! path at all).
//!
//! So this reports **facts with their evidence** and refuses to draw the
//! verdict a reader might want. That is the same posture D-014 took over virus
//! names: the bootblock text is reported and no conclusion is offered, because
//! the conclusion was measurably backwards. A tool that says "needs 1 MB" from
//! a disk that never said so is worse than one that says nothing.
//!
//! Every claim below is a **lower bound** — "at least Kickstart 2.0" — because
//! that is the shape of the evidence. A disk referencing `asl.library` cannot
//! run on 1.3; it may need far more than 2.0, and nothing here would know.

use ade_block::Geometry;

/// One thing that can be said about the disk, and why.
#[derive(Debug, Clone)]
pub struct Fact {
    /// The claim, phrased as what it is or what it needs.
    pub what: String,
    /// The evidence for it, named so a reader can check.
    pub because: String,
}

/// What a disk says about itself.
#[derive(Debug, Clone, Default)]
pub struct Specs {
    /// Derivable facts, each with its evidence.
    pub facts: Vec<Fact>,
    /// The libraries found, in the order they were looked for.
    pub libraries: Vec<String>,
}

/// Libraries that did not exist before Kickstart 2.0.
///
/// *Sourced 2026-09-01 from the Amiga ROM Kernel Reference Manual via
/// [AMIGAWIKI]: several libraries introduced in Release 2 use version 36,
/// including these.* A disk that opens one of them cannot be running on 1.3.
///
/// **Only sourced entries are here.** `locale.library` and `datatypes.library`
/// are believed later still and appear on 14 and 6 of 400 sampled corpus
/// images, but no source consulted gives their introduction version, so they
/// are recorded in SPEC as leads and are **not** used to make a claim. That is
/// the same rule F-020's signature table follows: an entry with no evidence
/// behind it is recorded as untested rather than quietly trusted.
const RELEASE_2_LIBRARIES: [&str; 6] = [
    "asl.library",
    "gadtools.library",
    "iffparse.library",
    "utility.library",
    "commodities.library",
    "rexxsyslib.library",
];

/// Things an ADF cannot tell anyone, and why not.
///
/// Listed rather than omitted. A report that simply stops is read as "there is
/// nothing more to know"; one that names its own blind spots tells a reader
/// where to go and stops them inferring from silence.
pub const UNKNOWABLE: [(&str, &str); 4] = [
    (
        "Memory",
        "no Amiga disk format records a memory requirement; it is a property of \
         the program, discoverable only by running it",
    ),
    (
        "Processor",
        "68000 against 68020 or later is decided by the instructions used, which \
         needs a disassembler ADE does not have",
    ),
    (
        "Chipset",
        "OCS, ECS and AGA are told apart by which chip registers a program \
         writes, which again needs disassembly — the `(AGA)` in a TOSEC name is \
         the catalogue's claim, not the disk's",
    ),
    (
        "Video standard",
        "PAL against NTSC is set at run time by the program, and nothing on the \
         disk declares it",
    ),
];

impl Specs {
    /// Read what `bytes` will admit to, using `image` for its structure.
    #[must_use]
    pub fn of(image: &crate::Image, bytes: &[u8]) -> Self {
        let mut specs = Self::default();
        specs.media(image.geometry());
        specs.storage(image);
        specs.boot(bytes);
        specs.volume(image);
        specs.libraries(bytes);
        specs
    }

    /// The drive it came out of.
    fn media(&mut self, geometry: &Geometry) {
        let bytes = geometry.total_bytes();
        let (what, because) = match bytes {
            901_120 => (
                "A 3.5-inch double-density disk, which every Amiga can read".to_owned(),
                "880 KB: 80 cylinders x 2 heads x 11 sectors".to_owned(),
            ),
            1_802_240 => (
                "A 3.5-inch high-density disk, which needs an HD drive — an A3000 \
                 or later, or an external one"
                    .to_owned(),
                "1.76 MB: 22 sectors a track rather than 11".to_owned(),
            ),
            450_560 => (
                "A 5.25-inch double-density disk, which needs an A1020 or \
                 compatible drive"
                    .to_owned(),
                "440 KB: 40 cylinders, which is the A1020's format".to_owned(),
            ),
            other => (
                format!("{other} bytes, which is no standard floppy"),
                format!(
                    "{} blocks of {}",
                    geometry.total_blocks(),
                    geometry.block_size()
                ),
            ),
        };
        self.facts.push(Fact { what, because });
    }

    /// Whether it is a hard disk rather than a floppy.
    fn storage(&mut self, image: &crate::Image) {
        if matches!(image.rdb(), Ok(Some(_))) {
            self.facts.push(Fact {
                what: "A hard disk, so it needs a controller and cannot be put in a \
                       floppy drive"
                    .to_owned(),
                because: "a Rigid Disk Block in the reserved area".to_owned(),
            });
        }
    }

    /// How it starts, which decides whether an OS disk is needed at all.
    fn boot(&mut self, bytes: &[u8]) {
        let Some(head) = bytes.get(..4) else { return };
        let (what, because) = if head.starts_with(b"DOS") {
            (
                "Starts through AmigaDOS, so it needs a Kickstart that mounts the \
                 filesystem below"
                    .to_owned(),
                "an AmigaDOS bootblock".to_owned(),
            )
        } else if head.iter().all(|b| *b == 0) {
            (
                "Not bootable: it has to be inserted after something else has \
                 started"
                    .to_owned(),
                "an empty bootblock".to_owned(),
            )
        } else {
            // Not "self-booting". A bootblock that is not AmigaDOS's is
            // *either* a custom loader that takes the machine over, or an
            // AmigaDOS one that has been damaged — `Abandoned Places_Disk2`
            // begins `\x00OS\x00`, which is the second. Nothing here can tell
            // them apart without running the code, so both are reported and
            // neither is chosen. C-008 again: the bootblock and the filesystem
            // are separate facts, and so are these two readings of one.
            (
                "Does not start through AmigaDOS: it either boots itself and \
                 needs no Workbench, or its bootblock is damaged"
                    .to_owned(),
                format!(
                    "a custom bootblock beginning {}",
                    head.iter()
                        .map(|b| {
                            if b.is_ascii_graphic() {
                                (*b as char).to_string()
                            } else {
                                format!("\\x{b:02x}")
                            }
                        })
                        .collect::<String>()
                ),
            )
        };
        self.facts.push(Fact { what, because });
    }

    /// What the filesystem is, described rather than dated.
    ///
    /// Deliberately **not** turned into a Kickstart version. Which release
    /// first mounted which dostype is not recorded in any source this project
    /// has consulted, and inventing the mapping would put a number in front of
    /// somebody that nothing here can support.
    fn volume(&mut self, image: &crate::Image) {
        let Ok(volume) = image.volume() else { return };
        let Some(dostype) = volume.dostype() else {
            return;
        };
        self.facts.push(Fact {
            what: format!("Formatted as {dostype}"),
            because: format!("dostype {:#010x} in the bootblock", dostype.raw()),
        });
    }

    /// Libraries that place a floor under the Kickstart version.
    fn libraries(&mut self, bytes: &[u8]) {
        let mut found: Vec<String> = Vec::new();
        for name in RELEASE_2_LIBRARIES {
            if contains_name(bytes, name.as_bytes()) {
                found.push(name.to_owned());
            }
        }
        if found.is_empty() {
            return;
        }
        self.facts.push(Fact {
            what: "Needs at least Kickstart 2.0".to_owned(),
            because: format!("it opens {}, which Release 2 introduced", found.join(", ")),
        });
        self.libraries = found;
    }
}

/// Whether `needle` appears in `haystack` as a whole name.
///
/// The byte before must not be part of a name, or the search finds one inside
/// another string. Measured: without this, a scan of 400 corpus images reported
/// `udos.library`, `ugraphics.library` and `uintuition.library` — none of which
/// exists. They were the real names with whatever byte preceded them.
fn contains_name(haystack: &[u8], needle: &[u8]) -> bool {
    let Some(last) = haystack.len().checked_sub(needle.len()) else {
        return false;
    };
    (0..=last).any(|at| {
        if haystack.get(at..at.saturating_add(needle.len())) != Some(needle) {
            return false;
        }
        at == 0
            || !haystack
                .get(at.saturating_sub(1))
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'.')
    })
}
