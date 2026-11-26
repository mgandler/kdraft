//! Report structures (JSON) and their text rendering.

use std::fmt::Write as _;
use std::path::Path;

use kdraft_board::geom::BBox;
use kdraft_board::{
    Board, BoardDiff, DrillRow, DrillShape, FootprintChange, HoleKind, MountType, Side,
    StackupLayer, TitleBlock, ViaType,
};
use serde::Serialize;

/// Millimetres with up to four decimals, trailing zeros removed.
pub fn fmt_mm(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// `2` -> "2:1", `0.5` -> "1:2".
pub fn fmt_scale(s: f64) -> String {
    if s >= 1.0 {
        format!("{}:1", fmt_mm(s))
    } else {
        format!("1:{}", fmt_mm(1.0 / s))
    }
}

#[derive(Serialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
    pub bbox: BBox,
}

#[derive(Serialize)]
pub struct OutlineSummary {
    pub size: Option<Size>,
    pub primitives: usize,
    pub contours: usize,
    pub open_chains: usize,
    pub closed: bool,
}

#[derive(Serialize)]
pub struct StackupSummary {
    pub layers: usize,
    pub copper_layers: usize,
    pub total_thickness: f64,
    pub copper_finish: Option<String>,
    pub dielectric_constraints: Option<bool>,
    pub stack: Vec<StackupLayer>,
}

#[derive(Serialize, Default)]
pub struct FootprintCounts {
    pub total: usize,
    pub front: usize,
    pub back: usize,
    pub smd: usize,
    pub through_hole: usize,
    pub unspecified: usize,
    pub dnp: usize,
    pub exclude_from_bom: usize,
    pub exclude_from_pos_files: usize,
    pub board_only: usize,
    pub pads: usize,
}

#[derive(Serialize, Default)]
pub struct ViaCounts {
    pub total: usize,
    pub through: usize,
    pub blind_buried: usize,
    pub micro: usize,
}

#[derive(Serialize, Default)]
pub struct HoleCounts {
    pub total: usize,
    pub plated: usize,
    pub non_plated: usize,
    pub slots: usize,
    pub drill_sizes: usize,
}

#[derive(Serialize)]
pub struct InspectReport {
    pub file: String,
    pub hash: String,
    pub version: i64,
    pub generator: Option<String>,
    pub generator_version: Option<String>,
    pub board_thickness: Option<f64>,
    pub outline: OutlineSummary,
    pub layers: usize,
    pub copper_layers: usize,
    pub stackup: Option<StackupSummary>,
    pub footprints: FootprintCounts,
    pub vias: ViaCounts,
    pub holes: HoleCounts,
    pub title_block: TitleBlock,
    pub warnings: Vec<String>,
}

pub fn inspect(path: &Path, b: &Board) -> InspectReport {
    let mut fc = FootprintCounts {
        total: b.footprints.len(),
        ..Default::default()
    };
    for f in &b.footprints {
        match f.side {
            Side::Front => fc.front += 1,
            Side::Back => fc.back += 1,
        }
        match f.attributes.mount {
            MountType::Smd => fc.smd += 1,
            MountType::ThroughHole => fc.through_hole += 1,
            MountType::Unspecified => fc.unspecified += 1,
        }
        fc.dnp += usize::from(f.attributes.dnp);
        fc.exclude_from_bom += usize::from(f.attributes.exclude_from_bom);
        fc.exclude_from_pos_files += usize::from(f.attributes.exclude_from_pos_files);
        fc.board_only += usize::from(f.attributes.board_only);
        fc.pads += f.pads.len();
    }
    let mut vc = ViaCounts {
        total: b.vias.len(),
        ..Default::default()
    };
    for v in &b.vias {
        match v.kind {
            ViaType::Through => vc.through += 1,
            ViaType::BlindBuried => vc.blind_buried += 1,
            ViaType::Micro => vc.micro += 1,
        }
    }
    let holes = b.holes();
    let hc = HoleCounts {
        total: holes.len(),
        plated: holes.iter().filter(|h| h.plated).count(),
        non_plated: holes.iter().filter(|h| !h.plated).count(),
        slots: holes
            .iter()
            .filter(|h| matches!(h.shape, DrillShape::Oval { .. }))
            .count(),
        drill_sizes: kdraft_board::drill_table(&holes).len(),
    };
    let o = &b.outline;
    InspectReport {
        file: path.display().to_string(),
        hash: b.hash.to_hex(),
        version: b.version,
        generator: b.generator.clone(),
        generator_version: b.generator_version.clone(),
        board_thickness: b.thickness,
        outline: OutlineSummary {
            size: o.bbox.map(|bb| Size {
                width: bb.width(),
                height: bb.height(),
                bbox: bb,
            }),
            primitives: o.primitives.len(),
            contours: o.contours.len(),
            open_chains: o.open_chains.len(),
            closed: o.is_closed(),
        },
        layers: b.layers.len(),
        copper_layers: b.copper_layer_count(),
        stackup: b.stackup.as_ref().map(|s| StackupSummary {
            layers: s.layers.len(),
            copper_layers: s.copper_count(),
            total_thickness: s.total_thickness(),
            copper_finish: s.copper_finish.clone(),
            dielectric_constraints: s.dielectric_constraints,
            stack: s.layers.clone(),
        }),
        footprints: fc,
        vias: vc,
        holes: hc,
        title_block: b.title_block.clone(),
        warnings: b.warnings.clone(),
    }
}

fn opt(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("-")
}

pub fn inspect_text(r: &InspectReport) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "file:        {}", r.file);
    let _ = writeln!(s, "hash:        {}", r.hash);
    let _ = writeln!(
        s,
        "format:      version {} ({} {})",
        r.version,
        opt(&r.generator),
        opt(&r.generator_version)
    );
    match &r.outline.size {
        Some(sz) => {
            let _ = writeln!(
                s,
                "size:        {} x {} mm (x {}..{}, y {}..{})",
                fmt_mm(sz.width),
                fmt_mm(sz.height),
                fmt_mm(sz.bbox.min.x),
                fmt_mm(sz.bbox.max.x),
                fmt_mm(sz.bbox.min.y),
                fmt_mm(sz.bbox.max.y)
            );
        }
        None => {
            let _ = writeln!(s, "size:        no Edge.Cuts outline");
        }
    }
    let _ = writeln!(
        s,
        "outline:     {} primitives, {} closed contours, {} open chains",
        r.outline.primitives, r.outline.contours, r.outline.open_chains
    );
    let _ = writeln!(
        s,
        "layers:      {} copper, {} total; board thickness {}",
        r.copper_layers,
        r.layers,
        r.board_thickness
            .map_or("-".to_string(), |t| format!("{} mm", fmt_mm(t)))
    );
    match &r.stackup {
        Some(st) => {
            let _ = writeln!(
                s,
                "stackup:     {} layers ({} copper), {} mm total, finish {}",
                st.layers,
                st.copper_layers,
                fmt_mm(st.total_thickness),
                opt(&st.copper_finish)
            );
            for l in &st.stack {
                let _ = writeln!(
                    s,
                    "  {:<16} {:<20} {:>8} {}",
                    if l.sublayer > 0 {
                        format!("  (sub {})", l.sublayer)
                    } else {
                        l.name.clone()
                    },
                    l.kind,
                    l.thickness
                        .map_or(String::new(), |t| format!("{} mm", fmt_mm(t))),
                    opt(&l.material)
                );
            }
        }
        None => {
            let _ = writeln!(s, "stackup:     none in file");
        }
    }
    let f = &r.footprints;
    let _ = writeln!(
        s,
        "footprints:  {} (front {}, back {}; smd {}, tht {}, other {}); {} pads",
        f.total, f.front, f.back, f.smd, f.through_hole, f.unspecified, f.pads
    );
    let _ = writeln!(
        s,
        "             dnp {}, excluded from BOM {}, from pos files {}, board-only {}",
        f.dnp, f.exclude_from_bom, f.exclude_from_pos_files, f.board_only
    );
    let v = &r.vias;
    let _ = writeln!(
        s,
        "vias:        {} (through {}, blind/buried {}, micro {})",
        v.total, v.through, v.blind_buried, v.micro
    );
    let h = &r.holes;
    let _ = writeln!(
        s,
        "holes:       {} (plated {}, non-plated {}, slots {}); {} drill sizes",
        h.total, h.plated, h.non_plated, h.slots, h.drill_sizes
    );
    let t = &r.title_block;
    let _ = writeln!(s, "title:       {}", opt(&t.title));
    let _ = writeln!(
        s,
        "             rev {}, date {}, company {}",
        opt(&t.rev),
        opt(&t.date),
        opt(&t.company)
    );
    for (k, c) in &t.comments {
        let _ = writeln!(s, "             comment {k}: {c}");
    }
    for w in &r.warnings {
        let _ = writeln!(s, "warning:     {w}");
    }
    s
}

pub fn shape_text(shape: &DrillShape) -> String {
    match shape {
        DrillShape::Round { diameter } => fmt_mm(*diameter),
        DrillShape::Oval { width, height } => format!("{} x {}", fmt_mm(*width), fmt_mm(*height)),
    }
}

pub fn drill_table_text(rows: &[DrillRow]) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{:<16} {:<6} {:<10} {:<4} {:>6}",
        "size (mm)", "shape", "plating", "type", "count"
    );
    for r in rows {
        let _ = writeln!(
            s,
            "{:<16} {:<6} {:<10} {:<4} {:>6}",
            shape_text(&r.shape),
            match r.shape {
                DrillShape::Round { .. } => "round",
                DrillShape::Oval { .. } => "slot",
            },
            if r.plated { "PTH" } else { "NPTH" },
            match r.kind {
                HoleKind::Via => "via",
                HoleKind::Pad => "pad",
            },
            r.count
        );
    }
    let total: usize = rows.iter().map(|r| r.count).sum();
    let _ = writeln!(
        s,
        "{:<16} {:<6} {:<10} {:<4} {:>6}",
        "total", "", "", "", total
    );
    s
}

#[derive(Serialize)]
pub struct DiffReport<'a> {
    pub identical_bytes: bool,
    pub changed: bool,
    pub diff: &'a BoardDiff,
}

fn pt(p: kdraft_board::geom::Point) -> String {
    format!("({}, {})", fmt_mm(p.x), fmt_mm(p.y))
}

fn bbox_text(b: &Option<BBox>) -> String {
    b.map_or("none".to_string(), |b| {
        format!(
            "{} x {} at {}",
            fmt_mm(b.width()),
            fmt_mm(b.height()),
            pt(b.min)
        )
    })
}

fn q(v: &Option<String>) -> String {
    v.as_ref().map_or("none".to_string(), |s| format!("{s:?}"))
}

pub fn diff_text(d: &BoardDiff, identical_bytes: bool) -> String {
    let mut s = String::new();
    if d.is_empty() {
        let _ = writeln!(
            s,
            "No semantic changes{}.",
            if identical_bytes {
                " (files are identical)"
            } else {
                " (files differ in bytes only)"
            }
        );
        return s;
    }
    if !d.footprints_added.is_empty() {
        let _ = writeln!(s, "Footprints added ({}):", d.footprints_added.len());
        for f in &d.footprints_added {
            let _ = writeln!(s, "  + {} [{}]", f.label(), f.lib_id);
        }
    }
    if !d.footprints_removed.is_empty() {
        let _ = writeln!(s, "Footprints removed ({}):", d.footprints_removed.len());
        for f in &d.footprints_removed {
            let _ = writeln!(s, "  - {} [{}]", f.label(), f.lib_id);
        }
    }
    if !d.footprints_changed.is_empty() {
        let _ = writeln!(s, "Footprints changed ({}):", d.footprints_changed.len());
        for c in &d.footprints_changed {
            let parts: Vec<String> = c
                .changes
                .iter()
                .map(|ch| match ch {
                    FootprintChange::Moved { from, to } => {
                        format!("moved {} -> {}", pt(*from), pt(*to))
                    }
                    FootprintChange::Rotated { from, to } => {
                        format!("rotated {} -> {} deg", fmt_mm(*from), fmt_mm(*to))
                    }
                    FootprintChange::Flipped { from, to } => {
                        format!("flipped {} -> {}", from.as_str(), to.as_str())
                    }
                    FootprintChange::Value { old, new } => {
                        format!("value {} -> {}", q(old), q(new))
                    }
                    FootprintChange::Reference { old, new } => {
                        format!("reference {} -> {}", q(old), q(new))
                    }
                    FootprintChange::LibId { old, new } => format!("footprint {old} -> {new}"),
                    FootprintChange::Dnp { old, new } => format!("dnp {old} -> {new}"),
                })
                .collect();
            let _ = writeln!(s, "  ~ {}: {}", c.id.label(), parts.join("; "));
        }
    }
    if let Some(o) = &d.outline {
        let _ = writeln!(
            s,
            "Outline changed: {} primitives -> {}; {} -> {}",
            o.old_primitives,
            o.new_primitives,
            bbox_text(&o.old_bbox),
            bbox_text(&o.new_bbox)
        );
    }
    if let Some(h) = &d.holes {
        let _ = writeln!(
            s,
            "Holes changed: {} -> {} ({} added, {} removed)",
            h.old_count, h.new_count, h.added, h.removed
        );
    }
    if d.stackup_changed {
        let _ = writeln!(s, "Stackup changed");
    }
    if let Some((a, b)) = d.thickness_changed {
        let f = |v: Option<f64>| v.map_or("none".to_string(), |t| format!("{} mm", fmt_mm(t)));
        let _ = writeln!(s, "Board thickness changed: {} -> {}", f(a), f(b));
    }
    for f in &d.title_block {
        let _ = writeln!(s, "Title block {}: {} -> {}", f.field, q(&f.old), q(&f.new));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm_and_scale_formatting() {
        assert_eq!(fmt_mm(1.0), "1");
        assert_eq!(fmt_mm(0.30000000004), "0.3");
        assert_eq!(fmt_mm(-0.00001), "0");
        assert_eq!(fmt_mm(12.3456789), "12.3457");
        assert_eq!(fmt_scale(2.0), "2:1");
        assert_eq!(fmt_scale(0.5), "1:2");
        assert_eq!(fmt_scale(1.0), "1:1");
    }
}
