//! Appearance commands: per-item style overrides, copy/paste of styles
//! and the document's category defaults.

use std::collections::BTreeMap;

use kdraft_doc::style::{category_style, ItemStyle, StyleKind, STYLE_KEYS};
use kdraft_doc::Item;
use serde::Deserialize;
use serde_json::Value as Json;

use crate::commands::{ids_arg, item_mut, spec};
use crate::error::{EngineError, R};
use crate::registry::{opt, req, ArgSpec, ArgType as T, CommandKind, Outcome, Registry};
use crate::session::Session;

const CATEGORIES: &[&str] = &["graphic", "text", "dimension", "callout", "notes", "table"];

fn key_args() -> Vec<ArgSpec> {
    STYLE_KEYS
        .iter()
        .map(|k| {
            let (ty, d): (T, &'static str) = match *k {
                "stroke" => (
                    T::String,
                    "line/border colour, #RRGGBB or #RRGGBBAA; null: default",
                ),
                "width" => (T::Number, "line width, mm"),
                "pattern" => (
                    T::Enum {
                        values: &["solid", "dashed", "dotted", "dash-dot"],
                    },
                    "line pattern",
                ),
                "fill" => (T::String, "fill of closed shapes, colour or none"),
                "opacity" => (T::Number, "opacity 0..1"),
                "arrows" => (
                    T::Enum {
                        values: &["none", "start", "end", "both"],
                    },
                    "arrowheads on lines and polylines",
                ),
                "arrow_size" => (T::Number, "arrowhead length, mm"),
                "text_color" => (T::String, "text colour"),
                "text_height" => (T::Number, "text cap height, mm"),
                "bold" => (T::Bool, "bold text"),
                "italic" => (T::Bool, "italic text"),
                "align" => (
                    T::Enum {
                        values: &["left", "center", "right"],
                    },
                    "text alignment",
                ),
                "line_spacing" => (T::Number, "baseline distance in text heights"),
                "decimals" => (T::Integer, "dimension decimals 0..6"),
                "header_fill" => (T::String, "table header fill, colour or none"),
                "shape" => (
                    T::Enum {
                        values: &["circle", "square", "hexagon", "none"],
                    },
                    "callout balloon shape",
                ),
                _ => (T::String, "style attribute"),
            };
            opt(k, ty, d)
        })
        .collect()
}

pub(crate) fn register(r: &mut Registry) {
    let ids = || opt("ids", T::IdList, "item ids (default: the selection)");
    let mut a = vec![ids()];
    a.extend(key_args());
    r.add(
        spec(
            "item.set_style",
            "Set style",
            CommandKind::Edit,
            "Overrides style attributes of items; omitted attributes stay, null removes an override (back to the document style). Attributes that do not apply to an item's kind are ignored for it.",
            a,
        ),
        item_set_style,
    );
    r.add(
        spec(
            "item.reset_style",
            "Reset to document style",
            CommandKind::Edit,
            "Removes style overrides of items (all, or the given attributes).",
            vec![
                ids(),
                opt("keys", T::StringList, "attributes to reset (default: all)"),
            ],
        ),
        item_reset_style,
    );
    r.add(
        spec(
            "style.copy",
            "Copy style",
            CommandKind::Selection,
            "Remembers the effective style of an item for style.paste.",
            vec![opt("id", T::Id, "item id (default: first selected)")],
        ),
        style_copy,
    );
    r.add(
        spec(
            "style.paste",
            "Paste style",
            CommandKind::Edit,
            "Applies the copied style to items (the attributes that apply to each item's kind).",
            vec![ids()],
        ),
        style_paste,
    );
    let mut d = vec![req(
        "category",
        T::Enum { values: CATEGORIES },
        "item category whose defaults change",
    )];
    d.extend(key_args());
    r.add(
        spec(
            "document.set_styles",
            "Set document styles",
            CommandKind::Edit,
            "Sets the document's default style of an item category; omitted attributes stay, null restores the built-in default. Items without own overrides follow.",
            d,
        ),
        document_set_styles,
    );
    r.add(
        spec(
            "view.set_layer_style",
            "Set layer colours",
            CommandKind::Edit,
            "Sets the layer colours of views: preset black, layer or grey (the built-in palettes; default: follow the document), one layer's or role's (role.pads ...) colour and opacity (null: back to the palette), or reset (all back to the palette).",
            vec![
                ids(),
                opt("preset", T::Enum { values: &["black", "layer", "grey", "default"] }, "colour preset"),
                opt("layer", T::String, "layer name for color and opacity"),
                opt("color", T::String, "layer colour #RRGGBB[AA]; null resets"),
                opt("opacity", T::Number, "layer opacity 0..1; null resets"),
                opt("reset", T::Bool, "clear all per-layer overrides"),
            ],
        ),
        view_set_layer_style,
    );
    r.add(
        spec(
            "document.set_layer_preset",
            "Set default layer colours",
            CommandKind::Edit,
            "Sets the document palette to a built-in one (black, layer, grey; same as document.set_layer_palette).",
            vec![req("preset", T::Enum { values: &["black", "layer", "grey", "default"] }, "preset")],
        ),
        document_set_layer_preset,
    );
    r.add(
        spec(
            "document.set_layer_palette",
            "Set document layer palette",
            CommandKind::Edit,
            "Sets the document's default layer palette, stored with its colours (views without their own colours follow it, live; fabrication and drill views stay black unless set). `name`: a built-in palette (Black clears); or `palette`: {name, colors: {key: \"#RRGGBB[AA]\"}}.",
            vec![
                opt("name", T::String, "built-in palette name"),
                opt("palette", T::String, "palette object {name, colors}"),
            ],
        ),
        document_set_layer_palette,
    );
    r.add(
        spec(
            "view.apply_palette",
            "Apply palette to views",
            CommandKind::Edit,
            "Gives views the colours of a palette for their layers (stored as overrides where they differ from the document palette), or `name` \"document\" to follow the document palette again.",
            vec![
                ids(),
                opt("name", T::String, "built-in palette name, or \"document\""),
                opt("palette", T::String, "palette object {name, colors}"),
            ],
        ),
        view_apply_palette,
    );
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaletteArgs {
    ids: Option<Vec<String>>,
    name: Option<String>,
    palette: Option<Json>,
}

fn palette_arg(a: &PaletteArgs) -> R<Option<kdraft_doc::palette::Palette>> {
    if let Some(p) = &a.palette {
        return kdraft_source::settings::palette_from_json(p)
            .map(Some)
            .ok_or_else(|| EngineError::invalid("palette must be {name, colors: {key: colour}}"));
    }
    match a.name.as_deref() {
        Some(n) => kdraft_doc::palette::Palette::builtin(n)
            .map(Some)
            .ok_or_else(|| {
                EngineError::invalid(format!(
                    "unknown built-in palette {n:?} (one of {}); pass user palettes as `palette`",
                    kdraft_doc::palette::BUILTIN.join(", ")
                ))
            }),
        None => Err(EngineError::invalid("give name or palette")),
    }
}

fn document_set_layer_palette(s: &mut Session, a: PaletteArgs) -> R<Outcome> {
    if a.ids.is_some() {
        return Err(EngineError::invalid(
            "ids does not apply to the document palette",
        ));
    }
    let p = palette_arg(&a)?.filter(|p| !p.is_black());
    s.doc_mut().styles.layer_palette = p;
    Ok(Outcome::changed(Vec::<String>::new()))
}

fn view_apply_palette(s: &mut Session, a: PaletteArgs) -> R<Outcome> {
    let reset = a.name.as_deref() == Some("document");
    let pal = if reset { None } else { palette_arg(&a)? };
    let (pi, ids) = crate::commands::ids_arg(s, a.ids)?;
    let mut changed = Vec::new();
    for id in &ids {
        let Item::View(v) = item_mut(s, pi, id)? else {
            continue;
        };
        // The palette by value (every layer and role); the view's own
        // overrides go, so what the inspector shows is what is drawn.
        let mut lc = v.layer_colors();
        lc.palette = pal.clone();
        lc.layers.clear();
        v.set_layer_colors(&lc);
        changed.push(id.clone());
    }
    if changed.is_empty() {
        return Err(EngineError::invalid("no view among the items"));
    }
    Ok(Outcome::changed(&changed))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerStyleArgs {
    ids: Option<Vec<String>>,
    preset: Option<String>,
    layer: Option<String>,
    /// Clears all overrides (back to the palette).
    reset: Option<bool>,
    #[serde(default, deserialize_with = "some")]
    color: Option<Json>,
    #[serde(default, deserialize_with = "some")]
    opacity: Option<Json>,
}

/// A preset name as a built-in palette (`default`: none).
fn parse_preset(p: &str) -> R<Option<kdraft_doc::palette::Palette>> {
    if p == "default" {
        return Ok(None);
    }
    kdraft_doc::layers::LayerPreset::parse(p)
        .map(|p| Some(p.palette()))
        .ok_or_else(|| {
            EngineError::invalid(format!(
                "preset must be black, layer, grey or default, got {p:?}"
            ))
        })
}

fn view_set_layer_style(s: &mut Session, a: LayerStyleArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    let preset = a.preset.as_deref().map(parse_preset).transpose()?;
    let color: Option<Option<kdraft_doc::style::Rgba>> = match &a.color {
        None => None,
        Some(Json::Null) => Some(None),
        Some(Json::String(c)) => Some(Some(kdraft_doc::style::Rgba::parse(c).ok_or_else(
            || EngineError::invalid(format!("{c:?} is not a colour (#RRGGBB or #RRGGBBAA)")),
        )?)),
        Some(o) => {
            return Err(EngineError::invalid(format!(
                "color must be a string or null, got {o}"
            )))
        }
    };
    let opacity: Option<Option<f64>> = match &a.opacity {
        None => None,
        Some(Json::Null) => Some(None),
        Some(v) => match v.as_f64().filter(|o| (0.0..=1.0).contains(o)) {
            Some(o) => Some(Some(o)),
            None => return Err(EngineError::invalid("opacity must be a number from 0 to 1")),
        },
    };
    if (color.is_some() || opacity.is_some()) && a.layer.is_none() {
        return Err(EngineError::invalid("give the layer for color or opacity"));
    }
    let reset = a.reset.unwrap_or(false);
    if preset.is_none() && a.layer.is_none() && !reset {
        return Err(EngineError::invalid("nothing to change"));
    }
    let mut changed = Vec::new();
    for id in &ids {
        let Item::View(v) = item_mut(s, pi, id)? else {
            continue;
        };
        let mut lc = v.layer_colors();
        if let Some(p) = &preset {
            lc.palette = p.clone();
        }
        if reset {
            lc.layers.clear();
        }
        if let Some(l) = &a.layer {
            lc.set(l, color, opacity);
        }
        v.set_layer_colors(&lc);
        changed.push(id.clone());
    }
    if changed.is_empty() {
        return Err(EngineError::invalid("no view among the items"));
    }
    Ok(Outcome::changed(&changed))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocPresetArgs {
    preset: String,
}

fn document_set_layer_preset(s: &mut Session, a: DocPresetArgs) -> R<Outcome> {
    let p = parse_preset(&a.preset)?.filter(|p| !p.is_black());
    s.doc_mut().styles.layer_palette = p;
    Ok(Outcome::changed(Vec::<String>::new()))
}

fn json_text(v: &Json) -> Option<String> {
    match v {
        Json::Null => None,
        Json::String(s) => Some(s.clone()),
        Json::Bool(b) => Some(if *b { "yes" } else { "no" }.to_string()),
        Json::Number(n) => Some(n.to_string()),
        other => Some(other.to_string()),
    }
}

/// Applies `values` (attribute -> JSON value, null clears) to `s`,
/// restricted to `allowed` keys when given. Returns whether anything
/// applied.
fn apply(s: &mut ItemStyle, values: &BTreeMap<String, Json>, allowed: Option<&[&str]>) -> R<bool> {
    let mut any = false;
    for (k, v) in values {
        if !STYLE_KEYS.contains(&k.as_str()) {
            return Err(EngineError::invalid(format!(
                "unknown style attribute {k:?} (known: {})",
                STYLE_KEYS.join(", ")
            )));
        }
        if allowed.is_some_and(|a| !a.contains(&k.as_str())) {
            continue;
        }
        s.set(k, json_text(v).as_deref())
            .map_err(|e| EngineError::invalid(e.to_string()))?;
        any = true;
    }
    Ok(any)
}

pub(crate) fn some<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Json>, D::Error> {
    Json::deserialize(d).map(Some)
}

/// Argument structs with one field per style attribute; `None` when
/// omitted, `Some(null)` for an explicit null.
macro_rules! style_args {
    ($name:ident { $($f:ident : $t:ty),* }) => {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $name {
            $($f: $t,)*
            #[serde(default, deserialize_with = "some")] stroke: Option<Json>,
            #[serde(default, deserialize_with = "some")] width: Option<Json>,
            #[serde(default, deserialize_with = "some")] pattern: Option<Json>,
            #[serde(default, deserialize_with = "some")] fill: Option<Json>,
            #[serde(default, deserialize_with = "some")] opacity: Option<Json>,
            #[serde(default, deserialize_with = "some")] arrows: Option<Json>,
            #[serde(default, deserialize_with = "some")] arrow_size: Option<Json>,
            #[serde(default, deserialize_with = "some")] text_color: Option<Json>,
            #[serde(default, deserialize_with = "some")] text_height: Option<Json>,
            #[serde(default, deserialize_with = "some")] bold: Option<Json>,
            #[serde(default, deserialize_with = "some")] italic: Option<Json>,
            #[serde(default, deserialize_with = "some")] align: Option<Json>,
            #[serde(default, deserialize_with = "some")] line_spacing: Option<Json>,
            #[serde(default, deserialize_with = "some")] decimals: Option<Json>,
            #[serde(default, deserialize_with = "some")] header_fill: Option<Json>,
            #[serde(default, deserialize_with = "some")] shape: Option<Json>,
        }

        impl $name {
            fn values(&self) -> BTreeMap<String, Json> {
                [
                    ("stroke", &self.stroke),
                    ("width", &self.width),
                    ("pattern", &self.pattern),
                    ("fill", &self.fill),
                    ("opacity", &self.opacity),
                    ("arrows", &self.arrows),
                    ("arrow_size", &self.arrow_size),
                    ("text_color", &self.text_color),
                    ("text_height", &self.text_height),
                    ("bold", &self.bold),
                    ("italic", &self.italic),
                    ("align", &self.align),
                    ("line_spacing", &self.line_spacing),
                    ("decimals", &self.decimals),
                    ("header_fill", &self.header_fill),
                    ("shape", &self.shape),
                ]
                .into_iter()
                .filter_map(|(k, v)| v.clone().map(|v| (k.to_string(), v)))
                .collect()
            }
        }
    };
}

style_args!(SetStyleArgs { ids: Option<Vec<String>> });
style_args!(DocStylesArgs { category: String });

fn item_set_style(s: &mut Session, a: SetStyleArgs) -> R<Outcome> {
    let values = a.values();
    let (pi, ids) = ids_arg(s, a.ids)?;
    if values.is_empty() {
        return Err(EngineError::invalid("no style attribute given"));
    }
    // Validate every value before changing anything.
    apply(&mut ItemStyle::default(), &values, None)?;
    let mut changed = Vec::new();
    let mut skipped = 0;
    for id in &ids {
        let it = item_mut(s, pi, id)?;
        if StyleKind::of(it).is_none() {
            skipped += 1;
            continue;
        }
        let mut over = it.style_override();
        let keys = kdraft_doc::style::keys_for(it);
        if apply(&mut over, &values, Some(&keys))? {
            it.set_style_override(&over);
            // The style's text height replaces a text's own height.
            if values.contains_key("text_height") {
                if let Item::Graphic(g) = it {
                    if let kdraft_doc::GraphicShape::Text { height, .. } = &mut g.shape {
                        *height = None;
                    }
                }
            }
            changed.push(id.clone());
        } else {
            skipped += 1;
        }
    }
    if changed.is_empty() {
        return Err(EngineError::invalid(
            "none of the attributes applies to the items",
        ));
    }
    let mut o = Outcome::changed(&changed);
    if skipped > 0 {
        o = o.msg(format!("{skipped} item(s) left unchanged"));
    }
    Ok(o)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetArgs {
    ids: Option<Vec<String>>,
    keys: Option<Vec<String>>,
}

fn item_reset_style(s: &mut Session, a: ResetArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.ids)?;
    if let Some(k) = a
        .keys
        .iter()
        .flatten()
        .find(|k| !STYLE_KEYS.contains(&k.as_str()))
    {
        return Err(EngineError::invalid(format!(
            "unknown style attribute {k:?}"
        )));
    }
    let mut changed = Vec::new();
    for id in &ids {
        let it = item_mut(s, pi, id)?;
        let mut over = it.style_override();
        if over.is_empty() {
            continue;
        }
        match &a.keys {
            None => over = ItemStyle::default(),
            Some(keys) => {
                for k in keys {
                    let _ = over.set(k, None);
                }
            }
        }
        it.set_style_override(&over);
        changed.push(id.clone());
    }
    Ok(Outcome::changed(&changed))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyArgs {
    id: Option<String>,
}

fn style_copy(s: &mut Session, a: CopyArgs) -> R<Outcome> {
    let (pi, ids) = ids_arg(s, a.id.map(|i| vec![i]))?;
    let id = &ids[0];
    let it = s.doc().pages[pi]
        .item(id)
        .ok_or_else(|| EngineError::invalid(format!("no item {id}")))?;
    let kind =
        StyleKind::of(it).ok_or_else(|| EngineError::invalid("this item has no style to copy"))?;
    let eff = it.effective_style(&s.doc().styles).only(kind.keys());
    s.style_clip = Some(eff);
    Ok(Outcome::default().msg("style copied"))
}

/// The overrides `clip` gives an item of kind `k`: only attributes of
/// that kind, and only where they differ from the category default (so
/// later document style changes still reach the rest).
pub fn pasted_override(
    clip: &ItemStyle,
    k: StyleKind,
    it: &Item,
    st: &kdraft_doc::Styles,
) -> ItemStyle {
    let base = category_style(k, st);
    let mut over = it.style_override();
    for key in k.keys() {
        let Some(v) = clip.get(key) else { continue };
        if base.get(key).as_deref() == Some(v.as_str()) {
            let _ = over.set(key, None);
        } else {
            let _ = over.set(key, Some(&v));
        }
    }
    over
}

fn style_paste(s: &mut Session, a: crate::commands::IdsArgs) -> R<Outcome> {
    let clip = s
        .style_clip
        .clone()
        .ok_or_else(|| EngineError::invalid("no style copied (use style.copy first)"))?;
    let (pi, ids) = ids_arg(s, a.ids)?;
    let st = s.doc().styles.clone();
    let mut changed = Vec::new();
    for id in &ids {
        let it = item_mut(s, pi, id)?;
        let Some(k) = StyleKind::of(it) else { continue };
        let mut over = pasted_override(&clip, k, it, &st);
        let keys = kdraft_doc::style::keys_for(it);
        over = {
            let mut o = over.only(&keys);
            o.extra = over.extra.clone();
            o
        };
        if over != it.style_override() {
            it.set_style_override(&over);
            changed.push(id.clone());
        }
    }
    Ok(Outcome::changed(&changed))
}

fn document_set_styles(s: &mut Session, a: DocStylesArgs) -> R<Outcome> {
    let k = StyleKind::parse(&a.category).ok_or_else(|| {
        EngineError::invalid(format!(
            "category must be one of {CATEGORIES:?}, got {:?}",
            a.category
        ))
    })?;
    let values = a.values();
    if values.is_empty() {
        return Err(EngineError::invalid("no style attribute given"));
    }
    if let Some(k2) = values.keys().find(|key| !k.keys().contains(&key.as_str())) {
        return Err(EngineError::invalid(format!(
            "{k2} does not apply to {}",
            k.as_str()
        )));
    }
    let mut cur = s.doc().styles.appearance.get(k).clone();
    apply(&mut cur, &values, Some(k.keys()))?;
    *s.doc_mut().styles.appearance.get_mut(k) = cur;
    Ok(Outcome::changed(Vec::<String>::new()))
}
