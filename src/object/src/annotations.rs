//! What a person has written about a program: names for its places and
//! comments on its lines (D-022).
//!
//! The file is text, one program's notes, meant to be read and shared as it
//! is:
//!
//! ```text
//! # ADE project 1
//! program 2fd4e1c67a2d28fced849ee1bb76e7391b93eb12 24160
//! name h0_008c CopperSetup
//! comment h0_0094 waits for the blitter before touching the list
//! ```
//!
//! Places are written as the export labels them. A name must be a label an
//! assembler takes, and not one ADE makes itself; a comment is one line of
//! printable text. What breaks a rule is refused when it is given, and when
//! a file is read the line is reported and skipped — never guessed at.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::flow::Place;

/// The longest name.
pub const MAX_NAME: usize = 64;
/// The longest comment.
pub const MAX_COMMENT: usize = 200;
/// The largest project file read.
pub const MAX_FILE: usize = 1 << 20;
/// The longest line read.
pub const MAX_LINE: usize = 4096;

const HEADER: &str = "# ADE project 1";

/// A program's names and comments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Annotations {
    /// Names, by place.
    pub names: BTreeMap<Place, String>,
    /// Comments, by place.
    pub comments: BTreeMap<Place, String>,
}

/// The 68000's registers and the names an assembler gives them, which a
/// label may not take.
const REGISTERS: [&str; 22] = [
    "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7",
    "sp", "pc", "sr", "ccr", "usp", "ssp",
];

/// A place written as the export labels it: `h1_0042`.
#[must_use]
pub fn place_text((k, at): Place) -> String {
    crate::export::label(k, at)
}

/// A place read back from its label: `h1_0042`.
#[must_use]
pub fn parse_place(text: &str) -> Option<Place> {
    let rest = text.strip_prefix('h')?;
    let (k, at) = rest.split_once('_')?;
    if k.is_empty() || at.is_empty() || !k.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((k.parse().ok()?, u32::from_str_radix(at, 16).ok()?))
}

/// Whether `name` looks like a label ADE makes itself.
fn ade_made(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    parse_place(&lower).is_some()
        || lower
            .strip_prefix("lab_")
            .is_some_and(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit()))
        || name.starts_with("_LVO")
}

/// Why `name` cannot name a place, or `None` when it can.
#[must_use]
pub fn name_problem(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Some("a name cannot be empty".to_owned());
    };
    if name.chars().count() > MAX_NAME {
        return Some(format!("a name is at most {MAX_NAME} characters"));
    }
    if !(first.is_ascii_alphabetic() || first == '_' || first == '.') {
        return Some("a name starts with a letter, `_` or `.`".to_owned());
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
        return Some("a name holds only letters, digits, `_` and `.`".to_owned());
    }
    if REGISTERS.contains(&name.to_ascii_lowercase().as_str()) {
        return Some(format!("`{name}` is a register"));
    }
    if ade_made(name) {
        return Some(format!("`{name}` is the form of a label ADE makes itself"));
    }
    None
}

/// Why `comment` cannot be a comment, or `None` when it can.
#[must_use]
pub fn comment_problem(comment: &str) -> Option<String> {
    if comment.trim().is_empty() {
        return Some("a comment cannot be empty".to_owned());
    }
    if comment.chars().count() > MAX_COMMENT {
        return Some(format!("a comment is at most {MAX_COMMENT} characters"));
    }
    if comment.chars().any(char::is_control) {
        return Some("a comment is one line, with no control characters".to_owned());
    }
    None
}

impl Annotations {
    /// Whether there is nothing in it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.comments.is_empty()
    }

    /// The name for a place: the user's, or the export's label.
    #[must_use]
    pub fn label(&self, place: Place) -> String {
        self.names
            .get(&place)
            .cloned()
            .unwrap_or_else(|| place_text(place))
    }

    /// Name `place`, or clear its name with `None`.
    ///
    /// # Errors
    /// Why the name cannot be: a rule it breaks, or another place has it.
    pub fn set_name(&mut self, place: Place, name: Option<&str>) -> Result<(), String> {
        let Some(name) = name else {
            self.names.remove(&place);
            return Ok(());
        };
        if let Some(why) = name_problem(name) {
            return Err(why);
        }
        if let Some((other, _)) = self
            .names
            .iter()
            .find(|(p, n)| **p != place && n.eq_ignore_ascii_case(name))
        {
            return Err(format!("`{name}` already names {}", place_text(*other)));
        }
        self.names.insert(place, name.to_owned());
        Ok(())
    }

    /// Comment on `place`, or clear its comment with `None`.
    ///
    /// # Errors
    /// Why the comment cannot be.
    pub fn set_comment(&mut self, place: Place, comment: Option<&str>) -> Result<(), String> {
        let Some(comment) = comment else {
            self.comments.remove(&place);
            return Ok(());
        };
        if let Some(why) = comment_problem(comment) {
            return Err(why);
        }
        self.comments.insert(place, comment.trim().to_owned());
        Ok(())
    }

    /// The file for a program, which `program` identifies as its SHA-1 in
    /// hex and its length.
    #[must_use]
    pub fn to_text(&self, program: (&str, usize)) -> String {
        let mut out = format!("{HEADER}\nprogram {} {}\n", program.0, program.1);
        for (p, n) in &self.names {
            let _ = writeln!(out, "name {} {n}", place_text(*p));
        }
        for (p, c) in &self.comments {
            let _ = writeln!(out, "comment {} {c}", place_text(*p));
        }
        out
    }

    /// Read a file: what it holds, and each line it could not take, as
    /// `line N: why`.
    #[must_use]
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut out = Self::default();
        let mut problems = Vec::new();
        if text.len() > MAX_FILE {
            problems.push(format!(
                "the file is over {MAX_FILE} bytes, and was not read"
            ));
            return (out, problems);
        }
        for (n, line) in text.lines().enumerate() {
            let number = n.saturating_add(1);
            let line = line.trim_end();
            if line.len() > MAX_LINE {
                problems.push(format!("line {number}: over {MAX_LINE} bytes"));
                continue;
            }
            if line.is_empty() || line.starts_with('#') || line.starts_with("program ") {
                continue;
            }
            let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
            let (place, value) = rest.split_once(' ').unwrap_or((rest, ""));
            let Some(place) = parse_place(place) else {
                problems.push(format!(
                    "line {number}: `{place}` is not a place like h0_0042"
                ));
                continue;
            };
            let result = match word {
                "name" => out.set_name(place, Some(value)),
                "comment" => out.set_comment(place, Some(value)),
                other => Err(format!("`{other}` is not `name` or `comment`")),
            };
            if let Err(why) = result {
                problems.push(format!("line {number}: {why}"));
            }
        }
        (out, problems)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test scaffolding"
)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_a_label_an_assembler_takes_and_ade_does_not_make() {
        for good in ["CopperSetup", "_main", ".loop", "lives2", "a8", "h0x"] {
            assert_eq!(name_problem(good), None, "{good}");
        }
        for bad in [
            "",
            "2fast",
            "with space",
            "dé",
            "d0",
            "SP",
            "h1_0042",
            "lab_00ff",
            "_LVOOpen",
        ] {
            assert!(name_problem(bad).is_some(), "{bad}");
        }
        assert!(name_problem(&"x".repeat(MAX_NAME + 1)).is_some());
    }

    #[test]
    fn two_places_cannot_share_a_name() {
        let mut a = Annotations::default();
        a.set_name((0, 4), Some("Start")).unwrap();
        assert!(a.set_name((0, 8), Some("start")).is_err());
        // Renaming a place to its own name is no clash.
        a.set_name((0, 4), Some("START")).unwrap();
        a.set_name((0, 4), None).unwrap();
        assert!(a.is_empty());
    }

    #[test]
    fn the_file_reads_back_what_was_written_and_says_what_it_skips() {
        let mut a = Annotations::default();
        a.set_name((1, 0x42), Some("lives")).unwrap();
        a.set_comment((0, 0x94), Some("waits for the blitter"))
            .unwrap();
        let text = a.to_text(("ab12", 100));
        assert!(text.starts_with("# ADE project 1\nprogram ab12 100\n"));
        let (back, problems) = Annotations::parse(&text);
        assert_eq!((back, problems), (a, Vec::new()));

        let (read, problems) = Annotations::parse(
            "name h0_0010 ok\nname h0_0014 d0\nname x0_1 y\nnote h0_0001 z\ncomment h0_0002 \u{7}",
        );
        assert_eq!(read.names.len(), 1);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(problems[0].starts_with("line 2:"));
    }
}
