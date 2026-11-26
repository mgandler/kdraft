//! User settings shared by the editor and the `kdraft` command-line tool:
//! one JSON file, `%APPDATA%\kdraft\settings.json` on Windows,
//! `$XDG_CONFIG_HOME/kdraft/settings.json` or
//! `~/.config/kdraft/settings.json` elsewhere (`--settings FILE` or
//! `KDRAFT_SETTINGS` override the location).
//!
//! Sections: `kicad` (kicad-cli choice, extra variables, cache), `documents`
//! (defaults for new documents and pages) and `ui` (the editor's own,
//! read by `kdraft-ui`). Reading is tolerant: a missing file gives the
//! defaults; an unreadable or malformed one is kept as `settings.json.bad`
//! and the defaults are used (with one warning). Unknown keys, at the top
//! and inside the known sections, are written back unchanged. Writes go to
//! a temporary file that is then renamed over the old one.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use kdraft_doc::palette::Palette;

use crate::kicad_cli::{Environment, Os};

/// Environment variable naming the settings file.
pub const SETTINGS_ENV: &str = "KDRAFT_SETTINGS";

/// `<config dir>/kdraft/settings.json`, if a config directory is known.
pub fn default_path(env: &dyn Environment) -> Option<PathBuf> {
    let var = |n: &str| env.var(n).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = match env.os() {
        Os::Windows => var("APPDATA")?,
        Os::MacOs => var("HOME")?.join("Library/Application Support"),
        Os::Linux => var("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| var("HOME").map(|h| h.join(".config")))?,
    };
    Some(base.join("kdraft").join("settings.json"))
}

/// The settings file to use: `flag`, else `KDRAFT_SETTINGS`, else the
/// default location.
pub fn resolve_path(flag: Option<&Path>, env: &dyn Environment) -> Option<PathBuf> {
    flag.map(Path::to_path_buf)
        .or_else(|| {
            env.var(SETTINGS_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| default_path(env))
}

/// Which kicad-cli to use.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CliChoice {
    /// The newest install found.
    #[default]
    Auto,
    Path(PathBuf),
}

impl CliChoice {
    pub fn parse(s: &str) -> CliChoice {
        let t = s.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("auto") {
            CliChoice::Auto
        } else {
            CliChoice::Path(PathBuf::from(t))
        }
    }

    pub fn text(&self) -> String {
        match self {
            CliChoice::Auto => "auto".into(),
            CliChoice::Path(p) => p.to_string_lossy().into_owned(),
        }
    }
}

/// The `kicad` section.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KicadSettings {
    pub cli: CliChoice,
    /// Extra `--define-var NAME=VALUE` for the commands that accept it.
    pub vars: Vec<(String, String)>,
    /// Cache directory for all documents; `None`: `.kdraft-cache` next to
    /// each document (the default).
    pub cache_dir: Option<PathBuf>,
}

/// Parses `NAME=VALUE`; the name must be non-empty and without `=`.
pub fn parse_var(s: &str) -> Option<(String, String)> {
    let (k, v) = s.split_once('=')?;
    let k = k.trim();
    (!k.is_empty() && !k.contains(char::is_whitespace)).then(|| (k.to_string(), v.to_string()))
}

impl KicadSettings {
    fn from_json(v: &Value) -> KicadSettings {
        let mut k = KicadSettings::default();
        if let Some(s) = v.get("kicad_cli").and_then(Value::as_str) {
            k.cli = CliChoice::parse(s);
        }
        if let Some(a) = v.get("define_vars").and_then(Value::as_array) {
            k.vars = a
                .iter()
                .filter_map(Value::as_str)
                .filter_map(parse_var)
                .collect();
        }
        k.cache_dir = v
            .get("cache_dir")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        k
    }

    fn to_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("kicad_cli".into(), json!(self.cli.text()));
        m.insert(
            "define_vars".into(),
            json!(self
                .vars
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()),
        );
        m.insert(
            "cache_dir".into(),
            self.cache_dir
                .as_ref()
                .map_or(Value::Null, |p| json!(p.to_string_lossy())),
        );
        m
    }
}

/// Paper sizes offered as a default.
pub const PAPERS: &[&str] = &["A4", "A3", "A2", "A1", "A0", "A", "B", "C", "D", "E"];

/// The `documents` section: defaults for new documents and pages.
#[derive(Debug, Clone, PartialEq)]
pub struct DocDefaults {
    pub paper: String,
    pub portrait: bool,
    /// Drawing sheet (`.kicad_wks` path); `None`: the project's sheet or
    /// KiCad's default.
    pub sheet: Option<PathBuf>,
    /// Name of the layer palette new documents take (built-in or one of
    /// the user's palettes).
    pub layer_palette: String,
    pub render_dpi: u32,
    /// `basic` or `high`.
    pub render_quality: String,
    pub callouts_per_document: bool,
    pub snap_grid: f64,
}

impl Default for DocDefaults {
    fn default() -> Self {
        DocDefaults {
            paper: "A3".into(),
            portrait: false,
            sheet: None,
            layer_palette: kdraft_doc::palette::BLACK.into(),
            render_dpi: 300,
            render_quality: "basic".into(),
            callouts_per_document: false,
            snap_grid: 0.5,
        }
    }
}

impl DocDefaults {
    fn from_json(v: &Value) -> DocDefaults {
        let mut d = DocDefaults::default();
        if let Some(p) = v
            .get("paper")
            .and_then(Value::as_str)
            .filter(|p| PAPERS.iter().any(|x| x.eq_ignore_ascii_case(p)))
        {
            d.paper = p.to_ascii_uppercase();
        }
        if let Some(b) = v.get("portrait").and_then(Value::as_bool) {
            d.portrait = b;
        }
        d.sheet = v
            .get("sheet")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        if let Some(p) = v
            .get("layer_palette")
            .and_then(Value::as_str)
            .filter(|p| !p.trim().is_empty())
        {
            d.layer_palette = p.to_string();
        }
        if let Some(n) = v
            .get("render_dpi")
            .and_then(Value::as_u64)
            .filter(|n| (10..=1200).contains(n))
        {
            d.render_dpi = n as u32;
        }
        if let Some(q) = v
            .get("render_quality")
            .and_then(Value::as_str)
            .filter(|q| matches!(*q, "basic" | "high"))
        {
            d.render_quality = q.to_string();
        }
        if let Some(s) = v.get("callout_numbering").and_then(Value::as_str) {
            d.callouts_per_document = s == "document";
        }
        if let Some(g) = v
            .get("snap_grid")
            .and_then(Value::as_f64)
            .filter(|g| g.is_finite() && *g >= 0.0 && *g <= 50.0)
        {
            d.snap_grid = g;
        }
        d
    }

    fn to_json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("paper".into(), json!(self.paper));
        m.insert("portrait".into(), json!(self.portrait));
        m.insert(
            "sheet".into(),
            self.sheet
                .as_ref()
                .map_or(Value::Null, |p| json!(p.to_string_lossy())),
        );
        m.insert("layer_palette".into(), json!(self.layer_palette));
        m.insert("render_dpi".into(), json!(self.render_dpi));
        m.insert("render_quality".into(), json!(self.render_quality));
        m.insert(
            "callout_numbering".into(),
            json!(if self.callouts_per_document {
                "document"
            } else {
                "page"
            }),
        );
        m.insert("snap_grid".into(), json!(self.snap_grid));
        m
    }
}

/// The settings file's content (all of it, unknown keys included).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SettingsFile {
    root: Map<String, Value>,
}

/// Result of [`SettingsFile::load`].
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub settings: SettingsFile,
    /// Set when the file could not be used (it was kept as `.bad`).
    pub warning: Option<String>,
}

impl SettingsFile {
    /// Reads `path`. Missing: defaults. Unreadable or malformed: defaults,
    /// the file renamed to `settings.json.bad`, and a warning.
    pub fn load(path: &Path) -> Loaded {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Loaded {
                    settings: SettingsFile::default(),
                    warning: None,
                }
            }
            Err(e) => return bad(path, &e.to_string()),
        };
        if text.trim().is_empty() {
            return Loaded {
                settings: SettingsFile::default(),
                warning: None,
            };
        }
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(root)) => Loaded {
                settings: SettingsFile { root },
                warning: None,
            },
            Ok(_) => bad(path, "not a JSON object"),
            Err(e) => bad(path, &e.to_string()),
        }
    }

    pub fn from_text(text: &str) -> Option<SettingsFile> {
        match serde_json::from_str::<Value>(text).ok()? {
            Value::Object(root) => Some(SettingsFile { root }),
            _ => None,
        }
    }

    /// A top-level value.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.root.get(key)
    }

    /// Sets a top-level value (`Null` removes it).
    pub fn set(&mut self, key: &str, v: Value) {
        if v.is_null() {
            self.root.remove(key);
        } else {
            self.root.insert(key.to_string(), v);
        }
    }

    /// Sets the keys of `fields` in object section `key`, keeping its other
    /// (unknown) keys; `Null` fields are removed.
    pub fn merge(&mut self, key: &str, fields: Map<String, Value>) {
        let sec = self
            .root
            .entry(key.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !sec.is_object() {
            *sec = Value::Object(Map::new());
        }
        if let Value::Object(m) = sec {
            for (k, v) in fields {
                if v.is_null() {
                    m.remove(&k);
                } else {
                    m.insert(k, v);
                }
            }
        }
    }

    pub fn kicad(&self) -> KicadSettings {
        self.get("kicad")
            .map(KicadSettings::from_json)
            .unwrap_or_default()
    }

    pub fn set_kicad(&mut self, k: &KicadSettings) {
        self.merge("kicad", k.to_json());
    }

    pub fn documents(&self) -> DocDefaults {
        self.get("documents")
            .map(DocDefaults::from_json)
            .unwrap_or_default()
    }

    pub fn set_documents(&mut self, d: &DocDefaults) {
        self.merge("documents", d.to_json());
    }

    /// The user's own palettes (`palettes`: name and colours).
    pub fn palettes(&self) -> Vec<Palette> {
        self.get("palettes")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(palette_from_json).collect())
            .unwrap_or_default()
    }

    pub fn set_palettes(&mut self, ps: &[Palette]) {
        self.set(
            "palettes",
            Value::Array(ps.iter().map(palette_to_json).collect()),
        );
    }

    /// Adds `p`, replacing a user palette of the same name; built-in names
    /// are refused.
    pub fn upsert_palette(&mut self, p: Palette) -> Result<(), String> {
        if kdraft_doc::palette::BUILTIN.contains(&p.name.as_str()) {
            return Err(format!(
                "{:?} is a built-in palette; choose another name",
                p.name
            ));
        }
        let mut all = self.palettes();
        all.retain(|q| q.name != p.name);
        all.push(p);
        self.set_palettes(&all);
        Ok(())
    }

    /// Built-in or user palette by name.
    pub fn palette(&self, name: &str) -> Option<Palette> {
        Palette::builtin(name).or_else(|| self.palettes().into_iter().find(|p| p.name == name))
    }

    /// Names of all palettes: built-ins, then the user's.
    pub fn palette_names(&self) -> Vec<String> {
        kdraft_doc::palette::BUILTIN
            .iter()
            .map(|s| s.to_string())
            .chain(self.palettes().into_iter().map(|p| p.name))
            .collect()
    }

    /// The palette new documents take (`None` for Black: nothing stored).
    pub fn default_palette(&self) -> Option<Palette> {
        let n = self.documents().layer_palette;
        if n == kdraft_doc::palette::BLACK {
            return None;
        }
        self.palette(&n)
    }

    /// Pretty JSON.
    pub fn text(&self) -> String {
        serde_json::to_string_pretty(&Value::Object(self.root.clone())).unwrap_or_default() + "\n"
    }

    /// Writes the file atomically (directory created as needed).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        write_atomic(path, &self.text())
    }

    /// `key=value` lines of the known settings, for `kdraft settings show`.
    pub fn summary(&self) -> Vec<(String, String)> {
        let k = self.kicad();
        let d = self.documents();
        let mut out = vec![
            ("kicad_cli".to_string(), k.cli.text()),
            (
                "define_vars".to_string(),
                k.vars
                    .iter()
                    .map(|(a, b)| format!("{a}={b}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            (
                "cache_dir".to_string(),
                k.cache_dir.map_or_else(
                    || "next to each document (.kdraft-cache)".to_string(),
                    |p| p.to_string_lossy().into_owned(),
                ),
            ),
        ];
        let doc = d.to_json();
        for (key, v) in doc {
            let t = match v {
                Value::String(s) => s,
                Value::Null => "KiCad default".to_string(),
                o => o.to_string(),
            };
            out.push((format!("documents.{key}"), t));
        }
        out
    }

    /// Sets a known setting from its text (`kicad_cli`, `define_vars`,
    /// `cache_dir`, `documents.<key>`).
    pub fn set_text(&mut self, key: &str, value: &str) -> Result<(), String> {
        let v = value.trim();
        match key {
            "kicad_cli" => {
                let mut k = self.kicad();
                k.cli = CliChoice::parse(v);
                self.set_kicad(&k);
            }
            "define_vars" => {
                let mut k = self.kicad();
                k.vars = Vec::new();
                for part in v.split_whitespace() {
                    k.vars.push(
                        parse_var(part).ok_or_else(|| format!("{part:?} is not NAME=VALUE"))?,
                    );
                }
                self.set_kicad(&k);
            }
            "cache_dir" => {
                let mut k = self.kicad();
                k.cache_dir = (!v.is_empty() && v != "default").then(|| PathBuf::from(v));
                self.set_kicad(&k);
            }
            _ => {
                let Some(field) = key.strip_prefix("documents.") else {
                    return Err(format!("unknown setting {key}"));
                };
                let before = self.documents();
                let mut m = before.to_json();
                if !m.contains_key(field) {
                    return Err(format!("unknown setting {key}"));
                }
                let parsed = match field {
                    "portrait" => json!(matches!(v, "yes" | "true" | "1" | "on")),
                    "render_dpi" => json!(v.parse::<u64>().map_err(|e| e.to_string())?),
                    "snap_grid" => json!(v.parse::<f64>().map_err(|e| e.to_string())?),
                    "sheet" if v.is_empty() || v == "default" => Value::Null,
                    _ => json!(v),
                };
                m.insert(field.to_string(), parsed.clone());
                let after = DocDefaults::from_json(&Value::Object(m));
                // Values that do not parse fall back silently in from_json;
                // say so here instead.
                if after.to_json().get(field) != Some(&parsed) && !parsed.is_null() {
                    return Err(format!("{value:?} is not a valid value for {key}"));
                }
                self.set_documents(&after);
            }
        }
        Ok(())
    }
}

/// `{"name": ..., "colors": {key: "#RRGGBB[AA]"}}` (also the export
/// format of a single palette).
pub fn palette_to_json(p: &Palette) -> Value {
    let colors: Map<String, Value> = p
        .colors
        .iter()
        .map(|(k, c)| (k.clone(), json!(c.to_string())))
        .collect();
    json!({ "name": p.name, "colors": colors })
}

pub fn palette_from_json(v: &Value) -> Option<Palette> {
    let name = v.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let mut p = Palette::new(name);
    for (k, c) in v.get("colors")?.as_object()? {
        if let Some(col) = c.as_str().and_then(kdraft_doc::style::Rgba::parse) {
            p.colors.insert(k.clone(), col);
        }
    }
    Some(p)
}

fn bad(path: &Path, why: &str) -> Loaded {
    let mut keep = path.as_os_str().to_os_string();
    keep.push(".bad");
    let kept = PathBuf::from(keep);
    let moved = std::fs::rename(path, &kept).is_ok();
    Loaded {
        settings: SettingsFile::default(),
        warning: Some(if moved {
            format!(
                "settings file {} could not be read ({why}); using defaults, the old file is kept as {}",
                path.display(),
                kept.display()
            )
        } else {
            format!(
                "settings file {} could not be read ({why}); using defaults",
                path.display()
            )
        }),
    }
}

/// Writes `text` to `path` through a temporary file in the same folder
/// and a rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// Total size of the files under `dir`, bytes (0 if missing).
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map_or(0, |m| m.len()),
                _ => {}
            }
        }
    }
    total
}

/// `12.3 MB` style size.
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "kB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u + 1 < UNITS.len() {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_unknown_keys() {
        let text = r#"{
            "future": {"x": 1},
            "kicad": {"kicad_cli": "/opt/k/kicad-cli", "define_vars": ["LIB=/l", "bad"], "later": true},
            "documents": {"paper": "a4", "portrait": true, "render_dpi": 5000, "extra": 2},
            "ui": {"theme": "dark"}
        }"#;
        let mut s = SettingsFile::from_text(text).unwrap();
        let k = s.kicad();
        assert_eq!(k.cli, CliChoice::Path("/opt/k/kicad-cli".into()));
        assert_eq!(k.vars, vec![("LIB".to_string(), "/l".to_string())]);
        let d = s.documents();
        assert_eq!(d.paper, "A4");
        assert!(d.portrait);
        assert_eq!(d.render_dpi, 300, "out of range falls back");
        s.set_text("kicad_cli", "auto").unwrap();
        s.set_text("documents.layer_palette", "Greyscale").unwrap();
        assert!(s.set_text("documents.paper", "Z9").is_err());
        assert!(s.set_text("nope", "1").is_err());
        let back = SettingsFile::from_text(&s.text()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.get("future"), Some(&json!({"x": 1})));
        assert_eq!(back.get("kicad").unwrap()["later"], json!(true));
        assert_eq!(back.get("documents").unwrap()["extra"], json!(2));
        assert_eq!(back.get("ui").unwrap()["theme"], json!("dark"));
        assert_eq!(back.kicad().cli, CliChoice::Auto);
        assert_eq!(back.documents().layer_palette, "Greyscale");
        // Palettes: user ones round trip; built-in names are refused.
        let mut p = Palette::new("Mine");
        p.colors
            .insert("F.SilkS".into(), kdraft_doc::style::Rgba::rgb(1, 2, 3));
        s.upsert_palette(p.clone()).unwrap();
        assert!(s.upsert_palette(Palette::new("Black")).is_err());
        let back = SettingsFile::from_text(&s.text()).unwrap();
        assert_eq!(back.palette("Mine"), Some(p));
        assert!(back.palette("KiCad Default").is_some());
        assert_eq!(back.palette_names().len(), 8);
    }

    #[test]
    fn missing_and_malformed_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k").join("settings.json");
        let l = SettingsFile::load(&p);
        assert_eq!(l.settings, SettingsFile::default());
        assert!(l.warning.is_none());
        let mut s = SettingsFile::default();
        s.set_text("documents.paper", "A2").unwrap();
        s.save(&p).unwrap();
        assert_eq!(SettingsFile::load(&p).settings, s);
        // No temporary file left behind.
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        std::fs::write(&p, "{ not json").unwrap();
        let l = SettingsFile::load(&p);
        assert_eq!(l.settings, SettingsFile::default());
        assert!(l.warning.unwrap().contains("settings.json.bad"));
        assert!(!p.exists());
        assert_eq!(
            std::fs::read_to_string(p.with_extension("json.bad")).unwrap(),
            "{ not json"
        );
        std::fs::write(&p, "[1, 2]").unwrap();
        assert!(SettingsFile::load(&p).warning.is_some());
    }

    #[test]
    fn sizes() {
        assert_eq!(size_text(999), "999 bytes");
        assert_eq!(size_text(12_345_678), "12.3 MB");
        assert_eq!(parse_var("A=b=c"), Some(("A".into(), "b=c".into())));
        assert_eq!(parse_var("=x"), None);
    }
}
