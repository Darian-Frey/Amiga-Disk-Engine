//! A minimal JSON writer for the scriptable surface (F-015).
//!
//! Hand-rolled rather than pulled from a crate, so the workspace keeps its zero
//! dependencies. The scope is narrow — ADE emits JSON and never parses it — so
//! this is a writer only, and the whole escaping surface is the string rule.
//!
//! # Output is pure ASCII
//!
//! Every byte above 0x7E is written as a `\uXXXX` escape. Amiga names are
//! ISO 8859-1, so a byte maps to exactly one code point and the escape is
//! lossless and reversible. That matters more than it looks: a name is a
//! sequence of bytes on the disk, not text, and a consumer that wants the
//! original bytes back must be able to get them.

use core::fmt::Write as _;

/// The version of the JSON surface, carried by every document ADE emits.
///
/// # The policy (D-015)
///
/// **Major** changes when something a consumer relies on stops being true: a
/// field is renamed or removed, a type changes, or a value's meaning changes
/// under an unchanged name. That last one is the dangerous case — a rename
/// breaks a consumer loudly, a redefinition breaks it silently.
///
/// **Minor** changes when a field is added. Additions are safe for a consumer
/// that ignores what it does not recognise, which every JSON reader does by
/// default, so they do not warrant a major. The minor is still worth carrying:
/// it lets a caller say "I need at least 1.2" instead of testing for a field
/// and guessing why it is missing.
///
/// # Why this is checkable rather than promised
///
/// A version nobody bumps is worse than no version, because it asserts
/// stability that is not being maintained. So the field names are inventoried
/// in `src/api/tests/schema.rs`: any change to what ADE emits fails that test,
/// and the fix is to edit the inventory *and* move this constant — in the same
/// commit, where a reviewer can see both. The inventory is the mechanism; this
/// string is what it protects.
pub const SCHEMA: &str = "1.9";

/// The name of the version field, so nothing spells it two ways.
pub const SCHEMA_FIELD: &str = "schema";

/// A JSON value, built before it is written so the structure cannot be
/// malformed by a stray `push_str`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number. JSON has one numeric type; ADE only emits integers.
    Num(u64),
    /// A string, escaped on write.
    Str(String),
    /// An array.
    Arr(Vec<Value>),
    /// An object. A `Vec` rather than a map, because field order is part of
    /// what makes the output readable and diffable.
    Obj(Vec<(&'static str, Value)>),
}

impl Value {
    /// A string value.
    #[must_use]
    pub fn str(s: impl Into<String>) -> Self {
        Self::Str(s.into())
    }

    /// A string value from raw Latin-1 bytes, as Amiga names are stored.
    #[must_use]
    pub fn latin1(bytes: &[u8]) -> Self {
        Self::Str(bytes.iter().map(|&b| char::from(b)).collect())
    }

    /// `Null` when `None`, otherwise the mapped value.
    #[must_use]
    pub fn opt<T>(value: Option<T>, f: impl FnOnce(T) -> Self) -> Self {
        value.map_or(Self::Null, f)
    }

    /// This value as a top-level document: the schema version, then it.
    ///
    /// # Why the version is added here and not by each builder
    ///
    /// Because a document is not a property of a value — it is a property of
    /// *being written to stdout*. `Inspection::to_json` is a whole document
    /// under `ade info` and a nested field under `ade check`, and a version
    /// stamped inside it would appear twice in the second case, on an object
    /// that is not a document.
    ///
    /// So every builder returns a plain object and the single emission point
    /// in the CLI calls this. That is the shape BUG-008 argued for: a rule one
    /// function enforces rather than one every new command must remember.
    ///
    /// **Every document gets it, including each line of a JSON Lines stream.**
    /// Versioning only the summary of such a stream fails in exactly the case
    /// the stream exists for — a consumer reading record by record, or picking
    /// up a run that was interrupted, has no summary to consult. Twelve bytes
    /// a line is the price.
    ///
    /// The version goes first so it can be read without parsing the rest.
    #[must_use]
    pub fn versioned(self) -> Self {
        let mut out = vec![(SCHEMA_FIELD, Self::str(SCHEMA))];
        match self {
            Self::Obj(fields) => out.extend(fields),
            // Nothing emits a bare array or scalar as a document today. If
            // something does, it is still versioned rather than quietly not.
            other => out.push(("value", other)),
        }
        Self::Obj(out)
    }

    /// Render to a compact string.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Num(n) => {
                let _ = write!(out, "{n}");
            }
            Self::Str(s) => write_string(out, s),
            Self::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Self::Obj(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(out, k);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// Write a JSON string literal, escaping everything that needs it.
fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            // Control characters and everything non-ASCII: escaped, so the
            // output is pure ASCII and safe through any pipe or terminal.
            c if (c as u32) < 0x20 || (c as u32) > 0x7E => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_scalars() {
        assert_eq!(Value::Null.to_json(), "null");
        assert_eq!(Value::Bool(true).to_json(), "true");
        assert_eq!(Value::Num(42).to_json(), "42");
        assert_eq!(Value::str("hi").to_json(), "\"hi\"");
    }

    #[test]
    fn escapes_the_characters_json_requires() {
        assert_eq!(Value::str("a\"b").to_json(), "\"a\\\"b\"");
        assert_eq!(Value::str("a\\b").to_json(), "\"a\\\\b\"");
        assert_eq!(Value::str("a\nb").to_json(), "\"a\\nb\"");
        assert_eq!(Value::str("a\tb").to_json(), "\"a\\tb\"");
        assert_eq!(Value::str("a\rb").to_json(), "\"a\\rb\"");
        // A raw NUL or other control byte must never reach the output.
        assert_eq!(Value::str("\u{0}").to_json(), "\"\\u0000\"");
        assert_eq!(Value::str("\u{1f}").to_json(), "\"\\u001f\"");
        assert_eq!(Value::str("\u{08}").to_json(), "\"\\b\"");
    }

    #[test]
    fn latin1_names_round_trip_losslessly() {
        // 0xE9 is the ISO 8859-1 'e-acute'.
        assert_eq!(
            Value::latin1(&[b'C', b'a', b'f', 0xE9]).to_json(),
            "\"Caf\\u00e9\""
        );

        // Every byte value must survive, including those that are not valid
        // UTF-8 alone — a filename is bytes, not text.
        for b in 0u8..=255 {
            let json = Value::latin1(&[b]).to_json();
            assert!(json.is_ascii(), "byte {b:#04x} produced non-ASCII output");
            assert!(json.starts_with('"') && json.ends_with('"'));
        }
    }

    #[test]
    fn output_is_always_ascii() {
        let v = Value::Arr(vec![
            Value::latin1(&[0xFF, 0xFE, 0x80]),
            Value::str("plain"),
        ]);
        assert!(v.to_json().is_ascii());
    }

    #[test]
    fn nests_without_malforming() {
        let v = Value::Obj(vec![
            ("name", Value::str("readme")),
            ("size", Value::Num(11)),
            ("tags", Value::Arr(vec![Value::str("a"), Value::str("b")])),
            ("missing", Value::Null),
            ("inner", Value::Obj(vec![("deep", Value::Bool(false))])),
        ]);
        assert_eq!(
            v.to_json(),
            "{\"name\":\"readme\",\"size\":11,\"tags\":[\"a\",\"b\"],\"missing\":null,\"inner\":{\"deep\":false}}"
        );
    }

    #[test]
    fn empty_containers_are_valid() {
        assert_eq!(Value::Arr(vec![]).to_json(), "[]");
        assert_eq!(Value::Obj(vec![]).to_json(), "{}");
        assert_eq!(Value::str("").to_json(), "\"\"");
    }

    #[test]
    fn opt_maps_none_to_null() {
        assert_eq!(Value::opt(None::<u64>, Value::Num).to_json(), "null");
        assert_eq!(Value::opt(Some(7u64), Value::Num).to_json(), "7");
    }

    #[test]
    fn a_key_needing_escapes_is_still_escaped() {
        assert_eq!(
            Value::Obj(vec![("a\"b", Value::Null)]).to_json(),
            "{\"a\\\"b\":null}"
        );
    }
}
