//! Stub board for rendering a drawing sheet with `kicad-cli`.
//!
//! A board file with no items, carrying only the paper size and title
//! block, exported to SVG with the drawing sheet included, lets KiCad
//! itself draw the frame and title block. How that SVG maps onto the page
//! is measured (spike S1), not assumed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kdraft_sexpr::Value;

use crate::error::SourceError;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SheetStub {
    /// `A4`, `A3`, ... or `User`.
    pub paper: String,
    /// Paper size in mm; written for `User` paper only.
    pub width: f64,
    pub height: f64,
    pub portrait: bool,
    pub title: Option<String>,
    pub date: Option<String>,
    pub rev: Option<String>,
    pub company: Option<String>,
    /// Comment number (1..9) to text.
    pub comments: BTreeMap<u32, String>,
    /// Project text variables, written to the stub's `.kicad_pro`.
    pub text_variables: BTreeMap<String, String>,
}

/// File format version written into the stub (a KiCad 9 board format, which
/// KiCad 10 also reads).
pub const STUB_VERSION: i64 = 20241229;

fn num(v: f64) -> Value {
    Value::num(v).unwrap_or_else(|_| Value::int(0))
}

/// Text of the stub `.kicad_pcb`.
pub fn stub_board_text(s: &SheetStub) -> String {
    let mut paper = vec![Value::str(&s.paper)];
    if s.paper == "User" {
        paper.push(num(s.width));
        paper.push(num(s.height));
    }
    if s.portrait {
        paper.push(Value::sym("portrait"));
    }
    let mut tb = Vec::new();
    for (k, v) in [
        ("title", &s.title),
        ("date", &s.date),
        ("rev", &s.rev),
        ("company", &s.company),
    ] {
        if let Some(v) = v {
            tb.push(Value::node(k, [Value::str(v)]));
        }
    }
    for (n, c) in &s.comments {
        tb.push(Value::node(
            "comment",
            [Value::int(i64::from(*n)), Value::str(c)],
        ));
    }
    let layer = |n: i64, name: &str, kind: &str| {
        Value::list([Value::int(n), Value::str(name), Value::sym(kind)])
    };
    let root = Value::node(
        "kicad_pcb",
        [
            Value::node("version", [Value::int(STUB_VERSION)]),
            Value::node("generator", [Value::str("kdraft")]),
            Value::node("generator_version", [Value::str("9.0")]),
            Value::node("general", [Value::node("thickness", [num(1.6)])]),
            Value::node("paper", paper),
            Value::node("title_block", tb),
            Value::node(
                "layers",
                [
                    layer(0, "F.Cu", "signal"),
                    layer(2, "B.Cu", "signal"),
                    layer(25, "Edge.Cuts", "user"),
                ],
            ),
            Value::node(
                "setup",
                [Value::node("pad_to_mask_clearance", [Value::int(0)])],
            ),
            Value::node("net", [Value::int(0), Value::str("")]),
        ],
    );
    root.to_canonical()
}

/// Text of the stub `.kicad_pro` (text variables only).
pub fn stub_project_text(s: &SheetStub) -> String {
    let vars: serde_json::Map<String, serde_json::Value> = s
        .text_variables
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    let v = serde_json::json!({
        "meta": { "filename": "kdraft-sheet-stub.kicad_pro", "version": 1 },
        "text_variables": vars,
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// Writes `<dir>/<stem>.kicad_pcb` and `<dir>/<stem>.kicad_pro`; returns
/// the board path.
pub fn write_stub(dir: &Path, stem: &str, s: &SheetStub) -> Result<PathBuf, SourceError> {
    std::fs::create_dir_all(dir).map_err(|e| SourceError::io("creating", dir, e))?;
    let pcb = dir.join(format!("{stem}.kicad_pcb"));
    let pro = dir.join(format!("{stem}.kicad_pro"));
    std::fs::write(&pcb, stub_board_text(s)).map_err(|e| SourceError::io("writing", &pcb, e))?;
    std::fs::write(&pro, stub_project_text(s)).map_err(|e| SourceError::io("writing", &pro, e))?;
    Ok(pcb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_parses_as_a_board_with_the_title_block() {
        let mut s = SheetStub {
            paper: "User".into(),
            width: 500.0,
            height: 250.5,
            title: Some("A \"quoted\" title".into()),
            rev: Some("B".into()),
            ..Default::default()
        };
        s.comments.insert(2, "second".into());
        s.text_variables.insert("PART".into(), "X-1".into());
        let text = stub_board_text(&s);
        let b = kdraft_board::Board::parse_str(&text).unwrap();
        assert_eq!(b.version, STUB_VERSION);
        assert_eq!(b.title_block.title.as_deref(), Some("A \"quoted\" title"));
        assert_eq!(
            b.title_block.comments.get(&2).map(String::as_str),
            Some("second")
        );
        assert!(b.footprints.is_empty() && b.outline.primitives.is_empty());
        assert!(text.contains("(paper \"User\" 500 250.5)"), "{text}");
        let pro: serde_json::Value = serde_json::from_str(&stub_project_text(&s)).unwrap();
        assert_eq!(pro["text_variables"]["PART"], "X-1");
    }
}
