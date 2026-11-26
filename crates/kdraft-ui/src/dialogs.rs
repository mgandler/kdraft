//! File dialogs that never block the UI thread: native dialogs (Windows
//! common item dialogs, the XDG desktop portal elsewhere, through `rfd`)
//! run on their own thread and report back through a channel; without
//! native dialogs (tests, `KDRAFT_TYPED_PATHS=1`) the app shows a typed
//! path prompt instead.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Open,
    OpenBoard,
    SaveAs,
    ExportPdf,
    ExportPng,
    ExportSvg,
    ChooseSheet,
    /// Settings > KiCad: a kicad-cli executable.
    KicadCli,
    /// Settings > Layer colours: a KiCad colour theme.
    KicadTheme,
    /// Settings > Layer colours: a kdraft palette to import.
    PaletteImport,
    /// Settings > Layer colours: where to export a palette.
    PaletteExport,
    /// Settings > Documents: the default drawing sheet.
    DefaultSheet,
}

impl DialogKind {
    pub fn title(self) -> &'static str {
        match self {
            DialogKind::Open => "Open document or board",
            DialogKind::OpenBoard => "New drawing set from a board",
            DialogKind::SaveAs => "Save document as",
            DialogKind::ExportPdf => "Export PDF",
            DialogKind::ExportPng => "Export PNG",
            DialogKind::ExportSvg => "Export SVG",
            DialogKind::ChooseSheet => "Choose drawing sheet",
            DialogKind::KicadCli => "Choose kicad-cli",
            DialogKind::KicadTheme => "Import KiCad colour theme",
            DialogKind::PaletteImport => "Import palette",
            DialogKind::PaletteExport => "Export palette",
            DialogKind::DefaultSheet => "Default drawing sheet",
        }
    }

    fn filters(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            DialogKind::Open => &[
                (
                    "kdraft documents and KiCad boards",
                    &["kdraft", "kicad_pcb"],
                ),
                ("kdraft documents", &["kdraft"]),
                ("KiCad boards", &["kicad_pcb"]),
            ],
            DialogKind::OpenBoard => &[("KiCad boards", &["kicad_pcb"])],
            DialogKind::SaveAs => &[("kdraft documents", &["kdraft"])],
            DialogKind::ExportPdf => &[("PDF", &["pdf"])],
            DialogKind::ExportPng => &[("PNG images", &["png"])],
            DialogKind::ExportSvg => &[("SVG drawings", &["svg"])],
            DialogKind::ChooseSheet | DialogKind::DefaultSheet => {
                &[("KiCad drawing sheets", &["kicad_wks"])]
            }
            DialogKind::KicadCli => &[],
            DialogKind::KicadTheme | DialogKind::PaletteImport | DialogKind::PaletteExport => {
                &[("JSON", &["json"])]
            }
        }
    }

    fn saves(self) -> bool {
        matches!(
            self,
            DialogKind::SaveAs
                | DialogKind::ExportPdf
                | DialogKind::ExportPng
                | DialogKind::ExportSvg
                | DialogKind::PaletteExport
        )
    }
}

type Waker = Arc<dyn Fn() + Send + Sync>;

pub struct Dialogs {
    tx: Sender<(DialogKind, Option<PathBuf>)>,
    rx: Receiver<(DialogKind, Option<PathBuf>)>,
    waker: Waker,
    /// A native dialog is open (no second one is started).
    pub open: Option<DialogKind>,
    /// Use native dialogs; off in tests.
    pub native: bool,
}

impl Dialogs {
    pub fn new(waker: Waker) -> Dialogs {
        let (tx, rx) = channel();
        Dialogs {
            tx,
            rx,
            waker,
            open: None,
            native: false,
        }
    }

    /// Starts a native dialog on a thread; false if native dialogs are
    /// off or one is already open.
    pub fn request(&mut self, kind: DialogKind, initial: Option<PathBuf>) -> bool {
        if !self.native || self.open.is_some() {
            return false;
        }
        let tx = self.tx.clone();
        let waker = Arc::clone(&self.waker);
        let spawned = std::thread::Builder::new()
            .name("kdraft-dialog".into())
            .spawn(move || {
                let mut d = rfd::FileDialog::new().set_title(kind.title());
                for (name, ext) in kind.filters() {
                    d = d.add_filter(*name, ext);
                }
                if let Some(p) = &initial {
                    if let Some(dir) = p.parent().filter(|d| d.is_dir()) {
                        d = d.set_directory(dir);
                    }
                    if kind.saves() {
                        if let Some(f) = p.file_name() {
                            d = d.set_file_name(f.to_string_lossy());
                        }
                    }
                }
                let r = if kind.saves() {
                    d.save_file()
                } else {
                    d.pick_file()
                };
                let _ = tx.send((kind, r));
                waker();
            });
        if spawned.is_ok() {
            self.open = Some(kind);
        }
        spawned.is_ok()
    }

    /// Finished dialogs: kind and the chosen path (`None`: cancelled).
    pub fn results(&mut self) -> Vec<(DialogKind, Option<PathBuf>)> {
        let r: Vec<_> = self.rx.try_iter().collect();
        if !r.is_empty() {
            self.open = None;
        }
        r
    }
}
