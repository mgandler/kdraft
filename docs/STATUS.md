# kdraft status

Last updated: 2026-11-25 (UI pass of the editor; fixes from the user's
first real M3 run: dimension selection, BOM numbering, dense pad
fields, dimensions following a dragged view).

M1 gate ("real board to PDF from the CLI"): reached for the native path and,
on kicad-cli 10.0.8, for the plotted path on two boards (see "Spike S1
results"). KiCad 9.0.x has not been run.

M2 gate ("drawing set laid out in the GUI"): the editor builds and runs on
the user's Windows PC (HEAD 936c6cb, glow renderer, kicad-cli 10.0.8). The
user exercised selection, drag-move, corner scaling with undo, scale
buttons, layer toggling with background plots, zoom/pan, page reorder by
drag, table dragging with snap guides, marquee selection, align, and the
distribute error message (`docs/GUI.md`). One real bug (thumbnail press
never selected the page) is fixed in this increment. Not exercised: Export
PDF from the GUI, Save, the reload banner, most shortcuts.

M3 gate ("first usable release": drill table and drawing, stackup, BOM,
notes, dimensions, callouts): all parts are implemented and verified
headless on demo boards (rendered PNGs looked at); none of the M3 output
has been seen on the user's PC, with kicad-cli, or in the GUI by a person.
The M2 and M3 code has not been built on Windows (M2 build status there:
it was built from 936c6cb and runs; M3 unknown).

## Decisions (2026-11-24)

See `docs/PLAN.md`, section Decisions: GPL-3.0-or-later (`LICENSE` from the
KiCad source mirror tag 10.0.8, `license` in `workspace.package`), Windows
as target platform, BOM from the schematic (`sch export bom`, no work yet),
drawing-sheet selection order page > document > project > default, mm only,
0.5 mm snap grid (stored as `(styles (snap_grid 0.5))`).

## Implemented

### kdraft-sexpr

- `Tree`: lossless, read-only tree over the source text. Flat node arena with
  byte offsets only (about 20 bytes per node); atoms decoded on demand.
  `Tree::parse(x)?.to_text() == x` for every input that parses: whitespace,
  CRLF/LF, quoting and number spelling are kept. Single linear pass with an
  explicit stack; nesting limited by `ParseOptions::max_depth` (default 512),
  so deep input gives an error instead of a stack overflow.
- Queries on `Node`: `head`, `find`, `require`, `find_all`, `lists`,
  `has_symbol`, `str_at`, `f64_at`, `i64_at`, `to_value`. Errors carry byte
  offset and 1-based line:column (column in characters).
- `Value`: owned tree with builders (`sym`, `str`, `num`, `int`, `node`) and
  the canonical printer (2-space indent; atom-only lists on one line; lists
  with list children one child per line and `)` on its own line; numbers as
  shortest round-trip decimal, no exponent, no trailing zeros, `-0` as `0`;
  atoms quoted only when needed unless created with `Atom::string`).
- Escapes: `\"`, `\\`, `\n`, `\r`, `\t`; others kept verbatim.
- Tests: unit tests, error cases (unbalanced, unterminated, stray `)`, depth
  limit, 200k-node file), proptests: canonical print/parse round trip and
  fixed point, byte-exact raw-text round trip on generated KiCad-like text,
  round trip of arbitrary parsable strings, lossless vs canonical agreement.

### kdraft-board

- `Board::load` / `from_bytes` / `from_tree`: file `version`, `generator`,
  `generator_version`, `general (thickness)`, `paper`, layer table (ordinal,
  name, type, user name), title block (title, date, rev, company, comments).
- Stackup: ordered layers (name, type, thickness, material, epsilon_r,
  loss_tangent, color); dielectric sublayers (`addsublayer`) become separate
  entries with `sublayer` 0, 1, ...; copper finish, dielectric constraints,
  edge connector, castellated pads, edge plating.
- Footprints (`footprint`, legacy head `module` accepted): lib id, layer and
  side, position, rotation, locked, uuid (or legacy `tstamp`), reference and
  value from `(property ...)` with fallback to `fp_text reference/value`,
  attributes (smd/through_hole, exclude_from_pos_files, exclude_from_bom,
  dnp, board_only), all properties as a map, courtyard and fab bounding
  boxes (lines, rects, circles, exact arcs, polygons; centre lines, stroke
  width not included).
- Pads: number, type, shape, relative and absolute position and rotation,
  size, drill (round, oval, offset), layers, net (KiCad 9 `(net N "name")`,
  KiCad 10 `(net "name")`, id-only `(net N)` via the net table), uuid.
- Vias: position, size, drill, layers, type (through, blind/buried, micro),
  net, uuid.
- Outline: Edge.Cuts `gr_line/arc/circle/rect/poly/curve` and footprint
  `fp_*` on Edge.Cuts (transformed to board coordinates), exact bounding box
  (curves: control-point box). Chaining into closed contours with a 0.01 mm
  end-point tolerance (grid hash, linear in practice); vertex list per
  contour; unclosed chains reported in `open_chains`, never an error.
  `Outline::outer()` is the contour with the largest box.
- Holes (`thru_hole` and `np_thru_hole` pads with non-zero drill, then vias,
  stable indices) and drill table grouped by size (1 nm), plated/non-plated
  and via/pad; ovals normalised to width <= height; ordering: plated first,
  tool diameter, round before slot, slot length, via before pad.
- `BoardHash` (blake3 of the file bytes) and `diff()`: footprints added /
  removed (matched by uuid, then reference), moved, rotated, flipped, value,
  reference, lib id and DNP changes; outline changed (order-independent
  fingerprint), hole set changed (counts added/removed), stackup and board
  thickness changed, title block field changes.
- `warnings` collects things read but not understood (legacy arcs without
  `mid`, curves without 4 points, vias without size).

kdraft-board additions in this increment: `geom::Affine` (SVG matrix order, compose,
invert, map boxes); pads keep `roundrect_rratio`.

### kdraft-source

- (as before) runner trait, discovery, job builders, cache, `BoardSource`.
- New: `SvgExport::drawing_sheet` (`--drawing-sheet`, cache key uses the
  sheet's content hash via `Arg::InputFile`, not its path); `stub` module:
  stub `.kicad_pcb` (paper incl. `User` size and portrait, title block) and
  `.kicad_pro` (text variables) for rendering a drawing sheet with
  kicad-cli.

### kdraft-plot (new)

Reading `kicad-cli` plot output without assuming its layout.

- `svg`: generic SVG reader on `roxmltree` (DTD allowed): root
  `width`/`height` with units (mm, cm, in, pt, pc, px, bare), `viewBox`,
  `preserveAspectRatio`, nested `transform`s (matrix, translate, scale,
  rotate with centre, skewX/Y), inherited presentation attributes and
  `style`, `path` (all commands incl. arcs, smooth curves, compact arc
  flags; parsing stops at the first error and keeps what was read), `rect`
  (rounded), `circle`, `ellipse`, `line`, `polyline`, `polygon`, `use`
  (also of `symbol`), nested `svg`. Hidden and zero-opacity elements are
  skipped, `text` counted only, white/transparent shapes treated as
  background. Geometry in root user units; exact cubic bounds; content box
  with and without half stroke widths; user unit to page mm mapping.
- `fit`: transform `u = sx*x + ox, v = sy*y + oy` (board mm to user units).
  Bounding-box estimate for all four sign combinations, refined by
  matching reference points (pad centres or outline vertices) against
  shape box centres with shrinking radii and per-axis least squares
  (spatial grid for nearest lookup); best combination by fraction within
  0.05 mm. Residuals (max, RMS, median, within 0.01/0.05 mm, per pad shape),
  6-parameter affine least squares for diagnosis, outline box deviation per
  side. `fit_board()` is what the exporter uses at runtime.
- `s1`: spike S1 runner (see below). `fake` (feature `test-support`, tests
  only): a synthetic kicad-cli stand-in whose SVGs deliberately differ per
  mode (1e-4/1e-3/1e-2 mm units, inch root size, nested transforms, Y-up
  mode, white background), with drill report, pos CSV and stats JSON.

### Spike S1 command

`kdraft spike s1 <board> [--out DIR] [--kicad-cli PATH] [--drawing-sheet
WKS]`, output in `DIR/<board>-kicad-<version>/`: `--help` probes with flag
presence, single-layer SVG exports (Edge.Cuts, F.Cu, B.Cu, F.Fab,
F.Courtyard falling back to F.CrtYd) for page size modes 0, 1, 2 plus a
mirrored F.Cu export; on the first failure, retries without each optional
flag and drops the ones whose removal helps; per mode fit and validation
on F.Cu and B.Cu pads; Edge.Cuts box check (exact or uniform inflation);
content boxes of all layers in board mm; overlay SVGs; drill (with report,
tolerant parse), pos CSV (row count, per-axis fit of positions) and stats
JSON (width/height fields, key names not assumed) cross-checks; two
drawing-sheet exports from a stub board (without and with
`--drawing-sheet`) with size, content margins, marker texts and `${`
count. `report.json`, `report.md` with PASS/FAIL per layer and mode and a
`VERDICT:` line. Exit 0 PASS, 1 FAIL, 2 could not run. Tested end to end
with the synthetic runner (transforms recovered to 1e-6, mirror detected,
rejected flag identified, layer-name fallback, cross-checks).

### kdraft-sheet (new)

A separate crate because both `kdraft-doc` (drawing area for layout) and
`kdraft-render` (sheet drawing) need it, and it reads `.kicad_pro` JSON,
which does not belong in the document model.

- `.kicad_wks` reader (also legacy `page_layout` root): `setup` (text size,
  line widths, margins), `line`, `rect`, `tbtext` (corner anchors, font
  size/bold/italic, justify, rotate, maxlen, maxheight, incrlabel),
  `polygon`, `bitmap` (extent from the PNG header and ppi; content
  ignored), `repeat`/`incrx`/`incry`, `option page1only`/`notonpage1`,
  `name`, `comment`. Item-level problems become warnings.
- Resolution to absolute page geometry for a paper size and page number;
  repeated copies dropped when they leave the margin area; label
  increment for numbers and single letters.
- Text variables: `${TITLE}`, `${REVISION}`, `${ISSUE_DATE}`,
  `${COMPANY}`, `${COMMENT1..9}`, `${SHEETNAME}`, `${SHEETPATH}`, `${#}`,
  `${##}`, `${PAPER}`, `${KICAD_VERSION}`, `${FILENAME}`,
  `${PROJECTNAME}`, then project variables; recursive; unknown stay
  verbatim. Legacy `%` codes converted for files older than 20210606.
- `.kicad_pro`: `text_variables`, `pcbnew.page_layout_descr_file` (the
  schematic one is read for information only); path resolution with
  `${KIPRJMOD}`, relative paths, `\` separators. `select_sheet()`: page,
  document, project, default.
- Built-in default: KiCad's own default drawing sheet, extracted verbatim
  from `common/drawing_sheet/drawing_sheet_default_description.cpp`
  (identical in 9.0.9 and 10.0.8), GPL-2.0-or-later, attribution in
  `assets/sheets/NOTICE`.
- Heuristics: frame (smallest rectangle spanning more than half the page),
  title block box (non-frame items in the frame's bottom-right quarter,
  grid ticks under 5 mm excluded), drawing area (larger of "above" or
  "left of" the title block).

### kdraft-doc

- (as before) model, reader, canonical writer, operations, anchors.
- New: document-level `(sheet "...")` override, `default` selects KiCad's
  default sheet; `styles.snap_grid` (0.5 mm default); drawing area from the
  sheet (replaces the 20 mm / 40 mm placeholder constants);
  `new_for_board(board, path, sheet)` creates three pages: Fabrication,
  Assembly top, Drill and stackup (drill view plus drill and stackup
  tables, placed beside or below the view, whichever allows the larger
  standard scale). No sheet override is written, so rendering picks the
  project sheet or the default.

### kdraft-render (new)

- Display list: paths (move/line/cubic/close; arcs and circles as cubics)
  with stroke (width, cap, join, dash) and/or fill (non-zero/even-odd),
  text runs (string, position, cap height, horizontal/vertical alignment,
  rotation, bold, italic, colour, invisible-but-searchable), groups with
  affine transform and clip rectangle. mm, Y down, origin top-left.
- View placement: `ViewMap` (centre of crop or outline box on `view.at`,
  back views mirrored about the vertical axis, rotation counter-clockwise,
  scale), frame box, clip to the frame, label under the view
  (`ASSEMBLY TOP   SCALE 5:1`).
- Native board rendering (schematic, not a plot): outline primitives with
  true arcs, holes (plated: circle; non-plated: circle with cross; slots
  as stadiums at pad rotation), pads (rect, roundrect with rratio, circle,
  oval; others as their size box) filtered by copper side, fab or
  courtyard boxes, reference designators centred and fitted (horizontal
  or 90 degrees, 0.35 to 2.5 mm cap height, omitted below). Text upright
  in back views. Views drawn natively get a frame and the note
  "NATIVE PREVIEW (board model, not kicad-cli plot output)" above it.
  Drill views are always native.
- SVG import: plot shapes to paths through `fit⁻¹` then the view map (the
  fit comes from `kdraft_plot::fit`, never hard-coded).
- Tables: layout with column widths from measured text, title row, header
  row, bold footer, borders; drill table (symbol letter placeholder, size,
  PTH/NPTH, via/pad/slot, count, total) and layer stack table (layer,
  type, material, thickness, total and copper finish). Document column
  selection by key. BOM and custom tables are reported as not rendered.
- Drawing sheet: native from the resolved `.kicad_wks` (maxlen/maxheight
  shrink with measured text), or from a kicad-cli stub-sheet SVG placed by
  fitting it against the native line/rectangle geometry; then the native
  texts are added invisibly for search. Falls back to native when the fit
  is not good.
- Fonts: IBM Plex Sans Regular and Bold 3.006 (OFL-1.1) embedded;
  measurement with `ttf-parser` (sum of advances, no kerning); cap height
  from the OS/2 table; italic as synthetic oblique (slant 0.2).
- PDF back end (krilla 0.8.2): one page per document page at its size,
  vector paths, text as glyph runs with the embedded (subset) font, one
  bookmark per page, metadata title and creator, no creation date.
  Deterministic (test compares two runs byte for byte).
- SVG back end: one SVG per page in mm, numbers with at most 4 decimals,
  deterministic; text as `<text>` in the font family (viewers without the
  font substitute).
- Also rendered: `Graphic` items (except images) and `NoteList` as plain
  numbered lines. Dimensions, callouts and images are reported as not
  rendered.

### kdraft-cli (`kdraft`)

- (as before) `inspect`, `drill-table`, `diff`, `sexpr-check`, `doc new`,
  `doc validate`.
- `doc new` now writes three pages and reports the chosen sheet.
- `export <file.kdraft> -o out.pdf [--format pdf|svg] [--kicad-cli PATH]
  [--no-kicad-cli] [--sheet-from-kicad-cli]`: loads document and board,
  resolves sheets per page, with kicad-cli exports Edge.Cuts and F.Cu
  (page size mode 2) through `FileBoardSource` and its cache
  (`<doc dir>/.kdraft-cache`), fits the transform, and uses plotted
  layers for views if the fit passes (else native, with a note).
  Without kicad-cli (absent or `--no-kicad-cli`) everything is native.
  `--sheet-from-kicad-cli` draws sheets with KiCad from stub boards.
  `${KICAD_VERSION}` becomes `KiCad <version>` or `kdraft <version>`.
- `spike s1` as above.
- M2: `doc new` and `export` run through `kdraft-engine` (`document.new`,
  `pipeline::export`); layer exports use page-size mode 0 and per-view
  drill marks. `export --format png --dpi N` (10..1200) writes one PNG per
  page with the raster back end. `cmd list [--json]` prints the command
  table; `cmd run <file> --do NAME [--args JSON] ... [-o out]` runs
  commands headless (each `--args` belongs to the preceding `--do`; saves
  to `-o`, else in place if anything changed; one JSON line per command
  with outcome and selection).

### kdraft-engine (new, M2)

No GUI dependency. Modules:

- `session`: `Session` holds the document (`Arc`, copy on write), the
  board (`Arc<Board>`, load errors kept, not fatal), project, resolved
  drawing sheets (cached per choice, default on load failure with a
  message), selection (page plus ordered item ids; first = reference),
  paths, revision and dirty flag (saved revision; undo back to it is
  clean), undo/redo log of document snapshots with the selection before
  and after (limit 500), and gestures (`begin_gesture`, previews via
  `Registry::preview` that each restart from the gesture's base,
  `commit_gesture` = one undo step, `cancel_gesture`). `new_for_board`,
  `open`, `save` / `prepare_save` + `mark_saved` (for writing on another
  thread; board reference re-relativised for a new folder; Windows `\\?\`
  prefixes stripped), `reload_board` / `replace_board` returning the
  `BoardDiff`, `drawing_area`, `item_bounds`.
- `registry`: named commands with serde-deserialised typed arguments
  (`deny_unknown_fields`), title, description, kind (edit / selection /
  file) and a hand-written argument description (`ArgSpec`: name, type,
  required, description; JSON via `cmd list --json`). A test checks every
  spec against its argument type (all args, required only, each required
  arg missing, unknown arg). Edit commands become one undo step when the
  document changed; on error the session is restored; selection commands
  may not change the document. 44 commands:
  `document.new/open/save/set_snap_grid/set_sheet`;
  `page.add/remove/duplicate/move/rename/set_sheet/set_paper/select`;
  `item.add_view/add_table/add_text/add_line/add_rect/remove/duplicate/
  move/set_position/set_scale/set_rotation/set_side/set_layers/set_kind/
  set_drill_marks/set_frame/set_geometry/move_to_page/copy_to_page/raise/
  lower`; `select.set/add/toggle/clear/all`;
  `layout.align/distribute/match/center/nudge`. Align: reference is the
  first selected item, or the page's drawing area when one item is
  selected. Distribute: equal gaps or equal centres, outer items fixed.
  Match: width/height (views by scale, rectangles, lines, circles) or view
  scale. Centre on the drawing area or page, per axis. Nudge in grid steps.
- `bounds`: one function for item boxes in sheet mm: views = outline box
  (or crop) through scale and rotation (option: full frame), tables from
  the renderer's table layout (anchor point for BOM/custom or without a
  board), graphics from geometry (text measured with the bundled font),
  note lists from their lines, callouts as a point.
- `snap`: pure snapping. Per axis the nearest object target within the
  tolerance wins (edges and centres of other items, page and drawing-area
  edges and centres, centred between neighbours, continuing a
  neighbour pair's gap), else the moving box's top-left snaps to the grid
  if within the tolerance. Returns the correction and guide segments.
- `pipeline` (moved from the CLI's export): layer requests (mode 0,
  `--drill-shape-opt` from the view), `fetch_plots`, `needed_layers`,
  stub-board sheets, sheet context, `RenderInputs` (owned `Arc` inputs
  that can go to a worker; render all pages, one page, one sheet, one
  item), `export` and `export_inputs` (PDF, SVG, PNG).
- Tests: command spec consistency; align with reference and with the
  drawing area plus undo of selection; reorder then undo restores the
  exact document; gestures (ten previews, one step; cancel); errors leave
  the session unchanged; add/distribute (gaps and centres)/match/centre/
  nudge/view properties/copy to page/z-order; save/open round trip with a
  relative board path and the dirty flag; proptests: snapping never
  exceeds the tolerance, grid-only snapping lands on the grid, and a
  random sequence of up to 25 commands (pages, items, selection, move,
  align, distribute, remove, duplicate, z-order, nudge, scale, rename,
  interleaved undo/redo) followed by full undo returns the original
  document and full redo the history tip. Align idempotence and
  distribute keeping outer items fixed are proptests in `kdraft-doc`.

### kdraft-render additions (M2)

- `raster` on tiny-skia 0.12: display list or a sheet region to an RGBA
  pixmap at a given px/mm; non-zero and even-odd fills, strokes with caps,
  joins and dashes, group transforms, clip rectangles as masks (nested and
  rotated clips work), text as glyph outlines from the bundled font
  (ttf-parser, glyph path cache), invisible text skipped. Strokes thinner
  than one device pixel are drawn one pixel wide (as PDF viewers do).
  Limit 100 Mpx per image. `page_png`, `encode_png_rgba`.
- `page::render_sheet` and `page::render_item` (the page render is now
  sheet plus items), `dl::items_bbox` (paths and measured text).
- Plotted views clip to frame united with plotted content plus 1 mm.
- Tests: fill rules, clip, dash, text placement, bad targets; golden PNGs
  of the three fixture pages at 3 px/mm with a tolerance (mean channel
  difference < 0.5, under 0.2 % of channels off by more than 48).

### kdraft-ui (new, M2; binary `kdraft-gui`)

eframe/egui 0.37, renderer `glow` (OpenGL) by default, `wgpu` as a cargo
feature. Plain modules with unit tests: `camera` (sheet mm to screen,
zoom at cursor, fit, half-octave zoom buckets, texture resolution capped
at 4096 px per side), `hit` (top-most hit with nested small items winning,
marquee, handles), `drag` (state machine: click select, shift toggle,
drag-move, corner-handle view scaling, marquee, cancel), `movectx`
(snapping during a drag), `pages` (drop position to `page.move`), `worker`
(threads with latest-wins slots and a repaint waker), `watch` (polling
size and mtime, debounced), `jobs` (raster jobs for thumbnails, sheets and
items with content keys independent of position; file IO; kicad-cli
discovery and plots), `summary` (board change banner text). `ui` is the
egui layer; `app` routes every button and key to engine commands. See
`docs/GUI.md` for layout, shortcuts and limits.

Headless tests (`crates/kdraft-ui/tests/gui.rs`, egui_kittest without a
renderer): toolbar "Align right" on a selected view plus toolbar Undo;
a canvas drag of a view (moves, stays within the snap tolerance of the
raw delta, one undo step); page Down/Add buttons plus Ctrl+Z twice; an
error message instead of a panic for Align with nothing selected; board
file changed then "Reload board" shows "1 footprint moved". The egui text
in these frames uses only glyphs present in IBM Plex Sans (the test
harness panics on missing glyphs).

## M3 increment (this session)

### New and changed, per crate

- **kdraft-sexpr**: `edit` module (byte-range replace/insert with inverse
  edits; proptest: an edit followed by its inverse returns the original
  bytes), `Node::span()`.
- **kdraft-board**: footprint graphics (`FpGraphic`: `fp_line/rect/
  circle/arc/poly/curve` on Fab, CrtYd/Courtyard and SilkS/Silkscreen of
  both sides, board and footprint-local shapes, stroke width, fill) and
  texts (`FpText`: `property` fields with a position and `fp_text`; local
  and board position, absolute angle, size, hidden, keep-upright from
  `unlocked`, mirror, justify). Text semantics taken from the KiCad
  10.0.8 parser source (`pcb_io_kicad_sexpr_parser.cpp`): the position is
  footprint-local, the angle is absolute, `unlocked` means "not keep
  upright", `hide` may be bare, `(hide yes)` on the text or inside
  `effects`. `Footprint::body()` (fab box, else courtyard, else pads) and
  `has_copper_pads()`. `edit::hide_fab_texts` (see below). `bom` module
  (CSV parser, PCB fallback, reference ranges and wrapping, stable item
  numbers). `drill_row_index()`.
- **kdraft-layout (new crate)**: uniform-grid spatial index for boxes and
  segments; designator declutter (`declutter`); dimension geometry
  (`dimension`). Pure, no fonts.
- **kdraft-source**: private copies of project files for every kicad-cli
  job (`private_copy`, `private_copy_edited`: board plus `.kicad_pro`, or a
  schematic with its sheet hierarchy, copied to `<cache>/inputs/<hash>/`;
  the hash is the cache input key). `FileBoardSource`, spike S1 and the BOM
  export all run on such copies. `SvgExport`: `--sketch-pads-on-fab-layers`,
  `--hdnp/--sdnp/--cdnp`, `--define-var`. `BomExport::kdraft`, `run_bom`,
  `schematic_for`. `layer_svg_fab` (plot from a copy with fab texts hidden).
- **kdraft-plot**: drill report parser reads plated/unplated sections, tool
  names and `(with N slots)`; S1 probes `sch export bom --help`; S1 compares
  plating and slot counts too.
- **kdraft-doc**: views: `(render plot|native)` (default native for
  assembly and drill views, plot otherwise) and `(assembly ...)` options
  (values, references, fab_other, pads, holes, pin1, dnp
  none|crossed|greyed|hidden, refs placed|pos_files|all, text_min,
  text_max). Dimensions: kinds horizontal/vertical/aligned/linear/radius/
  diameter/ordinate, axis, angle, tolerance, `(last ...)` positions;
  anchor `(outline-edge N)`; anchor parse/print. Dimension style in
  `(styles (dimension ...))` (written only where not default). Source
  `board_hash`. Note lists with title and wrap width, stable note ids.
  Tables: `(rows start count)` chunks and `(items ...)` BOM numbers.
  `doc new` builds the default set (see below).
- **kdraft-render**: `assembly` (native assembly views), `dims`
  (dimensions, broken style), `notes` (note lists with variables,
  callouts), `symbols` (drill symbols), BOM table, multi-line table cells.
  `render_item` takes the sheet context (variables, schematic BOM).
- **kdraft-engine**: `check` (drawing checks), `enable` (which commands can
  run on the selection), BOM fetch in the export pipeline, plot layer keys
  include the fab-text switches. 21 new commands: `item.set_render`,
  `item.set_assembly`, `dimension.add_linear/add_radius/add_ordinate/
  set_offset/set_tolerance/set_style/board_size`, `document.check`,
  `item.add_notes`, `notes.add/edit/remove/move`, `item.add_callout`.
  Saving stores the board hash and the current dimension anchor positions.
- **kdraft-cli**: `kdraft check <file.kdraft> [--json]` (exit 0 clean, 1
  warnings, 2 errors).
- **kdraft-ui**: page entry click/drag fix, cursor readout repaint,
  explicit canvas cursors, greyed-out toolbar actions (engine rule),
  alignment reference outline, scale snapping (Alt: free), red outline for
  items outside the drawing area, new view settings in the properties
  panel, texture keys for dimensions/callouts. No new tools.

### Native assembly views and designator declutter

Per footprint on the view's side: fab graphics (fallback courtyard
graphics, then the pad box, the latter only for parts that get a
designator), pad outlines (default on, light grey), holes (default off),
pin-1 dot for polarised parts (three or more pads, D/LED/CR/ZD/BT
prefixes, or library ids with LED/diode/polarized/tantal/CP_), DNP style
(default crossed). Designators for parts that are not `board_only` and have
copper pads (`refs placed`; `pos_files` also drops parts excluded from
position files; `all`).

Placement (`kdraft_layout::declutter`), all in paper mm: inside the body at
the largest height in [text_min, text_max] (defaults 0.9 and 2.5 mm) that
fits in 90 % of the body, rotated 90 degrees when that is more than 15 %
larger; shrunk (not below the minimum) to clear pin-1 dots and other
labels. Otherwise outside at the minimum height: rings of 10 candidate
positions around the body (above, right, below, left, corners) out to
1.6 x min height, then 16 directions with a leader out to 12 x min height,
each candidate free of labels, bodies, pin-1 marks and the outline
(segments) and inside the frame grown by 6 x min height; then the same
allowing other bodies (flagged "over other parts"); else unplaced
(reported by render warnings and `kdraft check` as an error). Labels never
overlap (proptest on random layouts); text is horizontal or reads bottom
to top, never mirrored; deterministic order (inside: larger bodies
first; outside: smaller bodies first; ties by input order).

`doc new` assembly scale: the standard scale that fits, unless fewer than
half of the designators fit inside their part at the minimum height; then
the largest of 1.5, 2.5, 3 (and the standard scales) that still fits the
drawing area. Never below the minimum text height.

Declutter metrics (`kdraft check`, `doc new` defaults, A3, 10.0.8 demos):

| Board | view, scale | designators | inside | outside | leader | over other parts | unplaced |
| --- | --- | --- | --- | --- | --- | --- | --- |
| pic_programmer | top 2:1 | 56 | 56 | 0 | 0 | 0 | 0 |
| video | top 1:1 | 86 | 82 | 4 | 0 | 0 | 0 |
| video | bottom 1:1 | 103 | 101 | 2 | 0 | 0 | 0 |
| CM5_MINIMA_3 | top 3:1 (dense rule) | 69 | 38 | 31 | 0 | 0 | 0 |
| CM5_MINIMA_3 | bottom 3:1 | 15 | 4 | 11 | 0 | 0 | 0 |
| tinytapeout-demo | top 2.5:1 (dense rule) | 147 | 70 | 73 | 4 | 3 | 0 |
| jetson-agx-thor | top 2:1 | 600 | 107 | 484 | 9 | 0 | 0 |
| jetson-agx-thor | bottom 2:1 | 520 | 62 | 450 | 8 | 0 | 0 |

### Fab-layer texts in plotted views

kicad-cli 10.0.8 has no option to hide reference or value texts on fab
layers (confirmed from its real `--help`). kdraft hides them in its private
copy of the board: `(hide yes)` inserted after the text's `(layer ...)`
(the form KiCad 9 and 10 format files use for `property` and `fp_text`,
checked on the demo boards: `20241229` and `20260206`), or `(hide no)`
flipped; every other byte unchanged. "Values" = the `Value` property /
`fp_text value` on F.Fab/B.Fab; "references" = `Reference` / `fp_text
reference` and `fp_text user "${REFERENCE}"` there; "other" = the remaining
fab-layer texts. The plot cache key is the edited copy's hash. Tested on
the fixture, a hand-written mix of hide forms and all KiCad 10 demo boards
(1046 value texts hidden; the board model reads them as hidden; editing
twice changes nothing). **UNVERIFIED: that kicad-cli then leaves the hidden
texts out of the plot** (expected; how to check: `docs/S1.md`).

### Dimensions, notes, callouts, BOM, checks

- Dimensions resolve their anchors through the view transform; the value
  is the board distance (sheet distance / scale; proptest: independent of
  the view scale, text never overlaps the dimension line). Radius and
  diameter on holes (tool diameter) and outline arcs/circles; ordinate
  values are board mm relative to the datum with Y up. Broken anchors:
  drawn at the stored last position, red, dashed, value with `?`; check
  reports `broken-anchor`. Diameter text uses `Ø` (U+00D8).
- Notes resolve `${BOARD_THICKNESS}` (stackup total, else general
  thickness), `${LAYER_COUNT}`, `${COPPER_FINISH}` ("not specified" when
  absent or "None"), `${BOARD_WIDTH}`, `${BOARD_HEIGHT}` (outline box),
  `${MIN_DRILL}` (smallest tool diameter), `${HOLE_COUNT}`, plus title block
  and project variables; unknown ones stay and are reported
  (`unknown-variable`). Callout targets `note:ID` (own page first) or
  `bom:KEY`.
- BOM: `sch export bom` on a private copy of `<board stem>.kicad_sch` and
  its sheets, with `--fields Reference,Value,Footprint,${QUANTITY},${DNP}
  --labels Refs,Value,Footprint,Qty,DNP --group-by Value,Footprint
  --string-delimiter '"' --ref-range-delimiter ""`. **UNVERIFIED**: these
  flags come from the KiCad 10 CLI reference (docs.kicad.org, read
  2026-11-24), not from a real help text or run; spike S1 now records
  `sch export bom --help`. The CSV parser is tolerant (quotes, `,`/`;`/tab,
  loose column names, ranges). Without kicad-cli or schematic: BOM from the
  PCB footprints (value + footprint id groups, exclude-from-BOM and
  board-only left out, DNP apart), titled "BILL OF MATERIALS (FROM PCB)".
  Item numbers are stored by group key (`value|footprint[|DNP]`) in the
  first BOM table; new groups get the next numbers. `doc new` splits the
  PCB-derived BOM into chunks that fit the drawing area (side by side when
  the estimated width allows, then continuation pages). If the exported
  BOM is longer, `check` reports the lines not shown (`bom-overflow`).
- `kdraft check`: `broken-anchor`, `stale` (board hash), `outside-area`,
  `overlap`, `table-overflow`, `small-text` (< 0.8 mm), `unplaced-designator`,
  `unknown-variable`, `bom-overflow`, `bom-callout`, info `designators`.

### doc new default set

Fabrication (plot when kicad-cli is available, else native with the note;
board-size dimensions above and right; five neutral fab notes left of the
title block, or in a band below the view when the sheet has no room
there), Assembly top (native), Assembly bottom (native, mirrored; only with
placeable parts on the back), Drill and stackup (drill symbols; a layer
stack too tall for the column gets its own "Layer stack" page), Bill of
materials page(s). Golden files were updated deliberately in each commit
that changed the default set or rendering.

### Looked at (PNG, raster back end)

Samples in `/home/claude/kdraft-samples/m3/`: pic_programmer, video,
CM5_MINIMA_3, tinytapeout-demo and jetson-agx-thor (all 10.0.8 demo
files), each as `.kdraft`, PDF, 100 dpi PNG per page and the `kdraft check`
output; looked at: every page type of each, plus 300 dpi crops of dense
areas (CM5, tinytapeout, jetson assembly; pic_programmer fabrication
dimensions).

## UI pass (2026-11-25)

Built and verified headless only (software-rendered screenshots, looked
at; kittests); not run on Windows. Commits 7c83831..HEAD.

- **Headless screenshots** (`kdraft-ui::shot`): egui runs without a window;
  its tessellated meshes are rasterised in software (barycentric
  triangles, top-left fill rule, bilinear texture sampling, clip rects,
  premultiplied blending in gamma space). `egui_kittest`'s snapshot
  feature needs wgpu and a GPU, so it was not used. Dev command
  `kdraft-gui --screenshot ... [--script]`, `--gallery`, `--bench`,
  `--shortcuts-md`. Screenshots: `kdraft-samples/ui/before`, `/after`.
- **Theme** (`theme`): token struct for light and dark (contrast targets
  tested), follows the OS until toggled; settings file (`settings`) with
  theme, recent files, grid, rulers, snap; IBM Plex Sans with a bold
  family and named text styles.
- **Icons** (`icons`): 78 icons drawn in code on a 16-unit grid with pixel
  hinting; gallery screenshot.
- **Widget kit** (`widgets`): number field (lead, unit, scrub, nudge,
  expressions, mixed), segmented control, dropdown, icon button with
  tooltip and shortcut, section header, disclosure, text field with
  elision, inline rename, insertion line, drag ghost, chips, pills.
- **Strings** (`strings`): UI text in one module (English).
- **Command metadata** (engine `meta`): category, icon id, default
  shortcut, menu flag and named variants for every command (data only);
  the UI catalogue (`actions`) adds the editor's own actions and builds
  menus, context menus, tooltips, the palette (fuzzy search), shortcut
  dispatch and the shortcut list.
- **Layout**: top strip with menus, left pages + item tree (`tree`),
  canvas with rulers/grid/labels/selection pill/hover/Alt measurements
  (`measure`, `rulers`) and floating tool bar, inspector sections, dock
  with issues and log, toasts, start screen. See `docs/GUI.md`.
- **Engine**: `item.rename`, `item.set_hidden`, `item.set_locked`,
  `item.reorder`, `bom.renumber`; items store `(name ...)`, `(hidden yes)`,
  `(locked yes)`; hidden items are not rendered; `Session::fork` for
  background checks; a new revision per gesture preview.
- **Tools** (`tools`, `anchors`): placement state machines with unit
  tests; anchor index per view (pads, holes, outline vertices, arc
  centres, footprint origins) in a grid; view, table, notes, text, line,
  rectangle, linear (H/V/aligned), radius/diameter, ordinate, callout.
- **Files** (`dialogs`): rfd dialogs on a thread; recent files.
- **Background**: drawing checks 0.6 s after a change on a forked
  session; schematic BOM fetched through kicad-cli when the document has
  BOM tables or callouts; textures limited to the renderer's maximum
  texture side.
- **Frame times** (release build, headless, egui run plus tessellation,
  1400 x 900, pointer sweeping over every page; GPU upload and background
  rasterising excluded): tinytapeout-demo (309 footprints, 4 layers):
  mean 0.83 ms, p95 1.83 ms, max 4.5 ms; dragging a view: mean 1.20 ms,
  p95 1.62 ms. jetson-agx-thor (1125 footprints, 10 layers): mean
  1.30 ms, p95 3.40 ms, max 4.9 ms; dragging a view: mean 2.33 ms, p95
  3.47 ms.
- **Not done**: no drag-to-place rubber band for views/tables (click
  places); no "Drill only" layer preset (it would need layers and drill
  marks in one undo step); no document tabs or window controls (single
  document, native title bar); German strings not yet added.

## Styles, editing, links, layer colours, 3D (2026-11-25)

Built and verified headless only (kittests, software screenshots and
exports, looked at); not run on Windows; no real kicad-cli. Commits
f8eba1b, 4eafa3d, 2ebc57e, 338600d, 41f57ce, 893b772 and the docs commit.
Screenshots: `kdraft-samples/ui2/` (both themes); exports of a page with
all of it: `kdraft-samples/m4/` (PDF, PNG and SVG pages, made with the
fixture runner, so plots and the 3D picture are recorded/stand-in data).

- **Styles** (kdraft-doc `style`, render `graphic`, engine
  `commands_style`, UI `ui_style`): per-item `(appearance ...)` overrides
  on document category defaults (graphic, text, dimension, callout, notes,
  table); colour, width, pattern, fill, opacity, arrows, text colour,
  height, bold, italic, alignment, line spacing, dimension decimals, table
  header fill. Display lists carry alpha (PDF, SVG, PNG, canvas). Copy and
  paste style, reset, Document styles window, colour picker with palette,
  recent colours (settings), hex and HSV/alpha sliders.
- **Multi-line text** (kdraft-layout `text`): newline-aware greedy wrap
  (property-tested: no line wider than the wrap unless one word, no word
  lost), block alignment, rotation, `${...}` variables; in-place editor on
  the canvas. Bundled font: all of µ ° ± Ø × ÷ ≤ ≥ ≈ Ω € ² ³ ½ and German
  umlauts present; **∅ (U+2205) missing** (use Ø).
- **Geometry editing** (UI `handles`, `ui_handles`, engine
  `commands_geom`): handles for line/polyline/polygon vertices, arc
  points, rectangle corners and edges, circle radius, text rotation and
  wrap, dimension anchors (drop on another anchor) and text along the
  line, callout anchors; polyline and circle tools; vertex insert (double
  click) and delete; rectangle corner radius; hole wins a tie with a pad
  while the pointer is inside the hole.
- **Placement links** (kdraft-doc `link`, engine `links`,
  `commands_link`, UI `ui_link`): see `docs/GUI.md` "Placement links" and
  the model in `docs/PLAN.md`; `view.pages_per_layer`, `doc new
  --layer-pages`.
- **Layer colours** (kdraft-doc `layers`): presets black / layer / grey,
  per-layer colour and opacity, draw order = list order, drag reordering.
  Decision from the real kicad-cli 10.0.8 plots (now fixtures in
  `fixtures/kicad-cli-svg`): copper layers are black with white drill
  knock-outs (filled circles in pads, a stroked white slot); edge cuts,
  fab and courtyard have none. Black ink takes the layer colour; the
  knock-outs are collected from all layers of a coloured view and drawn
  once on top, as a combined plot does (so a hole shows through every
  layer, like the board). **Bug fixed on the way**: the SVG reader treated
  white shapes as page background and dropped the knock-outs, so plotted
  pads never showed their holes; now only page-sized blank shapes are
  background. Native views colour pads, outline, bodies, designators,
  values and DNP marks from the same layer colours.
- **3D views** (`render3d`): see `docs/GUI.md`; image primitive in all
  back ends; background job and placeholder.
- **Fixture runner** (kdraft-source `fixture`): answers `version`, `pcb
  export svg` and `pcb render` from recordings; `kdraft-gui --fake-kicad
  DIR` and hidden `kdraft export --fake-kicad DIR` use it.
- **Leftovers**: view name label above the view's dimensions; pages list
  whole rows and title tooltips.
- **Not done**: ellipses; table column widths by dragging; in-place
  editing of note lists on the canvas (the inspector edits notes); layer
  legend item; drag-to-place frames for views and tables; 3D view template
  option in `doc new`; a spinning indicator (the 3D placeholder says
  "Rendering 3D view..."); italic text on the canvas (egui has no slant;
  exports are italic); rotated texts are edited unrotated; 3D pictures
  are not rotated with the view's rotation; the native layer view draws
  the same content for every layer (only plots differ per layer).

## Verified on Windows by a person (user, 2026-11-25, later build)

- The GUI's background schematic BOM fetch works with real kicad-cli
  10.0.8: log "BOM from the schematic: 62 lines", inspector "BOM: from the
  schematic".
- Reported then and fixed in the following commit: "Layer pages" refused
  without a selected view ("why do I need a fabrication view to call add
  layer pages?"), and the log showed errors "render thumb:<page>: page
  gone" / "render sheet:<page>: page gone" after deleting a page.

## Uniform layer pages (2026-11-25)

Reported on Windows: Edge.Cuts looked different on the F.Cu and In1.Cu
layer pages (the reference carried KiCad Default's raw colours as
overrides from before the palette fix, the other pages followed the
paper-adapted document palette). Now layer pages follow their reference's
colours (new link follow property `colours`), differences are shown with
Match buttons and a check finding, raw KiCad screen colours on a view get
a Reset hint, and the built-in KiCad palettes have paper variants.
Documents are not rewritten automatically.

## Settings, palettes (2026-11-25)

User request: settings stored (layout, kicad install path, ...), a
settings page, KiCad colour import, default layer colours on insertion.
Built: one settings file shared by the editor and the CLI (sections
kicad, documents, palettes, ui), kicad-cli precedence flag > env >
setting > newest install with all installs listed, extra `--define-var`
variables, a shared cache folder option with size and Clear, document
defaults used by `doc new` and the editor, window/panel/dock/tool layout
and per-document page and zoom remembered, a settings page (General,
KiCad, Documents, Layer colours, Cache), layer palettes with KiCad's
built-in themes and KiCad theme import (adapt for white paper), document
palettes stored resolved in `.kdraft` and applied on every insertion
path. CLI: `kdraft settings ...`, `kdraft kicad list`.

Verified headless only (unit, property and kittests; screenshots of every
settings section in both themes looked at). UNVERIFIED: KiCad theme files
written by KiCad itself (none were available; format taken from KiCad
10.0.8's source, tests use hand-written files), monitor checks of the
saved window position on a real multi-monitor Windows desktop, the
native file dialogs of the settings page. Not done: cache size limit with
eviction, UI language selection.

## Numbered callouts (2026-11-25)

Reported by the user on Windows (build 8d56f0b): "callout number cannot
be increased. should increase automatically on placement if the page
already has one or modified later by hand." Callouts only showed a
derived number (note or BOM item). Now a callout shows a label (default,
auto-numbered on placement per page or per document), a note number, a
BOM item number or the designator; labels are edited in the inspector,
in place on the canvas, with +/−, by "Duplicate with next number" and by
`callout.renumber`; balloon shapes circle/square/hexagon/none; optional
strict gap check. Commands: `item.add_callout` (label/kind), 
`callout.set_label`, `callout.bump`, `callout.set_kind`,
`callout.renumber`, `callout.set_shape`, `callout.duplicate_next`,
`document.set_callout_numbering`. Verified headless (engine property
tests, kittests, screenshots and PNG/PDF/SVG exports looked at); not yet
seen on Windows.

## Missing input: ask or default (2026-11-25)

Commands that miss an input now ask for it or take a default:

- Layer pages: works from any page; reference = selected view, the only
  view on the page, or a new placement (centred, fitted scale); pages
  after the current page or at the end; an empty untitled current page
  can be the first layer page; one-line summary before Create.
- Link placement to… without a selection: pick the item, then the
  reference.
- Dimension and callout tools on a page without a board view: place a
  fabrication view first, then the tool continues. Dimension commands
  ignore 3D views when choosing the default view and say what to do when
  there is none.
- `dimension.board_size` without a view on the page: adds a fabrication
  view first.
- `item.add_callout` without `target`: the part's BOM line, else the first
  note on the page, else a new note list with a note for the part.
- `notes.add` without a note list: the selected or first list on the page,
  else a new one; `item.add_notes` without `at`: top left of the drawing
  area.
- Text, line, rectangle, polyline and circle tools are enabled without a
  board.
- New pages are titled after their position ("Page 2"), not the page
  count.
- Kept as refusals (nothing sensible to default to): distribute with fewer
  than three items, match with fewer than two, align and arrange commands
  without a selection, removing the last page, style copy/paste without a
  selection, views/tables/dimensions without a board, renumbering a BOM
  without a BOM table (the message says to place one).
- Background renders of a page or item deleted while waiting are dropped
  quietly (waiting jobs are cancelled when the document changes); 3D
  renders use one worker slot per picture.

## Verified on Windows by a person (user, 2026-11-25, build f4d25ba)

- The new editor (styles, editing, links, layer colours, 3D) builds and
  runs (glow renderer); the UI-pass layout renders as in the headless
  screenshots.
- A real `kicad-cli pcb render` (10.0.8) works through kdraft: a 3D view
  was rendered in about 14 s ("2728 x 1524 px ... (cached)"), board with
  component 3D models, top, orthographic, transparent. Library 3D models
  resolve from the private copy of the board. Still open: models given
  as `${KIPRJMOD}/...`.
- Reported then and fixed in the following commit: the 3D picture had a
  wide empty border around the board (now trimmed, outline placed at the
  view's scale for top/bottom, two render passes), and a fresh 3D view
  showed "1 issue": the check rendered the page as if kicad-cli were
  missing ("3D view needs kicad-cli"); the GUI and `kdraft check` now
  tell the check that kicad-cli is there.

## First real M3 run on Windows (user, 2026-11-25)

HEAD 3c8816e built and ran on the user's PC (kicad-cli 10.0.8, user
board A); a 6-page PDF from `doc new` + `export` was inspected:

- Verified: Fabrication page (plotted view, board-size dimensions 225.00
  and 123.00, notes list), Assembly top and bottom (native, 1.5:1,
  readable), Drill and stackup, two BOM pages. The BOM came from the
  schematic through real kicad-cli (title "BILL OF MATERIALS", not the
  PCB fallback): `sch export bom` with the flags kdraft emits is
  **exercised on 10.0.8** (its `--help` text is still not recorded).
- Bug (fixed, ca28bd4): dimensions could not be selected in the editor
  (no item box). Now selectable by their lines and text, draggable
  (offset), deletable, with inspector; every M3 item kind audited.
- Bug (fixed, f802916): BOM item numbers out of sequence (1..22, 62 for
  J12, 23..): numbers stored from the PCB fallback at `doc new` were
  mixed with the schematic BOM at export. Numbers now carry their source;
  `doc new` fetches the schematic BOM like export; `bom.renumber`; DNP
  in its own column instead of a "(DNP)" value suffix.
- Bug (fixed, 8c23f89): a 2x7 1.27 mm header (J12) drew as a dark
  blob at 1.5:1 with its designator against J11's. Pads closer than
  0.5 mm on paper are drawn as one pad-field outline and the body counts
  as occupied (designator outside). Fixture `fixtures/dense-header.kicad_pcb`.
- Bug (fixed, 2368774): dragging a view left its dimensions behind, then
  they jumped. Dimensions, callouts and free graphics are now drawn live
  as vectors; previews get their own revision; callouts follow their
  view; deleting, duplicating or moving a view takes its dimensions and
  callouts along.

## Spike S1 results

**Verified on kicad-cli 10.0.8 (Windows), small sample: board
`fixtures/kicad9-minimal.kicad_pcb` (5 F.Cu pads, 4 B.Cu pads), board
outline at the origin.** Not generalised beyond this; KiCad 9.0.x has not
been run.

- All 5 layers (Edge.Cuts, F.Cu, B.Cu, F.Fab, F.Courtyard) exported in
  page-size modes 0, 1 and 2; no flag rejected; `svg`, `drill`, `pos`,
  `stats` `--help` probes all fine.
- Fitted transform in all three modes: sx = sy = 1.000000, ox = oy =
  0.0000, Y down, not mirrored; pad residuals 0.0000 mm. SVG user units
  are mm in board/page coordinates for modes 0 and 1. For mode 2 this
  fixture cannot tell (outline starts at 0,0). The mode-2 SVG had
  `width="49.9872mm" height="29.9974mm" viewBox="0.0000 0.0000 49.9872
  29.9974"` for a 50 x 30 mm outline: the page is the centre-line bbox,
  half the outline stroke is cut off at the page edge. Hence layer exports
  now use **page-size mode 0** (full page, nothing clipped); the transform
  is still fitted per export.
- Mirrored export (F.Cu, mode 0): sx = -1, ox = 297.0022 on an A4 page,
  same Y mapping: KiCad mirrors about the page width, not about the board.
- pos CSV vs board: x' = x, y' = -y (4 refs, 0 residual).
- Drill report: 3 tool lines, 5 holes; kdraft: 5 holes in 4 drill rows.
  Explanation from the fixture (raw report not available, so this is an
  inference): the board has PTH 0.3 mm (2 vias), PTH 1.0 mm round (1 pad),
  PTH 1.0 x 1.5 mm oval slot (1 pad) and NPTH 3.2 mm (1 pad). KiCad assigns
  drill tools by diameter, a slot taking its smaller dimension, so the
  slot shares the 1.0 mm tool: PTH tools 0.3 and 1.0 (2 holes, 1 of them
  a slot) plus NPTH 3.2 = 3 tool lines. kdraft keeps slots as separate
  rows (and separates vias from pads), giving 4 rows. S1's report parser
  counts holes per line and slots separately, which fits this reading;
  whether KiCad also splits tools by via/pad attribute is not decided by
  this board (no via and pad share a diameter). S1 now compares the report
  against `kicad_tool_groups()` (tool diameter = smaller oval dimension,
  plated and non-plated apart) on (diameter, hole count) pairs; kdraft's
  drawing table keeps its finer rows on purpose.
- `stats --format json` top-level keys: `board`, `components`,
  `drill_holes`, `metadata`, `pads`, `vias`.
- Stub-board sheet export (A3): SVG 419.989 x 297.002 mm, content margins
  l/t/r/b 10.000/10.000/10.011/9.998 mm, 41 `<text>` elements, title,
  rev, company and comment1 markers present, no `${` left. The project
  variable marker was not found in either sheet SVG: the stub defined the
  variable in `.kicad_pro` but no printed title-block field referenced
  it (the default sheet prints only TITLE, REVISION, COMPANY,
  COMMENT1..4 and built-ins). Fixed: the stub now puts
  `${KDRAFT_S1_VAR}` into COMMENT2; whether KiCad expands a project
  variable inside a title-block comment is for the next run.

### Second run: user board A (dense, real)

**Verified on kicad-cli 10.0.8 with one dense real board**, "user board A"
(4 layers, file version 20241229 / generator 9.0, outline 225 x 123 mm at
(45, 35)..(270, 158), 350 footprints, 1030 pads, 752 holes). KiCad 9.0.x
still not run.

- Modes 0 and 1: sx = sy = 1.000000, ox = oy = 0.0000, Y down; F.Cu 995
  pads and B.Cu 260 pads all within 0.05 mm, max and RMS residual
  0.0000 mm.
- Mode 2: sx = sy = 1, ox = -45.0000, oy = -35.0000: the page origin is
  the outline box origin (centre-line box), i.e. mode 2 is board-relative.
  Mode 0 stays the default.
- Mirrored F.Cu, mode 0: sx = -1, ox = 297.0022 (A4 page width), 100 % of
  pads within 0.05 mm.
- No flag rejected. Drill report: 11 tool lines, 752 holes; kdraft: 752
  holes in 11 rows. Pos: 304 rows = the 304 of 350 footprints without
  `exclude_from_pos_files`, x' = x, y' = -y, residual 0. Stats JSON
  parsed (32 numeric fields).
- Stub sheet: as in the first run (419.989 x 297.002 mm, margins
  10/10/10.011/9.998, variables resolved; project-variable marker not
  found, which the COMMENT2 fix above addresses; that run predates it).
- `doc new` + `export` gave a correct 3-page A3 PDF at 1:1 (plotted
  content placed correctly, drill page and tables fine). Defects:
  1. Drill table row "0.40 x 0.40 PTH slot x16" (fixed): an oval drill
     with equal sides is now read as a round hole (`(drill oval 0.4 0.4)`
     and `(drill oval 0.4)`), so it merges with round 0.40 rows and the
     native drill drawing draws a circle. Test
     `oval_drill_with_equal_sides_is_round_and_merges`.
  2. Stackup TOTAL row showed "finish None" (fixed): a finish of "None"
     or empty is left out.
  3. The 1:1 assembly view is hard to read: the F.Fab layer of a real
     board carries reference and value texts per footprint, which overlap
     at 1:1 on A3. No change now; see Next tasks. kdraft has no recorded
     real kicad-cli `--help` text in its tests, so no flag for hiding
     fab-layer value or reference texts is named here.
  4. Empty title-block fields render as bare labels ("Title:"), as KiCad
     does; no change.

### First real run of the plot path (`doc new` + `export`)

kicad-cli 10.0.8 on Windows, one tiny board
(`fixtures/kicad9-minimal.kicad_pcb`); PDF rasterised and inspected by the
main session. KiCad 9 not run.

- Works: kicad-cli layer SVGs imported and placed correctly at 5:1 on
  pages 1 and 2 (outline with rounded corner, Edge.Cuts circle, KiCad
  stroke-font fab texts incl. "LM358" at 45 degrees, connector fab
  outline); no NATIVE PREVIEW note on those views; drawing sheet and
  title block correct (`KiCad 10.0.8`, title, rev, date, sheet name,
  `Id: 2/3`); page 3 native drill drawing and both tables fine.
- Defect 1 (fixed): plotted views were clipped to the outline box plus
  3 mm, cutting off the "M" of an F.Fab text left of the outline. Plotted
  views now clip to the frame united with the plotted content box plus
  1 mm (`PLOT_CLIP_MARGIN`); align/distribute bounds stay the outline box.
  Test `plotted_view_clip_includes_content_past_the_outline`.
- Defect 2 (fixed): pads and holes were plotted as filled actual-size
  drill marks on fabrication and assembly views. Drill marks are now a
  per-view setting `(drill_marks none|small|actual)` in `.kdraft`
  (default: none for assembly, actual otherwise; new documents write it
  explicitly), passed as `--drill-shape-opt`; plots are cached per layer
  and drill-mark setting. Command `item.set_drill_marks`, properties panel.

Next run: see `docs/S1.md` ("Next run").

## Verified against real KiCad files

`scripts/fetch-fixtures.sh` fetches all `demos/**/*.kicad_pcb` from
`KiCad/kicad-source-mirror` at tags **9.0.9** (20 boards) and **10.0.8**
(19 boards) with a shallow, blobless, sparse clone (about 13 s). Most demo
boards in 10.0.8 are still saved in 9.0 format; files with KiCad 10 format
`version 20260206` / generator 10.0 are complex_hierarchy,
kit-dev-coldfire-xilinx_5213, pic_programmer and video (10.0.8 tag);
cm5_minima (20250513) and StickHub (20250907) are 9.99 development formats.

On every file (test `ext_round_trip_and_load`): byte-exact round trip,
model loads without error or warning, outline closes into exactly one
contour with no open chains, and `diff(b, b)` is empty. The 85 MB
jetson-agx-thor-baseboard parses, rebuilds and loads in about 0.5 s in a
release build (11.6 M nodes).

| Board (tag/demo/file) | version | footprints | pads | vias | holes | edge prims/contours/open | stackup layers | outline mm |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 9/complex_hierarchy | 20241229 | 68 | 165 | 0 | 165 | 4/1/0 | 9 | 100.69 x 80.03 |
| 9/custom_pads_test | 20241229 | 5 | 11 | 0 | 4 | 4/1/0 | 9 | 118.00 x 90.00 |
| 9/ecc83-pp | 20241229 | 15 | 33 | 0 | 33 | 4/1/0 | 9 | 52.07 x 46.36 |
| 9/ecc83-pp_v2 | 20241229 | 15 | 34 | 0 | 34 | 4/1/0 | 9 | 48.26 x 41.91 |
| 9/flat_hierarchy | 20241229 | 64 | 247 | 7 | 254 | 5/1/0 | 9 | 160.02 x 99.06 |
| 9/interf_u | 20241229 | 25 | 379 | 84 | 401 | 9/1/0 | 9 | 115.57 x 108.20 |
| 9/kit-dev-coldfire-xilinx_5213 | 20241229 | 160 | 825 | 253 | 526 | 4/1/0 | 13 | 157.48 x 91.44 |
| 9/microwave | 20241229 | 4 | 8 | 0 | 0 | 4/1/0 | - | 292.10 x 120.65 |
| 9/multichannel_mixer-unrouted | 20241030 | 114 | 327 | 19 | 154 | 4/1/0 | - | 110.00 x 111.00 |
| 9/multichannel_mixer | 20241229 | 114 | 327 | 29 | 164 | 4/1/0 | - | 110.00 x 111.00 |
| 9/pic_programmer | 20241229 | 63 | 247 | 6 | 251 | 5/1/0 | 9 | 160.02 x 99.06 |
| 9/RoyalBlue54L-Feather | malformed upstream file, see below | | | | | | | |
| 9/RoyalBlue54L-NFC-Antenna | 20241229 | 2 | 2 | 2 | 2 | 22/1/0 | - | 23.00 x 60.55 |
| 9/sonde xilinx | 20241229 | 25 | 108 | 3 | 77 | 4/1/0 | 9 | 80.40 x 43.18 |
| 9/StickHub | 20241229 | 94 | 278 | 87 | 88 | 20/1/0 | 9 | 16.50 x 40.00 |
| 9/test_pads_inside_pads | 20241229 | 4 | 14 | 0 | 13 | 1/1/0 | 9 | 57.18 x 47.86 |
| 9/carte_test (test_xil_95108) | 20241229 | 42 | 282 | 12 | 278 | 13/1/0 | 9 | 101.60 x 99.69 |
| 9/tinytapeout-demo | 20241229 | 309 | 757 | 405 | 692 | 20/1/0 | 15 | 104.50 x 81.00 |
| 9/video | 20241229 | 189 | 2118 | 808 | 1720 | 22/1/0 | 13 | 312.04 x 106.68 |
| 9/vme-wren | 20241229 | 1508 | 6883 | 4374 | 4904 | 29/1/0 | 29 | 160.13 x 233.47 |
| 10/CM5_MINIMA_3 | 20250513 | 112 | 633 | 444 | 482 | 30/1/0 | 17 | 61.20 x 64.20 |
| 10/complex_hierarchy | 20260206 | 68 | 165 | 0 | 165 | 4/1/0 | 9 | 100.69 x 80.03 |
| 10/ecc83-pp | 20241229 | 15 | 33 | 0 | 33 | 4/1/0 | 9 | 52.07 x 46.36 |
| 10/ecc83-pp_v2 | 20241229 | 15 | 34 | 0 | 34 | 4/1/0 | 9 | 48.26 x 41.91 |
| 10/interf_u | 20241229 | 25 | 379 | 84 | 401 | 9/1/0 | 9 | 115.57 x 108.20 |
| 10/jetson-agx-thor-baseboard | 20241229 | 1125 | 4862 | 3235 | 3348 | 6/1/0 | 28 | 180.00 x 100.00 |
| 10/kit-dev-coldfire-xilinx_5213 | 20260206 | 160 | 825 | 253 | 526 | 4/1/0 | 13 | 157.48 x 91.44 |
| 10/microwave | 20241229 | 4 | 8 | 0 | 0 | 4/1/0 | - | 292.10 x 120.65 |
| 10/multichannel_mixer-unrouted | 20241030 | 114 | 327 | 19 | 154 | 4/1/0 | - | 110.00 x 111.00 |
| 10/multichannel_mixer | 20241229 | 114 | 327 | 29 | 164 | 4/1/0 | - | 110.00 x 111.00 |
| 10/One-Air-Max | 20241229 | 210 | 655 | 410 | 445 | 58/1/0 | 13 | 99.45 x 72.00 |
| 10/pic_programmer | 20260206 | 63 | 247 | 6 | 251 | 5/1/0 | 9 | 160.02 x 99.06 |
| 10/RoyalBlue54L-Feather | 20241229 | 71 | 428 | 183 | 278 | 8/1/0 | 21 | 58.42 x 22.86 |
| 10/RoyalBlue54L-NFC-Antenna | 20241229 | 2 | 2 | 2 | 2 | 22/1/0 | - | 23.00 x 60.55 |
| 10/sonde xilinx | 20241229 | 25 | 108 | 3 | 77 | 4/1/0 | 9 | 80.40 x 43.18 |
| 10/StickHub | 20250907 | 94 | 278 | 87 | 88 | 20/1/0 | 9 | 16.50 x 40.00 |
| 10/tinytapeout-demo | 20241229 | 309 | 757 | 405 | 692 | 20/1/0 | 15 | 104.50 x 81.00 |
| 10/video | 20260206 | 189 | 2118 | 808 | 1720 | 22/1/0 | 13 | 312.04 x 106.68 |
| 10/vme-wren | 20241229 | 1508 | 6883 | 4374 | 4904 | 29/1/0 | 29 | 160.13 x 233.47 |

Notes:

- **9.0.9 `RoyalBlue54L-Feather.kicad_pcb` is malformed upstream**: 349
  corrupted teardrop blocks (`(curved_edges no)filter_ratio 0.9)`) close
  lists early; the file has 349 more `)` than `(`. kdraft reports
  `unexpected ')' at 640:1 (byte 14286)`. The 10.0.8 copy is clean and
  loads. The test lists it as a known-malformed file and fails if it ever
  parses.
- jetson-agx-thor: all 3235 vias have `(drill 0.1)` in the file; the drill
  table shows that as is.
- No counts were cross-checked against KiCad itself (no `kicad-cli` here);
  spike S1 compares them with the drill report, pos CSV and stats JSON.

Rotation convention (test `ext_pad_positions_match_track_ends`): on all
footprints whose rotation is not a multiple of 180 degrees, pad centres
computed as `origin + (x cos t + y sin t, -x sin t + y cos t)` (CCW on
screen, Y down) were compared with track and via end points of the same
net, against the opposite rotation sense. Pads that coincide (1e-4 mm) with
a track end under only one of the two: **8454 with kdraft's convention, 7
with the opposite** (front 6676 vs 5, back 1778 vs 2; 12547 pads checked,
both KiCad 9 and 10 format files). So back-side footprints need no extra
mirroring: their local coordinates are stored already flipped. Pad angles
in the file include the footprint angle: on those footprints 11809 pads
have file angle == footprint angle and 683 have file angle 0.

### This increment, on real files

- `.kicad_wks`: KiCad's default sheet (assets) plus all sheets fetched from
  the demos at 9.0.9 and 10.0.8 load without warnings and resolve on A3
  (test `ext_sheets_and_projects_load`): `interf_u/pagelayout_logo`
  (legacy `page_layout` root, `%` codes, 5 polygons) and
  `vme-wren/cern-ohl-left` (version 20231118), drawing area 386 x 231 mm
  for each. No demo sheet contains a bitmap, so the bitmap extent code is
  only exercised by construction.
- `.kicad_pro`: all 73 demo project files (37 + 36) load; none sets
  `pcbnew.page_layout_descr_file` (interf_u and vme-wren set only the
  schematic one), so project sheet selection is tested with a hand-written
  project only.
- Native export (`doc new` + `export --no-kicad-cli`) verified visually by
  rasterising the PDFs with `pdftoppm` (poppler, already installed) for
  both fixtures, 9.0.9 `ecc83-pp`, 10.0.8 `pic_programmer` (KiCad 10
  format, also with the assembly view switched to the back side) and
  10.0.8 `video` (KiCad 10 format, 189 footprints, 1720 holes). The
  integration test exports ecc83-pp, pic_programmer and video when
  `fixtures-ext/` is present.

### M2, on real files

- Raster back end looked at (Read tool on 100 dpi PNGs from
  `kdraft export --format png --no-kicad-cli`): both fixtures, 9.0.9
  `ecc83-pp` and 10.0.8 `video` (all three pages each). Sheet, title
  block, views, designators, tables and the native-preview note render as
  in the PDF. Compared with poppler's rendering of the same PDF page
  (video, page 2): before the fix, hairlines (pad outlines, fab boxes)
  faded out because tiny-skia anti-aliases sub-pixel strokes; now strokes
  are at least one device pixel, matching poppler visually (mean grey
  difference 4.2/255, mostly anti-aliasing). Samples in
  `/home/claude/kdraft-samples/m2/` (pages and 160 px thumbnails).
- No kicad-cli here: plotted content in the raster back end is tested
  only through the synthetic stand-in.

## Unverified or assumed

Marked `UNVERIFIED:` in the code where it applies.

- **kicad-cli 9.0.x has never been run.** On 10.0.8 two boards were run
  (the fixture and user board A, see "Spike S1 results"): transform per
  mode, mirroring, flags, layer spellings (Edge.Cuts, F.Cu, B.Cu, F.Fab,
  F.Courtyard/F.CrtYd, Dwgs.User), drill report path, pos and stats are
  verified there and only there. kdraft still fits the transform at
  runtime for every export; nothing assumes the 10.0.8 mapping.
- Mode 2 page origin = outline box origin: seen on one board away from
  the origin (user board A); mode 2 is not used by default.
- KiCad's drill tool grouping is **confirmed on 10.0.8** by a real report
  (user board A, `fixtures/kicad-cli/10.0.8/drill-report-board-a.rpt`):
  `T3  0.500mm  0.0197"  (4 holes)  (with 4 slots)` for four 0.50 x 1.40
  slots (a slot belongs to the tool of its smaller dimension), equal-sided
  ovals appear as plain round tools, NPTH holes are listed in a separate
  "unplated through holes" block, ordinary lines end in a doubled `))`.
  Whether vias and pads of one diameter share a tool is still not shown
  by a real report. Not checked on 9.0.x. Third S1 run (user board A,
  10.0.8, build 41f57ce): kdraft's tool grouping (`kicad_tool_groups`)
  **matches KiCad's report exactly**: 11 tools, counts, plating and slot
  counts (0.3 PTH x523, 0.4 x16, 0.5 x4 with 4 slots, 0.6 x36, 0.7 x10,
  1.1 x68, 1.2 x44, 1.3 x2, 1.4 x34, 3.2 x13, 1.6 NPTH x2); verified on
  this board.
- `pcb export svg` on 10.0.8 (real help text,
  `fixtures/kicad-cli/10.0.8/pcb_export_svg.txt`): has
  `--sp/--sketch-pads-on-fab-layers`, `--hdnp`, `--sdnp`, `--cdnp`,
  `-D/--define-var KEY=VALUE`, `--scale`, `--check-zones`, and no option
  to suppress reference or value texts on fab layers (confirmed). A test
  checks that every flag kdraft emits for svg/drill/pos/stats, `pcb
  render` and `sch export bom` is in the real help texts.
- `pcb render` and `sch export bom` on 10.0.8 (real help texts,
  `fixtures/kicad-cli/10.0.8/pcb_render.txt`, `sch_export_bom.txt`; third
  S1 run: exit 0, every flag kdraft uses mentioned, none rejected): all
  flags kdraft emits are **confirmed present** (render: `--width
  --height --side --background --quality --preset
  --use-board-stackup-colors --floor --perspective --zoom --rotate
  --define-var --variant`; the help's own isometric example is `-45,0,45`,
  kdraft's preset). Render quality also offers `user` and `job_settings`;
  `-h` is the height, kdraft always writes `--height`. The behaviour of
  `pcb render` itself (pictures, KIPRJMOD models) and of `sch export bom`
  output on real schematics is not yet exercised.
- `.kicad_prl` next to the board after a kdraft run (user's checkout): the
  cause is **inferred, not confirmed** (kicad-cli writing project-local
  settings next to the board it opens). kdraft now runs every kicad-cli job
  on a private copy inside the cache (`<cache>/inputs/<hash>/`); a fake
  runner test checks that the project folder is untouched.
- Fab texts hidden in the private copy are left out of kicad-cli plots:
  expected, UNVERIFIED (see "Fab-layer texts in plotted views").
- KiCad expands a project text variable referenced from a title-block
  comment of the stub sheet (`${KDRAFT_S1_VAR}` in COMMENT2): **confirmed**
  on 10.0.8 by the third S1 run (marker found in both sheet SVGs,
  "text_variable yes").
- kicad-cli sheets show KiCad's page number, so `${#}/${##}` reads 1/1 on
  every page; hence `--sheet-from-kicad-cli` is opt-in.
- `.kicad_wks` semantics from memory of KiCad's behaviour, checked only
  against the four real sheets above and KiCad's default sheet:
  increments in the corner frame, repeat clipping against the margin box,
  default justification left/centre, label increment rules, bitmap
  position as centre, `%` code cut-off version 20210606, `rotate` sense.
  Title block and drawing area detection are heuristics.
- Text sizes: KiCad's text height is taken as cap height of IBM Plex Sans;
  widths differ from KiCad's stroke font, so `maxlen` shrinking and title
  block fill differ slightly from KiCad.
- Built-in variables win over project variables of the same name
  (vme-wren defines `PROJECTNAME`); KiCad's precedence not checked.
- Drill table, pos and stats cross-checks in S1 use tolerant parsers; they
  read the 10.0.8 outputs of two boards, layouts on 9.0.x are unknown.
- Editor (`kdraft-ui`): built and run on Windows from 936c6cb; the M3
  changes (page entry fix, greyed buttons, reference outline, scale
  snapping, outside-area outline, new properties) have not been seen.
- Footprint text angle semantics (absolute angle, local position) are
  from the KiCad 10.0.8 parser source; kdraft draws designators upright
  and does not use the file angles of fab texts.
- Declutter thresholds (90 % body fill, 15 % rotation advantage, 1.6 x /
  12 x min height search radii, 0.8 mm small-text limit) are first guesses
  checked only visually on demo boards.
- `doc new` BOM chunk planning estimates column widths (0.8 cap heights
  per character); a real BOM with long values may overflow (check reports
  it).
- Snapping, zoom buckets, texture size cap and drag threshold values are
  first guesses, untested by a user.
- Earlier items still open: drill offset rotation, via types, string
  escapes, chaining tolerance, `kicad-cli version` output format on 9.0.x
  and non-Windows install paths.

## Dependencies

All pure Rust and GPL-3.0-compatible.

New in the UI pass:

| Crate | Version | Licence | Used by |
| --- | --- | --- | --- |
| rfd | 0.18.0 (no default features; `xdg-portal`) | MIT | kdraft-ui (file dialogs) |
| windows-sys | 0.61 (via rfd, Windows only) | MIT OR Apache-2.0 | Windows common item dialog |
| pollster, percent-encoding, raw-window-handle | (via rfd) | MIT OR Apache-2.0 | |

rfd on Windows pulls only windows-sys and windows-link (no C toolchain,
no setup); on Linux the XDG portal path pulls pollster and
percent-encoding (no GTK). tiny-skia is now also a dev-dependency of
kdraft-ui (golden screenshots). Code adapted from FigCraft (MIT OR
Apache-2.0) is listed in `NOTICE`.

Earlier: **No new external dependencies in
M3** (new internal crate `kdraft-layout`, GPL-3.0-or-later like the rest).

New in M2:

| Crate | Version | Licence | Used by |
| --- | --- | --- | --- |
| tiny-skia | 0.12.0 (features std, simd, png-format) | BSD-3-Clause | kdraft-render (raster back end) |
| png | 0.18.1 (via tiny-skia) | MIT OR Apache-2.0 | PNG encoding |
| eframe / egui / epaint | 0.37.0 (eframe without default features; `x11`, `wayland`, plus `glow` or `wgpu`) | MIT OR Apache-2.0 | kdraft-ui |
| egui_kittest | 0.37.0 (dev, no default features) | MIT OR Apache-2.0 | kdraft-ui headless tests |

eframe brings about 200 crates on Linux (fewer on Windows), all with
GPL-3.0-compatible licences: mostly MIT and/or Apache-2.0, plus winit and
glutin (Apache-2.0), glow (MIT/Apache/Zlib), clipboard-win (BSL-1.0),
libloading (ISC), Zlib (foldhash, slotmap), Unicode-3.0, BSD-2/3-Clause
options, self_cell (Apache-2.0 OR GPL-2.0-only; used under Apache-2.0).
egui's bundled fonts (`default_fonts`, which include the Ubuntu Font
Licence) are switched off; the UI uses the bundled IBM Plex Sans. No
`notify`: board changes are detected by polling. Renderer choice: `glow`
(OpenGL through glutin/WGL on Windows) as default because it compiles
much faster and works on any Windows PC with a GPU driver; `wgpu`
(DirectX 12/Vulkan with the WARP software fallback) as a feature for
machines without OpenGL 2+. Both build without a C toolchain; `cargo
check -p kdraft-ui --no-default-features --features wgpu` passes on Linux.

M1:

| Crate | Version | Licence | Used by |
| --- | --- | --- | --- |
| roxmltree | 0.21.2 | MIT OR Apache-2.0 | kdraft-plot (SVG reading) |
| krilla | 0.8.2 (no default features) | MIT OR Apache-2.0 | kdraft-render (PDF) |
| ttf-parser | 0.25.1 | MIT OR Apache-2.0 | kdraft-render (text measurement) |

krilla brings (all MIT/Apache-2.0 unless noted): pdf-writer, subsetter,
skrifa, read-fonts, write-fonts, font-types, kurbo, polycool, flate2,
miniz_oxide (MIT/Zlib/Apache), simd-adler32 (MIT), adler2, crc32fast,
xmp-writer, bumpalo, base64, float-cmp (MIT), siphasher, smallvec,
rustc-hash, tiny-skia-path (BSD-3-Clause), strict-num (MIT), arrayref
(BSD-2-Clause), yoke/zerofrom (Unicode-3.0). krilla 0.8.2 needs Rust 1.92,
so the workspace `rust-version` is now 1.92.

Existing: thiserror, blake3, uuid, serde, serde_json, clap, anyhow,
proptest and tempfile (dev).

Assets: IBM Plex Sans Regular and Bold 3.006 from github.com/IBM/plex
(`packages/plex-sans/fonts/complete/ttf/`, master at
ba3013d72a8419a22c758cc51c44e04ac7ea565b), SIL OFL 1.1, licence in
`assets/fonts/OFL-IBM-Plex.txt`; KiCad's default drawing sheet,
GPL-2.0-or-later (`assets/sheets/NOTICE`).

Spike S3 findings (krilla): builds without C toolchain; one page per
`start_page_with`, any page size; paths, fills, strokes, dashes, clip
paths, transforms; glyph runs with own positioning (`draw_glyphs`) and
font subsetting with ToUnicode, so text is searchable; document outline
(bookmarks) supported and used; deterministic output when no creation date
is set (file ID is a content hash). **Optional content (PDF layers) is not
supported**: krilla 0.8.2's own docs (`configure/PDF_A2.md`, `PDF_UA1.md`)
state "krilla does not support optional content". Per-layer toggling in
PDF would need another writer or a krilla extension.

New in this increment (krilla's `raster-images` feature, for PNG images
in PDFs):

| Crate | Version | Licence |
| --- | --- | --- |
| gif | 0.14.2 | MIT OR Apache-2.0 |
| image-webp | 0.2.4 | MIT OR Apache-2.0 |
| imagesize | 0.14.0 | MIT |
| weezl | 0.1.12 | MIT OR Apache-2.0 |
| color_quant | 1.1.0 | MIT |
| quick-error | 2.0.1 | MIT OR Apache-2.0 |
| zune-jpeg, zune-core (already in the lock file) | 0.5 | MIT OR Apache-2.0 OR Zlib |

All pure Rust, no build scripts.

## Known gaps

- KiCad 9.0.x never run; kicad-cli 10.0.8 run on two boards only; nothing
  of M3 run with a real kicad-cli (fab-text hiding in plots, BOM export).
- M3 not built on Windows; GUI changes of M3 not seen.
- Declutter: outside labels sit at the minimum height while inside labels
  go up to 2.5 mm (visible size contrast); in dense clusters an outside
  label can be nearer to a neighbour than to its own part (no association
  check besides leaders); leaders may cross other labels or bodies; values
  are only placed inside parts (else omitted, counted in a warning); the
  pin-1 heuristic misses polarised parts with other prefixes and marks
  non-polarised 3+ pad parts too; no fab user texts in native views.
- Dimensions: no angular dimensions; no GUI tools to create dimensions,
  notes or callouts (commands only); dimension and callout items have no
  sheet bounds for align/snap (`item_bounds` returns none for dimensions);
  ordinate dimensions are drawn without a datum marker; unknown children
  of `(styles (dimension ...))` are not preserved on round trip.
- BOM: no command to renumber/store BOM numbers after a schematic change
  (numbers of new groups are assigned at render time and not written);
  variants not passed; extra fields only via `field:<label>` columns of a
  CSV that contains them (kdraft asks for no extra fields yet).
- Drill: no KiCad-tool column (the grouping difference is documented
  only); symbols on very dense via fields shrink to 0.6 mm and still touch.
- Unverified: fractional `--zoom` of `pcb render` and that it centres
  the board outline box (kdraft's to-scale placement of top/bottom 3D
  views relies on it); whether `--define-var KIPRJMOD=<project folder>` resolves
  `${KIPRJMOD}` 3D model paths on the private copy; coloured layers with
  real (non-recorded) plots of other boards; the whole increment on
  Windows.
- Earlier gaps remain: tracks/zones/silkscreen not drawn natively, no KiCad
  10 padstacks, no DXF, PDF without optional content, opacity dropped,
  `scripts/fetch-fixtures.sh` needs bash, the engine commit a494835 of M2
  does not build on its own (every M3 commit builds and passes its tests).

## Next tasks (in order)

0. On Windows: try styles, the picker, multi-line text in place, handles,
   placement links (Insert > Layer pages), layer colours and a 3D view
   with kicad-cli (models from the project folder must appear).
1. On Windows: build and run the UI pass; go through "What to test by
   hand" in `docs/GUI.md` (dialogs, theme, menus, tools, live
   dimensions, issues).
2. On Windows: `cargo build`, `cargo test --workspace`, then run
   `kdraft doc new` + `export` with kicad-cli on board A and a demo board;
   check the hidden fab texts in a plot-rendered assembly view
   (`docs/S1.md`), and run `spike s1` once to record `sch export bom
   --help`; fix the BOM flags if needed.
3. Declutter quality on board A at 1:1 and the dense-rule scale: tune
   outside label height, add an association check (label nearest to its
   own part), leader routing that avoids labels.
4. BOM: extra fields setup, variants (`--variant`).
5. GUI: German strings, drag-to-place for views and tables, resizing
   graphics on the canvas, tile-based view textures for high zoom.
6. Detail views, larger sheets or split views for very dense boards; a
   per-view plot style for black-and-white plots (inner planes render solid
   black, seen in the user's recording).
7. KiCad 9.0.x runs; KiCad 10 variants and padstacks.
