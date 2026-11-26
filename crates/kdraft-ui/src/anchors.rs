//! Anchor snapping inside views: the pads, holes, outline vertices,
//! outline arc/circle centres and footprint origins of the board, mapped
//! through each view's transform and kept in a spatial index. Built once
//! per view, board and view placement; queried per pointer move.

use kdraft_board::geom::{circle_through, BBox, Point};
use kdraft_board::outline::EdgeShape;
use kdraft_board::{Board, Side};
use kdraft_doc::{Anchor, Id, View, ViewKind, ViewSide};
use kdraft_layout::grid::BoxGrid;
use kdraft_render::view::ViewMap;

use crate::tools::Pick;

#[derive(Debug, Clone, PartialEq)]
struct Cand {
    anchor: Anchor,
    /// Sheet position.
    at: Point,
    label: String,
    /// Sheet radius of circular anchors.
    radius: Option<f64>,
    /// Lower wins on ties (pads and holes before origins).
    rank: u8,
}

/// The anchors of one view.
#[derive(Debug, Clone)]
pub struct ViewAnchors {
    pub view: Id,
    cands: Vec<Cand>,
    grid: BoxGrid,
}

/// Grid cell of the index, sheet mm.
const CELL: f64 = 2.0;

impl ViewAnchors {
    pub fn build(view: &View, board: &Board, board_box: BBox) -> ViewAnchors {
        let map = ViewMap::new(view, board_box);
        let s = |p: Point| map.point(p);
        let scale = map.scale.abs();
        let mut cands = Vec::new();
        let side_only = match (view.kind.clone(), view.side) {
            (ViewKind::Assembly, ViewSide::Front) => Some(Side::Front),
            (ViewKind::Assembly, ViewSide::Back) => Some(Side::Back),
            _ => None,
        };
        for fp in &board.footprints {
            if side_only.is_some_and(|sd| sd != fp.side) {
                continue;
            }
            let Some(r) = fp.reference.clone() else {
                continue;
            };
            for pad in &fp.pads {
                if pad.number.is_empty() {
                    continue;
                }
                cands.push(Cand {
                    anchor: Anchor::Pad {
                        reference: r.clone(),
                        number: pad.number.clone(),
                    },
                    at: s(pad.position),
                    label: format!("pad {r}.{}", pad.number),
                    radius: None,
                    rank: 0,
                });
            }
            cands.push(Cand {
                anchor: Anchor::FpOrigin(r.clone()),
                at: s(fp.position),
                label: format!("origin {r}"),
                radius: None,
                rank: 2,
            });
        }
        for h in board.holes() {
            cands.push(Cand {
                anchor: Anchor::Hole(h.index),
                at: s(h.position),
                label: format!("hole {}", h.index),
                radius: Some(h.shape.tool_diameter() / 2.0 * scale),
                rank: 0,
            });
        }
        if let Some(outer) = board.outline.outer() {
            for (i, v) in outer.vertices.iter().enumerate() {
                cands.push(Cand {
                    anchor: Anchor::OutlineVertex(i),
                    at: s(*v),
                    label: format!("outline vertex {i}"),
                    radius: None,
                    rank: 1,
                });
            }
        }
        for (i, prim) in board.outline.primitives.iter().enumerate() {
            let circ = match prim.shape {
                EdgeShape::Circle { center, radius } => Some((center, radius)),
                EdgeShape::Arc { start, mid, end } => circle_through(start, mid, end),
                _ => None,
            };
            if let Some((c, r)) = circ {
                cands.push(Cand {
                    anchor: Anchor::OutlineEdge(i),
                    at: s(c),
                    label: format!("outline arc {i}"),
                    radius: Some(r * scale),
                    rank: 1,
                });
            }
        }
        let mut grid = BoxGrid::new(CELL);
        for c in &cands {
            grid.insert(BBox::from_point(c.at));
        }
        ViewAnchors {
            view: view.id.clone(),
            cands,
            grid,
        }
    }

    pub fn len(&self) -> usize {
        self.cands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cands.is_empty()
    }

    fn pick(&self, c: &Cand) -> Pick {
        Pick {
            view: self.view.clone(),
            anchor: c.anchor.clone(),
            sheet: c.at,
            label: c.label.clone(),
            radius: c.radius,
        }
    }

    /// The anchor nearest `p` within `tol` (sheet mm). With `circular`,
    /// only circular anchors count, hit inside their circle or near its
    /// rim as well as at the centre.
    pub fn nearest(&self, p: Point, tol: f64, circular: bool) -> Option<Pick> {
        let mut hits: Vec<(f64, u8, usize)> = Vec::new();
        if circular {
            for (i, c) in self.cands.iter().enumerate() {
                let Some(r) = c.radius else { continue };
                let dc = c.at.dist(p);
                let d = if dc <= r { 0.0 } else { dc - r };
                if d <= tol {
                    hits.push((d + dc * 1e-3, c.rank, i));
                }
            }
        } else {
            let g = tol * 1.01 + 1e-9;
            let q = BBox {
                min: Point::new(p.x - g, p.y - g),
                max: Point::new(p.x + g, p.y + g),
            };
            for i in self.grid.query(&q, None) {
                let c = &self.cands[i];
                let d = c.at.dist(p);
                if d <= tol {
                    // A hole and a pad at the same place: the hole wins
                    // while the pointer is inside the hole.
                    let inside_hole =
                        matches!(c.anchor, Anchor::Hole(_)) && c.radius.is_some_and(|r| d <= r);
                    let rank = if inside_hole { 0 } else { c.rank + 1 };
                    hits.push((d, rank, i));
                }
            }
        }
        // Distances equal to a micrometre tie.
        let key = |d: f64| (d * 1e3).round() as i64;
        hits.into_iter()
            .min_by(|a, b| key(a.0).cmp(&key(b.0)).then(a.1.cmp(&b.1)))
            .map(|(_, _, i)| self.pick(&self.cands[i]))
    }
}

/// Anchor indices of the views of a page, rebuilt when the key changes.
#[derive(Debug, Clone, Default)]
pub struct AnchorCache {
    key: Option<(u64, [u8; 32])>,
    pub views: Vec<(ViewAnchors, BBox)>,
}

impl AnchorCache {
    /// Ensures the index matches the document revision `rev` and board
    /// hash; `views` are the page's views with their sheet boxes.
    pub fn ensure(&mut self, rev: u64, board: &Board, board_box: BBox, views: &[(&View, BBox)]) {
        let key = (rev, board.hash.0);
        if self.key == Some(key) {
            return;
        }
        self.views = views
            .iter()
            .map(|(v, b)| (ViewAnchors::build(v, board, board_box), *b))
            .collect();
        self.key = Some(key);
    }

    pub fn invalidate(&mut self) {
        self.key = None;
    }

    /// The anchor under `p`: views whose box contains `p` (top-most
    /// last), nearest anchor within `tol`.
    pub fn nearest(&self, p: Point, tol: f64, circular: bool) -> Option<Pick> {
        self.views
            .iter()
            .rev()
            .filter(|(_, b)| {
                p.x >= b.min.x - tol
                    && p.x <= b.max.x + tol
                    && p.y >= b.min.y - tol
                    && p.y <= b.max.y + tol
            })
            .find_map(|(va, _)| va.nearest(p, tol, circular))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn board() -> Board {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad9-minimal.kicad_pcb");
        Board::load(p).unwrap()
    }

    #[test]
    fn snaps_to_pads_holes_and_vertices_through_the_view() {
        let b = board();
        let s = kdraft_engine::Session::new_for_board(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/kicad9-minimal.kicad_pcb"),
            None,
        )
        .unwrap();
        let v = s.doc().pages[0].views().next().unwrap().clone();
        let bb = kdraft_engine::bounds::board_box(Some(&b));
        let va = ViewAnchors::build(&v, &b, bb);
        assert!(va.len() > 5);
        let map = ViewMap::new(&v, bb);
        // The first outline vertex, slightly off.
        let v0 = b.outline.outer().unwrap().vertices[0];
        let at = map.point(v0);
        let hit = va
            .nearest(Point::new(at.x + 0.1, at.y - 0.1), 0.5, false)
            .unwrap();
        assert!(
            hit.sheet.dist(at) < 1e-9
                || hit.label.starts_with("pad")
                || hit.label.starts_with("hole")
        );
        // A pad.
        let fp = b.footprints.iter().find(|f| !f.pads.is_empty()).unwrap();
        let pad = &fp.pads[0];
        let ps = map.point(pad.position);
        let hit = va.nearest(ps, 0.3, false).unwrap();
        assert!(hit.sheet.dist(ps) < 1e-6, "{hit:?}");
        // Far away: nothing.
        assert!(va
            .nearest(Point::new(-1000.0, -1000.0), 1.0, false)
            .is_none());
        // Circular: a hole is hit anywhere inside it.
        if let Some(h) = b.holes().first() {
            let hs = map.point(h.position);
            let r = h.shape.tool_diameter() / 2.0 * map.scale;
            let hit = va
                .nearest(Point::new(hs.x + r * 0.5, hs.y), 0.2, true)
                .unwrap();
            assert!(hit.radius.is_some());
        }
    }
}
