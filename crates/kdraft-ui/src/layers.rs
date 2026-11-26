//! Layer selection for views: one-click presets derived from the board's
//! layer table, a filter, and the list order (checked layers first).

/// A preset: label and the layers it selects (only those the board has).
#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub label: &'static str,
    pub layers: Vec<String>,
}

fn pick(board: &[String], wanted: &[&str]) -> Vec<String> {
    wanted
        .iter()
        .filter(|w| board.iter().any(|b| b == *w))
        .map(|w| w.to_string())
        .collect()
}

/// The board's name for a layer that has two spellings (KiCad 9/10 short
/// canonical names and long ones): the first present.
fn first_of<'a>(board: &[String], names: &[&'a str]) -> Option<&'a str> {
    names.iter().copied().find(|n| board.iter().any(|b| b == n))
}

/// Presets for a board with layer names `board` (canonical names).
pub fn presets(board: &[String]) -> Vec<Preset> {
    let crt = |side: &str| {
        first_of(
            board,
            &[
                &format!("{side}.CrtYd") as &str,
                &format!("{side}.Courtyard") as &str,
            ],
        )
        .map(str::to_string)
    };
    let mut out = Vec::new();
    let mut push = |label: &'static str, mut layers: Vec<String>, extra: Option<String>| {
        if let Some(e) = extra {
            layers.insert(1, e);
        }
        if (layers.iter().any(|l| l != "Edge.Cuts") || label == "Outline only")
            && !layers.is_empty()
        {
            out.push(Preset { label, layers });
        }
    };
    push("Fab top", pick(board, &["F.Fab", "Edge.Cuts"]), None);
    push("Fab bottom", pick(board, &["B.Fab", "Edge.Cuts"]), None);
    push(
        "Assembly top",
        pick(board, &["F.Fab", "Edge.Cuts"]),
        crt("F"),
    );
    push(
        "Assembly bottom",
        pick(board, &["B.Fab", "Edge.Cuts"]),
        crt("B"),
    );
    push("Copper top", pick(board, &["F.Cu", "Edge.Cuts"]), None);
    push("Copper bottom", pick(board, &["B.Cu", "Edge.Cuts"]), None);
    push("Outline only", pick(board, &["Edge.Cuts"]), None);
    // Presets with the same layers as an earlier one (no courtyard
    // layers: assembly = fab) are dropped.
    let mut seen: Vec<Vec<String>> = Vec::new();
    out.retain(|p| {
        let mut k = p.layers.clone();
        k.sort();
        if seen.contains(&k) {
            false
        } else {
            seen.push(k);
            true
        }
    });
    out
}

/// Whether `layers` is exactly the preset (order ignored).
pub fn is_preset(layers: &[String], p: &Preset) -> bool {
    layers.len() == p.layers.len() && p.layers.iter().all(|l| layers.contains(l))
}

/// The list to show: the view's layers first, topmost (drawn last) at
/// the top, then the board's other layers in board order, filtered by
/// `filter` (case-insensitive substring of the name).
pub fn ordered(board: &[String], checked: &[String], filter: &str) -> Vec<(String, bool)> {
    let f = filter.trim().to_lowercase();
    let keep = |n: &str| f.is_empty() || n.to_lowercase().contains(&f);
    let mut out: Vec<(String, bool)> = Vec::new();
    for n in checked.iter().rev() {
        if keep(n) {
            out.push((n.clone(), true));
        }
    }
    for n in board.iter().filter(|n| !checked.contains(n)) {
        if keep(n) {
            out.push((n.clone(), false));
        }
    }
    out
}

/// The view's layer list after dragging the row of `from` onto the row of
/// `to` (both names of checked layers; rows are shown topmost first, so
/// dropping onto a row puts the layer just above it).
pub fn reordered(layers: &[String], from: &str, to: &str) -> Vec<String> {
    let mut v: Vec<String> = layers.iter().filter(|l| *l != from).cloned().collect();
    let at = v.iter().position(|l| l == to).map_or(v.len(), |i| i + 1);
    if layers.iter().any(|l| l == from) {
        v.insert(at.min(v.len()), from.to_string());
    }
    v
}

/// Rows shown when the list is collapsed.
pub const COLLAPSED_ROWS: usize = 6;

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Vec<String> {
        [
            "F.Cu",
            "In1.Cu",
            "B.Cu",
            "F.SilkS",
            "B.SilkS",
            "F.Fab",
            "B.Fab",
            "F.CrtYd",
            "B.CrtYd",
            "Edge.Cuts",
            "Dwgs.User",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn presets_from_the_layer_table() {
        let p = presets(&board());
        let get = |l: &str| p.iter().find(|x| x.label == l).unwrap().layers.clone();
        assert_eq!(get("Assembly top"), vec!["F.Fab", "F.CrtYd", "Edge.Cuts"]);
        assert_eq!(get("Copper bottom"), vec!["B.Cu", "Edge.Cuts"]);
        assert_eq!(get("Outline only"), vec!["Edge.Cuts"]);
        // A board without fab layers gets no fab presets.
        let small: Vec<String> = vec!["F.Cu".into(), "Edge.Cuts".into()];
        let p = presets(&small);
        assert!(p.iter().all(|x| !x.label.starts_with("Fab")));
        assert!(p.iter().any(|x| x.label == "Copper top"));
        // KiCad 10 long courtyard names.
        let long: Vec<String> = vec!["F.Fab".into(), "F.Courtyard".into(), "Edge.Cuts".into()];
        let a = presets(&long);
        assert_eq!(
            a.iter().find(|x| x.label == "Assembly top").unwrap().layers,
            vec!["F.Fab", "F.Courtyard", "Edge.Cuts"]
        );
    }

    #[test]
    fn checked_first_and_filter() {
        let checked: Vec<String> = vec!["Edge.Cuts".into(), "F.Cu".into(), "User.9".into()];
        let o = ordered(&board(), &checked, "");
        assert_eq!(o[0], ("User.9".into(), true));
        assert_eq!(o[1], ("F.Cu".into(), true));
        assert_eq!(o[2], ("Edge.Cuts".into(), true));
        assert!(!o[3].1);
        assert_eq!(o.len(), board().len() + 1);
        let o = ordered(&board(), &checked, "fab");
        assert_eq!(
            o.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
            vec!["F.Fab", "B.Fab"]
        );
        assert!(is_preset(
            &["Edge.Cuts".into(), "F.Cu".into()],
            &presets(&board())[4]
        ));
        let l: Vec<String> = vec!["B.Cu".into(), "F.Cu".into(), "Edge.Cuts".into()];
        // Drag Edge.Cuts (top row) onto B.Cu: just above B.Cu.
        assert_eq!(
            reordered(&l, "Edge.Cuts", "B.Cu"),
            vec!["B.Cu", "Edge.Cuts", "F.Cu"]
        );
        assert_eq!(
            reordered(&l, "B.Cu", "Edge.Cuts"),
            vec!["F.Cu", "Edge.Cuts", "B.Cu"]
        );
    }
}
