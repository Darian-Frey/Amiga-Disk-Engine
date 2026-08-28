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

use crate::{Severity, examine, json::Value};

/// What happened to one image.
#[derive(Debug, Clone)]
pub struct Record {
    /// Where it came from.
    pub path: PathBuf,
    /// Bytes on disk.
    pub size: u64,
    /// The container, as identified.
    pub container: String,
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

/// Examine one image, turning any failure into a record rather than an error.
#[must_use]
pub fn examine_one(path: &Path) -> Record {
    examine_inner(path, None)
}

/// Examine one image and name it from a dataset (F-013 and F-014 together).
#[must_use]
pub fn examine_and_identify(path: &Path, catalogue: &Catalogue) -> Record {
    examine_inner(path, Some(catalogue))
}

/// The shared body: the file is read **once** and both the health examination
/// and the content hash work from those bytes. Reading twice doubled the cost
/// of a corpus run for no benefit.
fn examine_inner(path: &Path, catalogue: Option<&Catalogue>) -> Record {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return Record {
                path: path.to_path_buf(),
                size: 0,
                container: "unreadable".to_owned(),
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
    // Hash before `examine` consumes the bytes.
    let identified = catalogue.map_or_else(Vec::new, |c| {
        c.identify(&bytes)
            .into_iter()
            .map(|e| e.name.clone())
            .collect()
    });
    let health = examine(bytes);

    Record {
        path: path.to_path_buf(),
        size,
        container: health.inspection.detection.kind.to_string(),
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
            Some(c) => examine_and_identify(path, c),
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
