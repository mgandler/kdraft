//! Drawing checks (`kdraft check`, command `document.check`).
//!
//! Findings are ordered by page, then item order on the page, then check
//! kind, so the output is stable. Severities: errors make a drawing wrong
//! (broken anchors, unplaced designators, tables overflowing the drawing
//! area), warnings make it doubtful (stale board, items outside the drawing
//! area or overlapping, small text), info lines carry metrics.

use serde::Serialize;

use kdraft_board::geom::BBox;
use kdraft_doc::{Item, ViewKind};
use kdraft_layout::grid::overlaps;
use kdraft_render::dl;

use crate::bounds::board_box;
use crate::pipeline::RenderInputs;
use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// Short stable code, e.g. `broken-anchor`.
    pub code: &'static str,
    /// 1-based page number (0: document level).
    pub page: usize,
    pub item: Option<String>,
    pub message: String,
}

/// Smallest text height on paper that passes, mm.
pub const MIN_TEXT_HEIGHT: f64 = 0.8;
/// Allowed excess over the drawing area, mm (stroke widths).
const AREA_TOLERANCE: f64 = 0.3;
/// Overlap below this area (mm²) is ignored (touching boxes).
const OVERLAP_EPS: f64 = 0.5;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn worst(&self) -> Option<Severity> {
        self.findings
            .iter()
            .map(|f| f.severity)
            .filter(|s| *s != Severity::Info)
            .max()
    }

    /// 0 clean, 1 warnings, 2 errors.
    pub fn exit_code(&self) -> i32 {
        match self.worst() {
            Some(Severity::Error) => 2,
            Some(Severity::Warning) => 1,
            _ => 0,
        }
    }

    pub fn text(&self) -> String {
        let mut s = String::new();
        for f in &self.findings {
            let sev = match f.severity {
                Severity::Info => "info",
                Severity::Warning => "warning",
                Severity::Error => "error",
            };
            let loc = match (f.page, &f.item) {
                (0, _) => "document".to_string(),
                (p, Some(i)) => format!("page {p}, item {i}"),
                (p, None) => format!("page {p}"),
            };
            s.push_str(&format!("{sev} [{}] {loc}: {}\n", f.code, f.message));
        }
        let n = |sv| self.findings.iter().filter(|f| f.severity == sv).count();
        s.push_str(&format!(
            "{} error(s), {} warning(s)\n",
            n(Severity::Error),
            n(Severity::Warning)
        ));
        s
    }
}

fn shrink(b: &BBox, d: f64) -> BBox {
    BBox {
        min: kdraft_board::geom::Point::new(b.min.x + d, b.min.y + d),
        max: kdraft_board::geom::Point::new(b.max.x - d, b.max.y - d),
    }
}

fn overlap_area(a: &BBox, b: &BBox) -> f64 {
    let w = a.max.x.min(b.max.x) - a.min.x.max(b.min.x);
    let h = a.max.y.min(b.max.y) - a.min.y.max(b.min.y);
    if w > 0.0 && h > 0.0 {
        w * h
    } else {
        0.0
    }
}

/// BOM rows that no BOM table shows; BOM items without a callout when
/// callouts to BOM items exist.
fn bom_checks(
    doc: &kdraft_doc::Document,
    board: &kdraft_board::Board,
    ctx: &kdraft_render::SheetContext,
    out: &mut Vec<Finding>,
) {
    let tables: Vec<(usize, &kdraft_doc::Table)> = doc
        .pages
        .iter()
        .enumerate()
        .flat_map(|(pi, p)| {
            p.items.iter().filter_map(move |i| match i {
                Item::Table(t) if t.kind == kdraft_doc::TableKind::Bom => Some((pi, t)),
                _ => None,
            })
        })
        .collect();
    let bom = kdraft_render::page::bom_for(board, ctx);
    if !tables.is_empty() {
        let n = bom.lines.len();
        let mut shown = vec![false; n];
        for (_, t) in &tables {
            let (a, k) = t.rows.unwrap_or((0, n));
            for s in shown.iter_mut().take(a.saturating_add(k).min(n)).skip(a) {
                *s = true;
            }
        }
        let missing: Vec<usize> = (0..n).filter(|i| !shown[*i]).collect();
        if !missing.is_empty() {
            let refs: Vec<String> = missing
                .iter()
                .take(8)
                .map(|&i| bom.lines[i].refs.first().cloned().unwrap_or_default())
                .collect();
            out.push(Finding {
                severity: Severity::Error,
                code: "bom-overflow",
                page: tables[tables.len() - 1].0 + 1,
                item: Some(tables[tables.len() - 1].1.id.to_string()),
                message: format!(
                    "{} BOM line(s) not shown in any BOM table (first: {}); add a continuation table or page",
                    missing.len(),
                    refs.join(", ")
                ),
            });
        }
    }
    // Linked views (layer pages) drawing the same layer in different
    // colours.
    {
        let ix = kdraft_doc::link::index(doc);
        let mut roots: Vec<kdraft_doc::Id> = Vec::new();
        for p in &doc.pages {
            for it in &p.items {
                if let (Item::View(_), Some(l)) = (it, it.link()) {
                    if !roots.contains(&l.to) {
                        roots.push(l.to.clone());
                    }
                }
            }
        }
        for r in roots {
            let Some(Item::View(rv)) = kdraft_doc::link::get(doc, &ix, &r) else {
                continue;
            };
            let members: Vec<&kdraft_doc::View> = std::iter::once(rv)
                .chain(kdraft_doc::link::followers(doc, &r).iter().filter_map(|f| {
                    match kdraft_doc::link::get(doc, &ix, f) {
                        Some(Item::View(v)) => Some(v),
                        _ => None,
                    }
                }))
                .collect();
            let mut layers: Vec<String> = Vec::new();
            for l in members.iter().flat_map(|v| v.layers.iter()) {
                let cols: Vec<_> = members
                    .iter()
                    .filter(|v| v.layers.contains(l))
                    .map(|v| v.layer_color(&doc.styles, l))
                    .collect();
                if cols.len() > 1 && cols.iter().any(|c| *c != cols[0]) && !layers.contains(l) {
                    layers.push(l.clone());
                }
            }
            if !layers.is_empty() {
                let page = ix.get(&r).map_or(0, |(p, _)| p + 1);
                out.push(Finding {
                    severity: Severity::Info,
                    code: "link-colours",
                    page,
                    item: Some(r.to_string()),
                    message: format!(
                        "linked views draw {} in different colours (Arrange > Match layer colours to reference)",
                        layers.join(", ")
                    ),
                });
            }
        }
    }
    // Gaps in callout label sequences, only when asked for.
    if doc.styles.strict_callout_numbering {
        let ranges: Vec<usize> = if doc.styles.callouts_per_document {
            vec![0]
        } else {
            (0..doc.pages.len()).collect()
        };
        for pi in ranges {
            let labels = kdraft_doc::callout::scope_labels(doc, pi);
            let gaps = kdraft_doc::callout::gaps(&labels);
            if !gaps.is_empty() {
                let list: Vec<String> = gaps.iter().take(8).map(u64::to_string).collect();
                out.push(Finding {
                    severity: Severity::Info,
                    code: "callout-gap",
                    page: if doc.styles.callouts_per_document {
                        0
                    } else {
                        pi + 1
                    },
                    item: None,
                    message: format!(
                        "callout numbers missing: {}{}",
                        list.join(", "),
                        if gaps.len() > 8 { ", ..." } else { "" }
                    ),
                });
            }
        }
    }
    let targets: Vec<&str> = doc
        .pages
        .iter()
        .flat_map(|p| p.items.iter())
        .filter_map(|i| match i {
            Item::Callout(c) => c.target.as_deref().and_then(|t| t.strip_prefix("bom:")),
            _ => None,
        })
        .collect();
    if !targets.is_empty() {
        for l in &bom.lines {
            if !targets.contains(&l.key().as_str()) {
                out.push(Finding {
                    severity: Severity::Warning,
                    code: "bom-callout",
                    page: 0,
                    item: None,
                    message: format!(
                        "BOM item {} ({}) has no callout",
                        l.refs.first().cloned().unwrap_or_default(),
                        l.value
                    ),
                });
            }
        }
    }
}

/// `e` reaches outside the drawing area `area` (and is not inside the
/// notes strip left of the title block), beyond a small tolerance for
/// stroke widths. The GUI marks such items with the same rule.
pub fn extends_outside(e: &BBox, area: &BBox, strip: Option<&BBox>) -> bool {
    let t = AREA_TOLERANCE;
    let within = |a: &BBox| {
        e.min.x >= a.min.x - t
            && e.min.y >= a.min.y - t
            && e.max.x <= a.max.x + t
            && e.max.y <= a.max.y + t
    };
    let strip = strip.map(|b| BBox {
        min: kdraft_board::geom::Point::new(b.min.x - 1.0, b.min.y - 1.0),
        max: kdraft_board::geom::Point::new(b.max.x + 1.0, b.max.y + 1.0),
    });
    !(within(area) || strip.as_ref().is_some_and(within))
}

/// Runs all checks on a session (native rendering; no kicad-cli).
pub fn check(s: &mut Session) -> Report {
    let mut out = Vec::new();
    let Some(board) = s.board_arc() else {
        out.push(Finding {
            severity: Severity::Error,
            code: "no-board",
            page: 0,
            item: None,
            message: s
                .board_state()
                .error
                .clone()
                .unwrap_or_else(|| "no board loaded".into()),
        });
        return Report { findings: out };
    };
    if let Some(h) = &s.doc().source.board_hash {
        if *h != board.hash.to_hex() {
            out.push(Finding {
                severity: Severity::Warning,
                code: "stale",
                page: 0,
                item: None,
                message: "the board changed since the document was last saved".into(),
            });
        }
    }
    let Ok((inputs, _)) = RenderInputs::from_session(s, None) else {
        return Report { findings: out };
    };
    let doc = inputs.doc.clone();
    let bb = board_box(Some(&board));
    // Placement links whose reference is gone (hand-edited files).
    let ix = kdraft_doc::link::index(&doc);
    for (pi, page) in doc.pages.iter().enumerate() {
        for it in &page.items {
            if let (Some(id), Some(l)) = (it.id(), it.link()) {
                if !ix.contains_key(&l.to) {
                    out.push(Finding {
                        severity: Severity::Error,
                        code: "link-broken",
                        page: pi + 1,
                        item: Some(id.to_string()),
                        message: format!("placement link to {} which does not exist", l.to),
                    });
                } else if kdraft_doc::link::get(&doc, &ix, &l.to)
                    .is_some_and(|r| r.link().is_some())
                {
                    out.push(Finding {
                        severity: Severity::Warning,
                        code: "link-chain",
                        page: pi + 1,
                        item: Some(id.to_string()),
                        message: format!("placement link to {}, which is itself linked", l.to),
                    });
                }
            }
        }
    }
    for (pi, page) in doc.pages.iter().enumerate() {
        let pn = pi + 1;
        let area = s.drawing_area(pi);
        // The strip left of the title block also takes notes.
        let strip = {
            let (sheet, _, _) = s.sheet_for_page(pi);
            kdraft_doc::notes_strip(&sheet, &page.size, 0.0)
        };
        let mut boxes: Vec<(String, BBox, &Item)> = Vec::new();
        for it in &page.items {
            let Some(id) = it.id().map(|i| i.to_string()) else {
                continue;
            };
            let (items, warnings) = inputs.render_item(it);
            for w in warnings {
                let (sev, code) = if w.contains("broken anchor") {
                    (Severity::Error, "broken-anchor")
                } else if w.contains("unplaced") {
                    (Severity::Error, "unplaced-designator")
                } else if w.contains("not rendered") {
                    (Severity::Info, "not-rendered")
                } else if w.contains("unknown variable") {
                    (Severity::Warning, "unknown-variable")
                } else if w.contains("overflow") {
                    (Severity::Error, "table-overflow")
                } else {
                    (Severity::Warning, "render")
                };
                out.push(Finding {
                    severity: sev,
                    code,
                    page: pn,
                    item: Some(id.clone()),
                    message: w,
                });
            }
            for t in dl::texts(&items) {
                if !t.invisible && t.size < MIN_TEXT_HEIGHT - 1e-9 {
                    out.push(Finding {
                        severity: Severity::Warning,
                        code: "small-text",
                        page: pn,
                        item: Some(id.clone()),
                        message: format!("text {:?} is {:.2} mm high", t.text, t.size),
                    });
                    break;
                }
            }
            if let Item::View(v) = it {
                if v.kind == ViewKind::Assembly && !v.wants_plot() {
                    let map = kdraft_render::view::ViewMap::new(v, bb);
                    let side = match v.side {
                        kdraft_doc::ViewSide::Front => kdraft_board::Side::Front,
                        kdraft_doc::ViewSide::Back => kdraft_board::Side::Back,
                    };
                    let l = kdraft_render::assembly::render(
                        &board,
                        &map,
                        side,
                        &v.assembly,
                        &inputs.fonts,
                    );
                    let st = &l.stats;
                    out.push(Finding {
                        severity: Severity::Info,
                        code: "designators",
                        page: pn,
                        item: Some(id.clone()),
                        message: format!(
                            "{} designators: {} inside, {} outside, {} with leader, {} over other parts, {} unplaced",
                            l.labels.len(),
                            st.inside,
                            st.outside,
                            st.leader,
                            st.over_bodies,
                            st.unplaced
                        ),
                    });
                }
            }
            // The extent used for area and overlap checks: views by their
            // outline box (their texts may legitimately reach further),
            // everything else by what is drawn.
            let extent = match it {
                Item::View(v) if v.kind == kdraft_doc::ViewKind::Render3d => {
                    Some(crate::bounds::render3d_bounds(v, bb, Some(&inputs.renders)))
                }
                Item::View(v) => Some(kdraft_doc::ops::view_bounds(v, bb)),
                _ => dl::items_bbox(&items, &inputs.fonts),
            };
            let Some(e) = extent else { continue };
            if let Some(a) = area {
                let inside = !extends_outside(&e, &a, strip.as_ref());
                if !inside {
                    let linked = it.link().is_some();
                    let (sev, code) = if matches!(it, Item::Table(_)) {
                        (Severity::Error, "table-overflow")
                    } else if linked {
                        (Severity::Warning, "link-outside-area")
                    } else {
                        (Severity::Warning, "outside-area")
                    };
                    out.push(Finding {
                        severity: sev,
                        code,
                        page: pn,
                        item: Some(id.clone()),
                        message: format!(
                            "{}extends outside the drawing area ({:.1}..{:.1} x {:.1}..{:.1} mm vs {:.1}..{:.1} x {:.1}..{:.1} mm)",
                            if linked {
                                "linked placement: this page's drawing area cannot hold it; "
                            } else {
                                ""
                            },
                            e.min.x, e.max.x, e.min.y, e.max.y, a.min.x, a.max.x, a.min.y, a.max.y
                        ),
                    });
                }
            }
            boxes.push((id, e, it));
        }
        // Overlaps, except a dimension or callout with its own view.
        for i in 0..boxes.len() {
            for j in i + 1..boxes.len() {
                let (ia, ba, a) = &boxes[i];
                let (ib, bbx, b) = &boxes[j];
                let tied = |x: &Item, y: &Item| match (x, y) {
                    (Item::Dimension(d), Item::View(v)) => d.view.as_ref() == Some(&v.id),
                    (Item::Callout(c), Item::View(v)) => c.view.as_ref() == Some(&v.id),
                    // Dimensions of one view cross each other's extension
                    // lines by design.
                    (Item::Dimension(d), Item::Dimension(e)) => {
                        d.view.is_some() && d.view == e.view
                    }
                    _ => false,
                };
                if tied(a, b) || tied(b, a) {
                    continue;
                }
                if overlaps(&shrink(ba, 0.01), &shrink(bbx, 0.01))
                    && overlap_area(ba, bbx) > OVERLAP_EPS
                {
                    out.push(Finding {
                        severity: Severity::Warning,
                        code: "overlap",
                        page: pn,
                        item: Some(ia.clone()),
                        message: format!("overlaps item {ib}"),
                    });
                }
            }
        }
    }
    bom_checks(&doc, &board, &inputs.ctx, &mut out);
    // Stable: page, then the order produced above (item order); within an
    // item the checks run in a fixed order.
    out.sort_by_key(|f| f.page);
    Report { findings: out }
}
