//! IFF preferences in the Decoded view (F-033), across the corpus.
//!
//! The layouts are AROS's `prefs/*.h`; the unit tests in
//! `ade_object::iffprefs` build files to them. The corpus adds two checks.
//! Every chunk ADE reads is exactly its structure's size, so a layout read
//! wrong in length would show here. And a `.country` file states its
//! country's telephone code and measuring system, which are facts about the
//! country: Commodore's files must give the real ones. Measured 2026-10-29:
//! 501 preference files, every chunk read but Workbench's own `WBCF` and
//! *ToolManager*'s. Local only; skips without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ade_core::layers::object::iffprefs;

/// The real telephone code of each country Commodore wrote a file for.
const TELEPHONE: &[(&str, &str)] = &[
    ("australia", "+61"),
    ("belgie", "+32"),
    ("belgique", "+32"),
    ("canada", "+1"),
    ("canada_francais", "+1"),
    ("canada_français", "+1"),
    ("danmark", "+45"),
    ("deutschland", "+49"),
    ("españa", "+34"),
    ("france", "+33"),
    ("great_britain", "+44"),
    ("italia", "+39"),
    ("nederland", "+31"),
    ("norge", "+47"),
    ("portugal", "+351"),
    ("schweiz", "+41"),
    ("suisse", "+41"),
    ("sverige", "+46"),
    ("svizzera", "+41"),
    ("united_kingdom", "+44"),
    ("united_states", "+1"),
    ("österreich", "+43"),
];

#[test]
fn corpus_preferences_read_and_countries_are_the_countries_they_name() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    let (mut files, mut countries) = (0usize, 0usize);
    let mut unread: BTreeMap<String, usize> = BTreeMap::new();
    for path in &paths {
        let Ok(image) = ade_core::Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        for (name, entry) in walk.entries.iter().filter(|(_, e)| e.kind.is_file()) {
            let Ok(c) = volume.read_file(entry) else {
                continue;
            };
            let bytes = c.into_bytes();
            if !iffprefs::is_preferences(&bytes) {
                continue;
            }
            files += 1;
            let prefs = iffprefs::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let fields = prefs.fields();
            for f in fields.iter().filter(|f| f.group == "Not read") {
                *unread.entry(f.label.clone()).or_default() += 1;
            }
            let file = name.rsplit('/').next().unwrap_or(name).to_lowercase();
            let Some(country) = file.strip_suffix(".country") else {
                continue;
            };
            let value = |label: &str| {
                fields
                    .iter()
                    .find(|f| f.group == "Country" && f.label == label)
                    .map(|f| f.value.clone())
                    .unwrap_or_default()
            };
            countries += 1;
            if country == "new_zealand" {
                // Commodore's is a copy of Australia's, code and all.
                assert_eq!(value("Code"), "AUS", "{}", path.display());
                continue;
            }
            let expected = TELEPHONE
                .iter()
                .find(|(c, _)| *c == country)
                .unwrap_or_else(|| {
                    panic!("{}: {name}: a country ADE has no code for", path.display())
                })
                .1;
            assert_eq!(
                value("Telephone code"),
                expected,
                "{}: {name}",
                path.display()
            );
            let measuring = match country {
                "great_britain" | "united_kingdom" => "British",
                "united_states" => "American",
                _ => "metric (ISO)",
            };
            assert_eq!(value("Measuring"), measuring, "{}: {name}", path.display());
        }
    }
    eprintln!("{files} preference files, {countries} countries; not read: {unread:?}");
    assert!(files >= 501, "{files}");
    assert!(countries >= 403, "{countries}");
    // Only what no independent source describes goes unread.
    for id in unread.keys() {
        assert!(id == "WBCF" || id.starts_with("TM"), "{id} was not read");
    }
}
