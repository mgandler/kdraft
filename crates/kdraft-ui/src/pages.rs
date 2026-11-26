//! Pages panel logic: drop position to insertion index, insertion index to
//! the engine's `page.move` target.

/// Insertion index (0..=n) for a drop at `y`, given each row's top and
/// bottom (screen y, rows in order).
pub fn insertion_index(rows: &[(f32, f32)], y: f32) -> usize {
    rows.iter()
        .position(|(top, bottom)| y < (top + bottom) / 2.0)
        .unwrap_or(rows.len())
}

/// `page.move` target for moving the page at `from` to insertion index
/// `insert`; `None` when it would not move.
pub fn move_target(from: usize, insert: usize, n: usize) -> Option<usize> {
    if from >= n || insert > n {
        return None;
    }
    let to = if insert > from { insert - 1 } else { insert };
    (to != from).then_some(to)
}

/// What the pages panel should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageAction {
    Select(usize),
    Move { from: usize, to: usize },
}

/// Click and drag of page entries. A click (press and release without
/// passing egui's drag threshold) selects; a drag selects the page when it
/// starts and moves it on release; a drop at its own position moves
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageDrag {
    #[default]
    Idle,
    Dragging {
        page: usize,
    },
}

impl PageDrag {
    /// Press and release on entry `page` without a drag.
    pub fn clicked(&mut self, page: usize) -> Vec<PageAction> {
        *self = PageDrag::Idle;
        vec![PageAction::Select(page)]
    }

    /// The pointer passed the drag threshold on entry `page`.
    pub fn drag_started(&mut self, page: usize) -> Vec<PageAction> {
        *self = PageDrag::Dragging { page };
        vec![PageAction::Select(page)]
    }

    /// The page being dragged.
    pub fn dragging(&self) -> Option<usize> {
        match *self {
            PageDrag::Dragging { page } => Some(page),
            PageDrag::Idle => None,
        }
    }

    /// Pointer released; `insert` is the insertion index under it (`None`
    /// outside the list cancels).
    pub fn released(&mut self, insert: Option<usize>, n: usize) -> Vec<PageAction> {
        let was = std::mem::take(self);
        match (was, insert) {
            (PageDrag::Dragging { page }, Some(ins)) => move_target(page, ins, n)
                .map(|to| PageAction::Move { from: page, to })
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_selects_drag_selects_then_moves() {
        let mut d = PageDrag::default();
        assert_eq!(d.clicked(1), vec![PageAction::Select(1)]);
        assert_eq!(d.released(Some(0), 3), vec![], "a click never reorders");
        assert_eq!(d.drag_started(1), vec![PageAction::Select(1)]);
        assert_eq!(d.dragging(), Some(1));
        assert_eq!(
            d.released(Some(0), 3),
            vec![PageAction::Move { from: 1, to: 0 }]
        );
        assert_eq!(d, PageDrag::Idle);
        // Dropped on its own position: no move.
        d.drag_started(1);
        assert_eq!(d.released(Some(1), 3), vec![]);
        d.drag_started(1);
        assert_eq!(d.released(Some(2), 3), vec![]);
        // Released outside the list.
        d.drag_started(0);
        assert_eq!(d.released(None, 3), vec![]);
    }

    #[test]
    fn drop_positions() {
        let rows = [(0.0, 100.0), (100.0, 200.0), (200.0, 300.0)];
        assert_eq!(insertion_index(&rows, -5.0), 0);
        assert_eq!(insertion_index(&rows, 49.0), 0);
        assert_eq!(insertion_index(&rows, 51.0), 1);
        assert_eq!(insertion_index(&rows, 260.0), 3);
        assert_eq!(insertion_index(&[], 10.0), 0);
    }

    #[test]
    fn targets() {
        // Page 0 dropped before page 2: ends at index 1.
        assert_eq!(move_target(0, 2, 3), Some(1));
        assert_eq!(move_target(0, 3, 3), Some(2));
        assert_eq!(move_target(2, 0, 3), Some(0));
        assert_eq!(move_target(1, 1, 3), None);
        assert_eq!(move_target(1, 2, 3), None);
        assert_eq!(move_target(5, 0, 3), None);
    }
}
