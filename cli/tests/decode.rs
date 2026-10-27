//! `ade decode` on what the Decoded view reads (F-033): the engine's tests
//! check the decoding; these check that the command reaches it by path and
//! prints it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    reason = "integration tests over files they build by hand, so the files cannot agree with \
              the reader about a field both have wrong (C-001's exception)"
)]

use std::{fs, path::PathBuf, process::Command};

fn ade() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("ade")
}

fn run(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(ade()).args(args).output().expect("ade runs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// A one-plane 16 × 1 ILBM: pen 1 in the left pixel.
fn picture() -> Vec<u8> {
    let chunk = |id: &[u8], data: &[u8]| {
        let mut c = id.to_vec();
        c.extend((data.len() as u32).to_be_bytes());
        c.extend(data);
        c
    };
    let mut bmhd = vec![0u8; 20];
    bmhd[1] = 16;
    bmhd[3] = 1;
    bmhd[8] = 1;
    let mut inner = b"ILBM".to_vec();
    inner.extend(chunk(b"BMHD", &bmhd));
    inner.extend(chunk(b"CMAP", &[0, 0, 0, 0xFF, 0xFF, 0xFF]));
    inner.extend(chunk(b"BODY", &[0x80, 0]));
    let mut out = b"FORM".to_vec();
    out.extend((inner.len() as u32).to_be_bytes());
    out.extend(inner);
    out
}

/// A tool icon with no image and one tool type.
fn icon() -> Vec<u8> {
    let mut b = vec![0u8; 78];
    b[0..2].copy_from_slice(&[0xE3, 0x10]);
    b[3] = 1;
    b[48] = 3;
    b[57] = 1;
    b[58..62].copy_from_slice(&[0x80, 0, 0, 0]);
    b.extend(8u32.to_be_bytes());
    b.extend(10u32.to_be_bytes());
    b.extend(b"DONOTWAIT\0");
    b
}

#[test]
fn icons_and_pictures_decode_by_path() {
    let dir = std::env::temp_dir().join(format!("ade-decode-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let blank = dir.join("blank.adf");
    let one = dir.join("one.adf");
    let two = dir.join("two.adf");
    fs::write(dir.join("Tool.info"), icon()).unwrap();
    fs::write(dir.join("pic"), picture()).unwrap();
    let s = |p: &PathBuf| p.to_str().unwrap().to_owned();
    assert_eq!(run(&["create", &s(&blank)]).2, Some(0));
    let (_, err, code) = run(&[
        "put",
        &s(&blank),
        &s(&dir.join("Tool.info")),
        &format!("--output={}", s(&one)),
    ]);
    assert_eq!(code, Some(0), "{err}");
    let (_, err, code) = run(&[
        "put",
        &s(&one),
        &s(&dir.join("pic")),
        &format!("--output={}", s(&two)),
    ]);
    assert_eq!(code, Some(0), "{err}");

    let (text, err, code) = run(&["decode", &s(&two), "Tool.info"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.starts_with("Workbench icon: Tool"), "{text}");
    assert!(text.contains("DONOTWAIT"), "{text}");
    assert!(text.contains("not set; Workbench places it"), "{text}");

    let (text, err, code) = run(&["decode", &s(&two), "pic"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.starts_with("IFF ILBM picture"), "{text}");
    assert!(text.contains("16 × 1"), "{text}");
    // Opaque: the left pixel the first colour met, the rest the second.
    assert!(text.contains("  1222222222222222"), "{text}");
    let _ = fs::remove_dir_all(&dir);
}

/// An 8SVX of four samples, all silence but one.
fn sound() -> Vec<u8> {
    let mut vhdr = vec![0u8; 20];
    vhdr[3] = 4;
    vhdr[12..14].copy_from_slice(&8000u16.to_be_bytes());
    vhdr[14] = 1;
    vhdr[16..20].copy_from_slice(&0x10000u32.to_be_bytes());
    let mut inner = b"8SVX".to_vec();
    for (id, data) in [
        (b"VHDR", vhdr),
        (b"NAME", b"blip".to_vec()),
        (b"BODY", vec![0, 100, 0, 0]),
    ] {
        inner.extend(id);
        inner.extend((data.len() as u32).to_be_bytes());
        inner.extend(data);
    }
    let mut out = b"FORM".to_vec();
    out.extend((inner.len() as u32).to_be_bytes());
    out.extend(inner);
    out
}

#[test]
fn a_sound_decodes_by_path_with_its_waveform() {
    let dir = std::env::temp_dir().join(format!("ade-decode-svx-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let (blank, one) = (dir.join("blank.adf"), dir.join("one.adf"));
    fs::write(dir.join("blip"), sound()).unwrap();
    let s = |p: &PathBuf| p.to_str().unwrap().to_owned();
    assert_eq!(run(&["create", &s(&blank)]).2, Some(0));
    let (_, err, code) = run(&[
        "put",
        &s(&blank),
        &s(&dir.join("blip")),
        &format!("--output={}", s(&one)),
    ]);
    assert_eq!(code, Some(0), "{err}");
    let (text, err, code) = run(&["decode", &s(&one), "blip"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(text.starts_with("IFF 8SVX sound"), "{text}");
    assert!(
        text.contains("blip") && text.contains("8000 samples a second"),
        "{text}"
    );
    assert!(text.contains("The waveform (128 × 32"), "{text}");
    let _ = fs::remove_dir_all(&dir);
}
