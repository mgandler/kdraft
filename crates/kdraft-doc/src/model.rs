//! The `.kdraft` document model.
//!
//! Sheet coordinates are millimetres with the origin at the sheet's top-left
//! corner and Y pointing down (as in KiCad drawing sheets). Positions that
//! refer to the board (anchors, crop rectangles) are board coordinates.

use std::fmt;

use kdraft_board::geom::{BBox, Point};
use kdraft_sexpr::Value;

pub const CURRENT_VERSION: u32 = 1;

/// Stable identifier. New ids are random UUIDs; any non-empty string read
/// from a file is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub String);

impl Id {
    pub fn new_v4() -> Id {
        Id(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_uuid(&self) -> bool {
        uuid::Uuid::parse_str(&self.0).is_ok()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub version: u32,
    pub source: Source,
    pub styles: Styles,
    /// Document-wide drawing sheet override: a `.kicad_wks` path relative to
    /// the document folder, or `default` for KiCad's default sheet. `None`
    /// falls back to the project's sheet, then the default.
    pub sheet: Option<String>,
    pub pages: Vec<Page>,
    /// Unknown top-level nodes, preserved in order.
    pub extra: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Source {
    /// Board path, relative to the document's folder, `/`-separated.
    pub board: String,
    pub variant: Option<String>,
    /// `kicad-cli` version string last used to generate views.
    pub kicad_cli: Option<String>,
    /// blake3 of the board file when the document was last saved; `kdraft
    /// check` reports a stale document when the board differs.
    pub board_hash: Option<String>,
    pub extra: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Styles {
    /// Default text height, mm.
    pub text_height: f64,
    /// Default line width, mm.
    pub line_width: f64,
    /// Snap grid for placing items on the sheet, mm.
    pub snap_grid: f64,
    /// Dimension style; written only where it differs from the default.
    pub dimension: kdraft_layout::dimension::DimStyle,
    /// Appearance defaults per item category (only overrides of the
    /// built-in defaults are stored).
    pub appearance: crate::style::StyleSet,
    /// Callout labels are numbered across the document instead of per
    /// page (`(callout_numbering document)`).
    pub callouts_per_document: bool,
    /// Report gaps in callout label sequences (`(strict_callout_numbering
    /// yes)`).
    pub strict_callout_numbering: bool,
    /// The document's default layer palette, resolved (`None`: the older
    /// `layer_preset`, else black).
    pub layer_palette: Option<crate::palette::Palette>,
    pub extra: Vec<Value>,
}

/// Default snap grid, mm.
pub const DEFAULT_SNAP_GRID: f64 = 0.5;

impl Default for Styles {
    fn default() -> Self {
        Styles {
            text_height: 2.5,
            line_width: 0.25,
            snap_grid: DEFAULT_SNAP_GRID,
            dimension: Default::default(),
            appearance: Default::default(),
            callouts_per_document: false,
            strict_callout_numbering: false,
            layer_palette: None,
            extra: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageSize {
    /// `A4`, `A3`, ..., or `User`.
    pub name: String,
    pub width: f64,
    pub height: f64,
}

impl PageSize {
    pub fn a3_landscape() -> PageSize {
        PageSize {
            name: "A3".to_string(),
            width: 420.0,
            height: 297.0,
        }
    }

    pub fn a4_landscape() -> PageSize {
        PageSize {
            name: "A4".to_string(),
            width: 297.0,
            height: 210.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub id: Id,
    /// Per-page drawing sheet override (`.kicad_wks` path relative to the
    /// document folder, or `default`); `None` uses the document's choice.
    pub sheet: Option<String>,
    pub size: PageSize,
    pub title: Option<String>,
    /// z-ordered, bottom first. Unknown nodes inside a page are kept here
    /// as [`Item::Unknown`] so their position in the z-order survives.
    pub items: Vec<Item>,
}

impl Page {
    pub fn new(title: &str, size: PageSize) -> Page {
        Page {
            id: Id::new_v4(),
            sheet: None,
            size,
            title: Some(title.to_string()),
            items: Vec::new(),
        }
    }

    pub fn views(&self) -> impl Iterator<Item = &View> {
        self.items.iter().filter_map(|i| match i {
            Item::View(v) => Some(v),
            _ => None,
        })
    }

    pub fn item(&self, id: &Id) -> Option<&Item> {
        self.items.iter().find(|i| i.id() == Some(id))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    View(View),
    Table(Table),
    Dimension(Dimension),
    Callout(Callout),
    NoteList(NoteList),
    Graphic(Graphic),
    /// An item kind this version does not know, preserved verbatim.
    Unknown(Value),
}

impl Item {
    /// The item's id; `None` for unknown items.
    pub fn id(&self) -> Option<&Id> {
        match self {
            Item::View(v) => Some(&v.id),
            Item::Table(v) => Some(&v.id),
            Item::Dimension(v) => Some(&v.id),
            Item::Callout(v) => Some(&v.id),
            Item::NoteList(v) => Some(&v.id),
            Item::Graphic(v) => Some(&v.id),
            Item::Unknown(_) => None,
        }
    }

    pub(crate) fn id_mut(&mut self) -> Option<&mut Id> {
        match self {
            Item::View(v) => Some(&mut v.id),
            Item::Table(v) => Some(&mut v.id),
            Item::Dimension(v) => Some(&mut v.id),
            Item::Callout(v) => Some(&mut v.id),
            Item::NoteList(v) => Some(&mut v.id),
            Item::Graphic(v) => Some(&mut v.id),
            Item::Unknown(_) => None,
        }
    }

    /// Moves the item on the sheet. Returns false for items without an own
    /// sheet position (dimensions follow their anchors; unknown items).
    pub fn translate(&mut self, dx: f64, dy: f64) -> bool {
        let d = Point::new(dx, dy);
        match self {
            Item::View(v) => v.at = v.at + d,
            Item::Table(t) => t.at = t.at + d,
            Item::NoteList(n) => n.at = n.at + d,
            Item::Callout(c) => {
                c.at = c.at + d;
            }
            Item::Graphic(g) => g.shape.translate(d),
            Item::Dimension(_) | Item::Unknown(_) => return false,
        }
        true
    }

    /// The item's reference point on the sheet: view centre, table and
    /// note-list top-left, callout text position, a graphic's first point
    /// (line/rect start, circle centre, arc start, first polygon vertex,
    /// text/image anchor). `None` for dimensions and unknown items.
    pub fn position(&self) -> Option<Point> {
        Some(match self {
            Item::View(v) => v.at,
            Item::Table(t) => t.at,
            Item::NoteList(n) => n.at,
            Item::Callout(c) => c.at,
            Item::Graphic(g) => match &g.shape {
                GraphicShape::Line { start, .. }
                | GraphicShape::Rect { start, .. }
                | GraphicShape::Arc { start, .. } => *start,
                GraphicShape::Circle { center, .. } => *center,
                GraphicShape::Polygon { points } | GraphicShape::Polyline { points } => {
                    *points.first()?
                }
                GraphicShape::Text { at, .. } | GraphicShape::Image { at, .. } => *at,
            },
            Item::Dimension(_) | Item::Unknown(_) => return None,
        })
    }

    pub(crate) fn extra_values(&self) -> Option<&Vec<Value>> {
        Some(match self {
            Item::View(v) => &v.extra,
            Item::Table(v) => &v.extra,
            Item::Dimension(v) => &v.extra,
            Item::Callout(v) => &v.extra,
            Item::NoteList(v) => &v.extra,
            Item::Graphic(v) => &v.extra,
            Item::Unknown(_) => return None,
        })
    }

    pub(crate) fn extra_values_mut(&mut self) -> Option<&mut Vec<Value>> {
        Some(match self {
            Item::View(v) => &mut v.extra,
            Item::Table(v) => &mut v.extra,
            Item::Dimension(v) => &mut v.extra,
            Item::Callout(v) => &mut v.extra,
            Item::NoteList(v) => &mut v.extra,
            Item::Graphic(v) => &mut v.extra,
            Item::Unknown(_) => return None,
        })
    }

    /// User-given name (`(name "...")`), shown in the item tree.
    pub fn name(&self) -> Option<&str> {
        self.extra_values()?
            .iter()
            .find(|v| v.is("name"))
            .and_then(|v| v.args().first())
            .and_then(Value::atom_text)
    }

    fn flag(&self, head: &str) -> bool {
        self.extra_values()
            .is_some_and(|e| e.iter().any(|v| v.is(head) && v.has_symbol("yes")))
    }

    /// Hidden items are not drawn on the sheet or in exports
    /// (`(hidden yes)`).
    pub fn hidden(&self) -> bool {
        self.flag("hidden")
    }

    /// Locked items cannot be picked or moved on the canvas
    /// (`(locked yes)`).
    pub fn locked(&self) -> bool {
        self.flag("locked")
    }

    /// Sets or clears the name (empty clears). False for unknown items.
    pub fn set_name(&mut self, name: Option<&str>) -> bool {
        let Some(e) = self.extra_values_mut() else {
            return false;
        };
        e.retain(|v| !v.is("name"));
        if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
            e.insert(0, Value::node("name", [Value::str(n)]));
        }
        true
    }

    fn set_flag(&mut self, head: &str, on: bool) -> bool {
        let Some(e) = self.extra_values_mut() else {
            return false;
        };
        e.retain(|v| !v.is(head));
        if on {
            e.push(Value::node(head, [Value::sym("yes")]));
        }
        true
    }

    pub fn set_hidden(&mut self, on: bool) -> bool {
        self.set_flag("hidden", on)
    }

    pub fn set_locked(&mut self, on: bool) -> bool {
        self.set_flag("locked", on)
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Item::View(_) => "view",
            Item::Table(_) => "table",
            Item::Dimension(_) => "dimension",
            Item::Callout(_) => "callout",
            Item::NoteList(_) => "notes",
            Item::Graphic(_) => "graphic",
            Item::Unknown(_) => "unknown",
        }
    }
}

/// An enum read from a symbol that keeps unknown spellings.
macro_rules! symbol_enum {
    ($name:ident { $($var:ident = $s:literal),* $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum $name {
            $($var,)*
            Other(String),
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$var => $s,)*
                    $name::Other(s) => s,
                }
            }

            pub fn parse(s: &str) -> $name {
                match s {
                    $($s => $name::$var,)*
                    other => $name::Other(other.to_string()),
                }
            }
        }
    };
}

symbol_enum!(ViewKind {
    Fabrication = "fabrication",
    Assembly = "assembly",
    Drill = "drill",
    Detail = "detail",
    Layer = "layer",
    Render3d = "render3d",
});

symbol_enum!(TableKind {
    Bom = "bom",
    Drill = "drill",
    Stackup = "stackup",
    Custom = "custom",
});

// `linear`: horizontal or vertical, whichever is larger; `radial`: older
// spelling of `radius`, read and written as is.
symbol_enum!(DimensionKind {
    Linear = "linear",
    Horizontal = "horizontal",
    Vertical = "vertical",
    Aligned = "aligned",
    Radius = "radius",
    Radial = "radial",
    Diameter = "diameter",
    Ordinate = "ordinate",
    Angular = "angular",
});

symbol_enum!(DrillMarks {
    None = "none",
    Small = "small",
    Actual = "actual",
});

impl DrillMarks {
    /// Default for a view kind when the document does not say: none on
    /// assembly views, actual size elsewhere (kicad-cli's own default).
    pub fn default_for(kind: &ViewKind) -> DrillMarks {
        match kind {
            ViewKind::Assembly => DrillMarks::None,
            _ => DrillMarks::Actual,
        }
    }
}

symbol_enum!(RenderMode {
    Plot = "plot",
    Native = "native",
});

symbol_enum!(DnpStyle {
    None = "none",
    Crossed = "crossed",
    Greyed = "greyed",
    Hidden = "hidden",
});

symbol_enum!(RefFilter {
    Placed = "placed",
    PosFiles = "pos_files",
    All = "all",
});

/// Default minimum and maximum designator cap height on paper, mm.
pub const DESIGNATOR_MIN_H: f64 = 0.9;
pub const DESIGNATOR_MAX_H: f64 = 2.5;

/// Settings of native assembly rendering; unset fields use the defaults
/// of the getters.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssemblyOptions {
    /// Footprint values (native: inside the part; plot: the `Value` text
    /// on fab layers). Default off.
    pub values: Option<bool>,
    /// Reference designators (native: placed designators; plot: the
    /// `Reference` text and `${REFERENCE}` user texts on fab layers).
    /// Default on.
    pub references: Option<bool>,
    /// Other footprint texts on fab layers (plot views; native views
    /// draw none). Default on.
    pub fab_other: Option<bool>,
    pub pads: Option<bool>,
    pub holes: Option<bool>,
    pub pin1: Option<bool>,
    pub dnp: Option<DnpStyle>,
    /// Which footprints get designators.
    pub refs: Option<RefFilter>,
    /// Designator cap height range on paper, mm.
    pub text_min: Option<f64>,
    pub text_max: Option<f64>,
    pub extra: Vec<Value>,
}

impl AssemblyOptions {
    pub fn values(&self) -> bool {
        self.values.unwrap_or(false)
    }
    pub fn references(&self) -> bool {
        self.references.unwrap_or(true)
    }
    pub fn fab_other(&self) -> bool {
        self.fab_other.unwrap_or(true)
    }
    /// The three fab-text switches for plot views.
    pub fn fab_show(&self) -> kdraft_board::FabTextShow {
        kdraft_board::FabTextShow {
            values: self.values(),
            references: self.references(),
            other: self.fab_other(),
        }
    }
    pub fn pads(&self) -> bool {
        self.pads.unwrap_or(true)
    }
    pub fn holes(&self) -> bool {
        self.holes.unwrap_or(false)
    }
    pub fn pin1(&self) -> bool {
        self.pin1.unwrap_or(true)
    }
    pub fn dnp(&self) -> DnpStyle {
        self.dnp.clone().unwrap_or(DnpStyle::Crossed)
    }
    /// Default `placed`: no designators for `board_only` footprints and
    /// footprints without copper pads (mounting holes, fiducials, logos).
    pub fn refs(&self) -> RefFilter {
        self.refs.clone().unwrap_or(RefFilter::Placed)
    }
    pub fn text_min(&self) -> f64 {
        self.text_min.unwrap_or(DESIGNATOR_MIN_H)
    }
    pub fn text_max(&self) -> f64 {
        self.text_max
            .unwrap_or(DESIGNATOR_MAX_H)
            .max(self.text_min())
    }
    pub fn is_empty(&self) -> bool {
        *self == AssemblyOptions::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewSide {
    Front,
    Back,
}

impl ViewSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            ViewSide::Front => "front",
            ViewSide::Back => "back",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub id: Id,
    pub kind: ViewKind,
    pub side: ViewSide,
    /// Sheet mm per board mm: 2 means 2:1.
    pub scale: f64,
    /// Degrees, counter-clockwise on the sheet.
    pub rotation: f64,
    /// Sheet position of the centre of the view content (the board outline
    /// box, or `crop` when set).
    pub at: Point,
    pub layers: Vec<String>,
    /// Board-coordinate crop rectangle.
    pub crop: Option<BBox>,
    /// Parent view for detail and section views.
    pub parent: Option<Id>,
    pub style: Option<String>,
    pub frame: bool,
    /// Drill marks in plotted layers (`--drill-shape-opt`); `None` uses
    /// [`DrillMarks::default_for`] the view kind.
    pub drill_marks: Option<DrillMarks>,
    /// `plot` (kicad-cli layers) or `native` (board model); `None` uses
    /// [`View::render_mode`]'s default.
    pub render: Option<RenderMode>,
    pub assembly: AssemblyOptions,
    pub extra: Vec<Value>,
}

impl View {
    /// Render mode in effect: native for assembly and drill views, plot
    /// otherwise (falls back to native when kicad-cli is unavailable).
    pub fn render_mode(&self) -> RenderMode {
        match &self.render {
            Some(r @ (RenderMode::Plot | RenderMode::Native)) => r.clone(),
            _ => match self.kind {
                ViewKind::Assembly | ViewKind::Drill => RenderMode::Native,
                _ => RenderMode::Plot,
            },
        }
    }

    /// Wants kicad-cli plots (drill views never do).
    pub fn wants_plot(&self) -> bool {
        !matches!(self.kind, ViewKind::Drill | ViewKind::Render3d)
            && self.render_mode() == RenderMode::Plot
    }

    /// Drill marks in effect for this view.
    pub fn drill_marks(&self) -> DrillMarks {
        self.drill_marks
            .clone()
            .unwrap_or_else(|| DrillMarks::default_for(&self.kind))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub id: Id,
    pub kind: TableKind,
    /// Sheet position of the table's top-left corner.
    pub at: Point,
    pub columns: Vec<String>,
    pub style: Option<String>,
    /// BOM tables: stable item numbers by group key, so numbers (and the
    /// callouts that show them) survive updates. New groups get the next
    /// free numbers.
    pub bom_items: Vec<(String, usize)>,
    /// BOM the stored numbers belong to (`schematic` or `pcb`; `None` in
    /// documents written before this was recorded: the PCB fallback).
    pub bom_source: Option<String>,
    /// A chunk of a split table: first row and row count.
    pub rows: Option<(usize, usize)>,
    pub extra: Vec<Value>,
}

impl Table {
    pub fn new(kind: TableKind, at: Point) -> Table {
        Table {
            id: Id::new_v4(),
            kind,
            at,
            columns: Vec::new(),
            style: None,
            bom_items: Vec::new(),
            bom_source: None,
            rows: None,
            extra: Vec::new(),
        }
    }
}

/// Typed reference to a board feature.
#[derive(Debug, Clone, PartialEq)]
pub enum Anchor {
    /// `(pad "U7" "3")`
    Pad { reference: String, number: String },
    /// `(hole 12)`: index into the board's hole list.
    Hole(usize),
    /// `(outline-vertex 4)`: vertex of the outer outline contour.
    OutlineVertex(usize),
    /// `(fp-origin "J2")`
    FpOrigin(String),
    /// `(view-point x y)`: unbound point in board coordinates.
    ViewPoint(Point),
    /// `(outline-edge N)`: primitive N of the board outline (for arcs and
    /// circles: their centre and radius).
    OutlineEdge(usize),
    /// Unknown anchor kind, preserved.
    Unknown(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dimension {
    pub id: Id,
    pub kind: DimensionKind,
    pub view: Option<Id>,
    pub from: Option<Anchor>,
    pub to: Option<Anchor>,
    /// Distance of the dimension line from the measured feature, sheet mm.
    pub offset: f64,
    pub style: Option<String>,
    /// Tolerance text after the value, e.g. `±0.1` or `+0.1/-0.05`.
    pub tolerance: Option<String>,
    /// Ordinate dimensions: `x` or `y`.
    pub axis: Option<String>,
    /// Radius/diameter: direction of the leader, degrees CCW on the sheet.
    pub angle: Option<f64>,
    /// Last resolved anchor positions in board coordinates (from, to; for
    /// radius/diameter: centre and a point on the circle). Used to draw a
    /// broken dimension where it was.
    pub last: Vec<Point>,
    pub extra: Vec<Value>,
}

impl Dimension {
    pub fn new(
        kind: DimensionKind,
        view: Id,
        from: Anchor,
        to: Option<Anchor>,
        offset: f64,
    ) -> Dimension {
        Dimension {
            id: Id::new_v4(),
            kind,
            view: Some(view),
            from: Some(from),
            to,
            offset,
            style: None,
            tolerance: None,
            axis: None,
            angle: None,
            last: Vec::new(),
            extra: Vec::new(),
        }
    }
}

impl Dimension {
    /// Text position along a linear dimension's line, sheet mm from the
    /// middle (`(text_shift v)`; 0 when absent).
    pub fn text_shift(&self) -> f64 {
        self.extra
            .iter()
            .find(|v| v.is("text_shift"))
            .and_then(|v| v.f64_at(1).ok())
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
    }

    pub fn set_text_shift(&mut self, v: f64) {
        let at = self.extra.iter().position(|x| x.is("text_shift"));
        self.extra.retain(|x| !x.is("text_shift"));
        if v != 0.0 && v.is_finite() {
            if let Ok(n) = Value::num(v) {
                let i = at.unwrap_or(self.extra.len()).min(self.extra.len());
                self.extra.insert(i, Value::node("text_shift", [n]));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Callout {
    pub id: Id,
    /// Sheet position of the callout text.
    pub at: Point,
    pub view: Option<Id>,
    pub anchor: Option<Anchor>,
    /// Free text shown in the balloon (normally a number) when there is no
    /// target.
    pub label: Option<String>,
    /// Derived content: `note:<id>`, `bom:<group key>` or `refdes`; wins
    /// over `label`.
    pub target: Option<String>,
    /// Intermediate leader points, sheet mm.
    pub leader: Vec<Point>,
    pub extra: Vec<Value>,
}

/// What a callout's balloon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalloutKind {
    /// Its own label.
    Label,
    /// The number of a note.
    Note,
    /// The BOM item number of a part.
    Bom,
    /// The reference designator of the anchored footprint.
    Refdes,
}

impl CalloutKind {
    pub const ALL: [CalloutKind; 4] = [
        CalloutKind::Label,
        CalloutKind::Note,
        CalloutKind::Bom,
        CalloutKind::Refdes,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CalloutKind::Label => "label",
            CalloutKind::Note => "note",
            CalloutKind::Bom => "bom",
            CalloutKind::Refdes => "refdes",
        }
    }

    pub fn parse(s: &str) -> Option<CalloutKind> {
        CalloutKind::ALL
            .into_iter()
            .find(|k| k.as_str() == s.trim())
    }
}

/// `target` value of a designator callout.
pub const REFDES_TARGET: &str = "refdes";

impl Callout {
    /// A label callout.
    pub fn new(at: Point, view: Option<Id>, anchor: Option<Anchor>, label: &str) -> Callout {
        Callout {
            id: Id::new_v4(),
            at,
            view,
            anchor,
            label: Some(label.to_string()),
            target: None,
            leader: Vec::new(),
            extra: Vec::new(),
        }
    }

    pub fn kind(&self) -> CalloutKind {
        match self.target.as_deref() {
            None => CalloutKind::Label,
            Some(REFDES_TARGET) => CalloutKind::Refdes,
            Some(t) if t.starts_with("bom:") => CalloutKind::Bom,
            Some(_) => CalloutKind::Note,
        }
    }

    /// The reference designator the anchor points at (pad or footprint
    /// origin).
    pub fn anchor_reference(&self) -> Option<&str> {
        match self.anchor.as_ref()? {
            Anchor::Pad { reference, .. } | Anchor::FpOrigin(reference) => Some(reference),
            _ => None,
        }
    }

    /// Shows `label` (clears a derived target).
    pub fn set_label(&mut self, label: &str) {
        self.label = Some(label.to_string());
        self.target = None;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// Stable id, e.g. "N1"; callouts refer to it.
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteList {
    pub id: Id,
    pub at: Point,
    /// Heading above the list (e.g. `NOTES`); `None` for none.
    pub title: Option<String>,
    /// Wrap width, mm (`None`: no wrapping).
    pub width: Option<f64>,
    pub notes: Vec<Note>,
    pub extra: Vec<Value>,
}

impl NoteList {
    /// 1-based number of the note with stable id `id`.
    pub fn number_of(&self, id: &str) -> Option<usize> {
        self.notes.iter().position(|n| n.id == id).map(|i| i + 1)
    }

    /// A new stable id (`N<k>`, the smallest unused).
    pub fn next_id(&self) -> String {
        (1..)
            .map(|k| format!("N{k}"))
            .find(|c| !self.notes.iter().any(|n| &n.id == c))
            .unwrap_or_default()
    }
}

/// Default fab notes for `doc new`: neutral, editable, using the board
/// variables (`${BOARD_WIDTH}` ...) resolved at render time.
pub const DEFAULT_FAB_NOTES: [&str; 5] = [
    "Board outline as drawn; overall size ${BOARD_WIDTH} x ${BOARD_HEIGHT} mm.",
    "Board thickness ${BOARD_THICKNESS} mm, ${LAYER_COUNT} copper layers; layer stack as in the layer stack table.",
    "Surface finish: ${COPPER_FINISH}.",
    "Holes as listed in the drill table: ${HOLE_COUNT} holes, smallest ${MIN_DRILL} mm.",
    "All dimensions in millimetres.",
];

#[derive(Debug, Clone, PartialEq)]
pub enum GraphicShape {
    Line {
        start: Point,
        end: Point,
    },
    Rect {
        start: Point,
        end: Point,
        /// Corner radius, mm (0: square corners).
        radius: f64,
    },
    Circle {
        center: Point,
        radius: f64,
    },
    Arc {
        start: Point,
        mid: Point,
        end: Point,
    },
    /// Closed polygon.
    Polygon {
        points: Vec<Point>,
    },
    /// Open polyline.
    Polyline {
        points: Vec<Point>,
    },
    /// Text, possibly several lines (`\n`). `at` is the baseline of the
    /// first line at the alignment edge (left, centre or right per the
    /// item's style).
    Text {
        at: Point,
        text: String,
        height: Option<f64>,
        /// Degrees, counter-clockwise on the sheet, about `at`.
        rotation: f64,
        /// Wrap width, mm (`None`: break at newlines only).
        wrap: Option<f64>,
    },
    Image {
        at: Point,
        path: String,
        width: f64,
        height: f64,
    },
}

impl GraphicShape {
    /// The defining points: line start and end, rectangle corners,
    /// polyline/polygon vertices, arc start/mid/end, circle centre and the
    /// point on the circle at angle 0, text/image anchor.
    pub fn points(&self) -> Vec<Point> {
        match self {
            GraphicShape::Line { start, end } | GraphicShape::Rect { start, end, .. } => {
                vec![*start, *end]
            }
            GraphicShape::Polyline { points } | GraphicShape::Polygon { points } => points.clone(),
            GraphicShape::Arc { start, mid, end } => vec![*start, *mid, *end],
            GraphicShape::Circle { center, radius } => {
                vec![*center, Point::new(center.x + radius, center.y)]
            }
            GraphicShape::Text { at, .. } | GraphicShape::Image { at, .. } => vec![*at],
        }
    }

    /// Sets the defining points of a shape; `Err` with a reason when the count
    /// does not fit.
    pub fn set_points(&mut self, v: &[Point]) -> Result<(), String> {
        let need = |n: usize| {
            if v.len() == n {
                Ok(())
            } else {
                Err(format!("this shape takes {n} points, got {}", v.len()))
            }
        };
        match self {
            GraphicShape::Line { start, end } | GraphicShape::Rect { start, end, .. } => {
                need(2)?;
                *start = v[0];
                *end = v[1];
            }
            GraphicShape::Polyline { points } => {
                if v.len() < 2 {
                    return Err("a polyline needs at least 2 points".into());
                }
                *points = v.to_vec();
            }
            GraphicShape::Polygon { points } => {
                if v.len() < 3 {
                    return Err("a polygon needs at least 3 points".into());
                }
                *points = v.to_vec();
            }
            GraphicShape::Arc { start, mid, end } => {
                need(3)?;
                *start = v[0];
                *mid = v[1];
                *end = v[2];
            }
            GraphicShape::Circle { center, radius } => {
                need(2)?;
                let r = v[0].dist(v[1]);
                if r <= 1e-9 {
                    return Err("the circle point must differ from the centre".into());
                }
                *center = v[0];
                *radius = r;
            }
            GraphicShape::Text { at, .. } | GraphicShape::Image { at, .. } => {
                need(1)?;
                *at = v[0];
            }
        }
        Ok(())
    }

    pub fn translate(&mut self, d: Point) {
        match self {
            GraphicShape::Line { start, end } | GraphicShape::Rect { start, end, .. } => {
                *start = *start + d;
                *end = *end + d;
            }
            GraphicShape::Circle { center, .. } => *center = *center + d,
            GraphicShape::Arc { start, mid, end } => {
                *start = *start + d;
                *mid = *mid + d;
                *end = *end + d;
            }
            GraphicShape::Polygon { points } | GraphicShape::Polyline { points } => {
                for p in points {
                    *p = *p + d;
                }
            }
            GraphicShape::Text { at, .. } | GraphicShape::Image { at, .. } => *at = *at + d,
        }
    }

    /// Sheet bounding box; text is reduced to its anchor point.
    pub fn bbox(&self) -> Option<BBox> {
        match self {
            GraphicShape::Line { start, end } | GraphicShape::Rect { start, end, .. } => {
                BBox::from_points([*start, *end])
            }
            GraphicShape::Circle { center, radius } => Some(BBox {
                min: Point::new(center.x - radius, center.y - radius),
                max: Point::new(center.x + radius, center.y + radius),
            }),
            GraphicShape::Arc { start, mid, end } => {
                Some(kdraft_board::geom::arc_bbox(*start, *mid, *end))
            }
            GraphicShape::Polygon { points } | GraphicShape::Polyline { points } => {
                BBox::from_points(points.iter().copied())
            }
            GraphicShape::Text { at, .. } => Some(BBox::from_point(*at)),
            GraphicShape::Image {
                at, width, height, ..
            } => Some(BBox {
                min: *at,
                max: Point::new(at.x + width, at.y + height),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Graphic {
    pub id: Id,
    pub shape: GraphicShape,
    pub style: Option<String>,
    pub extra: Vec<Value>,
}
