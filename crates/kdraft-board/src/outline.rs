//! Board outline from Edge.Cuts graphics and chaining into contours.

use std::collections::HashMap;

use serde::Serialize;

use crate::geom::{arc_bbox, union_opt, BBox, Point};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PolyItem {
    Point {
        at: Point,
    },
    Arc {
        start: Point,
        mid: Point,
        end: Point,
    },
}

/// One Edge.Cuts graphic in board coordinates.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EdgeShape {
    Line {
        start: Point,
        end: Point,
    },
    Arc {
        start: Point,
        mid: Point,
        end: Point,
    },
    Circle {
        center: Point,
        radius: f64,
    },
    /// Axis-aligned in the coordinates it was drawn in. A rectangle inside a
    /// rotated footprint is stored as a four-point polygon instead.
    Rect {
        start: Point,
        end: Point,
    },
    Poly {
        items: Vec<PolyItem>,
    },
    /// Cubic Bézier; the box is taken over the control points.
    Curve {
        points: [Point; 4],
    },
}

impl EdgeShape {
    /// Bounding box. Exact for lines, arcs, circles, rectangles and polygons;
    /// for curves the box of the control points (which contains the curve).
    pub fn bbox(&self) -> Option<BBox> {
        match self {
            EdgeShape::Line { start, end } => BBox::from_points([*start, *end]),
            EdgeShape::Arc { start, mid, end } => Some(arc_bbox(*start, *mid, *end)),
            EdgeShape::Circle { center, radius } => Some(BBox {
                min: Point::new(center.x - radius, center.y - radius),
                max: Point::new(center.x + radius, center.y + radius),
            }),
            EdgeShape::Rect { start, end } => BBox::from_points([*start, *end]),
            EdgeShape::Poly { items } => items.iter().fold(None, |acc, it| {
                let b = match it {
                    PolyItem::Point { at } => BBox::from_point(*at),
                    PolyItem::Arc { start, mid, end } => arc_bbox(*start, *mid, *end),
                };
                union_opt(acc, Some(b))
            }),
            EdgeShape::Curve { points } => BBox::from_points(points.iter().copied()),
        }
    }

    /// End points of an open element (line, arc, curve).
    fn ends(&self) -> Option<(Point, Point)> {
        match self {
            EdgeShape::Line { start, end } | EdgeShape::Arc { start, end, .. } => {
                Some((*start, *end))
            }
            EdgeShape::Curve { points } => Some((points[0], points[3])),
            _ => None,
        }
    }

    /// Vertices of a shape that is closed by itself.
    fn closed_vertices(&self) -> Option<Vec<Point>> {
        match self {
            EdgeShape::Circle { .. } => Some(Vec::new()),
            EdgeShape::Rect { start, end } => Some(vec![
                *start,
                Point::new(end.x, start.y),
                *end,
                Point::new(start.x, end.y),
            ]),
            EdgeShape::Poly { items } => {
                let mut v: Vec<Point> = Vec::new();
                for it in items {
                    match it {
                        PolyItem::Point { at } => push_vertex(&mut v, *at),
                        PolyItem::Arc { start, end, .. } => {
                            push_vertex(&mut v, *start);
                            push_vertex(&mut v, *end);
                        }
                    }
                }
                if v.len() > 1 && v.first().zip(v.last()).is_some_and(|(a, b)| a == b) {
                    v.pop();
                }
                Some(v)
            }
            _ => None,
        }
    }
}

fn push_vertex(v: &mut Vec<Point>, p: Point) {
    if v.last() != Some(&p) {
        v.push(p);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EdgeSource {
    Board,
    /// Index into `Board::footprints`.
    Footprint {
        index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EdgePrimitive {
    pub shape: EdgeShape,
    pub source: EdgeSource,
    pub uuid: Option<String>,
}

/// One element of a contour: a primitive, possibly walked end to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ContourElement {
    pub primitive: usize,
    pub reversed: bool,
}

/// A closed loop of outline primitives.
///
/// `vertices[k]` is where element `k` starts in walking order (for a
/// single closed shape: its own corner list; empty for a circle). Contours
/// start at the lowest-indexed primitive and walk it in file direction, so
/// vertex numbering is stable as long as the file's primitive order is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contour {
    pub elements: Vec<ContourElement>,
    pub vertices: Vec<Point>,
    pub bbox: Option<BBox>,
}

/// A chain of open primitives whose ends do not meet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenChain {
    pub elements: Vec<ContourElement>,
    pub start: Point,
    pub end: Point,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Outline {
    pub primitives: Vec<EdgePrimitive>,
    pub bbox: Option<BBox>,
    pub contours: Vec<Contour>,
    pub open_chains: Vec<OpenChain>,
    pub tolerance: f64,
}

/// Default end-point matching tolerance in mm.
pub const DEFAULT_CHAIN_TOLERANCE: f64 = 0.01;

impl Outline {
    pub fn build(primitives: Vec<EdgePrimitive>, tolerance: f64) -> Outline {
        let bbox = primitives
            .iter()
            .fold(None, |acc, p| union_opt(acc, p.shape.bbox()));
        let (contours, open_chains) = chain(&primitives, tolerance);
        Outline {
            primitives,
            bbox,
            contours,
            open_chains,
            tolerance,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.open_chains.is_empty() && !self.contours.is_empty()
    }

    /// The contour with the largest bounding box area: the board edge for
    /// normal boards, the others being cut-outs.
    pub fn outer(&self) -> Option<&Contour> {
        self.contours.iter().max_by(|a, b| {
            let area = |c: &Contour| c.bbox.map_or(0.0, |b| b.width() * b.height());
            area(a).total_cmp(&area(b))
        })
    }
}

type Cell = (i64, i64);

struct EndIndex {
    tol: f64,
    cells: HashMap<Cell, Vec<(usize, bool)>>,
}

impl EndIndex {
    fn cell(&self, p: Point) -> Cell {
        // Saturating float-to-int casts keep huge coordinates harmless.
        (
            (p.x / self.tol).floor() as i64,
            (p.y / self.tol).floor() as i64,
        )
    }

    fn insert(&mut self, p: Point, entry: (usize, bool)) {
        let c = self.cell(p);
        self.cells.entry(c).or_default().push(entry);
    }

    /// Unused element with an end within tolerance of `p`, lowest index first.
    /// Returns (index, true if the matching end is the element's end point).
    fn take_near(
        &mut self,
        p: Point,
        ends: &[(Point, Point)],
        used: &[bool],
    ) -> Option<(usize, bool)> {
        let (cx, cy) = self.cell(p);
        let mut best: Option<(usize, bool)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let Some(list) = self
                    .cells
                    .get(&(cx.saturating_add(dx), cy.saturating_add(dy)))
                else {
                    continue;
                };
                for &(i, at_end) in list {
                    if used[i] {
                        continue;
                    }
                    let q = if at_end { ends[i].1 } else { ends[i].0 };
                    if q.dist(p) <= self.tol && best.is_none_or(|b| (i, at_end) < b) {
                        best = Some((i, at_end));
                    }
                }
            }
        }
        best
    }
}

fn chain(prims: &[EdgePrimitive], tol: f64) -> (Vec<Contour>, Vec<OpenChain>) {
    let tol = if tol > 0.0 {
        tol
    } else {
        DEFAULT_CHAIN_TOLERANCE
    };
    let mut contours = Vec::new();
    let mut open = Vec::new();

    // Open elements, indexed by position in `open_ids`.
    let open_ids: Vec<usize> = prims
        .iter()
        .enumerate()
        .filter(|(_, p)| p.shape.ends().is_some())
        .map(|(i, _)| i)
        .collect();
    let ends: Vec<(Point, Point)> = open_ids
        .iter()
        .filter_map(|&i| prims[i].shape.ends())
        .collect();
    let mut index = EndIndex {
        tol,
        cells: HashMap::new(),
    };
    for (k, &(s, e)) in ends.iter().enumerate() {
        index.insert(s, (k, false));
        index.insert(e, (k, true));
    }
    let mut used = vec![false; ends.len()];

    let mut chained: Vec<(usize, Contour)> = Vec::new();
    for seed in 0..ends.len() {
        if used[seed] {
            continue;
        }
        used[seed] = true;
        let mut elems = vec![ContourElement {
            primitive: open_ids[seed],
            reversed: false,
        }];
        let start = ends[seed].0;
        let mut cursor = ends[seed].1;
        let mut closed =
            start.dist(cursor) <= tol && !is_degenerate_loop(&prims[open_ids[seed]].shape);
        // Walk forward from the seed's end.
        while !closed {
            let Some((k, at_end)) = index.take_near(cursor, &ends, &used) else {
                break;
            };
            used[k] = true;
            elems.push(ContourElement {
                primitive: open_ids[k],
                reversed: at_end,
            });
            cursor = if at_end { ends[k].0 } else { ends[k].1 };
            closed = cursor.dist(start) <= tol;
        }
        if closed {
            let vertices = elems
                .iter()
                .map(|e| {
                    let (s, en) = prims[e.primitive].shape.ends().unwrap_or_default();
                    if e.reversed {
                        en
                    } else {
                        s
                    }
                })
                .collect();
            let bbox = elems.iter().fold(None, |acc, e| {
                union_opt(acc, prims[e.primitive].shape.bbox())
            });
            chained.push((
                open_ids[seed],
                Contour {
                    elements: elems,
                    vertices,
                    bbox,
                },
            ));
        } else {
            // Extend backwards from the seed's start, then report open.
            let mut head = start;
            let mut front: Vec<ContourElement> = Vec::new();
            while let Some((k, at_end)) = index.take_near(head, &ends, &used) {
                used[k] = true;
                // Walked so that it ends at `head`.
                front.push(ContourElement {
                    primitive: open_ids[k],
                    reversed: !at_end,
                });
                head = if at_end { ends[k].0 } else { ends[k].1 };
            }
            front.reverse();
            front.extend(elems);
            open.push(OpenChain {
                elements: front,
                start: head,
                end: cursor,
            });
        }
    }

    for (i, p) in prims.iter().enumerate() {
        if let Some(vertices) = p.shape.closed_vertices() {
            chained.push((
                i,
                Contour {
                    elements: vec![ContourElement {
                        primitive: i,
                        reversed: false,
                    }],
                    vertices,
                    bbox: p.shape.bbox(),
                },
            ));
        }
    }
    chained.sort_by_key(|(i, _)| *i);
    contours.extend(chained.into_iter().map(|(_, c)| c));
    (contours, open)
}

/// A single line whose ends coincide is not a loop.
fn is_degenerate_loop(shape: &EdgeShape) -> bool {
    matches!(shape, EdgeShape::Line { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: (f64, f64), b: (f64, f64)) -> EdgePrimitive {
        EdgePrimitive {
            shape: EdgeShape::Line {
                start: Point::new(a.0, a.1),
                end: Point::new(b.0, b.1),
            },
            source: EdgeSource::Board,
            uuid: None,
        }
    }

    #[test]
    fn chains_shuffled_and_reversed_lines() {
        // Square 0..10 drawn out of order with mixed directions and a tiny gap.
        let prims = vec![
            line((0.0, 0.0), (10.0, 0.0)),
            line((0.0, 10.0), (10.0, 10.0)),
            line((0.0, 10.0), (0.0, 0.004)),
            line((10.0, 0.0), (10.0, 10.0)),
        ];
        let o = Outline::build(prims, 0.01);
        assert!(o.is_closed(), "{o:?}");
        assert_eq!(o.contours.len(), 1);
        let c = &o.contours[0];
        assert_eq!(c.vertices.len(), 4);
        let order: Vec<_> = c
            .elements
            .iter()
            .map(|e| (e.primitive, e.reversed))
            .collect();
        assert_eq!(order, vec![(0, false), (3, false), (1, true), (2, false)]);
        assert_eq!(c.vertices[0], Point::new(0.0, 0.0));
        assert_eq!(c.vertices[2], Point::new(10.0, 10.0));
        let b = o.bbox.unwrap();
        assert_eq!((b.width(), b.height()), (10.0, 10.0));
    }

    #[test]
    fn reports_open_chain_instead_of_failing() {
        let prims = vec![
            line((0.0, 0.0), (10.0, 0.0)),
            line((10.0, 0.0), (10.0, 10.0)),
            line((10.0, 10.0), (0.0, 10.0)),
        ];
        let o = Outline::build(prims, 0.01);
        assert!(!o.is_closed());
        assert!(o.contours.is_empty());
        assert_eq!(o.open_chains.len(), 1);
        let ch = &o.open_chains[0];
        assert_eq!(ch.elements.len(), 3);
        assert_eq!(ch.start, Point::new(0.0, 0.0));
        assert_eq!(ch.end, Point::new(0.0, 10.0));
    }

    #[test]
    fn open_chain_extends_backwards_from_seed() {
        let prims = vec![line((5.0, 0.0), (10.0, 0.0)), line((0.0, 0.0), (5.0, 0.0))];
        let o = Outline::build(prims, 0.01);
        let ch = &o.open_chains[0];
        assert_eq!(ch.start, Point::new(0.0, 0.0));
        assert_eq!(ch.end, Point::new(10.0, 0.0));
        assert_eq!(ch.elements[0].primitive, 1);
    }

    #[test]
    fn closed_shapes_and_arcs() {
        let prims = vec![
            EdgePrimitive {
                shape: EdgeShape::Circle {
                    center: Point::new(5.0, 5.0),
                    radius: 1.0,
                },
                source: EdgeSource::Board,
                uuid: None,
            },
            line((0.0, 0.0), (10.0, 0.0)),
            EdgePrimitive {
                // Lower half circle back to the start, bulging to +Y.
                shape: EdgeShape::Arc {
                    start: Point::new(10.0, 0.0),
                    mid: Point::new(5.0, 5.0),
                    end: Point::new(0.0, 0.0),
                },
                source: EdgeSource::Board,
                uuid: None,
            },
            EdgePrimitive {
                shape: EdgeShape::Rect {
                    start: Point::new(20.0, 0.0),
                    end: Point::new(30.0, 4.0),
                },
                source: EdgeSource::Board,
                uuid: None,
            },
        ];
        let o = Outline::build(prims, 0.01);
        assert!(o.is_closed());
        assert_eq!(o.contours.len(), 3);
        assert!(o.contours[0].vertices.is_empty());
        assert_eq!(o.contours[1].vertices.len(), 2);
        assert_eq!(o.contours[2].vertices.len(), 4);
        let b = o.contours[1].bbox.unwrap();
        assert!((b.max.y - 5.0).abs() < 1e-9);
        let outer = o.outer().unwrap();
        assert_eq!(outer.elements[0].primitive, 1);
    }

    #[test]
    fn poly_vertices_drop_closing_duplicate() {
        let items = [(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (0.0, 0.0)]
            .iter()
            .map(|&(x, y)| PolyItem::Point {
                at: Point::new(x, y),
            })
            .collect();
        let o = Outline::build(
            vec![EdgePrimitive {
                shape: EdgeShape::Poly { items },
                source: EdgeSource::Board,
                uuid: None,
            }],
            0.01,
        );
        assert_eq!(o.contours[0].vertices.len(), 3);
    }
}
