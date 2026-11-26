//! Resolving anchors against a board model.

use kdraft_board::geom::{circle_through, Point};
use kdraft_board::outline::EdgeShape;
use kdraft_board::{Board, Hole};

use crate::model::Anchor;

/// Why an anchor could not be resolved. Callers mark the dependent item
/// as broken; anchors are never dropped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnchorError {
    #[error("no footprint {0}")]
    NoFootprint(String),
    #[error("footprint {reference} has no pad {number}")]
    NoPad { reference: String, number: String },
    #[error("no hole {0}")]
    NoHole(usize),
    #[error("board has no closed outline")]
    NoOutline,
    #[error("outline has no vertex {0}")]
    NoVertex(usize),
    #[error("outline has no edge {0}")]
    NoEdge(usize),
    #[error("outline edge {0} is not an arc or circle")]
    NotCircular(usize),
    #[error("anchor has no radius")]
    NoRadius,
    #[error("anchor missing")]
    Missing,
    #[error("unknown anchor kind")]
    Unknown,
}

/// Board positions of a dimension's anchors: from and to, or for radius
/// and diameter dimensions the centre and a point on the circle.
pub fn dimension_positions(
    d: &crate::model::Dimension,
    board: &Board,
    holes: &[Hole],
) -> Result<Vec<Point>, AnchorError> {
    use crate::model::DimensionKind as K;
    if matches!(d.kind, K::Radius | K::Radial | K::Diameter) {
        let a = d.from.as_ref().ok_or(AnchorError::Missing)?;
        let (c, r) = resolve_circle(a, board, holes)?;
        return Ok(vec![c, Point::new(c.x + r, c.y)]);
    }
    let a = resolve(d.from.as_ref().ok_or(AnchorError::Missing)?, board, holes)?;
    let b = resolve(d.to.as_ref().ok_or(AnchorError::Missing)?, board, holes)?;
    Ok(vec![a, b])
}

/// Stores the current anchor positions in every resolvable dimension's
/// `last`; broken ones keep theirs. Returns the number of broken ones.
pub fn refresh_dimension_positions(doc: &mut crate::model::Document, board: &Board) -> usize {
    let holes = board.holes();
    let mut broken = 0;
    for p in &mut doc.pages {
        for it in &mut p.items {
            if let crate::model::Item::Dimension(d) = it {
                match dimension_positions(d, board, &holes) {
                    Ok(v) => d.last = v,
                    Err(_) => broken += 1,
                }
            }
        }
    }
    broken
}

/// Centre and radius (board mm) of a circular anchor: a round hole (or
/// the smaller dimension of a slot), or an outline arc or circle.
pub fn resolve_circle(
    anchor: &Anchor,
    board: &Board,
    holes: &[Hole],
) -> Result<(Point, f64), AnchorError> {
    match anchor {
        Anchor::Hole(i) => holes
            .get(*i)
            .map(|h| (h.position, h.shape.tool_diameter() / 2.0))
            .ok_or(AnchorError::NoHole(*i)),
        Anchor::OutlineEdge(i) => {
            let prim = board
                .outline
                .primitives
                .get(*i)
                .ok_or(AnchorError::NoEdge(*i))?;
            match prim.shape {
                EdgeShape::Circle { center, radius } => Ok((center, radius)),
                EdgeShape::Arc { start, mid, end } => {
                    circle_through(start, mid, end).ok_or(AnchorError::NotCircular(*i))
                }
                _ => Err(AnchorError::NotCircular(*i)),
            }
        }
        Anchor::Unknown(_) => Err(AnchorError::Unknown),
        _ => Err(AnchorError::NoRadius),
    }
}

/// Board coordinates of an anchor. `holes` is the board's hole list
/// ([`Board::holes`]); `(outline-vertex N)` counts vertices of the outer
/// contour ([`kdraft_board::outline::Outline::outer`]).
pub fn resolve(anchor: &Anchor, board: &Board, holes: &[Hole]) -> Result<Point, AnchorError> {
    match anchor {
        Anchor::Pad { reference, number } => {
            let fp = board
                .footprint_by_reference(reference)
                .ok_or_else(|| AnchorError::NoFootprint(reference.clone()))?;
            fp.pads
                .iter()
                .find(|p| &p.number == number)
                .map(|p| p.position)
                .ok_or_else(|| AnchorError::NoPad {
                    reference: reference.clone(),
                    number: number.clone(),
                })
        }
        Anchor::Hole(i) => holes
            .get(*i)
            .map(|h| h.position)
            .ok_or(AnchorError::NoHole(*i)),
        Anchor::OutlineVertex(i) => {
            let outer = board.outline.outer().ok_or(AnchorError::NoOutline)?;
            outer
                .vertices
                .get(*i)
                .copied()
                .ok_or(AnchorError::NoVertex(*i))
        }
        Anchor::FpOrigin(r) => board
            .footprint_by_reference(r)
            .map(|f| f.position)
            .ok_or_else(|| AnchorError::NoFootprint(r.clone())),
        Anchor::ViewPoint(p) => Ok(*p),
        // Arcs and circles: the centre; other edges: the start point.
        Anchor::OutlineEdge(i) => {
            let prim = board
                .outline
                .primitives
                .get(*i)
                .ok_or(AnchorError::NoEdge(*i))?;
            Ok(match prim.shape {
                EdgeShape::Circle { center, .. } => center,
                EdgeShape::Arc { start, mid, end } => {
                    circle_through(start, mid, end).map_or(start, |(c, _)| c)
                }
                EdgeShape::Line { start, .. } | EdgeShape::Rect { start, .. } => start,
                EdgeShape::Curve { points } => points[0],
                EdgeShape::Poly { ref items } => match items.first() {
                    Some(kdraft_board::outline::PolyItem::Point { at }) => *at,
                    Some(kdraft_board::outline::PolyItem::Arc { start, .. }) => *start,
                    None => return Err(AnchorError::NoEdge(*i)),
                },
            })
        }
        Anchor::Unknown(_) => Err(AnchorError::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn resolves_against_fixture() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad9-minimal.kicad_pcb");
        let b = Board::load(path).unwrap();
        let holes = b.holes();
        let pad = Anchor::Pad {
            reference: "J1".into(),
            number: "2".into(),
        };
        assert!(resolve(&pad, &b, &holes)
            .unwrap()
            .approx_eq(Point::new(40.0, 12.46), 1e-9));
        assert_eq!(
            resolve(&Anchor::FpOrigin("C1".into()), &b, &holes),
            Ok(Point::new(30.0, 10.0))
        );
        assert_eq!(
            resolve(&Anchor::OutlineVertex(3), &b, &holes),
            Ok(Point::new(45.0, 30.0))
        );
        assert_eq!(
            resolve(&Anchor::Hole(2), &b, &holes),
            Ok(Point::new(5.0, 5.0))
        );
        assert_eq!(
            resolve(&Anchor::ViewPoint(Point::new(1.0, 2.0)), &b, &holes),
            Ok(Point::new(1.0, 2.0))
        );
        assert_eq!(
            resolve(&Anchor::OutlineVertex(5), &b, &holes),
            Err(AnchorError::NoVertex(5))
        );
        assert_eq!(
            resolve(&Anchor::Hole(99), &b, &holes),
            Err(AnchorError::NoHole(99))
        );
        assert_eq!(
            resolve(&Anchor::FpOrigin("X9".into()), &b, &holes),
            Err(AnchorError::NoFootprint("X9".into()))
        );
        assert!(matches!(
            resolve(
                &Anchor::Pad {
                    reference: "J1".into(),
                    number: "7".into()
                },
                &b,
                &holes
            ),
            Err(AnchorError::NoPad { .. })
        ));
        let unknown = Anchor::Unknown(kdraft_sexpr::Value::sym("x"));
        assert_eq!(resolve(&unknown, &b, &holes), Err(AnchorError::Unknown));
    }
}
