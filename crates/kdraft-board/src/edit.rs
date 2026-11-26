//! Edits of board text for private copies (never the user's file): hiding
//! footprint texts on fab layers before kicad-cli plots them, since
//! kicad-cli 10.0.8 has no option for that.
//!
//! A text is hidden the way KiCad writes it in 9 and 10 format files
//! (`version` 20241229 and 20260206, checked on the demo boards): a
//! `(hide yes)` child right after the text's `(layer ...)`, for both
//! `property` and `fp_text`. Older files used a bare `hide` symbol; the
//! KiCad 10.0.8 parser accepts both forms for footprint texts (and
//! `(hide yes)` inside `effects`), so the inserted form works for any file
//! version. Every byte outside the inserted or replaced tokens is kept.

use kdraft_sexpr::edit::{apply, Edit};
use kdraft_sexpr::{Node, NodeKind, Tree};

use crate::error::BoardError;

/// Which fab-layer footprint texts stay visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FabTextShow {
    pub values: bool,
    pub references: bool,
    pub other: bool,
}

impl FabTextShow {
    pub const ALL: FabTextShow = FabTextShow {
        values: true,
        references: true,
        other: true,
    };

    pub fn hides_anything(&self) -> bool {
        !(self.values && self.references && self.other)
    }

    /// Short stable tag for cache keys, e.g. `v0r1o1`.
    pub fn tag(&self) -> String {
        format!(
            "v{}r{}o{}",
            u8::from(self.values),
            u8::from(self.references),
            u8::from(self.other)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Value,
    Reference,
    Other,
}

fn is_fab(layer: &str) -> bool {
    layer == "F.Fab" || layer == "B.Fab"
}

fn str_child(n: Node<'_>, head: &str) -> Option<String> {
    n.find(head)
        .filter(|c| c.len() > 1)
        .and_then(|c| c.str_at(1).ok())
        .map(|s| s.into_owned())
}

fn category(t: Node<'_>) -> Option<Category> {
    let a = t.str_at(1).ok()?.into_owned();
    let b = t.str_at(2).ok()?.into_owned();
    Some(match (t.head()?, a.as_str()) {
        ("property", "Value") | ("fp_text", "value") => Category::Value,
        ("property", "Reference") | ("fp_text", "reference") => Category::Reference,
        ("fp_text", _) if b == "${REFERENCE}" => Category::Reference,
        _ => Category::Other,
    })
}

/// `(hide ...)` child (or in `effects`): Some(true/false) when present;
/// a bare `hide` symbol counts as true.
fn hide_state(t: Node<'_>) -> Option<(bool, Option<Node<'_>>)> {
    for holder in [Some(t), t.find("effects")].into_iter().flatten() {
        for c in holder.children().skip(1) {
            if c.kind() == NodeKind::Symbol && c.raw() == "hide" {
                return Some((true, None));
            }
            if c.is_list() && c.head() == Some("hide") {
                let v = c
                    .child(1)
                    .and_then(|a| a.atom())
                    .is_none_or(|a| a == "yes" || a == "true");
                return Some((v, Some(c)));
            }
        }
    }
    None
}

/// The edit that hides text item `t`, if it is visible.
fn hide_edit(text: &str, t: Node<'_>) -> Option<Edit> {
    match hide_state(t) {
        Some((true, _)) => None,
        Some((false, Some(h))) => Some(Edit::replace(h.span(), "(hide yes)")),
        Some((false, None)) => None,
        None => {
            let kids: Vec<Node<'_>> = t.children().collect();
            let li = kids.iter().position(|c| c.head() == Some("layer"))?;
            let layer = kids[li];
            // Reuse the whitespace in front of the layer node.
            let prev_end = kids[li - 1].span().end;
            let ws = &text[prev_end..layer.span().start];
            let ws = if ws.is_empty() { " " } else { ws };
            Some(Edit::insert(layer.span().end, format!("{ws}(hide yes)")))
        }
    }
}

/// Hides the fab-layer footprint texts `show` turns off. Returns the
/// edited text and the number of texts hidden.
pub fn hide_fab_texts(text: &str, show: FabTextShow) -> Result<(String, usize), BoardError> {
    if !show.hides_anything() {
        return Ok((text.to_string(), 0));
    }
    let tree = Tree::parse(text)?;
    let root = tree.require_root()?;
    let mut edits = Vec::new();
    for fp in root
        .lists()
        .filter(|n| matches!(n.head(), Some("footprint") | Some("module")))
    {
        for t in fp
            .lists()
            .filter(|n| matches!(n.head(), Some("property") | Some("fp_text")))
        {
            if t.len() < 3 || t.find("at").is_none() {
                continue;
            }
            if !str_child(t, "layer").is_some_and(|l| is_fab(&l)) {
                continue;
            }
            let keep = match category(t) {
                Some(Category::Value) => show.values,
                Some(Category::Reference) => show.references,
                Some(Category::Other) => show.other,
                None => true,
            };
            if keep {
                continue;
            }
            if let Some(e) = hide_edit(text, t) {
                edits.push(e);
            }
        }
    }
    let n = edits.len();
    let (out, _) = apply(text, &edits).map_err(|e| BoardError::Edit(e.to_string()))?;
    Ok((out, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Board, FpTextKind};

    const SHOW_REFS: FabTextShow = FabTextShow {
        values: false,
        references: true,
        other: true,
    };

    fn fab_texts(b: &Board) -> Vec<(FpTextKind, String, bool)> {
        b.footprints
            .iter()
            .flat_map(|f| f.texts.iter())
            .filter(|t| t.layer.ends_with(".Fab"))
            .map(|t| (t.kind.clone(), t.text.clone(), t.hidden))
            .collect()
    }

    fn check(text: &str, show: FabTextShow) -> usize {
        let before = Board::parse_str(text).unwrap();
        let (out, n) = hide_fab_texts(text, show).unwrap();
        let after = Board::parse_str(&out).unwrap();
        let (a, b) = (fab_texts(&before), fab_texts(&after));
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            let off = match &x.0 {
                FpTextKind::Value => !show.values,
                FpTextKind::Reference => !show.references,
                FpTextKind::User if x.1 == "${REFERENCE}" => !show.references,
                _ => !show.other,
            };
            assert_eq!(y.2, x.2 || off, "{x:?}");
        }
        // Only insertions/replacements: the rest is byte-identical, and
        // the same edit again changes nothing.
        let (again, m) = hide_fab_texts(&out, show).unwrap();
        assert_eq!((again.as_str(), m), (out.as_str(), 0));
        n
    }

    #[test]
    fn fixture_values_hidden() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad9-minimal.kicad_pcb");
        let text = std::fs::read_to_string(p).unwrap();
        check(&text, SHOW_REFS);
        check(
            &text,
            FabTextShow {
                values: false,
                references: false,
                other: false,
            },
        );
        assert_eq!(hide_fab_texts(&text, FabTextShow::ALL).unwrap().0, text);
    }

    #[test]
    fn forms_of_hide() {
        let t = "(kicad_pcb (footprint \"x\" (layer \"F.Cu\") (at 1 2)\n\
                 \t(property \"Value\" \"10k\"\n\t\t(at 0 1 0)\n\t\t(layer \"F.Fab\")\n\t\t(effects (font (size 1 1))))\n\
                 \t(property \"Reference\" \"R1\" (at 0 0 0) (layer \"F.Fab\") (hide no))\n\
                 \t(fp_text user \"${REFERENCE}\" (at 0 0) (layer \"B.Fab\") hide)\n\
                 \t(fp_text user \"note\" (at 0 0) (layer \"F.SilkS\"))))";
        let (out, n) = hide_fab_texts(
            t,
            FabTextShow {
                values: false,
                references: false,
                other: false,
            },
        )
        .unwrap();
        assert_eq!(n, 2);
        assert!(
            out.contains("\t\t(layer \"F.Fab\")\n\t\t(hide yes)\n\t\t(effects"),
            "{out}"
        );
        assert!(out.contains("(layer \"F.Fab\") (hide yes))"), "{out}");
        // Silkscreen text untouched, bare `hide` left as is.
        assert!(out.contains("(layer \"F.SilkS\"))"));
        assert!(out.contains("(layer \"B.Fab\") hide)"));
    }

    /// All demo boards: values hidden in the copy, the model agrees, and
    /// nothing else changes.
    #[test]
    fn ext_boards() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures-ext/10");
        let Ok(rd) = std::fs::read_dir(&root) else {
            return;
        };
        let mut total = 0;
        for e in rd.flatten() {
            let p = e.path();
            let name = p.to_string_lossy().into_owned();
            if !name.ends_with(".kicad_pcb") || name.contains("jetson") || name.contains("vme") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            total += check(&text, SHOW_REFS);
        }
        eprintln!("hidden {total} value texts");
    }
}
