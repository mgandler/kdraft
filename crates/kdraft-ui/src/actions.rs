//! The action catalogue: every engine command that runs without further
//! arguments (and each named variant, such as "Align left") plus the
//! editor's own actions (file dialogs, undo, zoom, panels). Menus,
//! context menus, tooltips, the command palette, the shortcut overview
//! and keyboard dispatch are all generated from it.

use eframe::egui::{Key, Modifiers};
use kdraft_engine::meta::Category;
use kdraft_engine::Registry;
use serde_json::Value;

use crate::icons::Icon;
use crate::tools::ToolKind;

/// Editor actions that are not engine commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppAct {
    Open,
    OpenBoard,
    Save,
    SaveAs,
    ExportPdf,
    ExportPng,
    ExportSvg,
    ChooseSheet,
    ReloadBoard,
    CloseDocument,
    Undo,
    Redo,
    ZoomIn,
    ZoomOut,
    ZoomActual,
    FitPage,
    FitSelection,
    ToggleGrid,
    ToggleRulers,
    ToggleSnap,
    ToggleTheme,
    Palette,
    Shortcuts,
    ToggleLog,
    ToggleIssues,
    PrevPage,
    NextPage,
    MovePageUp,
    MovePageDown,
    RenamePage,
    Rename,
    ToggleHidden,
    ToggleLocked,
    Tool(ToolKind),
    DocStyles,
    LinkPick,
    LayerPages,
    RenumberCallouts,
    About,
    Settings,
}

/// What an entry does.
#[derive(Debug, Clone, PartialEq)]
pub enum Run {
    Cmd { name: &'static str, args: Value },
    App(AppAct),
}

/// A keyboard shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub mods: Modifiers,
    pub key: Key,
}

impl Shortcut {
    /// Parses `Ctrl+Shift+Z`, `Alt+A`, `Delete`, `Ctrl+]`.
    pub fn parse(s: &str) -> Option<Shortcut> {
        let mut mods = Modifiers::NONE;
        let parts: Vec<&str> = s.split('+').collect();
        // "Ctrl++" style is not used; the last part is the key.
        let (key, ms) = parts.split_last()?;
        for m in ms {
            match m.trim() {
                "Ctrl" | "Cmd" => {
                    mods.command = true;
                    mods.ctrl = !cfg!(target_os = "macos");
                    mods.mac_cmd = cfg!(target_os = "macos");
                }
                "Shift" => mods.shift = true,
                "Alt" => mods.alt = true,
                _ => return None,
            }
        }
        let key = Key::from_name(key.trim())?;
        Some(Shortcut { mods, key })
    }

    /// Whether the modifiers held match exactly (Ctrl and Cmd are the
    /// same "command" modifier).
    pub fn matches(&self, key: Key, held: Modifiers) -> bool {
        key == self.key
            && held.command == self.mods.command
            && held.shift == self.mods.shift
            && held.alt == self.mods.alt
    }
}

/// One catalogue entry.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Command name, variant id (`layout.align:left`) or `app.<name>`.
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: Category,
    pub icon: Option<Icon>,
    /// As written (`Ctrl+D`), first alternative only; for display.
    pub shortcut_text: Option<String>,
    pub shortcuts: Vec<Shortcut>,
    pub run: Run,
}

impl Entry {
    /// The engine command behind it, for enablement.
    pub fn command(&self) -> Option<&'static str> {
        match &self.run {
            Run::Cmd { name, .. } => Some(name),
            Run::App(_) => None,
        }
    }
}

fn shortcuts(s: Option<&str>) -> (Option<String>, Vec<Shortcut>) {
    let Some(s) = s else {
        return (None, Vec::new());
    };
    let all: Vec<Shortcut> = s.split(", ").filter_map(Shortcut::parse).collect();
    (s.split(", ").next().map(str::to_string), all)
}

/// Editor action: act, id, title, description, category, icon, shortcut.
type AppDef = (
    AppAct,
    &'static str,
    &'static str,
    &'static str,
    Category,
    Option<Icon>,
    Option<&'static str>,
);

const APP: &[AppDef] = {
    use AppAct as A;
    use Category::*;
    &[
        (
            A::Open,
            "app.open",
            "Open…",
            "Open a .kdraft document or a .kicad_pcb board",
            File,
            Some(Icon::Open),
            Some("Ctrl+O"),
        ),
        (
            A::OpenBoard,
            "app.open_board",
            "New from board…",
            "Create a drawing set for a .kicad_pcb board",
            File,
            Some(Icon::New),
            Some("Ctrl+N"),
        ),
        (
            A::Save,
            "app.save",
            "Save",
            "Save the document",
            File,
            Some(Icon::Save),
            Some("Ctrl+S"),
        ),
        (
            A::SaveAs,
            "app.save_as",
            "Save as…",
            "Save the document under a new name",
            File,
            None,
            Some("Ctrl+Shift+S"),
        ),
        (
            A::ExportPdf,
            "app.export_pdf",
            "Export PDF…",
            "Export all pages as one PDF",
            File,
            Some(Icon::Export),
            Some("Ctrl+E"),
        ),
        (
            A::ExportPng,
            "app.export_png",
            "Export PNG…",
            "Export the pages as PNG images",
            File,
            None,
            None,
        ),
        (
            A::ExportSvg,
            "app.export_svg",
            "Export SVG…",
            "Export the pages as SVG files",
            File,
            None,
            None,
        ),
        (
            A::ReloadBoard,
            "app.reload_board",
            "Reload board",
            "Read the board file again (changes are also picked up automatically)",
            File,
            Some(Icon::Reload),
            Some("F5"),
        ),
        (
            A::CloseDocument,
            "app.close",
            "Close document",
            "Close the document and show the start screen",
            File,
            None,
            Some("Ctrl+W"),
        ),
        (
            A::Undo,
            "app.undo",
            "Undo",
            "Undo the last change",
            Edit,
            Some(Icon::Undo),
            Some("Ctrl+Z"),
        ),
        (
            A::Redo,
            "app.redo",
            "Redo",
            "Redo the change undone last",
            Edit,
            Some(Icon::Redo),
            Some("Ctrl+Y, Ctrl+Shift+Z"),
        ),
        (
            A::Rename,
            "app.rename",
            "Rename",
            "Rename the selected item",
            Edit,
            None,
            Some("F2"),
        ),
        (
            A::ToggleHidden,
            "app.toggle_hidden",
            "Show / hide",
            "Hide the selected items on the sheet and in exports, or show them again",
            Edit,
            Some(Icon::Eye),
            Some("Ctrl+Shift+H"),
        ),
        (
            A::ToggleLocked,
            "app.toggle_locked",
            "Lock / unlock",
            "Locked items cannot be picked or moved on the canvas",
            Edit,
            Some(Icon::Lock),
            Some("Ctrl+Shift+L"),
        ),
        (
            A::ZoomIn,
            "app.zoom_in",
            "Zoom in",
            "",
            View,
            Some(Icon::ZoomIn),
            Some("Ctrl+=, Ctrl+Plus"),
        ),
        (
            A::ZoomOut,
            "app.zoom_out",
            "Zoom out",
            "",
            View,
            Some(Icon::ZoomOut),
            Some("Ctrl+-"),
        ),
        (
            A::ZoomActual,
            "app.zoom_100",
            "Zoom to 100 %",
            "True size on a 96 dpi screen",
            View,
            None,
            Some("Ctrl+0"),
        ),
        (
            A::FitPage,
            "app.fit_page",
            "Fit page",
            "Zoom to show the whole page",
            View,
            Some(Icon::FitPage),
            Some("Home, Shift+1"),
        ),
        (
            A::FitSelection,
            "app.fit_selection",
            "Fit selection",
            "Zoom to the selected items",
            View,
            Some(Icon::FitSelection),
            Some("Shift+2"),
        ),
        (
            A::ToggleGrid,
            "app.toggle_grid",
            "Show grid",
            "Show the snap grid on the sheet",
            View,
            Some(Icon::Grid),
            Some("Shift+G"),
        ),
        (
            A::ToggleRulers,
            "app.toggle_rulers",
            "Show rulers",
            "Millimetre rulers along the canvas",
            View,
            Some(Icon::Ruler),
            Some("Shift+R"),
        ),
        (
            A::ToggleSnap,
            "app.toggle_snap",
            "Snap",
            "Snap to items, the page, the drawing area and the grid",
            View,
            Some(Icon::Snap),
            Some("Shift+S"),
        ),
        (
            A::ToggleTheme,
            "app.toggle_theme",
            "Toggle dark theme",
            "Switch between the light and the dark theme",
            View,
            Some(Icon::Moon),
            None,
        ),
        (
            A::ToggleIssues,
            "app.issues",
            "Issues",
            "Show the drawing checks",
            View,
            Some(Icon::Issues),
            Some("Ctrl+Shift+I"),
        ),
        (
            A::ToggleLog,
            "app.log",
            "Message log",
            "Show all messages",
            View,
            Some(Icon::Log),
            Some("Ctrl+Shift+M"),
        ),
        (
            A::Palette,
            "app.palette",
            "Command palette…",
            "Search and run any command",
            View,
            Some(Icon::Command),
            Some("Ctrl+K, Ctrl+Shift+P"),
        ),
        (
            A::PrevPage,
            "app.prev_page",
            "Previous page",
            "",
            Page,
            None,
            Some("PageUp"),
        ),
        (
            A::NextPage,
            "app.next_page",
            "Next page",
            "",
            Page,
            None,
            Some("PageDown"),
        ),
        (
            A::MovePageUp,
            "app.move_page_up",
            "Move page up",
            "",
            Page,
            None,
            Some("Alt+ArrowUp"),
        ),
        (
            A::MovePageDown,
            "app.move_page_down",
            "Move page down",
            "",
            Page,
            None,
            Some("Alt+ArrowDown"),
        ),
        (
            A::RenamePage,
            "app.rename_page",
            "Rename page",
            "",
            Page,
            None,
            None,
        ),
        (
            A::ChooseSheet,
            "app.choose_sheet",
            "Drawing sheet…",
            "Choose a .kicad_wks drawing sheet for the current page",
            Page,
            None,
            None,
        ),
        (
            A::Shortcuts,
            "app.shortcuts",
            "Keyboard shortcuts",
            "All shortcuts and mouse gestures",
            Help,
            Some(Icon::Keyboard),
            Some("Ctrl+/"),
        ),
        (
            A::DocStyles,
            "app.document_styles",
            "Document styles…",
            "Default colours, line and text styles per item category",
            Edit,
            None,
            None,
        ),
        (
            A::LinkPick,
            "app.link_pick",
            "Link placement to…",
            "Link the selected items' placement to a reference item (click it on any page)",
            Arrange,
            Some(Icon::Link),
            None,
        ),
        (
            A::RenumberCallouts,
            "app.renumber_callouts",
            "Renumber callouts…",
            "Number the selected callouts, or all numbered callouts of the page, in reading order",
            Arrange,
            None,
            None,
        ),
        (
            A::LayerPages,
            "app.layer_pages",
            "Layer pages…",
            "One page per board layer, each view at the same place and scale (linked to a reference view or a new placement)",
            Insert,
            Some(Icon::Layers),
            None,
        ),
        (
            A::Settings,
            "app.settings",
            "Settings…",
            "kicad-cli, document defaults, layer colours, cache, appearance",
            File,
            Some(Icon::Settings),
            Some("Ctrl+Comma"),
        ),
        (A::About, "app.about", "About kdraft", "", Help, None, None),
    ]
};

/// Tools of the floating bar: kind, id, title, icon, shortcut.
pub const TOOLS: &[(ToolKind, &str, &str, Icon, &str)] = &[
    (ToolKind::Select, "tool.select", "Select", Icon::Select, "V"),
    (ToolKind::Pan, "tool.pan", "Hand (pan)", Icon::Pan, "H"),
    (
        ToolKind::View(crate::tools::ViewTool::Fabrication),
        "tool.view_fab",
        "Place fabrication view",
        Icon::ViewFab,
        "",
    ),
    (
        ToolKind::View(crate::tools::ViewTool::Assembly),
        "tool.view_assembly",
        "Place assembly view",
        Icon::ViewAssembly,
        "",
    ),
    (
        ToolKind::View(crate::tools::ViewTool::Drill),
        "tool.view_drill",
        "Place drill view",
        Icon::ViewDrill,
        "",
    ),
    (
        ToolKind::Table(crate::tools::TableTool::Drill),
        "tool.table_drill",
        "Place drill table",
        Icon::TableDrill,
        "",
    ),
    (
        ToolKind::Table(crate::tools::TableTool::Stackup),
        "tool.table_stackup",
        "Place layer stack table",
        Icon::TableStackup,
        "",
    ),
    (
        ToolKind::Table(crate::tools::TableTool::Bom),
        "tool.table_bom",
        "Place BOM table",
        Icon::TableBom,
        "",
    ),
    (
        ToolKind::DimLinear(crate::tools::LinearKind::Horizontal),
        "tool.dim_horizontal",
        "Horizontal dimension",
        Icon::DimLinear,
        "D",
    ),
    (
        ToolKind::DimLinear(crate::tools::LinearKind::Vertical),
        "tool.dim_vertical",
        "Vertical dimension",
        Icon::DimVertical,
        "Shift+D",
    ),
    (
        ToolKind::DimLinear(crate::tools::LinearKind::Aligned),
        "tool.dim_aligned",
        "Aligned dimension",
        Icon::DimAligned,
        "Alt+Shift+D",
    ),
    (
        ToolKind::DimRadius { diameter: false },
        "tool.dim_radius",
        "Radius dimension",
        Icon::DimRadius,
        "",
    ),
    (
        ToolKind::DimRadius { diameter: true },
        "tool.dim_diameter",
        "Diameter dimension",
        Icon::DimDiameter,
        "",
    ),
    (
        ToolKind::DimOrdinate,
        "tool.dim_ordinate",
        "Ordinate dimension",
        Icon::DimOrdinate,
        "O",
    ),
    (
        ToolKind::Callout(crate::tools::CalloutTool::Number),
        "tool.callout",
        "Callout (number)",
        Icon::Callout,
        "B",
    ),
    (
        ToolKind::Callout(crate::tools::CalloutTool::Bom),
        "tool.callout_bom",
        "Callout (BOM item)",
        Icon::Callout,
        "",
    ),
    (
        ToolKind::Callout(crate::tools::CalloutTool::Note),
        "tool.callout_note",
        "Callout (note)",
        Icon::Callout,
        "",
    ),
    (ToolKind::Notes, "tool.notes", "Note list", Icon::Notes, "N"),
    (ToolKind::Text, "tool.text", "Text", Icon::Text, "T"),
    (ToolKind::Line, "tool.line", "Line", Icon::Line, "L"),
    (ToolKind::Rect, "tool.rect", "Rectangle", Icon::Rect, "R"),
    (
        ToolKind::Polyline,
        "tool.polyline",
        "Polyline",
        Icon::Polyline,
        "P",
    ),
    (ToolKind::Circle, "tool.circle", "Circle", Icon::Circle, "C"),
];

/// The catalogue, built once from the registry.
#[derive(Debug, Clone, Default)]
pub struct Catalogue {
    pub entries: Vec<Entry>,
}

impl Catalogue {
    pub fn build(reg: &Registry) -> Catalogue {
        let mut entries = Vec::new();
        for (act, id, title, desc, cat, icon, sc) in APP {
            let (shortcut_text, shortcuts) = shortcuts(*sc);
            entries.push(Entry {
                id: (*id).to_string(),
                title: (*title).to_string(),
                description: (*desc).to_string(),
                category: *cat,
                icon: *icon,
                shortcut_text,
                shortcuts,
                run: Run::App(*act),
            });
        }
        for (kind, id, title, icon, sc) in TOOLS {
            let (shortcut_text, shortcuts) = shortcuts((!sc.is_empty()).then_some(*sc));
            entries.push(Entry {
                id: (*id).to_string(),
                title: (*title).to_string(),
                description: String::new(),
                category: Category::Insert,
                icon: Some(*icon),
                shortcut_text,
                shortcuts,
                run: Run::App(AppAct::Tool(*kind)),
            });
        }
        for s in reg.specs() {
            for v in s.variants {
                let (shortcut_text, shortcuts) = shortcuts(v.shortcut);
                entries.push(Entry {
                    id: v.id.to_string(),
                    title: v.title.to_string(),
                    description: s.description.to_string(),
                    category: s.category,
                    icon: v.icon.and_then(Icon::from_name),
                    shortcut_text,
                    shortcuts,
                    run: Run::Cmd {
                        name: s.name,
                        args: serde_json::from_str(v.args).unwrap_or(Value::Null),
                    },
                });
            }
            if s.menu {
                let (shortcut_text, shortcuts) = shortcuts(s.shortcut);
                entries.push(Entry {
                    id: s.name.to_string(),
                    title: s.title.to_string(),
                    description: s.description.to_string(),
                    category: s.category,
                    icon: s.icon.and_then(Icon::from_name),
                    shortcut_text,
                    shortcuts,
                    run: Run::Cmd {
                        name: s.name,
                        args: serde_json::json!({}),
                    },
                });
            }
        }
        Catalogue { entries }
    }

    /// The shortcut table as Markdown (for `docs/GUI.md`).
    pub fn shortcuts_markdown(&self) -> String {
        let mut out = String::from("| Menu | Action | Shortcut |\n| --- | --- | --- |\n");
        for cat in Category::ALL {
            for e in self.entries.iter().filter(|e| e.category == cat) {
                let Some(_) = &e.shortcut_text else { continue };
                let all: Vec<String> = e
                    .shortcuts
                    .iter()
                    .map(|s| {
                        let mut k = String::new();
                        if s.mods.command {
                            k.push_str("Ctrl+");
                        }
                        if s.mods.alt {
                            k.push_str("Alt+");
                        }
                        if s.mods.shift {
                            k.push_str("Shift+");
                        }
                        k.push_str(s.key.name());
                        k
                    })
                    .collect();
                out.push_str(&format!(
                    "| {} | {} | {} |\n",
                    cat.title(),
                    e.title,
                    all.join(", ")
                ));
            }
        }
        out
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The entry a key press runs.
    pub fn for_key(&self, key: Key, held: Modifiers) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.shortcuts.iter().any(|s| s.matches(key, held)))
    }

    /// Menu contents: the curated order (`None` = separator), then any
    /// entries of the category not listed.
    pub fn menu(&self, cat: Category) -> Vec<Option<&Entry>> {
        let order: &[&str] = match cat {
            Category::File => &[
                "app.open",
                "app.open_board",
                "-",
                "app.save",
                "app.save_as",
                "-",
                "app.export_pdf",
                "app.export_png",
                "app.export_svg",
                "-",
                "app.reload_board",
                "-",
                "app.settings",
                "-",
                "app.close",
            ],
            Category::Edit => &[
                "app.undo",
                "app.redo",
                "-",
                "item.duplicate",
                "callout.duplicate_next",
                "item.remove",
                "-",
                "app.rename",
                "app.toggle_hidden",
                "app.toggle_locked",
                "-",
                "select.all",
                "select.clear",
            ],
            Category::View => &[
                "app.palette",
                "-",
                "app.zoom_in",
                "app.zoom_out",
                "app.zoom_100",
                "app.fit_page",
                "app.fit_selection",
                "-",
                "app.toggle_rulers",
                "app.toggle_grid",
                "app.toggle_snap",
                "-",
                "app.issues",
                "document.check",
                "app.log",
                "-",
                "app.toggle_theme",
            ],
            Category::Insert => &[
                "tool.view_fab",
                "tool.view_assembly",
                "tool.view_drill",
                "-",
                "tool.table_drill",
                "tool.table_stackup",
                "tool.table_bom",
                "-",
                "tool.dim_horizontal",
                "tool.dim_vertical",
                "tool.dim_aligned",
                "tool.dim_radius",
                "tool.dim_diameter",
                "tool.dim_ordinate",
                "dimension.board_size",
                "-",
                "tool.callout",
                "tool.notes",
                "tool.text",
                "tool.line",
                "tool.rect",
            ],
            Category::Arrange => &[
                "layout.align:left",
                "layout.align:hcenter",
                "layout.align:right",
                "layout.align:top",
                "layout.align:vcenter",
                "layout.align:bottom",
                "-",
                "layout.distribute:horizontal",
                "layout.distribute:vertical",
                "-",
                "layout.match:width",
                "layout.match:height",
                "layout.match:scale",
                "layout.center",
                "-",
                "item.raise:front",
                "item.raise:forward",
                "item.lower:backward",
                "item.lower:back",
                "-",
                "app.renumber_callouts",
                "link.match_colours",
            ],
            Category::Page => &[
                "page.add",
                "page.duplicate",
                "page.remove",
                "-",
                "app.rename_page",
                "app.choose_sheet",
                "-",
                "app.move_page_up",
                "app.move_page_down",
                "-",
                "app.prev_page",
                "app.next_page",
            ],
            Category::Help => &["app.shortcuts", "app.about"],
        };
        let mut out: Vec<Option<&Entry>> = Vec::new();
        for id in order {
            if *id == "-" {
                if out.last().is_some_and(Option::is_some) {
                    out.push(None);
                }
            } else if let Some(e) = self.get(id) {
                out.push(Some(e));
            }
        }
        let listed: Vec<&str> = order.to_vec();
        let rest: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.category == cat && !listed.contains(&e.id.as_str()))
            .filter(|e| !e.id.starts_with("tool.select") && !e.id.starts_with("tool.pan"))
            .filter(|e| !e.id.starts_with("page.set_paper"))
            .collect();
        if !rest.is_empty() {
            if out.last().is_some_and(Option::is_some) {
                out.push(None);
            }
            out.extend(rest.into_iter().map(Some));
        }
        while out.last().is_some_and(Option::is_none) {
            out.pop();
        }
        out
    }

    /// Entries matching `query` (fuzzy over the title and the category),
    /// best first.
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let mut scored: Vec<(i64, usize, &Entry)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let hay = format!("{} {}", e.title, e.category.title());
                fuzzy(query, &hay).map(|s| (s, i, e))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, _, e)| e).collect()
    }
}

/// Fuzzy score of `query` in `text` (case-insensitive subsequence);
/// consecutive matches and matches at word starts score higher. `None`
/// when not all query characters occur in order.
pub fn fuzzy(query: &str, text: &str) -> Option<i64> {
    let q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut score = 0i64;
    let mut qi = 0;
    let mut last: Option<usize> = None;
    for (i, c) in t.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            let word_start = i == 0 || !t[i - 1].is_alphanumeric();
            score += 1;
            if word_start {
                score += 8;
            }
            if last.is_some_and(|l| l + 1 == i) {
                score += 5;
            }
            if i == 0 {
                score += 3;
            }
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then(|| score - t.len() as i64 / 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse_and_match() {
        let s = Shortcut::parse("Ctrl+Shift+Z").unwrap();
        assert_eq!(s.key, Key::Z);
        assert!(s.mods.shift && s.mods.command);
        let held = Modifiers {
            ctrl: true,
            command: true,
            shift: true,
            ..Default::default()
        };
        assert!(s.matches(Key::Z, held));
        assert!(!s.matches(Key::Z, Modifiers::COMMAND));
        assert_eq!(Shortcut::parse("Ctrl+]").unwrap().key, Key::CloseBracket);
        assert_eq!(Shortcut::parse("Delete").unwrap().key, Key::Delete);
        assert!(Shortcut::parse("Hyper+X").is_none());
    }

    #[test]
    fn catalogue_ids_unique_menus_and_keys() {
        let c = Catalogue::build(&Registry::new());
        let mut ids = std::collections::HashSet::new();
        for e in &c.entries {
            assert!(ids.insert(e.id.clone()), "{}", e.id);
        }
        // No two entries share a shortcut.
        let mut seen = Vec::new();
        for e in &c.entries {
            for s in &e.shortcuts {
                assert!(!seen.contains(s), "{} shares {:?}", e.id, s);
                seen.push(*s);
            }
        }
        for cat in Category::ALL {
            let m = c.menu(cat);
            assert!(!m.is_empty(), "{cat:?}");
            assert!(m.first().unwrap().is_some());
        }
        let arrange: Vec<&str> = c
            .menu(Category::Arrange)
            .into_iter()
            .flatten()
            .map(|e| e.id.as_str())
            .collect();
        assert!(arrange.contains(&"layout.align:left"));
        assert!(arrange.contains(&"item.raise:front"));
        assert_eq!(
            c.for_key(Key::D, Modifiers::COMMAND).map(|e| e.id.as_str()),
            Some("item.duplicate")
        );
    }

    #[test]
    fn fuzzy_search_ranks_word_starts() {
        assert!(fuzzy("al", "Align left").unwrap() > fuzzy("al", "Duplicate all").unwrap_or(-99));
        assert!(fuzzy("xyz", "Align left").is_none());
        let c = Catalogue::build(&Registry::new());
        let r = c.search("align left");
        assert_eq!(r[0].id, "layout.align:left");
        let r = c.search("exp pdf");
        assert_eq!(r[0].id, "app.export_pdf");
        assert_eq!(c.search("").len(), c.entries.len());
    }
}
