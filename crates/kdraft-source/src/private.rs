//! Private copies of project files for kicad-cli runs.
//!
//! kicad-cli writes files next to the board it opens (after kdraft runs on
//! a user's PC a `<board>.kicad_prl` appeared beside the board; the cause
//! is inferred, not confirmed). kdraft must not write into the user's
//! project folder, so every job runs on a copy inside the cache:
//!
//! ```text
//! <cache root>/inputs/<content hash>/<board file>
//! <cache root>/inputs/<content hash>/<project>.kicad_pro
//! <cache root>/inputs/<content hash>/<schematic files, relative layout kept>
//! ```
//!
//! The hash covers every copied file's relative name and bytes, so it is
//! also the input hash for cache keys of jobs run on the copy.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::SourceError;

type R<T> = Result<T, SourceError>;

/// Files copied for one job, all relative to the copy's directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateCopy {
    /// Directory holding the copy.
    pub dir: PathBuf,
    /// The main input inside `dir` (board or root schematic).
    pub main: PathBuf,
    /// blake3 over all copied files (relative names and contents).
    pub hash: [u8; 32],
    /// Relative paths of the copied files, main input first.
    pub files: Vec<PathBuf>,
    /// Schematic sheet files referenced but not copied (missing, or outside
    /// the project folder).
    pub skipped: Vec<String>,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn read(p: &Path) -> R<Vec<u8>> {
    std::fs::read(p).map_err(|e| SourceError::io("reading", p, e))
}

/// A relative path that stays inside its base folder.
fn safe_relative(s: &str) -> Option<PathBuf> {
    let p = PathBuf::from(s.replace('\\', "/"));
    if p.as_os_str().is_empty() {
        return None;
    }
    p.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        .then_some(p)
}

/// Sheet file names referenced by a schematic: `(sheet ... (property
/// "Sheetfile" "name.kicad_sch"))` (KiCad 6 and later). Tolerant: a file
/// that does not parse yields no references.
pub fn sheet_files(schematic_text: &str) -> Vec<String> {
    let Ok(tree) = kdraft_sexpr::Tree::parse(schematic_text) else {
        return Vec::new();
    };
    let Some(root) = tree.root() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for sheet in root.find_all("sheet") {
        for p in sheet.find_all("property") {
            let name = p.str_at(1).map(|s| s.into_owned()).unwrap_or_default();
            if name == "Sheetfile" || name == "Sheet file" {
                if let Ok(v) = p.str_at(2) {
                    out.push(v.into_owned());
                }
            }
        }
    }
    out
}

/// Gathers the files to copy: `main`, the project file with the same stem
/// and, for schematics, all sheet files reachable from `main`.
type Files = Vec<(PathBuf, Vec<u8>)>;

fn gather(main: &Path, schematic: bool) -> R<(Files, Vec<String>)> {
    let base = main.parent().unwrap_or(Path::new("")).to_path_buf();
    let main_name = PathBuf::from(main.file_name().unwrap_or(main.as_os_str()));
    let mut files = vec![(main_name.clone(), read(main)?)];
    let mut skipped = Vec::new();
    let pro = main.with_extension("kicad_pro");
    if pro.is_file() {
        files.push((
            PathBuf::from(pro.file_name().unwrap_or(pro.as_os_str())),
            read(&pro)?,
        ));
    }
    if schematic {
        let mut queue = vec![main_name];
        let mut seen: Vec<PathBuf> = queue.clone();
        while let Some(rel) = queue.pop() {
            let text = files
                .iter()
                .find(|(p, _)| *p == rel)
                .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            let dir = rel.parent().map(Path::to_path_buf).unwrap_or_default();
            for name in sheet_files(&text) {
                let Some(r) = safe_relative(&name).map(|r| dir.join(r)) else {
                    skipped.push(name);
                    continue;
                };
                if seen.contains(&r) {
                    continue;
                }
                seen.push(r.clone());
                let abs = base.join(&r);
                match std::fs::read(&abs) {
                    Ok(b) => {
                        files.push((r.clone(), b));
                        queue.push(r);
                    }
                    Err(_) => skipped.push(name),
                }
            }
        }
    }
    Ok((files, skipped))
}

fn content_hash(files: &[(PathBuf, Vec<u8>)]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"kdraft-private-copy-v1");
    for (p, b) in files {
        let name = p.to_string_lossy().replace('\\', "/");
        h.update(&(name.len() as u64).to_le_bytes());
        h.update(name.as_bytes());
        h.update(&(b.len() as u64).to_le_bytes());
        h.update(b);
    }
    *h.finalize().as_bytes()
}

/// Copies `main` (a board, or a root schematic when `schematic` is set)
/// and its companions into `<cache_root>/inputs/<hash>/`, reusing an
/// existing complete copy. Nothing is written next to `main`.
pub fn private_copy(main: &Path, cache_root: &Path, schematic: bool) -> R<PrivateCopy> {
    private_copy_edited(main, cache_root, schematic, None)
}

/// Edits the main file's bytes before they are copied.
pub type MainEdit<'a> = &'a dyn Fn(Vec<u8>) -> R<Vec<u8>>;

/// As [`private_copy`], with the main file's bytes passed through `edit`
/// first (e.g. hiding fab texts). The hash covers the edited bytes.
pub fn private_copy_edited(
    main: &Path,
    cache_root: &Path,
    schematic: bool,
    edit: Option<MainEdit<'_>>,
) -> R<PrivateCopy> {
    let (mut files, skipped) = gather(main, schematic)?;
    if let Some(f) = edit {
        let b = std::mem::take(&mut files[0].1);
        files[0].1 = f(b)?;
    }
    let hash = content_hash(&files);
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    let inputs = cache_root.join("inputs");
    let dir = inputs.join(&hex[..24]);
    let rels: Vec<PathBuf> = files.iter().map(|(p, _)| p.clone()).collect();
    let done = dir.join("COMPLETE");
    if !done.is_file() {
        let tmp = inputs.join(format!(
            "tmp-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&tmp).map_err(|e| SourceError::io("creating", &tmp, e))?;
        let write_all = || -> R<()> {
            for (rel, bytes) in &files {
                let target = tmp.join(rel);
                if let Some(p) = target.parent() {
                    std::fs::create_dir_all(p).map_err(|e| SourceError::io("creating", p, e))?;
                }
                std::fs::write(&target, bytes)
                    .map_err(|e| SourceError::io("writing", &target, e))?;
            }
            std::fs::write(tmp.join("COMPLETE"), "")
                .map_err(|e| SourceError::io("writing", tmp.join("COMPLETE"), e))
        };
        if let Err(e) = write_all() {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
        if std::fs::rename(&tmp, &dir).is_err() {
            // Another writer won, or a stale incomplete copy is in the way.
            let _ = std::fs::remove_dir_all(&tmp);
            if !done.is_file() {
                let _ = std::fs::remove_dir_all(&dir);
                return private_copy_direct(&files, &dir).map(|()| PrivateCopy {
                    main: dir.join(&rels[0]),
                    dir: dir.clone(),
                    hash,
                    files: rels.clone(),
                    skipped: skipped.clone(),
                });
            }
        }
    }
    Ok(PrivateCopy {
        main: dir.join(&rels[0]),
        dir,
        hash,
        files: rels,
        skipped,
    })
}

fn private_copy_direct(files: &[(PathBuf, Vec<u8>)], dir: &Path) -> R<()> {
    for (rel, bytes) in files {
        let target = dir.join(rel);
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p).map_err(|e| SourceError::io("creating", p, e))?;
        }
        std::fs::write(&target, bytes).map_err(|e| SourceError::io("writing", &target, e))?;
    }
    std::fs::write(dir.join("COMPLETE"), "")
        .map_err(|e| SourceError::io("writing", dir.join("COMPLETE"), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn copies_board_and_project_and_reuses() {
        let proj = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let b = proj.path().join("x.kicad_pcb");
        std::fs::write(&b, "(kicad_pcb)").unwrap();
        std::fs::write(proj.path().join("x.kicad_pro"), "{}").unwrap();
        let before = listing(proj.path());
        let c = private_copy(&b, cache.path(), false).unwrap();
        assert!(c.main.starts_with(cache.path()));
        assert_eq!(c.main.file_name().unwrap(), "x.kicad_pcb");
        assert!(c.dir.join("x.kicad_pro").is_file());
        assert_eq!(c.files.len(), 2);
        let again = private_copy(&b, cache.path(), false).unwrap();
        assert_eq!(again, c);
        // A project change gives a new copy.
        std::fs::write(proj.path().join("x.kicad_pro"), "{\"a\":1}").unwrap();
        let third = private_copy(&b, cache.path(), false).unwrap();
        assert_ne!(third.hash, c.hash);
        assert_eq!(listing(proj.path()), before);
    }

    #[test]
    fn schematic_hierarchy_is_followed() {
        let proj = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let root = proj.path().join("top.kicad_sch");
        std::fs::create_dir_all(proj.path().join("sub")).unwrap();
        std::fs::write(
            &root,
            "(kicad_sch (sheet (property \"Sheetname\" \"A\") (property \"Sheetfile\" \"sub/a.kicad_sch\")) (sheet (property \"Sheetfile\" \"../evil.kicad_sch\")) (sheet (property \"Sheetfile\" \"missing.kicad_sch\")))",
        )
        .unwrap();
        std::fs::write(
            proj.path().join("sub/a.kicad_sch"),
            "(kicad_sch (sheet (property \"Sheetfile\" \"b.kicad_sch\")))",
        )
        .unwrap();
        std::fs::write(proj.path().join("sub/b.kicad_sch"), "(kicad_sch)").unwrap();
        let c = private_copy(&root, cache.path(), true).unwrap();
        assert!(c.dir.join("sub/a.kicad_sch").is_file());
        assert!(c.dir.join("sub/b.kicad_sch").is_file());
        assert_eq!(c.files.len(), 3);
        assert_eq!(
            c.skipped,
            vec![
                "../evil.kicad_sch".to_string(),
                "missing.kicad_sch".to_string()
            ]
        );
    }
}
