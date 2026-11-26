//! Presentation metadata of the commands: menu category, icon id,
//! default shortcut and named variants (a command with fixed arguments,
//! such as "Align left"). Data only; front ends turn it into menus,
//! tooltips, a command palette and a shortcut list.
//!
//! Shortcuts are written `Ctrl+Shift+Key` with egui-style key names or
//! symbols; alternatives are separated by `, `. Icon ids name icons of the
//! GUI's icon set.

use serde::Serialize;

/// Menu a command belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    File,
    Edit,
    View,
    Insert,
    Arrange,
    Page,
    Help,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::File,
        Category::Edit,
        Category::View,
        Category::Insert,
        Category::Arrange,
        Category::Page,
        Category::Help,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::File => "File",
            Category::Edit => "Edit",
            Category::View => "View",
            Category::Insert => "Insert",
            Category::Arrange => "Arrange",
            Category::Page => "Page",
            Category::Help => "Help",
        }
    }
}

/// A command with fixed arguments, shown as its own menu entry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Variant {
    /// `<command>:<suffix>`, unique.
    pub id: &'static str,
    pub title: &'static str,
    /// Arguments as JSON object text.
    pub args: &'static str,
    pub icon: Option<&'static str>,
    pub shortcut: Option<&'static str>,
}

/// Metadata of one command.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Meta {
    pub category: Category,
    pub icon: Option<&'static str>,
    pub shortcut: Option<&'static str>,
    /// Shown in menus and the palette (runs without further arguments).
    pub menu: bool,
    pub variants: &'static [Variant],
}

const fn m(category: Category) -> Meta {
    Meta {
        category,
        icon: None,
        shortcut: None,
        menu: false,
        variants: &[],
    }
}

const fn menu(
    category: Category,
    icon: Option<&'static str>,
    shortcut: Option<&'static str>,
) -> Meta {
    Meta {
        category,
        icon,
        shortcut,
        menu: true,
        variants: &[],
    }
}

const fn v(
    id: &'static str,
    title: &'static str,
    args: &'static str,
    icon: Option<&'static str>,
    shortcut: Option<&'static str>,
) -> Variant {
    Variant {
        id,
        title,
        args,
        icon,
        shortcut,
    }
}

const ALIGN: &[Variant] = &[
    v(
        "layout.align:left",
        "Align left",
        r#"{"mode":"left"}"#,
        Some("align-left"),
        Some("Alt+A"),
    ),
    v(
        "layout.align:hcenter",
        "Align horizontal centres",
        r#"{"mode":"hcenter"}"#,
        Some("align-hcenter"),
        Some("Alt+H"),
    ),
    v(
        "layout.align:right",
        "Align right",
        r#"{"mode":"right"}"#,
        Some("align-right"),
        Some("Alt+D"),
    ),
    v(
        "layout.align:top",
        "Align top",
        r#"{"mode":"top"}"#,
        Some("align-top"),
        Some("Alt+W"),
    ),
    v(
        "layout.align:vcenter",
        "Align vertical centres",
        r#"{"mode":"vcenter"}"#,
        Some("align-vcenter"),
        Some("Alt+V"),
    ),
    v(
        "layout.align:bottom",
        "Align bottom",
        r#"{"mode":"bottom"}"#,
        Some("align-bottom"),
        Some("Alt+S"),
    ),
];

const DISTRIBUTE: &[Variant] = &[
    v(
        "layout.distribute:horizontal",
        "Distribute horizontally",
        r#"{"direction":"horizontal"}"#,
        Some("distribute-h"),
        Some("Alt+Shift+H"),
    ),
    v(
        "layout.distribute:vertical",
        "Distribute vertically",
        r#"{"direction":"vertical"}"#,
        Some("distribute-v"),
        Some("Alt+Shift+V"),
    ),
];

const MATCH: &[Variant] = &[
    v(
        "layout.match:width",
        "Match width",
        r#"{"property":"width"}"#,
        Some("match-width"),
        None,
    ),
    v(
        "layout.match:height",
        "Match height",
        r#"{"property":"height"}"#,
        Some("match-height"),
        None,
    ),
    v(
        "layout.match:scale",
        "Match scale",
        r#"{"property":"scale"}"#,
        Some("match-scale"),
        None,
    ),
];

const RAISE: &[Variant] = &[
    v(
        "item.raise:forward",
        "Bring forward",
        "{}",
        Some("raise"),
        Some("Ctrl+]"),
    ),
    v(
        "item.raise:front",
        "Bring to front",
        r#"{"to_top":true}"#,
        None,
        Some("Ctrl+Shift+]"),
    ),
];

const LOWER: &[Variant] = &[
    v(
        "item.lower:backward",
        "Send backward",
        "{}",
        Some("lower"),
        Some("Ctrl+["),
    ),
    v(
        "item.lower:back",
        "Send to back",
        r#"{"to_bottom":true}"#,
        None,
        Some("Ctrl+Shift+["),
    ),
];

const ADD_VIEW: &[Variant] = &[
    v(
        "item.add_view:fabrication",
        "Fabrication view",
        r#"{"kind":"fabrication"}"#,
        Some("view-fab"),
        None,
    ),
    v(
        "item.add_view:assembly",
        "Assembly view",
        r#"{"kind":"assembly"}"#,
        Some("view-assembly"),
        None,
    ),
    v(
        "item.add_view:assembly_back",
        "Assembly view (bottom)",
        r#"{"kind":"assembly","side":"back"}"#,
        Some("view-assembly"),
        None,
    ),
    v(
        "item.add_view:drill",
        "Drill view",
        r#"{"kind":"drill"}"#,
        Some("view-drill"),
        None,
    ),
    v(
        "item.add_view:render3d",
        "3D view",
        r#"{"kind":"render3d"}"#,
        Some("view-3d"),
        None,
    ),
];

const ADD_TABLE: &[Variant] = &[
    v(
        "item.add_table:drill",
        "Drill table",
        r#"{"kind":"drill"}"#,
        Some("table-drill"),
        None,
    ),
    v(
        "item.add_table:stackup",
        "Layer stack table",
        r#"{"kind":"stackup"}"#,
        Some("table-stackup"),
        None,
    ),
    v(
        "item.add_table:bom",
        "BOM table",
        r#"{"kind":"bom"}"#,
        Some("table-bom"),
        None,
    ),
];

const PAGE_PAPER: &[Variant] = &[
    v(
        "page.set_paper:a4",
        "Paper A4",
        r#"{"paper":"A4"}"#,
        None,
        None,
    ),
    v(
        "page.set_paper:a3",
        "Paper A3",
        r#"{"paper":"A3"}"#,
        None,
        None,
    ),
    v(
        "page.set_paper:a2",
        "Paper A2",
        r#"{"paper":"A2"}"#,
        None,
        None,
    ),
    v(
        "page.set_paper:a1",
        "Paper A1",
        r#"{"paper":"A1"}"#,
        None,
        None,
    ),
];

/// Metadata of the command `name` (defaults for unknown names).
pub fn meta(name: &str) -> Meta {
    use Category::*;
    match name {
        "document.new" | "document.open" | "document.save" => m(File),
        "document.set_sheet" => m(Page),
        "document.set_snap_grid" => m(View),
        "document.check" => menu(View, Some("issues"), Some("F7")),
        "bom.renumber" => menu(Edit, Some("table-bom"), None),
        "page.add" => menu(Page, Some("page-add"), Some("Ctrl+Shift+N")),
        "page.duplicate" => menu(Page, Some("page-duplicate"), None),
        "page.remove" => menu(Page, Some("page-delete"), None),
        "page.move" | "page.rename" | "page.select" | "page.set_sheet" => m(Page),
        "page.set_paper" => Meta {
            variants: PAGE_PAPER,
            ..m(Page)
        },
        "item.add_view" => Meta {
            variants: ADD_VIEW,
            ..m(Insert)
        },
        "item.add_table" => Meta {
            variants: ADD_TABLE,
            ..m(Insert)
        },
        "item.add_text" => Meta {
            icon: Some("text"),
            ..m(Insert)
        },
        "item.add_line" => Meta {
            icon: Some("line"),
            ..m(Insert)
        },
        "item.add_rect" => Meta {
            icon: Some("rect"),
            ..m(Insert)
        },
        "item.add_notes" => Meta {
            icon: Some("notes"),
            ..m(Insert)
        },
        "item.add_callout" => Meta {
            icon: Some("callout"),
            ..m(Insert)
        },
        "dimension.add_linear" => Meta {
            icon: Some("dim-linear"),
            ..m(Insert)
        },
        "dimension.add_radius" => Meta {
            icon: Some("dim-radius"),
            ..m(Insert)
        },
        "dimension.add_ordinate" => Meta {
            icon: Some("dim-ordinate"),
            ..m(Insert)
        },
        "dimension.board_size" => menu(Insert, Some("dim-linear"), None),
        "item.remove" => menu(Edit, Some("trash"), Some("Delete, Backspace")),
        "item.duplicate" => menu(Edit, Some("duplicate"), Some("Ctrl+D")),
        "callout.duplicate_next" => menu(Edit, Some("duplicate"), Some("Ctrl+Alt+D")),
        "callout.renumber" => m(Arrange),
        "select.all" => menu(Edit, None, Some("Ctrl+A")),
        "select.clear" => menu(Edit, None, Some("Escape")),
        "layout.align" => Meta {
            variants: ALIGN,
            ..m(Arrange)
        },
        "layout.distribute" => Meta {
            variants: DISTRIBUTE,
            ..m(Arrange)
        },
        "layout.match" => Meta {
            variants: MATCH,
            ..m(Arrange)
        },
        "layout.center" => menu(Arrange, Some("center"), None),
        "layout.nudge" => m(Arrange),
        "item.raise" => Meta {
            variants: RAISE,
            ..m(Arrange)
        },
        "item.lower" => Meta {
            variants: LOWER,
            ..m(Arrange)
        },
        "item.move_to_page" | "item.copy_to_page" | "item.reorder" => m(Arrange),
        "item.rename" | "item.set_hidden" | "item.set_locked" => m(Edit),
        "style.copy" => menu(Edit, None, Some("Ctrl+Alt+C")),
        "link.clear" => menu(Arrange, Some("unlink"), None),
        "link.select_reference" | "link.select_followers" => menu(Arrange, Some("link"), None),
        "link.set" | "link.set_offset" | "link.set_follow" => m(Arrange),
        "link.match_colours" => menu(Arrange, Some("link"), None),
        "view.pages_per_layer" => m(Insert),
        "style.paste" => menu(Edit, None, Some("Ctrl+Alt+V")),
        "item.reset_style" => menu(Edit, None, None),
        _ => m(Edit),
    }
}

#[cfg(test)]
mod tests {
    use crate::Registry;

    #[test]
    fn variants_have_valid_args_and_unique_ids_and_shortcuts() {
        let r = Registry::new();
        let mut ids = std::collections::HashSet::new();
        let mut keys = std::collections::HashSet::new();
        for s in r.specs() {
            assert!(ids.insert(s.name.to_string()));
            for k in s.shortcut.iter().flat_map(|x| x.split(", ")) {
                assert!(keys.insert(k.to_string()), "shortcut {k} used twice");
            }
            for v in s.variants {
                assert!(v.id.starts_with(&format!("{}:", s.name)), "{}", v.id);
                assert!(ids.insert(v.id.to_string()), "{}", v.id);
                let args: serde_json::Value = serde_json::from_str(v.args).unwrap();
                // Arguments other than the selection must be complete.
                r.check_args(s.name, &args)
                    .unwrap_or_else(|e| panic!("{}: {e}", v.id));
                for k in v.shortcut.iter().flat_map(|x| x.split(", ")) {
                    assert!(keys.insert(k.to_string()), "shortcut {k} used twice");
                }
            }
            if s.menu {
                r.check_args(s.name, &serde_json::json!({}))
                    .unwrap_or_else(|e| panic!("{} is in menus but needs arguments: {e}", s.name));
            }
        }
    }
}
