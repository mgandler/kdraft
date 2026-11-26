//! Every user-visible string of the editor that is not a command title
//! (those come from the engine registry). English; a German table can be
//! added later by turning these constants into lookups.

pub const APP: &str = "kdraft";
pub const UNTITLED: &str = "Untitled";
pub const MIXED: &str = "Mixed";

// Theme
pub const THEME_LIGHT: &str = "Switch to light theme";
pub const THEME_DARK: &str = "Switch to dark theme";
pub const THEME_SYSTEM: &str = "Follow system theme";

// Top strip and start screen
pub const MODIFIED: &str = "Modified (not saved)";
pub const START_TITLE: &str = "Fabrication and assembly drawings for KiCad";
pub const START_SUB: &str = "Open a drawing set, or start a new one from a board.";
pub const OPEN_DOTS: &str = "Open…";
pub const NEW_FROM_BOARD: &str = "New from board…";
pub const RECENT: &str = "Recent";
pub const NO_RECENT: &str = "No recent files.";
pub const OPENING: &str = "Opening…";
pub const PATH_PROMPT: &str =
    "Path (native dialogs are off; you can also drop a file onto the window):";
pub const ABOUT: &str = "Fabrication and assembly drawing sets for KiCad boards. GPL-3.0-or-later.";

// Panels
pub const PAGES: &str = "Pages";
pub const ITEMS: &str = "Items";
pub const ADD_PAGE: &str = "Add page";
pub const NO_ITEMS: &str = "No items on this page.";
pub const HIDE: &str = "Hide";
pub const SHOW: &str = "Show";
pub const LOCK: &str = "Lock";
pub const UNLOCK: &str = "Unlock";
