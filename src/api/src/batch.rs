//! Corpus-scale batch operations (F-014).
//!
//! The workflow this project exists to serve is not one disk but thousands.
//! Every measurement in SPEC's corpus observations was taken by hand-rolling a
//! script over 4652 images; this is that capability made part of the tool.
//!
//! # What a batch run has to get right
//!
//! **Nothing may abort the run.** An image that cannot be read is a *result*,
//! not an error: a run over four thousand disks that stops at the first bad
//! one has told you about one disk. Every failure becomes a record.
//!
//! **The summary is the product.** Per-image detail matters, but what a person
//! actually needs from four thousand disks is the histogram — how many mount,
//! which faults recur, which handful need attention. The stable fault codes
//! (F-015) are what make that possible: they are countable in a way messages
//! are not.
//!
//! **Order is deterministic.** Paths are sorted before work begins, so two runs
//! over the same corpus produce comparable output and a failure is reproducible.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ade_catalogue::Catalogue;
use ade_container::Kind;

use crate::{Severity, examine, json::Value};

/// What happened to one image.
#[derive(Debug, Clone)]
pub struct Record {
    /// Where it came from.
    pub path: PathBuf,
    /// Bytes on disk.
    pub size: u64,
    /// The container, as identified — a sentence for a person.
    pub container: String,
    /// The container as a stable code: `adf`, `extended-adf`, `scp`.
    ///
    /// A cataloguer keys on this; `container` above is prose and may be
    /// reworded (F-015).
    pub container_code: &'static str,
    /// What converting this image produced, when a conversion was asked for.
    pub conversion: Option<ConversionOutcome>,
    /// SHA-1 of the image as it sits on disk, when hashing was asked for.
    ///
    /// **Opt-in, because it is not free**: 349 MB/s measured, which is about
    /// twelve seconds over a 4.2 GB corpus against five for the health pass.
    /// A catalogue wants it as the primary key for duplicates; a health run
    /// does not want it at all, and ADE does not hash unless asked.
    pub sha1: Option<String>,
    /// The volume's name, where one mounted.
    pub volume: Option<String>,
    /// Files reached.
    pub files: usize,
    /// Directories reached.
    pub directories: usize,
    /// Bytes recovered from files.
    pub bytes_recovered: u64,
    /// Fault codes found, in the order reported.
    pub findings: Vec<&'static str>,
    /// The worst severity found, if anything was.
    pub worst: Option<Severity>,
    /// Why the file could not be examined at all.
    pub unreadable: Option<String>,
    /// What the dataset calls this image, when one was supplied (F-013).
    ///
    /// More than one name means the content hash is ambiguous; ADE reports
    /// them all rather than choosing.
    pub identified: Vec<String>,
}

impl Record {
    /// Whether the image mounted and reported nothing worse than information.
    #[must_use]
    pub fn is_sound(&self) -> bool {
        self.unreadable.is_none()
            && self.volume.is_some()
            && self.worst.is_none_or(|s| s < Severity::Warning)
    }
}

/// Everything a batch run found.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// Every image, in sorted path order.
    pub records: Vec<Record>,
    /// How many images were examined.
    pub examined: usize,
    /// How many could not be read at all.
    pub unreadable: usize,
    /// How many mounted a volume.
    pub mounted: usize,
    /// How many mounted and were sound.
    pub sound: usize,
    /// Containers seen, and how many of each.
    pub containers: BTreeMap<String, usize>,
    /// Fault codes seen, and **how many images carried each** — not how many
    /// times each occurred. One damaged disk can report the same code dozens
    /// of times, and a histogram that summed those would read as dozens of
    /// damaged disks.
    pub findings: BTreeMap<&'static str, usize>,
    /// Total bytes recovered across every image.
    pub bytes_recovered: u64,
    /// How many images the dataset could name, when one was supplied.
    pub identified: usize,
    /// Conversion outcomes by code, when a conversion was asked for.
    ///
    /// Keyed rather than counted flat, because "3601 converted" alone hides
    /// the answer a person actually wants from a bulk run: **which** images
    /// were refused, and why. The codes are the same ones on each record.
    pub conversions: BTreeMap<&'static str, usize>,
}

impl Summary {
    /// Images carrying at least one error-severity finding.
    #[must_use]
    pub fn at_risk(&self) -> Vec<&Record> {
        self.records
            .iter()
            .filter(|r| r.worst == Some(Severity::Error))
            .collect()
    }

    /// The summary as JSON (F-015).
    ///
    /// Counts first, records second: a caller aggregating thousands of images
    /// usually wants the histogram and should not have to parse past every
    /// record to reach it.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("examined", Value::Num(self.examined as u64)),
            ("unreadable", Value::Num(self.unreadable as u64)),
            ("mounted", Value::Num(self.mounted as u64)),
            ("sound", Value::Num(self.sound as u64)),
            ("bytes_recovered", Value::Num(self.bytes_recovered)),
            ("identified", Value::Num(self.identified as u64)),
            (
                // An array of pairs rather than an object: container names are
                // runtime strings, and a JSON object with arbitrary keys is
                // harder to consume than a list of records anyway.
                "containers",
                Value::Arr(
                    self.containers
                        .iter()
                        .map(|(name, count)| {
                            Value::Obj(vec![
                                ("name", Value::str(name.clone())),
                                ("count", Value::Num(*count as u64)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                // An array of pairs, for the same reason `containers` is one —
                // and here it matters more. These keys are **fault codes**, so
                // an object keyed on them changes shape with the data: a run
                // over a healthy corpus and a run over a broken one produce
                // documents with different fields, and no inventory can pin
                // that (D-015). Changed before the schema was declared 1.0
                // precisely so it would not need a major version later.
                "findings",
                Value::Arr(
                    self.findings
                        .iter()
                        .map(|(code, images)| {
                            Value::Obj(vec![
                                ("code", Value::str(*code)),
                                ("images", Value::Num(*images as u64)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                // Same array-of-pairs shape as `containers` and `findings`,
                // and for the same reason: keys that come from data cannot be
                // inventoried (D-015).
                "conversions",
                Value::Arr(
                    self.conversions
                        .iter()
                        .map(|(code, images)| {
                            Value::Obj(vec![
                                ("code", Value::str(*code)),
                                ("images", Value::Num(*images as u64)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

impl Record {
    /// One image's record as JSON (F-015).
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("path", Value::str(self.path.display().to_string())),
            ("size", Value::Num(self.size)),
            ("container", Value::str(self.container.clone())),
            ("container_code", Value::str(self.container_code)),
            ("sha1", Value::opt(self.sha1.as_ref(), Value::str)),
            (
                "conversion",
                Value::opt(self.conversion.as_ref(), ConversionOutcome::to_json),
            ),
            ("volume", Value::opt(self.volume.as_ref(), Value::str)),
            ("files", Value::Num(self.files as u64)),
            ("directories", Value::Num(self.directories as u64)),
            ("bytes_recovered", Value::Num(self.bytes_recovered)),
            (
                "findings",
                Value::Arr(self.findings.iter().map(|c| Value::str(*c)).collect()),
            ),
            (
                "worst",
                self.worst.map_or(Value::Null, |s| {
                    Value::str(match s {
                        Severity::Info => "info",
                        Severity::Warning => "warning",
                        Severity::Error => "error",
                    })
                }),
            ),
            (
                "unreadable",
                Value::opt(self.unreadable.as_ref(), Value::str),
            ),
            (
                "identified",
                Value::Arr(
                    self.identified
                        .iter()
                        .map(|n| Value::str(n.clone()))
                        .collect(),
                ),
            ),
        ])
    }
}

/// One image's identification as JSON (F-015, BUG-007).
///
/// Lives here because this is the module that already knows about the
/// catalogue; `ade-catalogue` sits below the JSON writer and cannot reach it.
///
/// **Every match is listed, and `match` says what several of them mean.**
/// Usually `duplicated`: the dataset holds one file under more than one name,
/// and every name is correct. `collision` would mean different content
/// claiming one CRC32, which is the case worth distrusting and has never been
/// observed in the Amiga set. `ambiguous` is kept, unchanged, as "more than
/// one entry" — a caller taking `matches[0]` should still look.
#[must_use]
pub fn identification_json(
    path: &str,
    matches: &[&ade_catalogue::Entry],
    kind: ade_catalogue::Match,
) -> Value {
    Value::Obj(vec![
        ("path", Value::str(path)),
        ("identified", Value::Bool(!matches.is_empty())),
        ("ambiguous", Value::Bool(matches.len() > 1)),
        ("match", Value::str(match_label(kind))),
        (
            "matches",
            Value::Arr(
                matches
                    .iter()
                    .map(|e| {
                        Value::Obj(vec![
                            ("name", Value::str(e.name.clone())),
                            ("source", Value::str(e.source.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// What happened when one image in a batch was converted (F-014, F-016).
///
/// A per-image outcome rather than a run-wide one, because a corpus is
/// heterogeneous by nature: over 4,652 real images a single target format is
/// lossless for most, refused for the flux captures and unimplemented for the
/// compressed ones, and a run that stopped at the first refusal would convert
/// nothing. Nothing aborts a batch — the same rule the health pass follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversionOutcome {
    /// A stable code: `converted`, `exists`, `lossy`, `refused`,
    /// `not-implemented`, `failed`.
    pub code: &'static str,
    /// Where the output went, when one was written.
    pub written: Option<PathBuf>,
    /// Why, when nothing was written.
    pub reason: Option<String>,
}

impl ConversionOutcome {
    /// Whether an output file was actually produced.
    #[must_use]
    pub fn wrote(&self) -> bool {
        self.code == "converted"
    }

    /// The outcome as JSON (F-015).
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Obj(vec![
            ("code", Value::str(self.code)),
            (
                "written",
                Value::opt(self.written.as_ref(), |p| {
                    Value::str(p.display().to_string())
                }),
            ),
            ("reason", Value::opt(self.reason.as_ref(), Value::str)),
        ])
    }
}

/// The stable code for a match kind (F-015: codes are a commitment).
const fn match_label(kind: ade_catalogue::Match) -> &'static str {
    match kind {
        ade_catalogue::Match::Unknown => "unknown",
        ade_catalogue::Match::Named => "named",
        ade_catalogue::Match::Duplicated => "duplicated",
        ade_catalogue::Match::Collision => "collision",
        ade_catalogue::Match::Unverified => "unverified",
    }
}

/// A bulk conversion: what to produce and where to put it (F-014, F-016).
#[derive(Debug, Clone)]
pub struct ConvertRequest {
    /// The container to write.
    pub to: Kind,
    /// The directory outputs go into. Created if it does not exist.
    pub into: PathBuf,
}

impl ConvertRequest {
    /// Where one input's output belongs.
    ///
    /// The input's stem plus the target's extension, in the output directory.
    /// Flat rather than mirroring the input tree, because `batch` walks one
    /// level (a corpus is a flat directory in every case this has met) and a
    /// deeper output shape would imply a deeper input one.
    #[must_use]
    pub fn destination(&self, input: &Path) -> PathBuf {
        let stem = input.file_stem().unwrap_or_default();
        let mut name = std::ffi::OsString::from(stem);
        name.push(".");
        name.push(extension_for(self.to));
        self.into.join(name)
    }
}

/// The file extension a container is conventionally written with.
///
/// An extended ADF is `.adf` like a plain one — the extension cannot
/// distinguish them, which is exactly why `ade convert` needs `--raw` and why
/// a bulk conversion writes into a directory of its own.
const fn extension_for(kind: Kind) -> &'static str {
    match kind {
        Kind::Adf { .. } | Kind::ExtendedAdf { .. } => "adf",
        Kind::Hardfile | Kind::RigidDisk => "hdf",
        Kind::Gzip => "adz",
        Kind::Dms => "dms",
        Kind::Scp => "scp",
        Kind::Ipf => "ipf",
        Kind::Unknown => "bin",
    }
}

/// Convert one image, turning every refusal into a reported outcome.
///
/// **Nothing here aborts a run.** A corpus is heterogeneous: one target format
/// is lossless for most images, refused for the flux captures and
/// unimplemented for the compressed ones, and a bulk conversion that stopped
/// at the first refusal would convert nothing.
fn convert_one(path: &Path, bytes: Vec<u8>, request: &ConvertRequest) -> ConversionOutcome {
    let destination = request.destination(path);
    // Never overwrite. A conversion that silently replaces an image is the
    // irreversible damage D-004 is about, and in bulk it is that damage
    // repeated four thousand times before anyone notices.
    if destination.exists() {
        return ConversionOutcome {
            code: "exists",
            written: None,
            reason: Some(format!("{} already exists", destination.display())),
        };
    }

    match crate::convert::convert_bytes(bytes, request.to) {
        Ok(out) => {
            if let Err(e) = std::fs::create_dir_all(&request.into) {
                return ConversionOutcome {
                    code: "failed",
                    written: None,
                    reason: Some(e.to_string()),
                };
            }
            match std::fs::write(&destination, &out) {
                Ok(()) => ConversionOutcome {
                    code: "converted",
                    written: Some(destination),
                    reason: None,
                },
                Err(e) => ConversionOutcome {
                    code: "failed",
                    written: None,
                    reason: Some(e.to_string()),
                },
            }
        }
        Err(e) => ConversionOutcome {
            code: e.code(),
            written: None,
            reason: Some(e.to_string()),
        },
    }
}

/// Examine one image, turning any failure into a record rather than an error.
#[must_use]
pub fn examine_one(path: &Path) -> Record {
    examine_inner(path, None, false, None)
}

/// Examine one image and name it from a dataset (F-013 and F-014 together).
#[must_use]
pub fn examine_and_identify(path: &Path, catalogue: &Catalogue) -> Record {
    examine_inner(path, Some(catalogue), false, None)
}

/// As [`examine_and_identify`], and hash the image as well.
///
/// The hash is what a catalogue keys on — ManifeST's `image_hash`, and the
/// column it finds duplicates with. Separate from the other two because it
/// costs about twelve seconds over a 4.2 GB corpus, which a health pass should
/// not pay for a field it will not read.
#[must_use]
pub fn examine_hashed(path: &Path, catalogue: Option<&Catalogue>) -> Record {
    examine_inner(path, catalogue, true, None)
}

/// The shared body: the file is read **once** and both the health examination
/// and the content hash work from those bytes. Reading twice doubled the cost
/// of a corpus run for no benefit.
fn examine_inner(
    path: &Path,
    catalogue: Option<&Catalogue>,
    hash: bool,
    convert: Option<&ConvertRequest>,
) -> Record {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return Record {
                path: path.to_path_buf(),
                size: 0,
                container: "unreadable".to_owned(),
                container_code: "unreadable",
                conversion: None,
                sha1: None,
                volume: None,
                files: 0,
                directories: 0,
                bytes_recovered: 0,
                findings: Vec::new(),
                worst: None,
                unreadable: Some(e.to_string()),
                identified: Vec::new(),
            };
        }
    };
    let size = bytes.len() as u64;
    // Both of these read the original bytes, so they happen before `examine`
    // consumes them.
    let identified = catalogue.map_or_else(Vec::new, |c| {
        c.identify(&bytes)
            .into_iter()
            .map(|e| e.name.clone())
            .collect()
    });
    let sha1 = hash.then(|| ade_catalogue::sha1::hex(&ade_catalogue::sha1::sha1(&bytes)));
    // Converting needs the original bytes too, and clones them: the health
    // examination consumes what it is given, and reading the file twice to
    // avoid one copy would cost more than the copy.
    let conversion = convert.map(|request| convert_one(path, bytes.clone(), request));
    let health = examine(bytes);

    Record {
        path: path.to_path_buf(),
        size,
        container: health.inspection.detection.kind.to_string(),
        container_code: health.inspection.detection.kind.code(),
        conversion,
        sha1,
        volume: health.examined.as_ref().map(|e| e.volume.clone()),
        files: health.files,
        directories: health.directories,
        bytes_recovered: health.bytes_recovered,
        findings: health.findings.iter().map(|f| f.code).collect(),
        worst: health.worst(),
        unreadable: None,
        identified,
    }
}

/// Examine every image in `paths`, reporting progress through `progress`.
///
/// Directories are walked one level deep — a corpus is a flat directory of
/// images in every case this has met, and recursing into an arbitrary tree is a
/// different feature with different failure modes.
///
/// The callback receives `(done, total)` after each image so a caller can show
/// progress without this module knowing what a terminal is.
#[must_use]
pub fn run(paths: &[PathBuf], progress: impl FnMut(usize, usize)) -> Summary {
    run_with(paths, None, progress)
}

/// As [`run`], but naming each image from a dataset as it goes (F-013).
#[must_use]
pub fn run_with(
    paths: &[PathBuf],
    catalogue: Option<&Catalogue>,
    progress: impl FnMut(usize, usize),
) -> Summary {
    run_full(paths, catalogue, false, progress)
}

/// As [`run_with`], with control over whether each image is hashed.
///
/// Hashing is what turns a health pass into a catalogue pass (F-013): the
/// SHA-1 is a cataloguer's primary key. It is a parameter rather than always
/// on because it costs about twelve seconds over a 4.2 GB corpus.
#[must_use]
pub fn run_full(
    paths: &[PathBuf],
    catalogue: Option<&Catalogue>,
    hash: bool,
    progress: impl FnMut(usize, usize),
) -> Summary {
    run_converting(paths, catalogue, hash, None, progress)
}

/// As [`run_full`], converting each image as it goes (F-014's bulk clause).
///
/// The conversion happens inside the same pass as the health check, from the
/// bytes already read. Converting a corpus separately would read all 4.2 GB a
/// second time to produce a report ADE has just produced.
#[must_use]
pub fn run_converting(
    paths: &[PathBuf],
    catalogue: Option<&Catalogue>,
    hash: bool,
    convert: Option<&ConvertRequest>,
    mut progress: impl FnMut(usize, usize),
) -> Summary {
    let mut files = collect(paths);
    // Sorted so two runs over one corpus are comparable and a failure is
    // reproducible.
    files.sort();

    let total = files.len();
    let mut summary = Summary::default();

    for (index, path) in files.iter().enumerate() {
        let record = match catalogue {
            _ if convert.is_some() => examine_inner(path, catalogue, hash, convert),
            Some(c) if hash => examine_hashed(path, Some(c)),
            Some(c) => examine_and_identify(path, c),
            None if hash => examine_hashed(path, None),
            None => examine_one(path),
        };

        summary.examined = summary.examined.saturating_add(1);
        if record.unreadable.is_some() {
            summary.unreadable = summary.unreadable.saturating_add(1);
        }
        if record.volume.is_some() {
            summary.mounted = summary.mounted.saturating_add(1);
        }
        if record.is_sound() {
            summary.sound = summary.sound.saturating_add(1);
        }
        summary.bytes_recovered = summary
            .bytes_recovered
            .saturating_add(record.bytes_recovered);
        let container = summary
            .containers
            .entry(record.container.clone())
            .or_insert(0usize);
        *container = container.saturating_add(1);
        // Counted once per image, not once per occurrence. A disk with a
        // damaged file can carry dozens of `data-block-structure` findings;
        // summing them makes 187 affected disks read as 1050, which is the
        // single most misleading thing a corpus report could say.
        let mut seen: BTreeSet<&'static str> = BTreeSet::new();
        for code in &record.findings {
            seen.insert(*code);
        }
        for code in seen {
            let count = summary.findings.entry(code).or_insert(0usize);
            *count = count.saturating_add(1);
        }
        if !record.identified.is_empty() {
            summary.identified = summary.identified.saturating_add(1);
        }
        if let Some(outcome) = &record.conversion {
            let count = summary.conversions.entry(outcome.code).or_insert(0usize);
            *count = count.saturating_add(1);
        }
        summary.records.push(record);

        progress(index.saturating_add(1), total);
    }

    summary
}

/// Expand the given paths into a list of files.
fn collect(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            let Ok(entries) = std::fs::read_dir(path) else {
                continue;
            };
            for entry in entries.flatten() {
                let candidate = entry.path();
                if candidate.is_file() {
                    out.push(candidate);
                }
            }
        } else if path.is_file() {
            out.push(path.clone());
        }
    }
    out
}
