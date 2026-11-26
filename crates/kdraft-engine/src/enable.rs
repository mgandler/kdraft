//! Whether a command can run on a session's current selection, shared by
//! the commands' own checks and the GUI's enabled buttons.

use crate::session::Session;

/// Smallest number of items a selection-based command needs; `None` for
/// commands without such a limit.
pub fn min_items(command: &str) -> Option<usize> {
    match command {
        "layout.distribute" => Some(3),
        "layout.match" => Some(2),
        // One item aligns to the drawing area.
        "layout.align" | "layout.center" | "layout.nudge" => Some(1),
        _ => None,
    }
}

/// The reason `command` cannot run on `n` items, if any.
pub fn check_items(command: &str, n: usize) -> Result<(), String> {
    match min_items(command) {
        Some(m) if n < m => Err(match (command, m) {
            ("layout.distribute", _) => "distribute needs three or more items".to_string(),
            ("layout.match", _) => "match needs two or more items".to_string(),
            _ => "nothing selected".to_string(),
        }),
        _ => Ok(()),
    }
}

/// Can `command` run on the session's current selection?
pub fn can_run(s: &Session, command: &str) -> Result<(), String> {
    match command {
        "style.paste" if s.style_clip.is_none() => {
            return Err("no style copied".to_string());
        }
        "style.copy" | "style.paste" | "item.reset_style" if s.selection.items.is_empty() => {
            return Err("nothing selected".to_string());
        }
        _ => {}
    }
    check_items(command, s.selection.items.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits() {
        assert!(check_items("layout.distribute", 2).is_err());
        assert!(check_items("layout.distribute", 3).is_ok());
        assert!(check_items("layout.match", 1).is_err());
        assert!(check_items("layout.align", 1).is_ok());
        assert!(check_items("layout.align", 0).is_err());
        assert!(check_items("page.add", 0).is_ok());
    }
}
