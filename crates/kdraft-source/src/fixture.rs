//! A stand-in for kicad-cli built from recorded outputs, for tests and
//! headless screenshots on machines without KiCad: `version` answers
//! 10.0.8, `pcb export svg` copies a recorded layer plot
//! (`<dir>/<Layer_Name>-mode0[-mirror].svg`, an empty plot of the same
//! page for layers without a recording), `pcb render` writes the PNG the
//! given function makes for `--width`, `--height` and `--zoom`. Everything
//! else fails like a missing feature.

use std::path::{Path, PathBuf};

use crate::runner::{arg_value, CommandOutput, FakeRunner};

/// The recorded version.
pub const VERSION: &str = "10.0.8";

/// An empty plot of an A4 landscape page (the recordings' page).
fn empty_plot() -> String {
    "<?xml version=\"1.0\" standalone=\"no\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" \
     version=\"1.1\" width=\"297.0022mm\" height=\"210.0072mm\" \
     viewBox=\"0.0000 0.0000 297.0022 210.0072\">\n</svg>\n"
        .to_string()
}

fn plot_for(dir: &Path, layers: &str, mirror: bool) -> String {
    let first = layers.split(',').next().unwrap_or("");
    let stem = first.replace('.', "_");
    let name = if mirror {
        format!("{stem}-mode0-mirror.svg")
    } else {
        format!("{stem}-mode0.svg")
    };
    std::fs::read_to_string(dir.join(&name))
        .or_else(|_| {
            if mirror {
                std::fs::read_to_string(dir.join(format!("{stem}-mode0.svg")))
            } else {
                Err(std::io::Error::other("no recording"))
            }
        })
        .unwrap_or_else(|_| empty_plot())
}

/// A picture maker for `pcb render`: width, height, zoom to PNG bytes.
pub type RenderFn = dyn Fn(u32, u32, f64) -> Vec<u8> + Send + Sync;

/// A runner answering from recordings in `svg_dir`; `render` draws the
/// pictures for `pcb render`.
pub fn fixture_runner(svg_dir: PathBuf, render: Box<RenderFn>) -> FakeRunner {
    FakeRunner::new(move |_, args| {
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        match a.as_slice() {
            ["version", ..] | ["--version", ..] => CommandOutput::ok(format!("{VERSION}\n")),
            ["pcb", "export", "svg", ..] => {
                let out = arg_value(args, "-o").unwrap_or_default();
                let layers = arg_value(args, "--layers").unwrap_or_default();
                let mirror = args.iter().any(|x| x == "--mirror" || x == "-m");
                match std::fs::write(out, plot_for(&svg_dir, layers, mirror)) {
                    Ok(()) => CommandOutput::ok(""),
                    Err(e) => CommandOutput::failed(1, e.to_string()),
                }
            }
            ["pcb", "render", ..] => {
                let out = arg_value(args, "-o").unwrap_or_default();
                let num = |f: &str, d: f64| {
                    arg_value(args, f)
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(d)
                };
                let png = render(
                    num("--width", 1600.0) as u32,
                    num("--height", 900.0) as u32,
                    num("--zoom", 1.0),
                );
                match std::fs::write(out, png) {
                    Ok(()) => CommandOutput::ok(""),
                    Err(e) => CommandOutput::failed(1, e.to_string()),
                }
            }
            _ => CommandOutput::failed(1, "not recorded in the fixture runner"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandRunner;
    use std::ffi::OsString;

    #[test]
    fn answers_version_svg_and_render() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/kicad-cli-svg");
        let r = fixture_runner(
            dir,
            Box::new(|w, h, z| vec![w as u8, h as u8, (z * 10.0) as u8]),
        );
        let t = tempfile::tempdir().unwrap();
        let v = r.run(Path::new("k"), &[OsString::from("version")]).unwrap();
        assert_eq!(v.stdout_text().trim(), VERSION);
        let out = t.path().join("a.svg");
        let args: Vec<OsString> = [
            "pcb",
            "export",
            "svg",
            "-o",
            out.to_str().unwrap(),
            "--layers",
            "F.Cu",
            "b.kicad_pcb",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        assert!(r.run(Path::new("k"), &args).unwrap().success());
        assert!(std::fs::read_to_string(&out).unwrap().contains("PCBNEW"));
        let args: Vec<OsString> = [
            "pcb",
            "export",
            "svg",
            "-o",
            out.to_str().unwrap(),
            "--layers",
            "In7.Cu",
            "b",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        assert!(r.run(Path::new("k"), &args).unwrap().success());
        assert!(!std::fs::read_to_string(&out).unwrap().contains("PCBNEW"));
        let png = t.path().join("r.png");
        let args: Vec<OsString> = [
            "pcb",
            "render",
            "-o",
            png.to_str().unwrap(),
            "--width",
            "7",
            "--height",
            "5",
            "--zoom",
            "1.5",
            "b",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        assert!(r.run(Path::new("k"), &args).unwrap().success());
        assert_eq!(std::fs::read(&png).unwrap(), vec![7, 5, 15]);
    }
}
