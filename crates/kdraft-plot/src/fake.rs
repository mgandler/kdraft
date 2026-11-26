//! A synthetic stand-in for `kicad-cli`, for tests only.
//!
//! It answers `version`, `--help` (also `pcb render --help`), `pcb export svg|drill|pos|stats` by
//! writing files derived from the parsed board. The SVG it writes is NOT
//! KiCad's format: units, offsets, Y direction and nesting differ per page
//! size mode on purpose, so that code under test must derive the transform
//! instead of assuming one. [`fake_transform`] gives the ground truth.

use std::fmt::Write as _;
use std::path::Path;

use kdraft_board::geom::{circle_through, Affine, Point};
use kdraft_board::outline::EdgeShape;
use kdraft_board::Board;
use kdraft_source::{arg_value, CommandOutput, FakeRunner};

use crate::fit::pad_on_layer;

/// Behaviour switches.
#[derive(Debug, Clone, Default)]
pub struct FakeOptions {
    /// Exports containing this flag fail with "Unknown argument".
    pub reject_flag: Option<String>,
    /// Layer names that fail as unknown.
    pub reject_layers: Vec<String>,
    /// `pcb export stats` fails as an unknown command.
    pub no_stats: bool,
    pub version: String,
}

/// Page size (mm) and the board-mm to user-unit map the fake uses.
pub fn fake_transform(board: &Board, mode: u8, mirror: bool) -> (f64, f64, Affine) {
    let bb = board
        .outline
        .bbox
        .unwrap_or(kdraft_board::geom::BBox::from_point(Point::ORIGIN));
    let (w, h, m) = match mode {
        // 1e-4 mm user units, board offset on an A4 page.
        0 => (
            297.0,
            210.0,
            Affine::scale(1e4, 1e4).then(&Affine::translate(4e5, 3e5)),
        ),
        // 1e-3 mm units, different offset.
        1 => (
            420.0,
            297.0,
            Affine::scale(1e3, 1e3).then(&Affine::translate(2e4, 1e4)),
        ),
        // Board area, 1e-2 mm units, Y up.
        _ => {
            let (w, h) = (bb.width() + 2.0, bb.height() + 2.0);
            let m = Affine::translate(-bb.min.x + 1.0, -bb.min.y + 1.0)
                .then(&Affine::scale(100.0, -100.0))
                .then(&Affine::translate(0.0, h * 100.0));
            (w, h, m)
        }
    };
    let m = if mirror {
        let cx = bb.center().x;
        Affine::translate(-cx, 0.0)
            .then(&Affine::scale(-1.0, 1.0))
            .then(&Affine::translate(cx, 0.0))
            .then(&m)
    } else {
        m
    };
    (w, h, m)
}

fn user_units_per_mm(mode: u8) -> f64 {
    match mode {
        0 => 1e4,
        1 => 1e3,
        _ => 100.0,
    }
}

/// Synthetic layer SVG.
pub fn fake_svg(board: &Board, layer: &str, mode: u8, mirror: bool) -> String {
    let (w, h, m) = fake_transform(board, mode, mirror);
    let k = user_units_per_mm(mode);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "<?xml version=\"1.0\" standalone=\"no\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">"
    );
    let (wa, ha) = if mode == 1 {
        (format!("{}in", w / 25.4), format!("{}in", h / 25.4))
    } else {
        (format!("{w}mm"), format!("{h}mm"))
    };
    let _ = writeln!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{wa}\" height=\"{ha}\" viewBox=\"0 0 {} {}\">",
        w * k,
        h * k
    );
    if mode == 0 {
        let _ = writeln!(
            s,
            "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#FFFFFF\"/>",
            w * k,
            h * k
        );
    }
    // Split the map into an outer translate and an inner matrix to
    // exercise nested transforms.
    let outer = Affine::translate(m.e / 2.0, m.f / 2.0);
    let inner = Affine::new(m.a, m.b, m.c, m.d, m.e / 2.0, m.f / 2.0);
    let _ = writeln!(s, "<g transform=\"translate({} {})\">", outer.e, outer.f);
    let _ = writeln!(
        s,
        "<g style=\"fill:#000000; stroke:#000000; stroke-linecap:round\" transform=\"matrix({} {} {} {} {} {})\">",
        inner.a, inner.b, inner.c, inner.d, inner.e, inner.f
    );
    match layer {
        "Edge.Cuts" => edge(&mut s, board),
        l if l.ends_with(".Cu") => copper(&mut s, board, l),
        l if l.ends_with(".Fab") => boxes(&mut s, board, l, true),
        _ => boxes(&mut s, board, layer, false),
    }
    let _ = writeln!(s, "</g>\n</g>\n</svg>");
    s
}

fn edge(s: &mut String, board: &Board) {
    for p in &board.outline.primitives {
        match &p.shape {
            EdgeShape::Line { start, end } => {
                let _ = writeln!(
                    s,
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke-width=\"0.1\"/>",
                    start.x, start.y, end.x, end.y
                );
            }
            EdgeShape::Arc { start, mid, end } => {
                if let Some((c, r)) = circle_through(*start, *mid, *end) {
                    let tau = std::f64::consts::TAU;
                    let ang = |p: &Point| (p.y - c.y).atan2(p.x - c.x);
                    let (a0, am, a1) = (ang(start), ang(mid), ang(end));
                    let pos = |from: f64, to: f64| (to - from).rem_euclid(tau);
                    // Sweep 1 runs in increasing angle (clockwise on screen).
                    let increasing = pos(a0, am) < pos(a0, a1);
                    let sweep = u8::from(increasing);
                    let span = if increasing { pos(a0, a1) } else { pos(a1, a0) };
                    let large = u8::from(span > std::f64::consts::PI);
                    let _ = writeln!(
                        s,
                        "<path fill=\"none\" stroke-width=\"0.1\" d=\"M{} {} A{r} {r} 0 {large} {sweep} {} {}\"/>",
                        start.x, start.y, end.x, end.y
                    );
                }
            }
            EdgeShape::Circle { center, radius } => {
                let _ = writeln!(
                    s,
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{radius}\" fill=\"none\" stroke-width=\"0.1\"/>",
                    center.x, center.y
                );
            }
            EdgeShape::Rect { start, end } => {
                let _ = writeln!(
                    s,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke-width=\"0.1\"/>",
                    start.x.min(end.x),
                    start.y.min(end.y),
                    (end.x - start.x).abs(),
                    (end.y - start.y).abs()
                );
            }
            other => {
                if let Some(b) = other.bbox() {
                    let _ = writeln!(
                        s,
                        "<polyline fill=\"none\" stroke-width=\"0.1\" points=\"{} {} {} {} {} {} {} {} {} {}\"/>",
                        b.min.x, b.min.y, b.max.x, b.min.y, b.max.x, b.max.y, b.min.x, b.max.y,
                        b.min.x, b.min.y
                    );
                }
            }
        }
    }
}

fn copper(s: &mut String, board: &Board, layer: &str) {
    for fp in &board.footprints {
        for pad in &fp.pads {
            if !pad_on_layer(pad, layer) || pad.size.w <= 0.0 {
                continue;
            }
            let c = pad.position;
            let (hw, hh) = (pad.size.w / 2.0, pad.size.h / 2.0);
            match pad.shape.as_str() {
                "circle" => {
                    let _ = writeln!(
                        s,
                        "<circle cx=\"{}\" cy=\"{}\" r=\"{hw}\" stroke=\"none\"/>",
                        c.x, c.y
                    );
                }
                "oval" if (hw - hh).abs() > 1e-9 => {
                    let r = hw.min(hh);
                    let d = if hw > hh {
                        Point::new(hw - r, 0.0)
                    } else {
                        Point::new(0.0, hh - r)
                    };
                    let d = d.rotate_kicad(pad.rotation);
                    let _ = writeln!(
                        s,
                        "<path fill=\"none\" stroke-width=\"{}\" d=\"M{} {} L{} {}\"/>",
                        2.0 * r,
                        c.x - d.x,
                        c.y - d.y,
                        c.x + d.x,
                        c.y + d.y
                    );
                }
                _ => {
                    let pts: Vec<Point> = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
                        .iter()
                        .map(|&(x, y)| c + Point::new(x, y).rotate_kicad(pad.rotation))
                        .collect();
                    let _ = write!(s, "<polygon stroke=\"none\" points=\"");
                    for p in pts {
                        let _ = write!(s, "{} {} ", p.x, p.y);
                    }
                    let _ = writeln!(s, "\"/>");
                }
            }
            let _ = writeln!(
                s,
                "<text x=\"{}\" y=\"{}\" opacity=\"0\">{}</text>",
                c.x, c.y, pad.number
            );
        }
    }
}

fn boxes(s: &mut String, board: &Board, layer: &str, fab: bool) {
    let side_front = layer.starts_with("F.");
    for fp in &board.footprints {
        if (fp.side == kdraft_board::Side::Front) != side_front {
            continue;
        }
        let b = if fab { fp.fab } else { fp.courtyard };
        if let Some(b) = b {
            let _ = writeln!(
                s,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke-width=\"0.05\"/>",
                b.min.x,
                b.min.y,
                b.width().max(1e-6),
                b.height().max(1e-6)
            );
        }
    }
}

/// Synthetic sheet: A3 frame with 10 mm margins, title-block texts, and one
/// project variable left unexpanded.
pub fn fake_sheet_svg(stub: &Board) -> String {
    let tb = &stub.title_block;
    let mut s = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"420mm\" height=\"297mm\" viewBox=\"0 0 420000 297000\">\n<g fill=\"none\" stroke=\"#000000\" stroke-width=\"150\">\n<rect x=\"10000\" y=\"10000\" width=\"400000\" height=\"277000\"/>\n</g>\n",
    );
    for (i, t) in [
        &tb.title,
        &tb.rev,
        &tb.company,
        &tb.comments.get(&1).cloned(),
    ]
    .iter()
    .enumerate()
    {
        if let Some(t) = t {
            let _ = writeln!(
                s,
                "<text x=\"300000\" y=\"{}\" opacity=\"0\">{t}</text>",
                250000 + i * 5000
            );
        }
    }
    s.push_str("<text x=\"300000\" y=\"280000\" opacity=\"0\">${KDRAFT_S1_VAR}</text>\n</svg>\n");
    s
}

const HELP_SVG: &str = "Usage: svg [--help] [--output VAR] [--layers VAR] [--mirror] [--theme VAR] \
[--black-and-white] [--exclude-drawing-sheet] [--drawing-sheet VAR] [--define-var VAR] \
[--page-size-mode VAR] [--mode-single] [--mode-multi] [--drill-shape-opt VAR] [--variant VAR] input";
const HELP_DRILL: &str = "Usage: drill [--format VAR] [--generate-map] [--map-format VAR] \
[--generate-report] [--report-path VAR] input";
const HELP_POS: &str =
    "Usage: pos [--format VAR] [--units VAR] [--side VAR] [--exclude-dnp] [--variant VAR] input";
/// The usage line of the real kicad-cli 10.0.8 `pcb render --help`
/// (shortened).
const HELP_RENDER: &str = "Usage: render [--help] [--output OUTPUT_FILE] [--define-var KEY=VALUE]... \
[--variant VAR] [--width WIDTH] [--height HEIGHT] [--side SIDE] [--background BG] [--quality QUALITY] \
[--preset PRESET] [--use-board-stackup-colors VAR] [--floor] [--perspective] [--zoom ZOOM] \
[--pan VECTOR] [--pivot PIVOT] [--rotate ANGLES] INPUT_FILE";
const HELP_STATS: &str = "Usage: stats [--format VAR] [--units VAR] input";
const HELP_BOM: &str = "Usage: bom [--fields VAR] [--labels VAR] [--group-by VAR] [--exclude-dnp] \
[--field-delimiter VAR] [--string-delimiter VAR] [--ref-delimiter VAR] [--ref-range-delimiter VAR] \
[--keep-tabs] [--keep-line-breaks] [--variant VAR] input";

/// A runner that behaves like the synthetic `kicad-cli` described above.
pub fn fake_kicad(board: Board, opts: FakeOptions) -> FakeRunner {
    let version = if opts.version.is_empty() {
        "9.0.6".to_string()
    } else {
        opts.version.clone()
    };
    FakeRunner::new(move |_, args| {
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        if a == ["version"] {
            return CommandOutput::ok(format!("{version}\n"));
        }
        if a.contains(&"--help") {
            let text = match (a.get(1), a.get(2)) {
                (Some(&"render"), _) => HELP_RENDER,
                (_, Some(&"svg")) => HELP_SVG,
                (_, Some(&"drill")) => HELP_DRILL,
                (_, Some(&"pos")) => HELP_POS,
                (_, Some(&"stats")) if !opts.no_stats => HELP_STATS,
                (_, Some(&"bom")) => HELP_BOM,
                _ => return CommandOutput::failed(1, "Unknown command"),
            };
            return CommandOutput::ok(text);
        }
        if let Some(f) = &opts.reject_flag {
            if a.contains(&f.as_str()) {
                return CommandOutput::failed(1, format!("Unknown argument: {f}"));
            }
        }
        let Some(out) = arg_value(args, "-o") else {
            return CommandOutput::failed(1, "no -o");
        };
        let write = |p: &Path, text: &str| match std::fs::write(p, text) {
            Ok(()) => CommandOutput::ok(""),
            Err(e) => CommandOutput::failed(3, e.to_string()),
        };
        match a.get(..3) {
            Some(["pcb", "export", "svg"]) => {
                let layer = arg_value(args, "--layers").unwrap_or("");
                if opts.reject_layers.iter().any(|l| l == layer) {
                    return CommandOutput::failed(1, format!("Invalid layer name \"{layer}\""));
                }
                if !a.contains(&"--exclude-drawing-sheet") {
                    let stub = args.last().map(Board::load);
                    return match stub {
                        Some(Ok(b)) => write(Path::new(out), &fake_sheet_svg(&b)),
                        _ => CommandOutput::failed(1, "cannot load input"),
                    };
                }
                let mode: u8 = arg_value(args, "--page-size-mode")
                    .and_then(|m| m.parse().ok())
                    .unwrap_or(0);
                let svg = fake_svg(&board, layer, mode, a.contains(&"--mirror"));
                write(Path::new(out), &svg)
            }
            Some(["pcb", "export", "drill"]) => {
                let dir = Path::new(out.trim_end_matches(['/', '\\']));
                if let Err(e) = std::fs::create_dir_all(dir) {
                    return CommandOutput::failed(3, e.to_string());
                }
                let _ = std::fs::write(dir.join("board.drl"), "M48\n");
                if let Some(rp) = arg_value(args, "--report-path") {
                    let mut r = String::from("Drill report (synthetic)\n\n");
                    // Tools grouped the way KiCad is assumed to group them.
                    for (i, g) in kdraft_board::kicad_tool_groups(&board.holes())
                        .iter()
                        .enumerate()
                    {
                        let d = g.diameter;
                        let _ = writeln!(
                            r,
                            "    T{}  {:.3}mm  {:.4}\"  ({} {}){}",
                            i + 1,
                            d,
                            d / 25.4,
                            g.holes,
                            if g.holes == 1 { "hole" } else { "holes" },
                            if g.slots > 0 {
                                format!("  (with {} slot)", g.slots)
                            } else {
                                String::new()
                            }
                        );
                    }
                    return write(Path::new(rp), &r);
                }
                CommandOutput::ok("")
            }
            Some(["pcb", "export", "pos"]) => {
                let mut c = String::from("Ref,Val,Package,PosX,PosY,Rot,Side\n");
                for fp in board
                    .footprints
                    .iter()
                    .filter(|f| !f.attributes.exclude_from_pos_files)
                {
                    let _ = writeln!(
                        c,
                        "\"{}\",\"{}\",\"x\",{:.4},{:.4},{:.4},{}",
                        fp.reference.as_deref().unwrap_or(""),
                        fp.value.as_deref().unwrap_or(""),
                        fp.position.x,
                        -fp.position.y,
                        fp.rotation,
                        if fp.side == kdraft_board::Side::Front {
                            "top"
                        } else {
                            "bottom"
                        }
                    );
                }
                write(Path::new(out), &c)
            }
            Some(["pcb", "export", "stats"]) if !opts.no_stats => {
                let (bw, bh) = board
                    .outline
                    .bbox
                    .map_or((0.0, 0.0), |b| (b.width(), b.height()));
                write(
                    Path::new(out),
                    &format!("{{\"board\": {{\"width\": {bw}, \"height\": {bh}}}}}"),
                )
            }
            _ => CommandOutput::failed(1, "Unknown command"),
        }
    })
}
