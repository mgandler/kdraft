# kdraft

Native Rust tool that generates fabrication and assembly drawing sets for KiCad
boards (KiCad 9 and 10), modelled on Altium Draftsman. Read `docs/PLAN.md` first:
it holds the architecture, document model, data sources and roadmap.

## Layout

Cargo workspace under `crates/`. Dependency direction, top to bottom:

```
kdraft-ui           kdraft-cli
        kdraft-engine
kdraft-render        kdraft-layout
kdraft-doc    kdraft-plot
kdraft-sheet  kdraft-source
        kdraft-board
        kdraft-sexpr
```

- `kdraft-sheet`: `.kicad_wks`, text variables, `.kicad_pro`, sheet choice.
- `kdraft-plot`: generic SVG reader, board-to-plot transform fit, spike S1
  (`fake` module behind feature `test-support`: synthetic kicad-cli).
- `kdraft-render`: display list, native board views, plot import, tables,
  sheet drawing, SVG and PDF (krilla) back ends.
- `kdraft-render` also has the tiny-skia raster back end (`raster`), used
  by PNG export and the editor canvas.
- `kdraft-engine`: `Session` (document, board, sheets, selection, undo
  log, gestures), command `Registry`, item `bounds`, pure `snap`,
  `pipeline` (kicad-cli plots, render inputs, export).
- `kdraft-layout`: pure layout geometry: spatial grid, designator
  declutter, dimension geometry (no fonts; callers pass text widths).
- `kdraft-engine` also has `check` (drawing checks) and `enable` (which
  commands can run on the selection; GUI buttons use it).
- kicad-cli never runs on the user's files: `kdraft_source::private_copy`
  copies board + `.kicad_pro` (or a schematic hierarchy) into
  `<cache>/inputs/<hash>/`; fab texts are hidden by editing that copy
  (`kdraft_board::hide_fab_texts`, lossless edits via `kdraft_sexpr::edit`).
- `kdraft-ui`: binary `kdraft-gui`; plain logic modules with tests
  (`actions` catalogue, `tools`, `anchors`, `tree`, `layers`, `measure`,
  `rulers`, `drag`, `hit`, `camera`, `settings`, `theme`, `widgets`
  helpers); egui code in `ui.rs`, `ui_left.rs`, `ui_canvas.rs`,
  `ui_inspector.rs`, `ui_menu.rs`; state and actions in `app.rs`,
  `appx.rs`; annotations drawn live by `vector.rs`. See `docs/GUI.md`.
- Command presentation (category, icon id, shortcut, variants) is data
  in `kdraft-engine/src/meta.rs`; menus, palette and shortcuts come from
  it. UI strings live in `kdraft-ui/src/strings.rs`.
- `assets/`: bundled font and KiCad's default sheet (with NOTICE files).

Nothing below `kdraft-ui` may depend on egui. Every user-facing action becomes a
named command in the engine registry; GUI and CLI call the same code. New
commands: add the argument struct (`deny_unknown_fields`) and its `ArgSpec`
list in `kdraft-engine/src/commands.rs`; the spec test checks they agree.
The GUI never blocks its thread on kicad-cli, file IO or rasterising: use
the workers in `kdraft-ui/src/jobs.rs`.

## Commands

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p kdraft-cli -- inspect path/to/board.kicad_pcb
cargo run -p kdraft-cli -- doc new board.kicad_pcb -o b.kdraft
cargo run -p kdraft-cli -- export b.kdraft -o b.pdf --no-kicad-cli
cargo run -p kdraft-cli -- export b.kdraft -o b.png --format png --dpi 100 --no-kicad-cli
cargo run -p kdraft-cli -- check b.kdraft [--json]
cargo run -p kdraft-cli -- cmd list
cargo run --release -p kdraft-ui --bin kdraft-gui -- b.kdraft
cargo run -p kdraft-ui --bin kdraft-gui -- --screenshot out.png b.kdraft
cargo run --release -p kdraft-cli -- spike s1 board.kicad_pcb   # needs kicad-cli
KDRAFT_UPDATE_GOLDEN=1 cargo test -p kdraft-render            # regenerate goldens
scripts/fetch-fixtures.sh        # KiCad demos into fixtures-ext/ (git-ignored)
```

## Conventions

- Rust 2021, stable toolchain (1.92 or newer, needed by krilla),
  `#![forbid(unsafe_code)]` in every crate.
- Library crates return `Result` with `thiserror` error types; no `unwrap`,
  `expect` or `panic!` outside tests. `anyhow` only in the CLI binary.
- Lengths are millimetres as `f64` in the board and document models. KiCad's
  Y axis points down; keep KiCad coordinates, do not flip in the model.
- The S-expression layer is lossless: parse then write returns the input bytes.
  Typed models read from the tree; they never own the file text.
- Parsers are tolerant: read the needed subset, ignore unknown nodes, never fail
  on a node kdraft does not use. KiCad 9 (`version 2024xxxx`/`2025xxxx`) and
  KiCad 10 files must both load.
- `.kdraft` writer output is canonical (fixed key order and number formatting)
  and round-trips unknown nodes.
- Tests sit next to the code; shared fixtures in `fixtures/` are small,
  self-authored files. Third-party boards go to `fixtures-ext/` via the fetch
  script and are never committed.
- Licence: GPL-3.0-or-later (`LICENSE`, `license` in `workspace.package`).
  Dependencies must be GPL-3.0-compatible, pure Rust (no C toolchain,
  system libraries or pkg-config). Keep the dependency set small and list
  every dependency with its licence in `docs/STATUS.md`.
- Windows is the target platform: everything must build, test and behave
  correctly there (`std::path`, no symlinks, no shell needed for normal use
  or for the test suite). Code stays portable.
- Millimetres only; no dual units.
- Keep comments to what the code cannot say. English identifiers and docs.

## Environment notes

- `kicad-cli` may be absent (it is in the cloud workspace). Everything that
  shells out goes through the `CommandRunner` trait in `kdraft-source`, with a
  fake runner in tests. Tests that need a real `kicad-cli` are `#[ignore]` and
  read `KDRAFT_KICAD_CLI`.
- Spike S1 has run on kicad-cli 10.0.8 (two boards), not on 9.0.x
  (`docs/S1.md`, `docs/STATUS.md`). Plot placement still always goes
  through `kdraft_plot::fit` at runtime; KiCad-like SVG layouts exist only
  in test code. Layer exports use page-size mode 0.
- Real kicad-cli 10.0.8 help texts and a drill report are fixtures in
  `fixtures/kicad-cli/10.0.8/`; emitted flags are tested against them.
  `sch export bom` has no recorded help text yet (flags UNVERIFIED).
- Disk space in the cloud workspace is limited: build with
  `CARGO_INCREMENTAL=0`, delete stale `target/debug/deps/*kdraft*`
  artefacts when below about 3 GB, avoid `--release` unless measuring.
- No display here: the user runs the GUI on Windows. Look at it with the
  software screenshot path: `kdraft-gui --screenshot out.png --size
  1400x900 [--theme dark] [--script steps.json] file` (and read the PNG);
  goldens in `crates/kdraft-ui/tests/golden` (`KDRAFT_UPDATE_GOLDEN=1`).
  Keep logic out of egui code and test it headless (unit tests,
  `egui_kittest`).
- static.rust-lang.org is not reachable from the cloud workspace, so no
  Windows target can be added for cross-checking.

## Status

See `docs/STATUS.md` for what is implemented, what is verified against real
KiCad files, and the next tasks. Update it at the end of every work session.
