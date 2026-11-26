//! The editor's own settings: the `ui` section of the shared settings
//! file (see `kdraft_source::settings`). Theme, recent files, grid,
//! rulers, snap, recent colours, the window and panel layout, the last
//! used tool variants, and per-document view state (last page, zoom and
//! pan per page; keyed by the document's path, pruned with the recent
//! files; never stored in the `.kdraft` file). Unknown keys are kept.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::theme::ThemePref;

/// Most recent files kept.
pub const MAX_RECENT: usize = 10;
/// Recent colours kept.
pub const MAX_RECENT_COLORS: usize = 10;

/// Default window size, points.
pub const DEFAULT_WINDOW: (f32, f32) = (1400.0, 900.0);
/// Smallest window size restored, points.
pub const MIN_WINDOW: (f32, f32) = (640.0, 400.0);

/// Window placement (outer position and inner size, points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowGeom {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
}

/// A page's camera: zoom and the sheet point at the canvas centre, with
/// the page size it was made for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageCam {
    pub zoom: f64,
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
}

/// View state of one document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DocView {
    /// Id of the page last shown.
    pub page: Option<String>,
    /// Page id to camera.
    pub cams: BTreeMap<String, PageCam>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub theme: ThemePref,
    /// Most recent first.
    pub recent: Vec<PathBuf>,
    pub show_grid: bool,
    pub show_rulers: bool,
    pub snap: bool,
    /// Recently used colours (`#RRGGBB[AA]`), most recent first.
    pub recent_colors: Vec<String>,
    pub window: Option<WindowGeom>,
    pub left_width: Option<f32>,
    pub inspector_width: Option<f32>,
    pub dock_height: Option<f32>,
    /// Open dock tab (`issues`, `log`), `None`: closed.
    pub dock: Option<String>,
    /// Tool group to the id of its last used tool (`note` -> `tool.callout_bom`).
    pub tools: BTreeMap<String, String>,
    /// Canonical document path to its view state.
    pub docs: BTreeMap<String, DocView>,
    /// Unknown keys, kept.
    pub extra: Map<String, Value>,
}

const KNOWN: [&str; 14] = [
    "theme",
    "recent",
    "show_grid",
    "show_rulers",
    "snap",
    "recent_colors",
    "window",
    "left_width",
    "inspector_width",
    "dock_height",
    "dock",
    "tools",
    "documents",
    "version",
];

fn f32_of(v: &Value, k: &str) -> Option<f32> {
    v.get(k)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite())
        .map(|x| x as f32)
}

impl Settings {
    pub fn defaults() -> Settings {
        Settings {
            theme: ThemePref::System,
            show_rulers: true,
            snap: true,
            ..Default::default()
        }
    }

    pub fn from_json(v: &Value) -> Settings {
        let mut s = Settings::defaults();
        if let Some(t) = v
            .get("theme")
            .and_then(Value::as_str)
            .and_then(ThemePref::parse)
        {
            s.theme = t;
        }
        if let Some(r) = v.get("recent").and_then(Value::as_array) {
            s.recent = r
                .iter()
                .filter_map(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
                .take(MAX_RECENT)
                .collect();
        }
        if let Some(r) = v.get("recent_colors").and_then(Value::as_array) {
            s.recent_colors = r
                .iter()
                .filter_map(Value::as_str)
                .filter(|c| kdraft_doc::style::Rgba::parse(c).is_some())
                .map(str::to_string)
                .take(MAX_RECENT_COLORS)
                .collect();
        }
        for (key, slot) in [
            ("show_grid", &mut s.show_grid),
            ("show_rulers", &mut s.show_rulers),
            ("snap", &mut s.snap),
        ] {
            if let Some(b) = v.get(key).and_then(Value::as_bool) {
                *slot = b;
            }
        }
        if let Some(w) = v.get("window") {
            if let (Some(x), Some(y), Some(ww), Some(h)) = (
                f32_of(w, "x"),
                f32_of(w, "y"),
                f32_of(w, "w"),
                f32_of(w, "h"),
            ) {
                s.window = Some(WindowGeom {
                    x,
                    y,
                    w: ww,
                    h,
                    maximized: w.get("maximized").and_then(Value::as_bool).unwrap_or(false),
                });
            }
        }
        s.left_width = f32_of(v, "left_width");
        s.inspector_width = f32_of(v, "inspector_width");
        s.dock_height = f32_of(v, "dock_height");
        s.dock = v
            .get("dock")
            .and_then(Value::as_str)
            .filter(|d| matches!(*d, "issues" | "log"))
            .map(str::to_string);
        if let Some(m) = v.get("tools").and_then(Value::as_object) {
            s.tools = m
                .iter()
                .filter_map(|(k, t)| Some((k.clone(), t.as_str()?.to_string())))
                .collect();
        }
        if let Some(m) = v.get("documents").and_then(Value::as_object) {
            for (path, d) in m {
                let mut dv = DocView {
                    page: d.get("page").and_then(Value::as_str).map(str::to_string),
                    cams: BTreeMap::new(),
                };
                if let Some(c) = d.get("cams").and_then(Value::as_object) {
                    for (pid, cam) in c {
                        let n =
                            |k: &str| cam.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
                        if let (Some(zoom), Some(cx), Some(cy), Some(w), Some(h)) =
                            (n("zoom"), n("cx"), n("cy"), n("w"), n("h"))
                        {
                            dv.cams.insert(pid.clone(), PageCam { zoom, cx, cy, w, h });
                        }
                    }
                }
                s.docs.insert(path.clone(), dv);
            }
        }
        if let Some(m) = v.as_object() {
            s.extra = m
                .iter()
                .filter(|(k, _)| !KNOWN.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
        }
        s
    }

    pub fn to_json(&self) -> Value {
        let mut m = self.extra.clone();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        put("theme", json!(self.theme.as_str()));
        put(
            "recent",
            json!(self
                .recent
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()),
        );
        put("show_grid", json!(self.show_grid));
        put("show_rulers", json!(self.show_rulers));
        put("snap", json!(self.snap));
        put("recent_colors", json!(self.recent_colors));
        if let Some(w) = self.window {
            put(
                "window",
                json!({ "x": w.x, "y": w.y, "w": w.w, "h": w.h, "maximized": w.maximized }),
            );
        }
        for (k, v) in [
            ("left_width", self.left_width),
            ("inspector_width", self.inspector_width),
            ("dock_height", self.dock_height),
        ] {
            if let Some(v) = v {
                put(k, json!(v));
            }
        }
        put("dock", self.dock.as_ref().map_or(Value::Null, |d| json!(d)));
        put("tools", json!(self.tools));
        let docs: Map<String, Value> = self
            .docs
            .iter()
            .map(|(p, d)| {
                let cams: Map<String, Value> = d
                    .cams
                    .iter()
                    .map(|(id, c)| {
                        (
                            id.clone(),
                            json!({ "zoom": c.zoom, "cx": c.cx, "cy": c.cy, "w": c.w, "h": c.h }),
                        )
                    })
                    .collect();
                (p.clone(), json!({ "page": d.page, "cams": cams }))
            })
            .collect();
        put("documents", Value::Object(docs));
        Value::Object(m)
    }

    /// The `ui` section of the shared settings file (older files kept
    /// these keys at the top level).
    pub fn load(path: &Path) -> Settings {
        Settings::from_file(&kdraft_source::settings::SettingsFile::load(path).settings)
    }

    pub fn from_file(f: &kdraft_source::settings::SettingsFile) -> Settings {
        match f.get("ui") {
            Some(v) if v.is_object() => Settings::from_json(v),
            _ => {
                let legacy: Map<String, Value> = LEGACY_KEYS
                    .iter()
                    .filter_map(|k| f.get(k).map(|v| (k.to_string(), v.clone())))
                    .collect();
                Settings::from_json(&Value::Object(legacy))
            }
        }
    }

    /// Pretty JSON text of the `ui` section.
    pub fn text(&self) -> String {
        serde_json::to_string_pretty(&self.to_json()).unwrap_or_default()
    }

    /// Puts colour `c` first in the recent colours.
    pub fn add_recent_color(&mut self, c: &str) {
        self.recent_colors.retain(|q| !q.eq_ignore_ascii_case(c));
        self.recent_colors.insert(0, c.to_string());
        self.recent_colors.truncate(MAX_RECENT_COLORS);
    }

    /// Puts `p` first in the recent list (no duplicates, capped) and drops
    /// view state of documents no longer in it.
    pub fn add_recent(&mut self, p: &Path) {
        let p = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
        self.recent.retain(|q| q != &p);
        self.recent.insert(0, p);
        self.recent.truncate(MAX_RECENT);
        self.prune_docs();
    }

    /// Keeps view state only for documents in the recent list.
    pub fn prune_docs(&mut self) {
        let keep: Vec<String> = self.recent.iter().map(|p| doc_key(p)).collect();
        self.docs.retain(|k, _| keep.contains(k));
    }
}

/// Key of a document in [`Settings::docs`]: its canonical path.
pub fn doc_key(p: &Path) -> String {
    let abs = std::fs::canonicalize(p)
        .or_else(|_| std::path::absolute(p))
        .unwrap_or_else(|_| p.to_path_buf());
    abs.to_string_lossy().into_owned()
}

/// Where and how large the window opens: the saved placement when it is
/// sane and (if the monitor size is known) on the monitor, else the
/// default size centred (`pos` `None`: let the system place it).
pub fn window_plan(saved: Option<WindowGeom>, monitor: Option<(f32, f32)>) -> WindowGeom {
    let fallback = |m: Option<(f32, f32)>| {
        let (w, h) = DEFAULT_WINDOW;
        let (w, h) = match m {
            Some((mw, mh)) => (w.min(mw * 0.9), h.min(mh * 0.9)),
            None => (w, h),
        };
        let (x, y) = m.map_or((f32::NAN, f32::NAN), |(mw, mh)| {
            ((mw - w) / 2.0, (mh - h) / 2.0)
        });
        WindowGeom {
            x,
            y,
            w: w.max(MIN_WINDOW.0),
            h: h.max(MIN_WINDOW.1),
            maximized: false,
        }
    };
    let Some(s) = saved else {
        return fallback(monitor);
    };
    let finite = [s.x, s.y, s.w, s.h].iter().all(|v| v.is_finite());
    if !finite || s.w < MIN_WINDOW.0 || s.h < MIN_WINDOW.1 {
        return WindowGeom {
            maximized: s.maximized,
            ..fallback(monitor)
        };
    }
    if let Some((mw, mh)) = monitor {
        // At least a title-bar sized part must be on the monitor.
        let visible = s.x + s.w > 80.0 && s.x < mw - 80.0 && s.y >= -10.0 && s.y < mh - 40.0;
        if !visible {
            return WindowGeom {
                maximized: s.maximized,
                ..fallback(monitor)
            };
        }
    }
    s
}

/// Keys older versions wrote at the top level (moved under `ui`).
const LEGACY_KEYS: [&str; 6] = [
    "theme",
    "recent",
    "show_grid",
    "show_rulers",
    "snap",
    "recent_colors",
];

/// The settings file: `KDRAFT_SETTINGS`, else
/// `<config dir>/kdraft/settings.json`.
pub fn default_path() -> Option<PathBuf> {
    kdraft_source::settings::resolve_path(None, &kdraft_source::SystemEnvironment)
}

/// Writes the editor's `ui` section (JSON text) and, when given, the
/// shared sections into the settings file, keeping everything else in it;
/// atomic. Used from a worker.
pub fn write(path: &Path, text: &str, shared: Option<&Map<String, Value>>) -> Result<(), String> {
    let ui: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut f = kdraft_source::settings::SettingsFile::load(path).settings;
    for k in LEGACY_KEYS {
        f.set(k, Value::Null);
    }
    f.set("ui", ui);
    if let Some(sh) = shared {
        for (k, v) in sh {
            f.set(k, v.clone());
        }
    }
    f.save(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tolerance() {
        let mut s = Settings::defaults();
        s.theme = ThemePref::Dark;
        s.show_grid = true;
        s.add_recent(Path::new("/a/b.kdraft"));
        s.add_recent(Path::new("/a/c.kicad_pcb"));
        s.add_recent(Path::new("/a/b.kdraft"));
        s.add_recent_color("#FF0000");
        s.add_recent_color("#00FF00");
        s.add_recent_color("#ff0000");
        assert_eq!(s.recent_colors, vec!["#ff0000", "#00FF00"]);
        assert_eq!(s.recent.len(), 2);
        assert!(s.recent[0].ends_with("b.kdraft"));
        s.window = Some(WindowGeom {
            x: 10.0,
            y: 20.0,
            w: 1000.0,
            h: 700.0,
            maximized: true,
        });
        s.left_width = Some(300.0);
        s.dock = Some("log".into());
        s.tools.insert("note".into(), "tool.callout_bom".into());
        s.docs.insert(
            doc_key(Path::new("/a/b.kdraft")),
            DocView {
                page: Some("p1".into()),
                cams: [(
                    "p1".to_string(),
                    PageCam {
                        zoom: 3.0,
                        cx: 100.0,
                        cy: 80.0,
                        w: 420.0,
                        h: 297.0,
                    },
                )]
                .into(),
            },
        );
        s.extra.insert("future".into(), json!(7));
        let back = Settings::from_json(&s.to_json());
        assert_eq!(back, s);
        let bad = Settings::from_json(&json!({"theme": 3, "recent": "x", "snap": "no"}));
        assert_eq!(bad, Settings::defaults());
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k").join("settings.json");
        assert_eq!(Settings::load(&p), Settings::defaults());
        // Shared sections written by the CLI survive the editor's writes.
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"kicad": {"kicad_cli": "/x"}, "theme": "light"}"#).unwrap();
        assert_eq!(
            Settings::load(&p).theme,
            ThemePref::Light,
            "legacy top level"
        );
        write(&p, &s.text(), None).unwrap();
        assert_eq!(Settings::load(&p), s);
        let f = kdraft_source::settings::SettingsFile::load(&p).settings;
        assert!(f.get("theme").is_none());
        assert_eq!(f.kicad().cli.text(), "/x");
        std::fs::write(&p, "{ not json").unwrap();
        assert_eq!(Settings::load(&p), Settings::defaults());
        for i in 0..20 {
            s.add_recent(Path::new(&format!("/x/{i}.kdraft")));
        }
        assert_eq!(s.recent.len(), MAX_RECENT);
        // Pruned with the recent list.
        assert!(s.docs.is_empty());
    }

    #[test]
    fn window_placement() {
        let g = |x, y, w, h| WindowGeom {
            x,
            y,
            w,
            h,
            maximized: false,
        };
        let mon = Some((1920.0, 1080.0));
        // On screen: kept.
        assert_eq!(
            window_plan(Some(g(100.0, 50.0, 1200.0, 800.0)), mon),
            g(100.0, 50.0, 1200.0, 800.0)
        );
        // Off screen (a monitor that is gone): default size, centred.
        let p = window_plan(Some(g(3000.0, 200.0, 1200.0, 800.0)), mon);
        assert_eq!((p.w, p.h), DEFAULT_WINDOW);
        assert_eq!(
            (p.x, p.y),
            ((1920.0 - 1400.0) / 2.0, (1080.0 - 900.0) / 2.0)
        );
        // Tiny or broken: default.
        let p = window_plan(Some(g(10.0, 10.0, 0.0, 5.0)), mon);
        assert_eq!((p.w, p.h), DEFAULT_WINDOW);
        let p = window_plan(Some(g(f32::NAN, 10.0, 900.0, 600.0)), None);
        assert_eq!((p.w, p.h), DEFAULT_WINDOW);
        // Nothing saved.
        assert_eq!(window_plan(None, None).w, DEFAULT_WINDOW.0);
    }
}
