//! Background jobs of the editor: rasterising thumbnails, sheets and items,
//! file IO, kicad-cli discovery and plots. Everything here runs off the UI
//! thread; results carry the key they were made for so stale ones can be
//! dropped.

use std::path::PathBuf;
use std::sync::Arc;

use kdraft_board::geom::{BBox, Point};
use kdraft_board::Board;
use kdraft_doc::{Document, Id, Item, Page};
use kdraft_engine::pipeline::{self, Plots, RenderInputs};
use kdraft_engine::Session;
use kdraft_render::dl::Color;
use kdraft_render::raster::{self, Target};
use kdraft_source::{Cache, CommandRunner, FileBoardSource, KicadCli, SystemRunner};

use crate::camera;

/// Content hash used as cache key.
pub type Key = [u8; 32];

fn hash_debug(parts: &[&dyn std::fmt::Debug]) -> Key {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(format!("{p:?}").as_bytes());
        h.update(&[0]);
    }
    *h.finalize().as_bytes()
}

/// Key of an item's appearance, independent of where it sits on the
/// sheet: the item moved to its reference point at the origin, plus what
/// else its rendering depends on.
pub fn item_key(item: &Item, doc: &Document, inputs_gen: &Key) -> Key {
    let mut it = item.clone();
    if let Some(p) = it.position() {
        it.translate(-p.x, -p.y);
    }
    // Dimensions and callouts are drawn through their view, and callouts
    // show note or BOM numbers: those are part of the key.
    let view = match item {
        Item::Dimension(d) => d.view.clone(),
        Item::Callout(c) => c.view.clone(),
        _ => None,
    };
    let deps: Vec<&Item> = match item {
        Item::Dimension(_) | Item::Callout(_) => doc
            .pages
            .iter()
            .flat_map(|p| p.items.iter())
            .filter(|i| {
                (view.is_some() && i.id() == view.as_ref())
                    || (matches!(item, Item::Callout(_))
                        && matches!(i, Item::NoteList(_) | Item::Table(_)))
            })
            .collect(),
        _ => Vec::new(),
    };
    hash_debug(&[&it, &doc.styles, &deps, inputs_gen])
}

/// Key of a page's drawing sheet.
pub fn sheet_key(doc: &Document, index: usize, inputs_gen: &Key) -> Key {
    let p: &Page = &doc.pages[index];
    hash_debug(&[
        &p.size,
        &p.title,
        &p.sheet,
        &doc.sheet,
        &index,
        &doc.pages.len(),
        &doc.styles,
        inputs_gen,
    ])
}

/// Key of a whole page (thumbnails).
pub fn page_key(doc: &Document, index: usize, inputs_gen: &Key) -> Key {
    hash_debug(&[
        &doc.pages[index],
        &index,
        &doc.pages.len(),
        &doc.styles,
        &doc.sheet,
        inputs_gen,
    ])
}

/// Key of the render inputs besides the document: board, plots, sheets.
pub fn inputs_key(board: Option<&Board>, plots_gen: u64, sheet_gen: u64) -> Key {
    hash_debug(&[&board.map(|b| b.hash.0), &plots_gen, &sheet_gen])
}

#[derive(Debug, Clone)]
pub enum RasterWhat {
    /// Whole page, for the pages panel.
    Thumb { page: Id },
    /// Drawing sheet of a page at canvas resolution.
    Sheet { page: Id },
    /// One item at canvas resolution.
    Item { item: Box<Item> },
}

pub struct RasterJob {
    pub inputs: Arc<RenderInputs>,
    pub what: RasterWhat,
    pub key: Key,
    /// Zoom bucket (canvas) or a fixed size (thumbs).
    pub bucket: i32,
    pub pixels_per_point: f64,
    /// Thumbnail width, pixels.
    pub thumb_width: f64,
    /// Largest texture side the GPU accepts, pixels.
    pub max_side: f64,
}

pub struct RasterOut {
    pub key: Key,
    pub bucket: i32,
    pub rgba: Vec<u8>,
    pub size: [usize; 2],
    /// Sheet region the image covers.
    pub region: BBox,
    /// Reference point of the item when rendered (items only).
    pub at: Option<Point>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    /// The page or item is not in the job's document any more (deleted
    /// while the job waited): an expected race, dropped quietly.
    pub gone: bool,
}

fn grow(b: BBox, d: f64) -> BBox {
    BBox {
        min: Point::new(b.min.x - d, b.min.y - d),
        max: Point::new(b.max.x + d, b.max.y + d),
    }
}

pub fn run_raster(j: RasterJob) -> RasterOut {
    let fonts = Arc::clone(&j.inputs.fonts);
    let mut out = RasterOut {
        key: j.key,
        bucket: j.bucket,
        rgba: Vec::new(),
        size: [0, 0],
        region: BBox::from_point(Point::ORIGIN),
        at: None,
        warnings: Vec::new(),
        error: None,
        gone: false,
    };
    let index = |id: &Id| j.inputs.doc.pages.iter().position(|p| &p.id == id);
    let (items, region, ppm, background) = match &j.what {
        RasterWhat::Thumb { page } => {
            let Some((pl, w)) = index(page).and_then(|i| j.inputs.render_page(i)) else {
                out.gone = true;
                return out;
            };
            out.warnings = w;
            let region = BBox {
                min: Point::ORIGIN,
                max: Point::new(pl.width, pl.height),
            };
            let ppm = j.thumb_width / pl.width.max(1.0);
            (pl.items, region, ppm, Some(Color::WHITE))
        }
        RasterWhat::Sheet { page } => {
            let Some(i) = index(page) else {
                out.gone = true;
                return out;
            };
            let Some((items, w)) = j.inputs.render_sheet(i) else {
                out.gone = true;
                return out;
            };
            out.warnings = w;
            let p = &j.inputs.doc.pages[i];
            let region = BBox {
                min: Point::ORIGIN,
                max: Point::new(p.size.width, p.size.height),
            };
            let ppm = camera::texture_ppm_max(&region, j.bucket, j.pixels_per_point, j.max_side);
            (items, region, ppm, Some(Color::WHITE))
        }
        RasterWhat::Item { item } => {
            let (items, w) = j.inputs.render_item(item);
            out.warnings = w;
            out.at = item.position();
            let Some(b) = kdraft_render::dl::items_bbox(&items, &fonts) else {
                // Nothing visible: an empty image at the item's position.
                out.region = BBox::from_point(out.at.unwrap_or(Point::ORIGIN));
                return out;
            };
            let region = grow(b, 0.5);
            let ppm = camera::texture_ppm_max(&region, j.bucket, j.pixels_per_point, j.max_side);
            (items, region, ppm, None)
        }
    };
    out.region = region;
    match raster::render(
        &items,
        &Target {
            region,
            px_per_mm: ppm,
            background,
        },
        &fonts,
    ) {
        Ok(p) => {
            out.size = [p.width() as usize, p.height() as usize];
            out.rgba = p.take();
        }
        Err(e) => out.error = Some(e.to_string()),
    }
    out
}

static RUNNER: std::sync::Mutex<Option<Arc<dyn CommandRunner>>> = std::sync::Mutex::new(None);

/// Replaces the program runner for kicad-cli (the fixture runner for
/// headless screenshots); kicad-cli is then the stand-in at
/// `/fake/kicad-cli` instead of a discovered one.
pub fn set_runner(r: Arc<dyn CommandRunner>) {
    if let Ok(mut g) = RUNNER.lock() {
        *g = Some(r);
    }
}

/// The runner in use and whether it is a stand-in.
pub fn runner() -> (Arc<dyn CommandRunner>, bool) {
    match RUNNER.lock().ok().and_then(|g| g.clone()) {
        Some(r) => (r, true),
        None => (Arc::new(SystemRunner), false),
    }
}

/// File and kicad-cli jobs (one thread: they are IO bound and ordered).
pub enum IoJob {
    Open(PathBuf),
    NewForBoard {
        board: PathBuf,
        /// Fetches the schematic BOM for the BOM pages when given.
        kicad: Option<KicadCli>,
        /// The user's document defaults and default palette.
        defaults: Box<kdraft_source::settings::DocDefaults>,
        palette: Option<kdraft_doc::palette::Palette>,
    },
    Save {
        path: PathBuf,
        text: String,
        rev: u64,
    },
    Export {
        inputs: Arc<RenderInputs>,
        path: PathBuf,
        format: pipeline::Format,
    },
    LoadBoard(PathBuf),
    WriteSettings {
        path: PathBuf,
        text: String,
        /// Shared sections (kicad, documents, palettes) to write too.
        shared: Option<serde_json::Map<String, serde_json::Value>>,
    },
    /// Size of a cache folder, bytes.
    CacheSize(PathBuf),
    /// Deletes a cache folder.
    ClearCache(PathBuf),
}

pub enum IoOut {
    Opened(Result<Box<Session>, String>),
    Saved(Result<(PathBuf, u64), String>),
    Exported(Result<(PathBuf, Vec<String>), String>),
    Board(PathBuf, Result<Box<Board>, String>),
    SettingsWritten(Result<(), String>),
    CacheSize(PathBuf, u64),
    CacheCleared(Result<PathBuf, String>),
}

pub fn run_io(j: IoJob) -> IoOut {
    match j {
        IoJob::Open(p) => IoOut::Opened(
            Session::open(&p)
                .map(Box::new)
                .map_err(|e| format!("{}: {e}", p.display())),
        ),
        IoJob::NewForBoard {
            board,
            kicad,
            defaults,
            palette,
        } => {
            let doc = board.with_extension("kdraft");
            let bom = kicad.and_then(|cli| {
                let k = pipeline::Kicad {
                    cli,
                    runner: Arc::new(SystemRunner),
                };
                let mut notes = Vec::new();
                pipeline::fetch_bom(&board, &k, &pipeline::cache_dir(Some(&doc)), &mut notes)
            });
            IoOut::Opened(
                Session::new_for_board_with(&board, Some(&doc), bom, &defaults, palette)
                    .map(Box::new)
                    .map_err(|e| format!("{}: {e}", board.display())),
            )
        }
        IoJob::Save { path, text, rev } => IoOut::Saved(
            std::fs::write(&path, text)
                .map(|_| (path.clone(), rev))
                .map_err(|e| format!("writing {}: {e}", path.display())),
        ),
        IoJob::Export {
            inputs,
            path,
            format,
        } => IoOut::Exported(
            pipeline::export_inputs(&inputs, &path, format)
                .map(|r| (path.clone(), r.notes))
                .map_err(|e| format!("exporting {}: {e}", path.display())),
        ),
        IoJob::WriteSettings { path, text, shared } => {
            IoOut::SettingsWritten(crate::settings::write(&path, &text, shared.as_ref()))
        }
        IoJob::CacheSize(p) => IoOut::CacheSize(p.clone(), kdraft_source::settings::dir_size(&p)),
        IoJob::ClearCache(p) => IoOut::CacheCleared(match std::fs::remove_dir_all(&p) {
            Ok(()) => Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(p),
            Err(e) => Err(format!("{}: {e}", p.display())),
        }),
        IoJob::LoadBoard(p) => {
            let r = Board::load(&p)
                .map(Box::new)
                .map_err(|e| format!("{}: {e}", p.display()));
            IoOut::Board(p, r)
        }
    }
}

/// What a 3D render pass produced.
pub enum Render3dPass {
    Probe(Vec<u8>),
    Done(Arc<kdraft_render::render3d::Picture>),
}

/// kicad-cli jobs.
pub enum CliJob {
    /// Finds kicad-cli: KDRAFT_KICAD_CLI, then the setting, then the
    /// newest install; with the settings' extra variables.
    Discover {
        choice: kdraft_source::settings::CliChoice,
        vars: Vec<(String, String)>,
    },
    /// Lists the installs found (Settings > KiCad).
    ListInstalls,
    /// Runs `kicad-cli version` on a path (the Test button).
    TestCli(PathBuf),
    /// Schematic BOM through `kicad-cli sch export bom`.
    Bom {
        cli: KicadCli,
        board_path: PathBuf,
        board_hash: [u8; 32],
        cache: PathBuf,
    },
    Plots {
        cli: KicadCli,
        board_path: PathBuf,
        board: Arc<Board>,
        layers: Vec<kdraft_engine::pipeline::LayerNeed>,
        cache: PathBuf,
    },
    /// One pass of a 3D view's picture through `kicad-cli pcb render`:
    /// pass 1 the small probe, pass 2 (with the probe) the final picture.
    Render3d {
        cli: KicadCli,
        board_path: PathBuf,
        board_hash: [u8; 32],
        key: String,
        view: Box<kdraft_doc::View>,
        board_box: kdraft_board::geom::BBox,
        probe: Option<Vec<u8>>,
        cache: PathBuf,
    },
}

pub enum CliOut {
    Found(Result<(KicadCli, kdraft_source::CliSource), String>),
    Installs(Vec<kdraft_source::Install>),
    Tested(PathBuf, Result<String, String>),
    Bom {
        board_hash: [u8; 32],
        bom: Option<Arc<kdraft_board::bom::Bom>>,
        notes: Vec<String>,
    },
    Plots {
        board_hash: [u8; 32],
        layers: Vec<kdraft_engine::pipeline::LayerNeed>,
        plots: Option<Arc<Plots>>,
        notes: Vec<String>,
    },
    Render3d {
        board_hash: [u8; 32],
        key: String,
        view: Box<kdraft_doc::View>,
        board_box: kdraft_board::geom::BBox,
        result: Result<Render3dPass, String>,
    },
}

pub fn run_cli(j: CliJob) -> CliOut {
    match j {
        CliJob::Discover { choice, vars } => {
            let (r, fake) = runner();
            let found = if fake {
                // The stand-in answers for any path; a custom path is
                // still shown as chosen.
                let p = match &choice {
                    kdraft_source::settings::CliChoice::Path(p) => p.clone(),
                    kdraft_source::settings::CliChoice::Auto => PathBuf::from("/fake/kicad-cli"),
                };
                KicadCli::probe(&p, r.as_ref()).map(|c| {
                    let src = if p == std::path::Path::new("/fake/kicad-cli") {
                        kdraft_source::CliSource::Auto
                    } else {
                        kdraft_source::CliSource::Setting
                    };
                    (c, src)
                })
            } else {
                kdraft_source::resolve(None, &choice, &kdraft_source::SystemEnvironment, r.as_ref())
            };
            CliOut::Found(
                found
                    .map(|(mut c, src)| {
                        c.vars = vars;
                        (c, src)
                    })
                    .map_err(|e| e.to_string()),
            )
        }
        CliJob::ListInstalls => {
            let (r, fake) = runner();
            CliOut::Installs(if fake {
                let p = PathBuf::from("/fake/kicad-cli");
                vec![kdraft_source::Install {
                    cli: KicadCli::probe(&p, r.as_ref()).ok(),
                    path: p,
                    error: None,
                }]
            } else {
                kdraft_source::list_installs(&kdraft_source::SystemEnvironment, r.as_ref())
            })
        }
        CliJob::TestCli(p) => {
            let (r, _) = runner();
            CliOut::Tested(
                p.clone(),
                KicadCli::probe(&p, r.as_ref())
                    .map(|c| c.version_string)
                    .map_err(|e| e.to_string()),
            )
        }
        CliJob::Bom {
            cli,
            board_path,
            board_hash,
            cache,
        } => {
            let k = pipeline::Kicad {
                cli,
                runner: runner().0,
            };
            let mut notes = Vec::new();
            let bom = pipeline::fetch_bom(&board_path, &k, &cache, &mut notes);
            CliOut::Bom {
                board_hash,
                bom,
                notes,
            }
        }
        CliJob::Plots {
            cli,
            board_path,
            board,
            layers,
            cache,
        } => {
            let src = FileBoardSource::new(&board_path, cli, runner().0, Cache::new(&cache));
            let mut notes = Vec::new();
            let plots = pipeline::fetch_plots(&layers, &src, &board, &mut notes).map(Arc::new);
            CliOut::Plots {
                board_hash: board.hash.0,
                layers,
                plots,
                notes,
            }
        }
        CliJob::Render3d {
            cli,
            board_path,
            board_hash,
            key,
            view,
            board_box,
            probe,
            cache,
        } => {
            let src = FileBoardSource::new(&board_path, cli, runner().0, Cache::new(&cache));
            let dir = board_path.parent();
            let run = |j: &kdraft_source::jobs::RenderJob| -> Result<Vec<u8>, String> {
                let job = j.job(src.path(), src.cli()).map_err(|e| e.to_string())?;
                let p = src.run_job(&job).map_err(|e| e.to_string())?;
                std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
            };
            let result = match probe {
                None => run(&pipeline::probe_job(&view, board_box, dir)).map(Render3dPass::Probe),
                Some(p) => pipeline::final_job(&view, board_box, dir, &p).and_then(|(j, z)| {
                    let png = run(&j)?;
                    let pic = pipeline::finish_render(&view, board_box, &png, z)?;
                    if z > 1.0001 && pic.note.contains("touches the edge") {
                        let png = run(&kdraft_source::jobs::RenderJob { zoom: None, ..j })?;
                        pipeline::finish_render(&view, board_box, &png, 1.0)
                    } else {
                        Ok(pic)
                    }
                    .map(|p| Render3dPass::Done(Arc::new(p)))
                }),
            };
            CliOut::Render3d {
                board_hash,
                key,
                view,
                board_box,
                result,
            }
        }
    }
}

/// Ids of items on a page that have an id.
pub fn item_ids(page: &Page) -> Vec<Id> {
    page.items.iter().filter_map(|i| i.id().cloned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/kicad9-minimal.kicad_pcb");
        Session::new_for_board(&p, None).unwrap()
    }

    #[test]
    fn item_key_ignores_position_only() {
        let s = session();
        let d = s.doc();
        let g = [0u8; 32];
        let it = &d.pages[0].items[0];
        let mut moved = it.clone();
        moved.translate(12.0, -3.0);
        assert_eq!(item_key(it, d, &g), item_key(&moved, d, &g));
        let mut scaled = it.clone();
        if let Item::View(v) = &mut scaled {
            v.scale *= 2.0;
        }
        assert_ne!(item_key(it, d, &g), item_key(&scaled, d, &g));
        assert_ne!(item_key(it, d, &g), item_key(it, d, &[1u8; 32]));
    }

    #[test]
    fn raster_jobs_produce_images() {
        let mut s = session();
        let (inputs, _) = RenderInputs::from_session(&mut s, None).unwrap();
        let inputs = Arc::new(inputs);
        let item = s.doc().pages[0].items[0].clone();
        let out = run_raster(RasterJob {
            inputs: Arc::clone(&inputs),
            what: RasterWhat::Item {
                item: Box::new(item.clone()),
            },
            key: [0; 32],
            bucket: camera::zoom_bucket(2.0),
            pixels_per_point: 1.0,
            thumb_width: 0.0,
            max_side: 4096.0,
        });
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.rgba.len(), out.size[0] * out.size[1] * 4);
        assert!(out.size[0] > 100);
        assert_eq!(out.at, item.position());
        // The image covers the view's frame.
        let frame = s.item_bounds(&item).unwrap();
        assert!(out.region.min.x <= frame.min.x && out.region.max.x >= frame.max.x);
        let page = inputs.doc.pages[2].id.clone();
        let t = run_raster(RasterJob {
            inputs,
            what: RasterWhat::Thumb { page },
            key: [0; 32],
            bucket: 0,
            pixels_per_point: 1.0,
            thumb_width: 160.0,
            max_side: 4096.0,
        });
        assert_eq!(t.size[0], 160);
        assert_eq!(t.size[1], 114);
    }
}

#[cfg(test)]
mod samples {
    use super::*;

    /// Writes the pages-panel thumbnails of a board's default document as
    /// PNG into `KDRAFT_SAMPLES_DIR` (manual: `cargo test -p kdraft-ui
    /// write_sample_thumbnails -- --ignored`).
    #[test]
    #[ignore]
    fn write_sample_thumbnails() {
        let Some(dir) = std::env::var_os("KDRAFT_SAMPLES_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let board = std::env::var_os("KDRAFT_SAMPLE_BOARD")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures/kicad9-minimal.kicad_pcb")
            });
        let stem = board.file_stem().unwrap().to_string_lossy().into_owned();
        let mut s = Session::new_for_board(&board, None).unwrap();
        let (inputs, _) = RenderInputs::from_session(&mut s, None).unwrap();
        let inputs = Arc::new(inputs);
        for page in 0..s.doc().pages.len() {
            let out = run_raster(RasterJob {
                inputs: Arc::clone(&inputs),
                what: RasterWhat::Thumb {
                    page: s.doc().pages[page].id.clone(),
                },
                key: [0; 32],
                bucket: 0,
                pixels_per_point: 1.0,
                thumb_width: crate::app::THUMB_WIDTH,
                max_side: 4096.0,
            });
            let size = tiny_skia_size(out.size);
            let pix = kdraft_render_pixmap(out.rgba, size);
            std::fs::write(dir.join(format!("{stem}-thumb-p{}.png", page + 1)), pix).unwrap();
        }
    }

    fn tiny_skia_size(s: [usize; 2]) -> (u32, u32) {
        (s[0] as u32, s[1] as u32)
    }

    fn kdraft_render_pixmap(rgba: Vec<u8>, (w, h): (u32, u32)) -> Vec<u8> {
        kdraft_render::raster::encode_png_rgba(rgba, w, h).unwrap()
    }
}
