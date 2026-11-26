//! One-line summaries for the change banner.

use kdraft_board::{BoardDiff, FootprintChange};

/// Short summary of a board change, e.g. "2 footprints moved, 1 added,
/// outline changed".
pub fn diff_summary(d: &BoardDiff) -> String {
    if d.is_empty() {
        return "no semantic change (file rewritten)".to_string();
    }
    let mut parts = Vec::new();
    let count = |f: &dyn Fn(&FootprintChange) -> bool| {
        d.footprints_changed
            .iter()
            .filter(|c| c.changes.iter().any(f))
            .count()
    };
    let moved = count(&|c| matches!(c, FootprintChange::Moved { .. }));
    let rotated = count(&|c| matches!(c, FootprintChange::Rotated { .. }));
    let flipped = count(&|c| matches!(c, FootprintChange::Flipped { .. }));
    let other = d
        .footprints_changed
        .len()
        .saturating_sub(moved.max(rotated).max(flipped));
    let n = |k: usize, what: &str| format!("{k} footprint{} {what}", if k == 1 { "" } else { "s" });
    if !d.footprints_added.is_empty() {
        parts.push(n(d.footprints_added.len(), "added"));
    }
    if !d.footprints_removed.is_empty() {
        parts.push(n(d.footprints_removed.len(), "removed"));
    }
    if moved > 0 {
        parts.push(n(moved, "moved"));
    }
    if rotated > 0 {
        parts.push(n(rotated, "rotated"));
    }
    if flipped > 0 {
        parts.push(n(flipped, "flipped"));
    }
    if other > 0 && moved + rotated + flipped == 0 {
        parts.push(n(other, "changed"));
    }
    if d.outline.is_some() {
        parts.push("outline changed".to_string());
    }
    if let Some(h) = &d.holes {
        parts.push(format!("holes {} -> {}", h.old_count, h.new_count));
    }
    if d.stackup_changed {
        parts.push("stackup changed".to_string());
    }
    if d.thickness_changed.is_some() {
        parts.push("thickness changed".to_string());
    }
    if !d.title_block.is_empty() {
        parts.push("title block changed".to_string());
    }
    parts.join(", ")
}

/// A readable name for an anchor: "pad U7.3", "hole 12",
/// "outline vertex 4", "origin J2".
pub fn anchor_label(a: &kdraft_doc::Anchor) -> String {
    use kdraft_doc::Anchor as A;
    match a {
        A::Pad { reference, number } => format!("pad {reference}.{number}"),
        A::Hole(i) => format!("hole {i}"),
        A::OutlineVertex(i) => format!("outline vertex {i}"),
        A::OutlineEdge(i) => format!("outline edge {i}"),
        A::FpOrigin(r) => format!("origin {r}"),
        A::ViewPoint(p) => format!("point {:.2}, {:.2}", p.x, p.y),
        A::Unknown(_) => "unknown anchor".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdraft_board::Board;

    #[test]
    fn summarises_a_moved_footprint() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad9-minimal.kicad_pcb");
        let text = std::fs::read_to_string(p).unwrap();
        let a = Board::from_bytes(text.clone().into_bytes()).unwrap();
        assert_eq!(
            diff_summary(&kdraft_board::diff(&a, &a)),
            "no semantic change (file rewritten)"
        );
        let r1 = a.footprint_by_reference("R1").unwrap().position;
        let needle = format!("(at {} {}", r1.x, r1.y);
        assert!(text.contains(&needle), "{needle}");
        let b = Board::from_bytes(
            text.replacen(&needle, &format!("(at {} {}", r1.x + 1.0, r1.y), 1)
                .into_bytes(),
        )
        .unwrap();
        assert_eq!(
            diff_summary(&kdraft_board::diff(&a, &b)),
            "1 footprint moved"
        );
    }
}
