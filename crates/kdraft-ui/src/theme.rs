//! Design tokens for a light and a dark theme, text styles and egui
//! visuals. The sheet is always white paper on a neutral pasteboard.
//!
//! The token structure and the way they are applied to egui `Visuals`
//! follow FigCraft's `crates/ui-egui/src/theme.rs` (MIT OR Apache-2.0,
//! see NOTICE); the colours and sizes are kdraft's own.

use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle, Visuals};

/// Theme preference stored in the settings file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePref {
    /// Follow the operating system (Windows: apps light/dark setting).
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemePref::System => "system",
            ThemePref::Light => "light",
            ThemePref::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<ThemePref> {
        match s {
            "system" => Some(ThemePref::System),
            "light" => Some(ThemePref::Light),
            "dark" => Some(ThemePref::Dark),
            _ => None,
        }
    }

    /// Dark or not, with the system's answer (`None`: unknown, light).
    pub fn is_dark(self, system_dark: Option<bool>) -> bool {
        match self {
            ThemePref::System => system_dark.unwrap_or(false),
            ThemePref::Light => false,
            ThemePref::Dark => true,
        }
    }
}

/// Colours of one theme. Contrast notes are WCAG ratios against the
/// surface named.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    /// Top strip and status bar.
    pub chrome: Color32,
    /// Side panels.
    pub panel: Color32,
    /// Pasteboard behind the sheet.
    pub canvas: Color32,
    /// Paper (white in both themes).
    pub paper: Color32,
    pub paper_shadow: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    /// Body text (>= 12:1 on panel).
    pub text: Color32,
    /// Labels, secondary text (>= 4.5:1 on panel and field).
    pub text_secondary: Color32,
    /// Hints, disabled (>= 3:1 on panel).
    pub text_tertiary: Color32,
    pub icon: Color32,
    pub icon_secondary: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub field: Color32,
    pub field_border: Color32,
    /// Selection, active tool, focus (>= 3:1 on canvas and paper).
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// Accent-coloured text on panels (>= 4.5:1).
    pub accent_text: Color32,
    pub selected_row: Color32,
    /// Alignment reference outline.
    pub reference: Color32,
    pub danger: Color32,
    pub warning: Color32,
    pub success: Color32,
    /// Alt-hover measurements and dimension previews.
    pub measure: Color32,
    /// Snap guides: to other items, to page/area, equal spacing.
    pub guide_object: Color32,
    pub guide_page: Color32,
    pub guide_spacing: Color32,
    /// Anchor marker while picking.
    pub anchor: Color32,
    pub ruler: Color32,
    pub ruler_text: Color32,
    pub grid: Color32,
    pub shadow: Color32,
    /// Floating tool bar and toasts.
    pub float: Color32,
}

/// The light and dark accent: a calm blue that works on white paper and
/// on both pasteboards.
const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x1F, 0x6F, 0xEB);
const ACCENT_DARK: Color32 = Color32::from_rgb(0x3D, 0x8B, 0xFF);

impl Tokens {
    pub fn light() -> Tokens {
        Tokens {
            dark: false,
            chrome: Color32::from_rgb(0xEC, 0xED, 0xF0),
            panel: Color32::from_rgb(0xF8, 0xF8, 0xF9),
            canvas: Color32::from_rgb(0xD9, 0xDC, 0xE1),
            paper: Color32::WHITE,
            paper_shadow: Color32::from_black_alpha(40),
            border: Color32::from_rgb(0xDD, 0xDF, 0xE3),
            border_strong: Color32::from_rgb(0xC4, 0xC8, 0xCE),
            text: Color32::from_rgb(0x1C, 0x1E, 0x21),
            text_secondary: Color32::from_rgb(0x5A, 0x60, 0x69),
            text_tertiary: Color32::from_rgb(0x84, 0x8A, 0x93),
            icon: Color32::from_rgb(0x2A, 0x2D, 0x32),
            icon_secondary: Color32::from_rgb(0x6C, 0x72, 0x7B),
            hover: Color32::from_rgb(0xEB, 0xEC, 0xEF),
            pressed: Color32::from_rgb(0xDE, 0xE0, 0xE4),
            field: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            field_border: Color32::from_rgb(0xD5, 0xD8, 0xDD),
            accent: ACCENT_LIGHT,
            accent_hover: Color32::from_rgb(0x18, 0x5C, 0xC8),
            on_accent: Color32::WHITE,
            accent_text: Color32::from_rgb(0x15, 0x5B, 0xC9),
            selected_row: Color32::from_rgb(0xDD, 0xE9, 0xFC),
            reference: Color32::from_rgb(0xE0, 0x6C, 0x00),
            danger: Color32::from_rgb(0xD1, 0x24, 0x2F),
            warning: Color32::from_rgb(0xB5, 0x6A, 0x00),
            success: Color32::from_rgb(0x1A, 0x7F, 0x37),
            measure: Color32::from_rgb(0xE5, 0x1F, 0x6B),
            guide_object: Color32::from_rgb(0xE8, 0x6A, 0x10),
            guide_page: Color32::from_rgb(0x1E, 0x9E, 0x52),
            guide_spacing: Color32::from_rgb(0xB0, 0x3A, 0xD8),
            anchor: Color32::from_rgb(0xE5, 0x1F, 0x6B),
            ruler: Color32::from_rgb(0xF1, 0xF2, 0xF4),
            ruler_text: Color32::from_rgb(0x6C, 0x72, 0x7B),
            grid: Color32::from_rgba_unmultiplied(0x1F, 0x6F, 0xEB, 40),
            shadow: Color32::from_black_alpha(36),
            float: Color32::WHITE,
        }
    }

    pub fn dark() -> Tokens {
        Tokens {
            dark: true,
            chrome: Color32::from_rgb(0x1E, 0x20, 0x23),
            panel: Color32::from_rgb(0x26, 0x28, 0x2C),
            canvas: Color32::from_rgb(0x15, 0x16, 0x19),
            paper: Color32::WHITE,
            paper_shadow: Color32::from_black_alpha(120),
            border: Color32::from_rgb(0x36, 0x39, 0x3E),
            border_strong: Color32::from_rgb(0x4A, 0x4E, 0x55),
            text: Color32::from_rgb(0xEC, 0xED, 0xEF),
            text_secondary: Color32::from_rgb(0xA9, 0xAE, 0xB6),
            text_tertiary: Color32::from_rgb(0x7D, 0x83, 0x8C),
            icon: Color32::from_rgb(0xE3, 0xE5, 0xE8),
            icon_secondary: Color32::from_rgb(0xA0, 0xA5, 0xAD),
            hover: Color32::from_rgb(0x32, 0x35, 0x3A),
            pressed: Color32::from_rgb(0x3D, 0x41, 0x47),
            field: Color32::from_rgb(0x1D, 0x1F, 0x22),
            field_border: Color32::from_rgb(0x3E, 0x42, 0x48),
            accent: ACCENT_DARK,
            accent_hover: Color32::from_rgb(0x63, 0xA2, 0xFF),
            on_accent: Color32::WHITE,
            accent_text: Color32::from_rgb(0x6F, 0xA8, 0xFF),
            selected_row: Color32::from_rgb(0x1E, 0x3A, 0x63),
            reference: Color32::from_rgb(0xFF, 0x8A, 0x1F),
            danger: Color32::from_rgb(0xFF, 0x5C, 0x5C),
            warning: Color32::from_rgb(0xE8, 0xA3, 0x2E),
            success: Color32::from_rgb(0x4C, 0xC3, 0x6B),
            measure: Color32::from_rgb(0xFF, 0x3D, 0x84),
            guide_object: Color32::from_rgb(0xFF, 0x7A, 0x1F),
            guide_page: Color32::from_rgb(0x2D, 0xC2, 0x6A),
            guide_spacing: Color32::from_rgb(0xC7, 0x5B, 0xF0),
            anchor: Color32::from_rgb(0xFF, 0x3D, 0x84),
            ruler: Color32::from_rgb(0x1E, 0x20, 0x23),
            ruler_text: Color32::from_rgb(0x8E, 0x94, 0x9C),
            grid: Color32::from_rgba_unmultiplied(0x3D, 0x8B, 0xFF, 44),
            shadow: Color32::from_black_alpha(110),
            float: Color32::from_rgb(0x2E, 0x31, 0x36),
        }
    }

    pub fn for_dark(dark: bool) -> Tokens {
        if dark {
            Tokens::dark()
        } else {
            Tokens::light()
        }
    }

    /// The tokens installed by [`apply`] (light before that).
    pub fn get(ctx: &egui::Context) -> Tokens {
        ctx.data(|d| d.get_temp::<Tokens>(tokens_id()))
            .unwrap_or_else(Tokens::light)
    }
}

fn tokens_id() -> egui::Id {
    egui::Id::unique("kdraft-theme-tokens")
}

/// Font families besides egui's two.
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Named text styles of the UI.
pub mod text {
    use super::*;

    pub const BODY: f32 = 12.5;
    pub const SMALL: f32 = 11.0;
    pub const HEADING: f32 = 12.5;
    pub const TITLE: f32 = 13.5;

    pub fn body() -> FontId {
        FontId::new(BODY, FontFamily::Proportional)
    }
    pub fn small() -> FontId {
        FontId::new(SMALL, FontFamily::Proportional)
    }
    /// Section headers.
    pub fn heading() -> FontId {
        FontId::new(HEADING, bold())
    }
    /// Document name in the top strip.
    pub fn title() -> FontId {
        FontId::new(TITLE, bold())
    }
    /// Numbers in fields, rulers and pills.
    pub fn mono() -> FontId {
        FontId::new(SMALL, FontFamily::Monospace)
    }
}

/// Installs tokens, visuals, spacing and text styles.
pub fn apply(ctx: &egui::Context, t: Tokens) {
    ctx.data_mut(|d| d.insert_temp(tokens_id(), t));
    ctx.set_visuals(visuals(t));
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(7.0, 3.0);
        s.spacing.interact_size = egui::vec2(22.0, 22.0);
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.window_margin = egui::Margin::same(12);
        s.spacing.indent = 14.0;
        s.text_styles = [
            (TextStyle::Small, text::small()),
            (TextStyle::Body, text::body()),
            (TextStyle::Button, text::body()),
            (TextStyle::Heading, FontId::new(15.0, bold())),
            (
                TextStyle::Monospace,
                FontId::new(12.0, FontFamily::Monospace),
            ),
        ]
        .into();
        s.interaction.tooltip_delay = 0.35;
        s.animation_time = 0.12;
    });
}

fn visuals(t: Tokens) -> Visuals {
    let mut v = if t.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = t.panel;
    v.window_fill = t.panel;
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.hover;
    v.code_bg_color = t.field;
    v.override_text_color = Some(t.text);
    v.weak_text_color = Some(t.text_secondary);
    v.window_stroke = Stroke::new(1.0, t.border_strong);
    v.window_corner_radius = egui::CornerRadius::same(8);
    v.menu_corner_radius = egui::CornerRadius::same(6);
    v.selection.bg_fill = t.accent.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, t.accent_text);
    v.hyperlink_color = t.accent_text;
    v.error_fg_color = t.danger;
    v.warn_fg_color = t.warning;
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
        &mut v.widgets.noninteractive,
    ] {
        w.corner_radius = egui::CornerRadius::same(4);
        w.fg_stroke = Stroke::new(1.0, t.text);
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
    v.widgets.noninteractive.bg_fill = t.panel;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text_secondary);
    v.widgets.inactive.bg_fill = t.field;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    // A visible edge for check boxes and combo boxes on light panels.
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.field_border);
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.weak_bg_fill = t.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.field_border);
    v.widgets.active.bg_fill = t.pressed;
    v.widgets.active.weak_bg_fill = t.pressed;
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
    v.widgets.open.bg_fill = t.pressed;
    v.widgets.open.weak_bg_fill = t.pressed;
    v.popup_shadow = egui::Shadow {
        offset: [0, 3],
        blur: 12,
        spread: 0,
        color: t.shadow,
    };
    v.window_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: t.shadow,
    };
    v.text_cursor.stroke = Stroke::new(1.5, t.accent);
    v.striped = false;
    v.indent_has_left_vline = false;
    v
}

/// WCAG contrast ratio of two opaque colours.
pub fn contrast(a: Color32, b: Color32) -> f64 {
    fn lum(c: Color32) -> f64 {
        let ch = |x: u8| {
            let v = f64::from(x) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_targets() {
        for t in [Tokens::light(), Tokens::dark()] {
            assert!(contrast(t.text, t.panel) >= 12.0, "{}", t.dark);
            assert!(contrast(t.text_secondary, t.panel) >= 4.5, "{}", t.dark);
            assert!(contrast(t.text_secondary, t.field) >= 4.5, "{}", t.dark);
            assert!(contrast(t.text_tertiary, t.panel) >= 3.0, "{}", t.dark);
            assert!(contrast(t.accent_text, t.panel) >= 4.5, "{}", t.dark);
            assert!(contrast(t.text, t.selected_row) >= 7.0, "{}", t.dark);
            assert!(contrast(t.accent, t.paper) >= 3.0, "{}", t.dark);
            assert!(contrast(t.measure, t.paper) >= 3.0, "{}", t.dark);
            assert!(contrast(t.on_accent, t.accent) >= 3.0, "{}", t.dark);
            assert!(contrast(t.icon, t.panel) >= 7.0, "{}", t.dark);
            assert!(contrast(t.danger, t.panel) >= 3.0, "{}", t.dark);
            assert!(contrast(t.ruler_text, t.ruler) >= 3.0, "{}", t.dark);
        }
    }

    #[test]
    fn prefs() {
        assert!(ThemePref::System.is_dark(Some(true)));
        assert!(!ThemePref::System.is_dark(None));
        assert!(ThemePref::Dark.is_dark(Some(false)));
        assert_eq!(ThemePref::parse("light"), Some(ThemePref::Light));
        assert_eq!(
            ThemePref::parse(ThemePref::Dark.as_str()),
            Some(ThemePref::Dark)
        );
    }
}
