use std::collections::BTreeMap;

use serde::Serialize;

use crate::geom::{BBox, Point};
use crate::outline::{EdgeShape, Outline};

/// blake3 hash of the board file bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct BoardHash(#[serde(serialize_with = "ser_hex")] pub [u8; 32]);

impl BoardHash {
    pub fn of(bytes: &[u8]) -> BoardHash {
        BoardHash(*blake3::hash(bytes).as_bytes())
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

fn ser_hex<S: serde::Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&BoardHash(*v).to_hex())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Board {
    /// File format version, e.g. 20241229 (KiCad 9) or 20260206 (KiCad 10).
    pub version: i64,
    pub generator: Option<String>,
    pub generator_version: Option<String>,
    /// `(general (thickness ...))` in mm.
    pub thickness: Option<f64>,
    pub paper: Option<String>,
    pub layers: Vec<Layer>,
    pub title_block: TitleBlock,
    pub stackup: Option<Stackup>,
    pub footprints: Vec<Footprint>,
    pub vias: Vec<Via>,
    pub outline: Outline,
    pub hash: BoardHash,
    /// Things kdraft read but could not interpret; never fatal.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Layer {
    pub ordinal: i64,
    /// Canonical name, e.g. `F.SilkS`.
    pub name: String,
    /// `signal`, `power`, `mixed`, `jumper` or `user`.
    pub kind: String,
    /// User-visible name, e.g. `F.Silkscreen`.
    pub user_name: Option<String>,
}

impl Layer {
    pub fn is_copper(&self) -> bool {
        self.name.ends_with(".Cu")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TitleBlock {
    pub title: Option<String>,
    pub date: Option<String>,
    pub rev: Option<String>,
    pub company: Option<String>,
    /// Comment number (1..9) to text.
    pub comments: BTreeMap<u32, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Stackup {
    /// Top to bottom. A dielectric with sublayers yields one entry per
    /// sublayer, sharing the name, with `sublayer` counting from 0.
    pub layers: Vec<StackupLayer>,
    pub copper_finish: Option<String>,
    pub dielectric_constraints: Option<bool>,
    pub edge_connector: Option<String>,
    pub castellated_pads: Option<bool>,
    pub edge_plating: Option<bool>,
}

impl Stackup {
    pub fn copper_count(&self) -> usize {
        self.layers.iter().filter(|l| l.kind == "copper").count()
    }

    /// Sum of all layer thicknesses.
    pub fn total_thickness(&self) -> f64 {
        self.layers.iter().filter_map(|l| l.thickness).sum()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StackupLayer {
    pub name: String,
    /// `copper`, `core`, `prepreg`, `Top Solder Mask`, ...
    pub kind: String,
    pub sublayer: u32,
    pub thickness: Option<f64>,
    pub material: Option<String>,
    pub epsilon_r: Option<f64>,
    pub loss_tangent: Option<f64>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Front,
    Back,
}

impl Side {
    pub fn from_layer(layer: &str) -> Side {
        if layer.starts_with("B.") {
            Side::Back
        } else {
            Side::Front
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Front => "front",
            Side::Back => "back",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MountType {
    Smd,
    ThroughHole,
    /// Neither `smd` nor `through_hole` in `(attr ...)`.
    Unspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Attributes {
    pub mount: MountType,
    pub exclude_from_pos_files: bool,
    pub exclude_from_bom: bool,
    pub dnp: bool,
    pub board_only: bool,
}

impl Default for Attributes {
    fn default() -> Self {
        Attributes {
            mount: MountType::Unspecified,
            exclude_from_pos_files: false,
            exclude_from_bom: false,
            dnp: false,
            board_only: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Footprint {
    /// Library id, e.g. `Resistor_SMD:R_0603_1608Metric`.
    pub lib_id: String,
    pub uuid: Option<String>,
    pub reference: Option<String>,
    pub value: Option<String>,
    /// Footprint layer, `F.Cu` or `B.Cu`.
    pub layer: String,
    pub side: Side,
    pub position: Point,
    /// Degrees, counter-clockwise on screen.
    pub rotation: f64,
    pub locked: bool,
    pub attributes: Attributes,
    /// All `(property name value)` entries, including Reference and Value.
    pub properties: BTreeMap<String, String>,
    pub pads: Vec<Pad>,
    /// Bounding box of courtyard graphics in board coordinates.
    pub courtyard: Option<BBox>,
    /// Bounding box of fab-layer graphics in board coordinates.
    pub fab: Option<BBox>,
    /// Graphics on the fab, courtyard and silkscreen layers of both sides.
    pub graphics: Vec<FpGraphic>,
    /// Footprint texts: `property` fields with a position and `fp_text`.
    pub texts: Vec<FpText>,
}

impl Footprint {
    /// Has a pad with copper (not an NPTH, non-zero size).
    pub fn has_copper_pads(&self) -> bool {
        self.pads.iter().any(|p| {
            !matches!(p.kind, PadType::NpThruHole)
                && p.layers.iter().any(|l| l.ends_with(".Cu"))
                && p.size.w > 0.0
        })
    }

    /// Body box in board coordinates and where it came from: fab graphics
    /// on the part's side, else courtyard graphics, else the pads' boxes
    /// (`FpLayer::Silk` marks that last case).
    pub fn body(&self) -> Option<(BBox, FpLayer)> {
        for kind in [FpLayer::Fab, FpLayer::Courtyard] {
            let b = self
                .graphics
                .iter()
                .filter(|g| g.kind == kind && g.side == self.side)
                .filter_map(|g| g.shape.bbox())
                .reduce(|a, b| a.union(&b));
            if let Some(b) = b {
                return Some((b, kind));
            }
        }
        let b = self
            .pads
            .iter()
            .filter(|p| p.size.w > 0.0)
            .filter_map(|p| {
                let (hw, hh) = (p.size.w / 2.0, p.size.h / 2.0);
                BBox::from_points(
                    [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
                        .map(|(x, y)| p.position + Point::new(x, y).rotate_kicad(p.rotation)),
                )
            })
            .reduce(|a, b| a.union(&b))?;
        Some((b, FpLayer::Silk))
    }
}

/// Which family of footprint layers a graphic or text is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FpLayer {
    Fab,
    Courtyard,
    Silk,
}

impl FpLayer {
    /// `F.Fab`, `B.CrtYd`, `F.SilkS` (and the user names `F.Courtyard`,
    /// `F.Silkscreen`) to family and side.
    pub fn classify(layer: &str) -> Option<(FpLayer, Side)> {
        let side = if layer.starts_with("F.") {
            Side::Front
        } else if layer.starts_with("B.") {
            Side::Back
        } else {
            return None;
        };
        let kind = match &layer[2..] {
            "Fab" => FpLayer::Fab,
            "CrtYd" | "Courtyard" => FpLayer::Courtyard,
            "SilkS" | "Silkscreen" => FpLayer::Silk,
            _ => return None,
        };
        Some((kind, side))
    }
}

/// One footprint graphic (`fp_line`, `fp_rect`, `fp_circle`, `fp_arc`,
/// `fp_poly`, `fp_curve`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FpGraphic {
    pub layer: String,
    pub kind: FpLayer,
    pub side: Side,
    /// Board coordinates.
    pub shape: EdgeShape,
    /// Footprint-local coordinates (file values, unrotated).
    pub local: EdgeShape,
    /// Stroke width, mm (0 if not given).
    pub width: f64,
    pub filled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FpTextKind {
    Reference,
    Value,
    /// Another `property` with a position (e.g. Footprint, Datasheet).
    Property(String),
    /// `fp_text user`.
    User,
}

/// A footprint text. KiCad (10.0.8 parser) stores the position relative
/// to the footprint (unrotated) and the angle as an absolute angle.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FpText {
    pub kind: FpTextKind,
    /// Text as stored, e.g. `${REFERENCE}`.
    pub text: String,
    pub layer: String,
    /// Footprint-local position (file value).
    pub local: Point,
    /// Board position.
    pub position: Point,
    /// Absolute text angle, degrees counter-clockwise (file value).
    pub angle: f64,
    /// Character width and height, mm.
    pub size: Size,
    pub thickness: Option<f64>,
    pub hidden: bool,
    /// KiCad's "keep upright": absent `unlocked` means true.
    pub keep_upright: bool,
    pub mirrored: bool,
    /// `left`, `right`, `top`, `bottom` from `(justify ...)`.
    pub justify: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PadType {
    Smd,
    ThruHole,
    NpThruHole,
    Connect,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pad {
    pub number: String,
    pub kind: PadType,
    /// `rect`, `roundrect`, `circle`, `oval`, `trapezoid`, `custom`, ...
    pub shape: String,
    /// Position relative to the footprint origin, unrotated (file value).
    pub rel_position: Point,
    /// Rotation relative to the footprint, degrees.
    pub rel_rotation: f64,
    /// Absolute board position.
    pub position: Point,
    /// Absolute rotation, degrees (the file stores this value).
    pub rotation: f64,
    pub size: Size,
    /// `(roundrect_rratio r)`: corner radius as a fraction of the smaller
    /// side, for `roundrect` pads.
    pub roundrect_rratio: Option<f64>,
    pub drill: Option<Drill>,
    pub layers: Vec<String>,
    pub net: Option<String>,
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum DrillShape {
    Round {
        diameter: f64,
    },
    /// Slot; `width` and `height` in the pad's frame.
    Oval {
        width: f64,
        height: f64,
    },
}

impl DrillShape {
    /// The smaller dimension: the drill tool diameter.
    pub fn tool_diameter(&self) -> f64 {
        match *self {
            DrillShape::Round { diameter } => diameter,
            DrillShape::Oval { width, height } => width.min(height),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Drill {
    pub shape: DrillShape,
    /// Hole offset from the pad centre, in the pad's frame.
    pub offset: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViaType {
    Through,
    /// KiCad `blind` (covers blind and buried) and `buried`.
    BlindBuried,
    Micro,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Via {
    pub position: Point,
    pub size: f64,
    pub drill: f64,
    pub layers: Vec<String>,
    pub kind: ViaType,
    pub net: Option<String>,
    pub uuid: Option<String>,
}
