# kdraft: native Rust fabrication and assembly drawing tool for KiCad 9+

Condensed from the planning doc "Plan: Native Rust Draftsman for KiCad 9+"
(https://claude.ai/artifact/C3sgdNPggKwHimHnyPyBHd, 2026-11-24). The doc holds the
Draftsman analysis, the full feature table, risks and sources. This file is the
working reference for implementation.

## Goal

A standalone desktop app plus headless CLI that produces fabrication and assembly
drawing sets for KiCad boards, analogous to Altium Draftsman, with additions:
alignment and distribution, page reordering, anchored dimensions, change reports,
drawing checks, per-variant output, text file format, CI use.

## Key decisions

| Topic | Decision |
| --- | --- |
| Board semantics | Parse `.kicad_pcb`, `.kicad_pro`, `.kicad_wks` directly (headless, stable anchors) |
| View geometry | `kicad-cli pcb export svg`, one export per layer, composited in-app, cached by board hash |
| Live link | KiCad IPC API, optional, behind a cargo feature (GUI-only, PCB Editor only, no plotting in 9/10) |
| GUI | egui chrome + canvas; views as tile-cached rasters, annotations as live vectors |
| PDF | krilla; SVG and DXF writers later |
| Document file | `.kdraft`, canonical S-expression text, diffable |
| Headless | `kdraft` CLI on the same engine |
| KiCad versions | 9.0.x and 10.0.x; detect `kicad-cli` version, gate flags such as `--variant` |

## Workspace

| Crate | Role |
| --- | --- |
| `kdraft-sexpr` | Lossless S-expression reader/writer for KiCad files and `.kdraft` |
| `kdraft-board` | Semantic model: outline, footprints, pads, holes, stackup, title block; hash; model diff |
| `kdraft-source` | `BoardSource` trait, file backend, `kicad-cli` discovery, job runner, content-addressed cache; IPC backend behind a feature |
| `kdraft-doc` | Document model, anchors, styles, `.kdraft` read/write, migrations |
| `kdraft-layout` | Table layout, dimension geometry, align/distribute, designator declutter, snap index |
| `kdraft-render` | Display list, SVG import, back ends: raster, PDF, SVG, DXF |
| `kdraft-engine` | Command registry, sessions, undo log, selection, drawing checks |
| `kdraft-ui` | egui desktop front end |
| `kdraft-cli` | Headless binary `kdraft` |

Rule: nothing below `kdraft-ui` depends on egui. Every user action is a named
command in the engine registry; GUI and CLI call the same table.

## Document model

```
Document
  source        board path, variant, kicad-cli version last used
  styles        text, line, dimension, table styles
  pages[]       ordered; each has sheet (.kicad_wks), size, title fields
    items[]     z-ordered
      View      kind, layer set, side, scale, rotation, crop, style, frame
      Table     kind (BOM, drill, stackup, custom), columns, filters, split rules
      Dimension kind, anchors[], offset, style, tolerance
      Callout   anchor, target (BOM item or note), leader path
      NoteList  items with stable ids, parameter bindings
      Graphic   line, polyline, arc, circle, rect (corner radius), polygon,
                text (multi-line, wrap, rotation), image
      (every item: optional appearance overrides and placement link;
       views: layer colours, 3D render options)
```

Rules:

- Every item has a UUID. Page order is one ordered list, so reordering is a
  one-line diff and never rewrites page contents.
- Sheet coordinates are millimetres. Positions inside a view are board
  coordinates, so they survive scale and rotation changes.
- Anchors are typed references: `(pad "U7" "3")`, `(hole 12)`,
  `(outline-vertex 4)`, `(fp-origin "J2")`, `(view-point x y)` as unbound fallback.
- Detail and section views reference their parent view by id.
- Writer output is canonical: fixed key order, fixed number formatting.
- `version` field plus migrations; unknown nodes are preserved on round-trip.
- Generated geometry is never stored in `.kdraft`; it lives in the cache.
- Appearance: an item stores only the style attributes it overrides, in
  `(appearance (stroke "#C81E1E") (width 0.35) (pattern dashed) ...)`; the
  document stores category defaults in `(styles (appearance (graphic ...)
  (text ...) (dimension ...) (callout ...) (notes ...) (table ...)))`.
  Effective style = built-in, then document category, then item. Colours are
  sRGB `#RRGGBB` or `#RRGGBBAA`.
- Placement links: a follower stores `(link (to "<id>") (follow position
  scale rotation) (offset dx dy))` and keeps its own position as the last
  resolved value. Followed properties: `position`, `x`, `y`, and between
  views `scale`, `rotation`, `side`; `size` between rectangles (size) and
  texts (wrap width). Default: position, scale and rotation from view to
  view, position otherwise. The reference may be on any page. Rules,
  applied by the engine after every command and gesture preview: links
  never chain (linking to a follower links to its root; a reference that
  becomes a follower hands its followers over; cycles are rejected); a
  change a command makes to a follower's followed properties moves the
  reference (the follower's offset stays), then every follower is placed
  from its reference; deleting a reference makes its first follower (page
  order, then z-order) the reference of the others, in the same undo step;
  duplicating or copying a follower keeps its link, duplicating a page keeps
  links to the original reference, duplicating a reference does not take
  followers along. `kdraft check` reports links to missing items and linked
  placements outside a page's drawing area.

Example:

```
(kdraft (version 1)
  (source (board "../hp-controller.kicad_pcb") (variant "default"))
  (page (id "p1") (sheet "A3-fab.kicad_wks") (title "Assembly top")
    (view (id "v1") (kind assembly) (side front) (scale 2)
      (at 148 105) (layers F.Fab F.Courtyard Edge.Cuts))
    (dimension (id "d1") (kind linear) (view "v1")
      (from (outline-vertex 0)) (to (outline-vertex 1)) (offset 8))))
```

## Data sources

| Need | Source |
| --- | --- |
| View graphics | `kicad-cli pcb export svg --layers L --mode-single --exclude-drawing-sheet` |
| Anchors | Parsed `.kicad_pcb` |
| Drill table | Parsed pads and vias; cross-check with `pcb export drill --generate-report` |
| Drill drawing | Own renderer from parsed holes |
| Layer stack legend | `(setup (stackup ...))` in `.kicad_pcb` |
| BOM | `kicad-cli sch export bom` (`--variant` on KiCad 10) |
| Positions | `kicad-cli pcb export pos --format csv` |
| Fab note values | `kicad-cli pcb export stats --format json` |
| Realistic view | `kicad-cli pcb render` |
| Sheet frame, title block | `.kicad_wks`, title block, project text variables |

## Sync pipeline

1. File watcher sees a board/project/schematic change, debounces.
2. Re-parse and hash; unchanged hash stops the run.
3. Diff the model: footprints added/removed/moved/rotated/flipped, outline,
   holes, stackup, title block.
4. Queue only views whose inputs changed.
5. Run `kicad-cli` jobs in a pool; cache key = board hash + CLI version + args.
6. Re-resolve anchors; missing anchors are marked broken, never dropped.
7. Regenerate tables; BOM item numbers stay stable via stored group keys.
8. Show the change report; clear stale badges per view.

## Roadmap

| Phase | Scope | Gate |
| --- | --- | --- |
| P0 Spikes | S1 plot-to-board coordinate mapping, S2 canvas performance, S3 krilla PDF, S4 IPC plugin | Go/no-go on backend and canvas |
| M1 Headless core | Parser, board model, cache, `.kdraft` format, sheet renderer, CLI fab and assembly PDF | Real board to PDF from the CLI |
| M2 Editor MVP | Canvas, select/move/scale, align/distribute, snapping, pages panel with reorder, undo | Drawing set laid out in the GUI |
| M3 Fab content | Drill table and drawing, stackup legend, BOM, notes, dimensions, callouts | First usable release |
| M4 Sync and variants | Auto-update, change report, variants, templates, drawing checks | Board edit to fresh PDF in CI |
| M5 Advanced views | Detail, section, via views, IPC plugin, DXF | Draftsman PCB parity without 3D |

Roadmap status (2026-11-24): M1 done (native and plotted on 10.0.8); M2
implemented, run on Windows by the user (interaction partly exercised);
M3 implemented and verified headless on demo boards, not yet on Windows or
with kicad-cli; native assembly rendering pulled forward from M5 into M3
(the kicad-cli F.Fab plot of a dense real board was unreadable); drawing
checks (`kdraft check`) pulled forward from M4.

Update (2026-11-25): item styles and colours, multi-line text, geometry
editing with handles, placement links (layer pages), view layer colours
and 3D views (`kicad-cli pcb render`; "without 3D" in M5 no longer
holds) implemented and verified headless; not yet on Windows or with a
real kicad-cli.

## Decisions (2026-11-24, owner: Martin)

- Licence: GPL-3.0-or-later, like KiCad. Dependencies must be
  GPL-3.0-compatible; still a small, pure-Rust set.
- Platform: Windows is the target. Code stays portable, but everything must
  build, test and behave correctly on Windows (paths, `kicad-cli.exe` under
  `C:\Program Files\KiCad\<ver>\bin`, no shell scripts needed for normal use).
- BOM source: the schematic, via `kicad-cli sch export bom`.
- Drawing sheets: KiCad's default sheet by default; changeable per document
  and per page. Selection order: page override, document override, the
  project's sheet from `.kicad_pro` (`pcbnew.page_layout_descr_file`,
  relative paths and `${KIPRJMOD}` resolved), then the default. Two render
  paths: native `.kicad_wks` reader (always available, source of
  searchable title-block text) and, with `kicad-cli`, KiCad itself drawing
  the sheet from a stub board carrying paper size and title block
  (`--drawing-sheet` for custom sheets), used as page background. The stub
  export's coordinates are measured in S1, not assumed.
- Units: millimetres only; no dual-unit dimensions.
- Default snap grid for placing items on a sheet: 0.5 mm.
- The earlier web prototype (FastAPI/kiutils/Svelte) is archived. Only three
  ideas are ported: sheet via a kicad-cli stub board, sheet discovery from
  `.kicad_pro`, and the 0.5 mm snap grid.

## Prior art to borrow from

- PdfCraft (storytold/pdfcraft): engine facade, schema-described command table
  shared by CLI and GUI, pages inspector with undoable page moves.
- CADCraft (storytold/cadcraft): associative dimensions, object snaps, DXF
  reader/writer, copy-on-write entity store for undo.
- krilla (LaurenzV/krilla): PDF writer. hayro: PDF rasteriser for golden tests.
