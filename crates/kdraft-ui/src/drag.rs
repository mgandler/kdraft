//! Pointer state machine of the canvas: click selection, drag-move,
//! corner-drag view scaling, marquee. It turns pointer events (in sheet
//! mm) into actions the app executes as engine commands.

use kdraft_board::geom::{BBox, Point};
use kdraft_doc::Id;

use crate::hit::{rect_from, Handle};

/// What the pointer went down on.
#[derive(Debug, Clone, PartialEq)]
pub enum Press {
    Item {
        id: Id,
        selected: bool,
    },
    /// A corner handle of the single selected view: its box and scale.
    ScaleHandle {
        handle: Handle,
        bbox: BBox,
        scale: f64,
    },
    /// An edit handle of the single selected item.
    Handle(crate::handles::EditHandle),
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Start dragging an edit handle (one gesture).
    BeginEdit(crate::handles::EditHandle),
    /// The pointer position (sheet mm, unsnapped) while dragging a handle.
    PreviewEdit(Point),
    /// A handle was clicked without dragging.
    HandleClicked(crate::handles::EditHandle),
    /// Select only this item.
    SelectOnly(Id),
    /// Add or remove one item.
    Toggle(Id),
    ClearSelection,
    /// Start a move gesture of the selection.
    BeginMove,
    /// Raw (unsnapped) delta since the press, sheet mm.
    PreviewMove(Point),
    /// Start a scale gesture of the selected view.
    BeginScale,
    PreviewScale(f64),
    /// End the gesture as one undo step.
    Commit,
    /// End the gesture and restore.
    Cancel,
    /// Marquee in progress (for drawing).
    Marquee(BBox),
    /// Marquee released: select what lies inside.
    MarqueeSelect {
        rect: BBox,
        additive: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Drag {
    #[default]
    Idle,
    Pending {
        start: Point,
        press: Press,
        shift: bool,
    },
    Moving {
        start: Point,
    },
    Editing,
    Scaling {
        centre: Point,
        start_dist: f64,
        scale: f64,
    },
    Marquee {
        start: Point,
        now: Point,
        additive: bool,
    },
}

/// Corner-handle scaling snaps to the standard scales (the ones the scale
/// buttons offer); with `free` (Alt held) the raw factor is used.
pub fn snap_scale(raw: f64, free: bool) -> f64 {
    if free || !raw.is_finite() || raw <= 0.0 {
        return raw;
    }
    kdraft_doc::STANDARD_SCALES
        .iter()
        .copied()
        .min_by(|a, b| (raw / a).ln().abs().total_cmp(&(raw / b).ln().abs()))
        .unwrap_or(raw)
}

/// Pointer travel before a press becomes a drag, sheet mm (the app passes
/// a screen distance converted to mm).
impl Drag {
    pub fn is_idle(&self) -> bool {
        matches!(self, Drag::Idle)
    }

    pub fn in_gesture(&self) -> bool {
        matches!(
            self,
            Drag::Moving { .. } | Drag::Scaling { .. } | Drag::Editing
        )
    }

    pub fn press(&mut self, p: Point, press: Press, shift: bool) -> Vec<Action> {
        let mut out = Vec::new();
        if self.in_gesture() {
            out.push(Action::Cancel);
        }
        if let Press::Item { id, selected } = &press {
            // Pressing an unselected item selects it at once, so dragging
            // moves it.
            if !selected && !shift {
                out.push(Action::SelectOnly(id.clone()));
            }
        }
        *self = Drag::Pending {
            start: p,
            press,
            shift,
        };
        out
    }

    pub fn moved(&mut self, p: Point, threshold: f64) -> Vec<Action> {
        match self.clone() {
            Drag::Pending {
                start,
                press,
                shift,
            } => {
                if start.dist(p) < threshold {
                    return vec![];
                }
                match press {
                    Press::Item { id, selected } => {
                        let mut out = Vec::new();
                        if shift && !selected {
                            out.push(Action::Toggle(id));
                        }
                        *self = Drag::Moving { start };
                        out.push(Action::BeginMove);
                        out.push(Action::PreviewMove(p - start));
                        out
                    }
                    Press::ScaleHandle { bbox, scale, .. } => {
                        let centre = bbox.center();
                        let d = start.dist(centre);
                        if d < 1e-9 {
                            *self = Drag::Idle;
                            return vec![];
                        }
                        *self = Drag::Scaling {
                            centre,
                            start_dist: d,
                            scale,
                        };
                        let mut out = vec![Action::BeginScale];
                        out.extend(self.moved(p, threshold));
                        out
                    }
                    Press::Handle(h) => {
                        *self = Drag::Editing;
                        vec![Action::BeginEdit(h), Action::PreviewEdit(p)]
                    }
                    Press::Empty => {
                        *self = Drag::Marquee {
                            start,
                            now: p,
                            additive: shift,
                        };
                        vec![Action::Marquee(rect_from(start, p))]
                    }
                }
            }
            Drag::Moving { start } => vec![Action::PreviewMove(p - start)],
            Drag::Editing => vec![Action::PreviewEdit(p)],
            Drag::Scaling {
                centre,
                start_dist,
                scale,
            } => {
                let f = (p.dist(centre) / start_dist).max(1e-3);
                vec![Action::PreviewScale(scale * f)]
            }
            Drag::Marquee {
                start, additive, ..
            } => {
                *self = Drag::Marquee {
                    start,
                    now: p,
                    additive,
                };
                vec![Action::Marquee(rect_from(start, p))]
            }
            Drag::Idle => vec![],
        }
    }

    pub fn release(&mut self, p: Point) -> Vec<Action> {
        let s = std::mem::take(self);
        match s {
            Drag::Pending { press, shift, .. } => match press {
                Press::Item { id, selected } => {
                    if shift {
                        vec![Action::Toggle(id)]
                    } else if selected {
                        vec![Action::SelectOnly(id)]
                    } else {
                        vec![]
                    }
                }
                Press::ScaleHandle { .. } => vec![],
                Press::Handle(h) => vec![Action::HandleClicked(h)],
                Press::Empty => {
                    if shift {
                        vec![]
                    } else {
                        vec![Action::ClearSelection]
                    }
                }
            },
            Drag::Moving { .. } | Drag::Scaling { .. } | Drag::Editing => vec![Action::Commit],
            Drag::Marquee {
                start, additive, ..
            } => vec![Action::MarqueeSelect {
                rect: rect_from(start, p),
                additive,
            }],
            Drag::Idle => vec![],
        }
    }

    pub fn cancel(&mut self) -> Vec<Action> {
        let was = std::mem::take(self);
        if was.in_gesture() {
            vec![Action::Cancel]
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_snapping() {
        assert_eq!(snap_scale(1.3, false), 1.0);
        assert_eq!(snap_scale(1.438, false), 2.0);
        assert_eq!(snap_scale(1.6, false), 2.0);
        assert_eq!(snap_scale(4.4, false), 4.0);
        assert_eq!(snap_scale(4.6, false), 5.0);
        assert_eq!(snap_scale(0.3, false), 0.5);
        assert_eq!(snap_scale(1.438, true), 1.438);
    }

    fn id(s: &str) -> Id {
        Id(s.into())
    }

    #[test]
    fn click_selects_and_drag_moves() {
        let mut d = Drag::default();
        let a = d.press(
            Point::new(1.0, 1.0),
            Press::Item {
                id: id("a"),
                selected: false,
            },
            false,
        );
        assert_eq!(a, vec![Action::SelectOnly(id("a"))]);
        assert!(d.moved(Point::new(1.1, 1.0), 0.5).is_empty());
        let a = d.moved(Point::new(3.0, 1.0), 0.5);
        assert_eq!(
            a,
            vec![Action::BeginMove, Action::PreviewMove(Point::new(2.0, 0.0))]
        );
        assert_eq!(
            d.moved(Point::new(4.0, 2.0), 0.5),
            vec![Action::PreviewMove(Point::new(3.0, 1.0))]
        );
        assert_eq!(d.release(Point::new(4.0, 2.0)), vec![Action::Commit]);
        assert!(d.is_idle());
    }

    #[test]
    fn click_on_selected_narrows_shift_toggles() {
        let mut d = Drag::default();
        d.press(
            Point::ORIGIN,
            Press::Item {
                id: id("a"),
                selected: true,
            },
            false,
        );
        assert_eq!(d.release(Point::ORIGIN), vec![Action::SelectOnly(id("a"))]);
        let a = d.press(
            Point::ORIGIN,
            Press::Item {
                id: id("b"),
                selected: false,
            },
            true,
        );
        assert!(a.is_empty());
        assert_eq!(d.release(Point::ORIGIN), vec![Action::Toggle(id("b"))]);
        // Shift-drag of an unselected item adds it, then moves.
        d.press(
            Point::ORIGIN,
            Press::Item {
                id: id("c"),
                selected: false,
            },
            true,
        );
        let a = d.moved(Point::new(5.0, 0.0), 0.5);
        assert_eq!(a[0], Action::Toggle(id("c")));
        assert_eq!(a[1], Action::BeginMove);
    }

    #[test]
    fn empty_click_clears_marquee_selects_escape_cancels() {
        let mut d = Drag::default();
        d.press(Point::ORIGIN, Press::Empty, false);
        assert_eq!(d.release(Point::ORIGIN), vec![Action::ClearSelection]);
        d.press(Point::ORIGIN, Press::Empty, true);
        assert!(matches!(
            d.moved(Point::new(5.0, 5.0), 0.5)[0],
            Action::Marquee(_)
        ));
        let a = d.release(Point::new(6.0, -2.0));
        assert_eq!(
            a,
            vec![Action::MarqueeSelect {
                rect: rect_from(Point::ORIGIN, Point::new(6.0, -2.0)),
                additive: true
            }]
        );
        d.press(
            Point::ORIGIN,
            Press::Item {
                id: id("a"),
                selected: true,
            },
            false,
        );
        d.moved(Point::new(5.0, 0.0), 0.5);
        assert_eq!(d.cancel(), vec![Action::Cancel]);
        assert!(d.release(Point::ORIGIN).is_empty());
    }

    #[test]
    fn corner_handle_scales_by_distance_from_centre() {
        let mut d = Drag::default();
        let bbox = BBox {
            min: Point::new(-10.0, -10.0),
            max: Point::new(10.0, 10.0),
        };
        d.press(
            Point::new(10.0, 10.0),
            Press::ScaleHandle {
                handle: Handle::BottomRight,
                bbox,
                scale: 2.0,
            },
            false,
        );
        let a = d.moved(Point::new(20.0, 20.0), 0.5);
        assert_eq!(a[0], Action::BeginScale);
        assert_eq!(a[1], Action::PreviewScale(4.0));
        assert_eq!(d.release(Point::new(20.0, 20.0)), vec![Action::Commit]);
    }
}
