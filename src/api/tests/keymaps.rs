//! Keymaps in the Decoded view (F-033), across the corpus.
//!
//! The layout is AROS's; the unit tests in `ade_object::keymap` build files
//! to it. What the corpus adds is the keyboards themselves: a British keymap
//! types `£` on Shift+3 and an American one `#`, and a German one swaps Y and
//! Z and puts `ü` beside P. Those are facts about the keyboards, not about
//! the format, so a reading that gets them right has the format right too.
//! They are checked on Commodore's own keymaps, the ones on Workbench disks:
//! a game may name a keymap anything, and *Metal Masters* ships an American
//! layout called `d`. Local only; skips without the corpus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::panic,
    reason = "test scaffolding"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ade_core::layers::object::keymap::{self, Keymap, Output, Qualifier};

/// What `code` types with `q` held, as one character.
fn char_of(k: &Keymap, code: u8, q: Qualifier) -> Option<u8> {
    k.keys
        .iter()
        .find(|key| key.code == code)?
        .outputs
        .iter()
        .find_map(|(kq, o)| match o {
            Output::Char(c) | Output::Accentable(c, _) if *kq == q => Some(*c),
            _ => None,
        })
}

#[test]
fn corpus_keymaps_decode_and_type_what_their_keyboards_do() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../disks");
    let Ok(read) = std::fs::read_dir(&corpus) else {
        eprintln!("skipping: no corpus");
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();

    let (mut found, mut parsed, mut dead_without_effect, mut checked) =
        (0usize, 0usize, 0usize, 0usize);
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    for path in &paths {
        let Ok(image) = ade_core::Image::open(path) else {
            continue;
        };
        let Ok(volume) = image.volume() else { continue };
        let Ok(walk) = volume.walk(volume.root()) else {
            continue;
        };
        for (_, entry) in walk.entries.iter().filter(|(_, e)| e.kind.is_file()) {
            let Ok(c) = volume.read_file(entry) else {
                continue;
            };
            let bytes = c.into_bytes();
            if !keymap::is_keymap(&bytes) {
                continue;
            }
            found += 1;
            let k = keymap::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            parsed += 1;
            assert!(!k.keys.is_empty(), "{}", path.display());
            let _ = k.fields();
            *by_name.entry(k.name.clone()).or_default() += 1;

            let shift3 = char_of(&k, 0x03, Qualifier::Shift);
            let commodore = path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("Workbench"));
            match if commodore { k.name.as_str() } else { "" } {
                "gb" => {
                    checked += 1;
                    assert_eq!(shift3, Some(0xA3), "{}: £", path.display());
                }
                "usa0" | "usa1" | "usa2" => {
                    checked += 1;
                    assert_eq!(shift3, Some(b'#'), "{}: #", path.display());
                }
                "d" => {
                    checked += 1;
                    assert_eq!(char_of(&k, 0x15, Qualifier::None), Some(b'z'));
                    assert_eq!(char_of(&k, 0x31, Qualifier::None), Some(b'y'));
                    assert_eq!(char_of(&k, 0x1A, Qualifier::None), Some(0xFC));
                }
                _ => {}
            }

            // Every dead key changes some key: a reading that lost the
            // accent tables would leave them changing nothing.
            let deads: Vec<u8> = k
                .keys
                .iter()
                .flat_map(|key| &key.outputs)
                .filter_map(|(_, o)| match o {
                    Output::Dead(i) => Some(*i),
                    _ => None,
                })
                .collect();
            for i in deads {
                let changes = k.keys.iter().flat_map(|key| &key.outputs).any(|(_, o)| {
                    matches!(o, Output::Accentable(own, after)
                        if after.get(usize::from(i).saturating_sub(1)).is_some_and(|c| c != own))
                });
                if !changes {
                    dead_without_effect += 1;
                }
            }
        }
    }
    eprintln!(
        "{found} keymaps, {parsed} parsed, {checked} checked against their keyboard; {by_name:?}"
    );
    // Measured 2026-10-29: 1,391 keymaps on 296 disks, every one read; the
    // 195 on Workbench disks type what their keyboards do.
    assert_eq!(parsed, found);
    assert!(found >= 1391, "{found}");
    assert!(by_name.get("gb").copied().unwrap_or(0) >= 100);
    assert!(
        checked >= 195,
        "{checked} Workbench keymaps checked against their keyboards"
    );
    assert_eq!(dead_without_effect, 0);
}
