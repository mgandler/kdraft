//! Interactive placement tools: plain state machines over sheet points and
//! picked anchors. The egui layer feeds pointer clicks, moves and Escape
//! and executes the [`Placement`] that comes out as one engine command
//! (one undo step).

use kdraft_board::geom::Point;
use kdraft_doc::{Anchor, Id};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewTool {
    Fabrication,
    Assembly,
    Drill,
}

impl ViewTool {
    pub fn kind(self) -> &'static str {
        match self {
            ViewTool::Fabrication => "fabrication",
            ViewTool::Assembly => "assembly",
            ViewTool::Drill => "drill",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TableTool {
    Drill,
    Stackup,
    Bom,
}

impl TableTool {
    pub fn kind(self) -> &'static str {
        match self {
            TableTool::Drill => "drill",
            TableTool::Stackup => "stackup",
            TableTool::Bom => "bom",
        }
    }
}

/// What a callout placed with the tool shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CalloutTool {
    /// The next number (label).
    #[default]
    Number,
    /// The part's BOM item number.
    Bom,
    /// A note's number.
    Note,
}

impl CalloutTool {
    /// `kind` argument of `item.add_callout`.
    pub fn kind(self) -> &'static str {
        match self {
            CalloutTool::Number => "label",
            CalloutTool::Bom => "bom",
            CalloutTool::Note => "note",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinearKind {
    Horizontal,
    Vertical,
    Aligned,
}

impl LinearKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LinearKind::Horizontal => "horizontal",
            LinearKind::Vertical => "vertical",
            LinearKind::Aligned => "aligned",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ToolKind {
    #[default]
    Select,
    Pan,
    View(ViewTool),
    Table(TableTool),
    DimLinear(LinearKind),
    DimRadius {
        diameter: bool,
    },
    DimOrdinate,
    Callout(CalloutTool),
    Notes,
    Text,
    Line,
    Rect,
    Polyline,
    Circle,
}

impl ToolKind {
    /// Tools that pick board anchors (and show anchor snapping).
    pub fn picks_anchors(self) -> bool {
        matches!(
            self,
            ToolKind::DimLinear(_)
                | ToolKind::DimRadius { .. }
                | ToolKind::DimOrdinate
                | ToolKind::Callout(_)
        )
    }

    /// Only circular anchors (holes, outline arcs and circles).
    pub fn circular_only(self) -> bool {
        matches!(self, ToolKind::DimRadius { .. })
    }

    pub fn is_select(self) -> bool {
        matches!(self, ToolKind::Select)
    }
}

/// An anchor picked in a view.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub view: Id,
    pub anchor: Anchor,
    /// Sheet position.
    pub sheet: Point,
    /// "pad U7.3", "hole 12", "outline vertex 4".
    pub label: String,
    /// Sheet radius for circular anchors.
    pub radius: Option<f64>,
}

/// What a finished tool interaction creates.
#[derive(Debug, Clone, PartialEq)]
pub enum Placement {
    View {
        kind: ViewTool,
        at: Point,
    },
    Table {
        kind: TableTool,
        at: Point,
    },
    Notes {
        at: Point,
    },
    Text {
        at: Point,
    },
    Line {
        start: Point,
        end: Point,
    },
    Rect {
        start: Point,
        end: Point,
    },
    Polyline {
        points: Vec<Point>,
    },
    Circle {
        center: Point,
        radius: f64,
    },
    DimLinear {
        view: Id,
        from: Anchor,
        to: Anchor,
        kind: LinearKind,
        offset: f64,
    },
    DimRadius {
        view: Id,
        anchor: Anchor,
        diameter: bool,
        angle: f64,
    },
    DimOrdinate {
        view: Id,
        datum: Anchor,
        to: Anchor,
        axis: &'static str,
        offset: f64,
    },
    Callout {
        view: Id,
        anchor: Anchor,
        at: Point,
        variant: CalloutTool,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Stage {
    #[default]
    Start,
    /// One anchor picked (linear: from; radius: the circle; ordinate:
    /// datum; callout: the anchor) or the first corner of a line/rect.
    One(Option<Pick>, Point),
    /// Two anchors picked (linear: from, to; ordinate: datum, to).
    Two(Pick, Pick),
}

/// What the app should do after an input.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Nothing,
    Place(Placement),
    /// A hint for the status line ("pick a pad, hole or outline vertex").
    Hint(&'static str),
    /// Back to the select tool.
    Exit,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tool {
    pub kind: ToolKind,
    pub stage: Stage,
    /// Last pointer position (sheet mm), for previews.
    pub pointer: Option<Point>,
    /// Polyline points so far.
    pub poly: Vec<Point>,
}

/// Smallest dimension offset the tool produces, sheet mm.
pub const MIN_OFFSET: f64 = 1.0;

/// Offset of a linear dimension through `q` for anchors at sheet points
/// `p1`, `p2` (the engine's convention: positive is above for horizontal,
/// right for vertical, left of p1->p2 for aligned).
pub fn linear_offset(p1: Point, p2: Point, kind: LinearKind, q: Point) -> f64 {
    let n0 = match kind {
        LinearKind::Horizontal => Point::new(0.0, -1.0),
        LinearKind::Vertical => Point::new(1.0, 0.0),
        LinearKind::Aligned => {
            let d = p2 - p1;
            let l = (d.x * d.x + d.y * d.y).sqrt().max(1e-9);
            Point::new(d.y / l, -d.x / l)
        }
    };
    let dot = |a: Point, b: Point| a.x * b.x + a.y * b.y;
    let pos = dot(q, n0) - dot(p1, n0).max(dot(p2, n0));
    let neg_n = Point::new(-n0.x, -n0.y);
    let neg = dot(q, neg_n) - dot(p1, neg_n).max(dot(p2, neg_n));
    if pos >= neg {
        pos.max(MIN_OFFSET)
    } else {
        -neg.max(MIN_OFFSET)
    }
}

/// Axis and offset of an ordinate dimension of the point `to` from
/// `datum` with the pointer at `q`: a mostly vertical pointer move from
/// `to` shows the x coordinate.
pub fn ordinate_axis_offset(datum: Point, to: Point, q: Point) -> (&'static str, f64) {
    let d = q - to;
    if d.y.abs() >= d.x.abs() {
        ("x", q.y - datum.y)
    } else {
        ("y", datum.x - q.x)
    }
}

/// Leader angle (degrees CCW on the sheet) from `c` towards `q`.
pub fn leader_angle(c: Point, q: Point) -> f64 {
    let a = (-(q.y - c.y)).atan2(q.x - c.x).to_degrees();
    if a < 0.0 {
        a + 360.0
    } else {
        a
    }
}

impl Tool {
    pub fn new(kind: ToolKind) -> Tool {
        Tool {
            kind,
            stage: Stage::Start,
            pointer: None,
            poly: Vec::new(),
        }
    }

    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Start
    }

    /// Pointer moved.
    pub fn moved(&mut self, p: Point) {
        self.pointer = Some(p);
    }

    /// Escape: one stage back; from the start, leave the tool.
    pub fn escape(&mut self) -> Out {
        self.poly.clear();
        match std::mem::take(&mut self.stage) {
            Stage::Start => Out::Exit,
            _ => Out::Nothing,
        }
    }

    /// A click at sheet point `p` with the anchor under it (snapped), if
    /// any.
    pub fn click(&mut self, p: Point, pick: Option<Pick>) -> Out {
        self.pointer = Some(p);
        use ToolKind as K;
        match self.kind {
            K::Select | K::Pan => Out::Nothing,
            K::View(kind) => self.done(Placement::View { kind, at: p }),
            K::Table(kind) => self.done(Placement::Table { kind, at: p }),
            K::Notes => self.done(Placement::Notes { at: p }),
            K::Text => self.done(Placement::Text { at: p }),
            K::Polyline => {
                if self.poly.last().is_some_and(|q| q.dist(p) < 1e-6) {
                    return Out::Nothing;
                }
                self.poly.push(p);
                self.stage = Stage::One(None, self.poly[0]);
                Out::Nothing
            }
            K::Circle => match std::mem::take(&mut self.stage) {
                Stage::One(_, c) => {
                    let r = c.dist(p);
                    if r < 1e-6 {
                        self.stage = Stage::One(None, c);
                        return Out::Nothing;
                    }
                    self.done(Placement::Circle {
                        center: c,
                        radius: r,
                    })
                }
                _ => {
                    self.stage = Stage::One(None, p);
                    Out::Nothing
                }
            },
            K::Line | K::Rect => match std::mem::take(&mut self.stage) {
                Stage::One(_, start) => {
                    if start.dist(p) < 1e-6 {
                        self.stage = Stage::One(None, start);
                        return Out::Nothing;
                    }
                    let pl = if self.kind == K::Line {
                        Placement::Line { start, end: p }
                    } else {
                        Placement::Rect { start, end: p }
                    };
                    self.done(pl)
                }
                _ => {
                    self.stage = Stage::One(None, p);
                    Out::Nothing
                }
            },
            K::DimLinear(kind) => match std::mem::take(&mut self.stage) {
                Stage::Start => self.first(pick, p, "pick a pad, hole, outline vertex or origin"),
                Stage::One(Some(a), q) => match pick {
                    Some(b) if b.view == a.view && b.anchor != a.anchor => {
                        self.stage = Stage::Two(a, b);
                        Out::Nothing
                    }
                    Some(_) => {
                        self.stage = Stage::One(Some(a), q);
                        Out::Hint("pick a second anchor in the same view")
                    }
                    None => {
                        self.stage = Stage::One(Some(a), q);
                        Out::Hint("pick a second anchor")
                    }
                },
                Stage::Two(a, b) => {
                    let offset = linear_offset(a.sheet, b.sheet, kind, p);
                    self.done(Placement::DimLinear {
                        view: a.view,
                        from: a.anchor,
                        to: b.anchor,
                        kind,
                        offset,
                    })
                }
                Stage::One(None, _) => Out::Nothing,
            },
            K::DimRadius { diameter } => match std::mem::take(&mut self.stage) {
                Stage::One(Some(a), _) => {
                    let angle = leader_angle(a.sheet, p);
                    self.done(Placement::DimRadius {
                        view: a.view,
                        anchor: a.anchor,
                        diameter,
                        angle,
                    })
                }
                _ => match pick {
                    Some(a) if a.radius.is_some() => {
                        self.stage = Stage::One(Some(a), p);
                        Out::Nothing
                    }
                    _ => Out::Hint("pick a hole, an outline arc or a circle"),
                },
            },
            K::DimOrdinate => match std::mem::take(&mut self.stage) {
                Stage::Start => self.first(pick, p, "pick the datum (origin) anchor"),
                Stage::One(Some(d), q) => match pick {
                    Some(b) if b.view == d.view && b.anchor != d.anchor => {
                        self.stage = Stage::Two(d, b);
                        Out::Nothing
                    }
                    _ => {
                        self.stage = Stage::One(Some(d), q);
                        Out::Hint("pick the anchor to measure, in the same view")
                    }
                },
                Stage::Two(d, b) => {
                    let (axis, offset) = ordinate_axis_offset(d.sheet, b.sheet, p);
                    let pl = Placement::DimOrdinate {
                        view: d.view.clone(),
                        datum: d.anchor.clone(),
                        to: b.anchor,
                        axis,
                        offset,
                    };
                    // Keep the datum for the next ordinate.
                    self.stage = Stage::One(Some(d), p);
                    Out::Place(pl)
                }
                Stage::One(None, _) => Out::Nothing,
            },
            K::Callout(variant) => match std::mem::take(&mut self.stage) {
                Stage::One(Some(a), _) => self.done(Placement::Callout {
                    view: a.view,
                    anchor: a.anchor,
                    at: p,
                    variant,
                }),
                _ => self.first(
                    pick,
                    p,
                    "pick the part (pad or origin) the callout points at",
                ),
            },
        }
    }

    fn first(&mut self, pick: Option<Pick>, p: Point, hint: &'static str) -> Out {
        match pick {
            Some(a) => {
                self.stage = Stage::One(Some(a), p);
                Out::Nothing
            }
            None => Out::Hint(hint),
        }
    }

    /// Finishes a polyline (double-click or Enter): at least two points.
    pub fn finish(&mut self) -> Out {
        if self.kind != ToolKind::Polyline {
            return Out::Nothing;
        }
        let mut pts = std::mem::take(&mut self.poly);
        pts.dedup_by(|a, b| a.dist(*b) < 1e-6);
        self.stage = Stage::Start;
        if pts.len() >= 2 {
            Out::Place(Placement::Polyline { points: pts })
        } else {
            Out::Hint("a polyline needs two or more points")
        }
    }

    fn done(&mut self, pl: Placement) -> Out {
        self.stage = Stage::Start;
        Out::Place(pl)
    }

    /// The preview of a linear dimension's offset at the pointer.
    pub fn preview_offset(&self) -> Option<(Pick, Pick, f64)> {
        match (&self.kind, &self.stage, self.pointer) {
            (ToolKind::DimLinear(k), Stage::Two(a, b), Some(q)) => {
                Some((a.clone(), b.clone(), linear_offset(a.sheet, b.sheet, *k, q)))
            }
            _ => None,
        }
    }

    /// Hint for the current stage.
    pub fn stage_hint(&self) -> &'static str {
        use ToolKind as K;
        match (&self.kind, &self.stage) {
            (K::View(_) | K::Table(_) | K::Notes | K::Text, _) => "click to place",
            (K::Line | K::Rect, Stage::Start) => "click the first corner",
            (K::Line | K::Rect, _) => "click the second corner",
            (K::Polyline, Stage::Start) => "click the first point",
            (K::Polyline, _) => "click more points; double-click or Enter finishes",
            (K::Circle, Stage::Start) => "click the centre",
            (K::Circle, _) => "click a point on the circle",
            (K::DimLinear(_), Stage::Start) => "pick the first anchor",
            (K::DimLinear(_), Stage::One(..)) => "pick the second anchor",
            (K::DimLinear(_), Stage::Two(..)) => "click to set the offset",
            (K::DimRadius { .. }, Stage::Start) => "pick a hole, arc or circle",
            (K::DimRadius { .. }, _) => "click to set the leader direction",
            (K::DimOrdinate, Stage::Start) => "pick the datum",
            (K::DimOrdinate, Stage::One(..)) => "pick the anchor to measure",
            (K::DimOrdinate, Stage::Two(..)) => "click to place the leader",
            (K::Callout(_), Stage::Start) => "pick the part",
            (K::Callout(_), _) => "click to place the balloon",
            (K::Select | K::Pan, _) => "",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(v: &str, a: Anchor, x: f64, y: f64) -> Pick {
        Pick {
            view: Id(v.into()),
            anchor: a,
            sheet: Point::new(x, y),
            label: String::new(),
            radius: None,
        }
    }

    #[test]
    fn view_and_table_place_with_one_click() {
        let mut t = Tool::new(ToolKind::View(ViewTool::Assembly));
        assert_eq!(
            t.click(Point::new(10.0, 20.0), None),
            Out::Place(Placement::View {
                kind: ViewTool::Assembly,
                at: Point::new(10.0, 20.0)
            })
        );
        assert!(t.is_idle());
        assert_eq!(t.escape(), Out::Exit);
    }

    #[test]
    fn linear_dimension_two_anchors_then_offset() {
        let mut t = Tool::new(ToolKind::DimLinear(LinearKind::Horizontal));
        // Clicking empty space only hints.
        assert!(matches!(t.click(Point::ORIGIN, None), Out::Hint(_)));
        let a = pick("v", Anchor::OutlineVertex(0), 10.0, 50.0);
        let b = pick("v", Anchor::OutlineVertex(1), 60.0, 40.0);
        assert_eq!(t.click(a.sheet, Some(a.clone())), Out::Nothing);
        // Same anchor again or another view: not accepted.
        assert!(matches!(t.click(a.sheet, Some(a.clone())), Out::Hint(_)));
        let other = pick("w", Anchor::Hole(1), 0.0, 0.0);
        assert!(matches!(t.click(other.sheet, Some(other)), Out::Hint(_)));
        assert_eq!(t.click(b.sheet, Some(b.clone())), Out::Nothing);
        // Pointer 8 mm above the topmost point (y = 40): offset 8.
        t.moved(Point::new(30.0, 32.0));
        assert!((t.preview_offset().unwrap().2 - 8.0).abs() < 1e-9);
        let out = t.click(Point::new(30.0, 32.0), None);
        assert_eq!(
            out,
            Out::Place(Placement::DimLinear {
                view: Id("v".into()),
                from: Anchor::OutlineVertex(0),
                to: Anchor::OutlineVertex(1),
                kind: LinearKind::Horizontal,
                offset: 8.0
            })
        );
        assert!(t.is_idle());
    }

    #[test]
    fn escape_steps_back() {
        let mut t = Tool::new(ToolKind::DimLinear(LinearKind::Vertical));
        let a = pick("v", Anchor::Hole(0), 0.0, 0.0);
        t.click(a.sheet, Some(a));
        assert_eq!(t.escape(), Out::Nothing);
        assert!(t.is_idle());
        assert_eq!(t.escape(), Out::Exit);
    }

    #[test]
    fn offsets_follow_the_engine_convention() {
        let (p1, p2) = (Point::new(0.0, 10.0), Point::new(20.0, 12.0));
        assert_eq!(
            linear_offset(p1, p2, LinearKind::Horizontal, Point::new(5.0, 4.0)),
            6.0
        );
        assert_eq!(
            linear_offset(p1, p2, LinearKind::Horizontal, Point::new(5.0, 20.0)),
            -8.0
        );
        // Between the points: the minimum on the nearer side.
        assert_eq!(
            linear_offset(p1, p2, LinearKind::Horizontal, Point::new(5.0, 10.5)).abs(),
            MIN_OFFSET
        );
        assert_eq!(
            linear_offset(p1, p2, LinearKind::Vertical, Point::new(30.0, 0.0)),
            10.0
        );
        assert_eq!(
            linear_offset(p1, p2, LinearKind::Vertical, Point::new(-3.0, 0.0)),
            -3.0
        );
        assert_eq!(
            ordinate_axis_offset(
                Point::new(0.0, 100.0),
                Point::new(30.0, 60.0),
                Point::new(31.0, 110.0)
            ),
            ("x", 10.0)
        );
        assert_eq!(
            ordinate_axis_offset(
                Point::new(0.0, 100.0),
                Point::new(30.0, 60.0),
                Point::new(-8.0, 61.0)
            ),
            ("y", 8.0)
        );
        assert!((leader_angle(Point::ORIGIN, Point::new(1.0, -1.0)) - 45.0).abs() < 1e-9);
        assert!((leader_angle(Point::ORIGIN, Point::new(0.0, 1.0)) - 270.0).abs() < 1e-9);
    }

    #[test]
    fn radius_needs_a_circle_and_ordinate_keeps_the_datum() {
        let mut t = Tool::new(ToolKind::DimRadius { diameter: true });
        let plain = pick("v", Anchor::OutlineVertex(0), 0.0, 0.0);
        assert!(matches!(t.click(plain.sheet, Some(plain)), Out::Hint(_)));
        let mut hole = pick("v", Anchor::Hole(3), 10.0, 10.0);
        hole.radius = Some(1.0);
        assert_eq!(t.click(hole.sheet, Some(hole)), Out::Nothing);
        match t.click(Point::new(20.0, 0.0), None) {
            Out::Place(Placement::DimRadius {
                diameter, angle, ..
            }) => {
                assert!(diameter);
                assert!((angle - 45.0).abs() < 1e-9);
            }
            o => panic!("{o:?}"),
        }
        let mut t = Tool::new(ToolKind::DimOrdinate);
        let d = pick("v", Anchor::OutlineVertex(0), 0.0, 100.0);
        let b = pick("v", Anchor::Hole(1), 30.0, 60.0);
        t.click(d.sheet, Some(d));
        t.click(b.sheet, Some(b.clone()));
        assert!(matches!(
            t.click(Point::new(30.0, 108.0), None),
            Out::Place(Placement::DimOrdinate { axis: "x", .. })
        ));
        // Datum kept: the next anchor goes straight to stage two.
        t.click(b.sheet, Some(b));
        assert!(matches!(t.stage, Stage::Two(..)));
    }

    #[test]
    fn line_and_callout() {
        let mut t = Tool::new(ToolKind::Line);
        assert_eq!(t.click(Point::new(1.0, 1.0), None), Out::Nothing);
        assert_eq!(t.click(Point::new(1.0, 1.0), None), Out::Nothing);
        assert_eq!(
            t.click(Point::new(5.0, 1.0), None),
            Out::Place(Placement::Line {
                start: Point::new(1.0, 1.0),
                end: Point::new(5.0, 1.0)
            })
        );
        let mut t = Tool::new(ToolKind::Callout(CalloutTool::Number));
        let a = pick(
            "v",
            Anchor::Pad {
                reference: "U1".into(),
                number: "1".into(),
            },
            5.0,
            5.0,
        );
        t.click(a.sheet, Some(a));
        assert!(matches!(
            t.click(Point::new(20.0, 0.0), None),
            Out::Place(Placement::Callout { .. })
        ));
    }
}
