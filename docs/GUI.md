# kdraft-gui: the editor

Status (2026-11-25): the UI pass (light/dark theme, menus, palette, item
tree, inspector, tools, rulers) is built and looked at **only headless**
(software-rendered screenshots, see "Headless screenshots"); nobody has
run it on Windows yet. The previous editor (HEAD 3c8816e and the fixes up
to 2368774) was run by the user on Windows with a 350-footprint board.

## Start it on Windows

Needs stable Rust (1.92 or newer) and a GPU driver with OpenGL 2 or newer
(any normal Windows PC). In the kdraft folder:

```
cargo run --release -p kdraft-ui --bin kdraft-gui
cargo run --release -p kdraft-ui --bin kdraft-gui -- path\to\board.kicad_pcb
cargo run --release -p kdraft-ui --bin kdraft-gui -- path\to\drawing.kdraft
```

Opening a `.kicad_pcb` creates a new three-page document (as
`kdraft doc new`); it is saved next to the board as `<board>.kdraft` when
you press Save. The built program is `target\release\kdraft-gui.exe`.

Renderer: eframe's OpenGL back end (`glow`, the default feature). If the
window stays black or does not open (remote desktop, VM without OpenGL),
try the wgpu back end (DirectX 12 / Vulkan, with Windows' WARP software
fallback):

```
cargo run --release -p kdraft-ui --bin kdraft-gui --no-default-features --features wgpu
```

`kicad-cli` is found like in the CLI: `KDRAFT_KICAD_CLI`, `PATH`, then
`C:\Program Files\KiCad\<version>\bin`. The status bar shows the version or
"not found". Without it, views show the native preview from the board
model (marked "NATIVE PREVIEW").

## Layout

- **Top strip**: wordmark, menus (File, Edit, View, Insert, Arrange,
  Page, Help; generated from the action catalogue: engine commands with
  their metadata plus the editor's own actions; disabled entries are
  greyed by the same rule the commands use), undo/redo, the document name
  with a dot when modified and the board file, command palette and
  light/dark toggle.
- **Left**: Pages (thumbnails; a click anywhere selects; dragging past a
  few pixels reorders with an insertion line and a ghost; double click
  renames; right click: rename, duplicate, delete, move up/down, drawing
  sheet, add) and below it the **item tree** of the current page, top-most
  first: type icon, name (default names such as "Fabrication view",
  "Drill table", "Horizontal dimension"), eye and lock toggles (shown on
  hover, or when hidden/locked). Click selects, Ctrl+click toggles,
  Shift+click selects a range, double click or F2 renames, dragging a row
  changes the stacking order (one undo step), right click opens the item
  menu.
- **Centre**: the sheet as white paper on a neutral pasteboard, mm rulers
  along the top and left edge (follow zoom and pan; the cursor is marked),
  optional snap grid (View > Show grid, Shift+G). Views, tables, note
  lists and the drawing sheet are cached rasters; dimensions, callouts
  and free graphics are drawn live as vectors from their current geometry
  (they follow a dragged view frame by frame). The selected item gets a
  thin accent outline, its name above it and a value pill below
  ("2:1 · 201 / 141.5" for a view, "W x H" otherwise); a selected
  dimension highlights its lines with handles on the dimension line and
  the text. Hover highlights the item under the pointer (also from the
  item tree). Items reaching outside the drawing area get a red outline.
  Snap guides: orange to other items, green to page and drawing area,
  violet for equal spacing. **Alt** while hovering shows the gaps in mm
  between the selection and the hovered item, or to the drawing-area
  edges. Floating tool bar at the bottom: Select, Hand, then groups with a
  caret for the variants (views, tables, dimensions, callout/notes/text,
  line/rectangle), snap and grid toggles.
- **Right**: inspector. Header with the selection's name and the zoom
  menu; arrange bar (align x6, distribute x2); sections: Page (title,
  paper, orientation, sheet with "Choose…"), Document (snap grid, board
  reload, BOM source and Renumber), Position (X/Y fields), Scale and
  rotation, View (kind, side, render, drill marks, frame), Layers
  (preset chips from the board's layer table, filter, checked layers
  first, the rest collapsed), Assembly (fab texts; native: pads, holes,
  pin-1, DNP style, designator filter, text heights), Table, Dimension
  (kind, value, anchors by name, offset, tolerance, decimals and text
  height for all dimensions), Notes (edit, remove, add), Callout,
  Geometry, Item (duplicate, forward, backward, delete; name; hidden;
  locked). Several items: match and centre, common scale.
- **Number fields**: lead letter or icon, unit; click to type (numbers or
  expressions such as `100+5`, comma accepted), Enter or focus loss
  commits, Escape cancels, arrow keys nudge (Shift x10); drag the lead to
  scrub (Shift x10), one undo step.
- **Bottom**: status bar (cursor mm, zoom, tool hint, board and hash,
  kicad-cli state, background jobs with a spinner, issue count, log
  toggle). The dock (Ctrl+Shift+I / Ctrl+Shift+M) shows the drawing checks
  by severity (they run in the background 0.6 s after a change; clicking
  one selects the page and item and zooms to it) or the message log.
  Warnings and errors also appear as toasts (bottom right, a few seconds).
- **Start screen** (no document): Open…, New from board…, recent files.

## Tools

Press the key or pick the tool in the floating bar or the Insert menu;
Escape steps back, a second Escape returns to Select. Free points snap to
the grid (snap on); anchor tools snap to pads, holes, outline vertices,
outline arc centres and footprint origins within 10 px (marker and name,
for example "pad U7.3"). Each placement is one undo step; the tool then
returns to Select (ordinate keeps its datum for the next one).

Missing input is asked for or defaulted, not refused:

- Dimension and callout tools on a page without a board view switch to
  "Place fabrication view" first (a message says so); after the view is
  placed the chosen tool continues.
- Callout on a part without a BOM line and no note on the page: a note
  list with "Note for R5" is started at the top left of the drawing area
  and the balloon points at it (edit the text).
- Insert > Board size dimensions on a page without a view adds a
  fabrication view (centred, scale leaving room for the dimensions) first.
- Text, line, rectangle, polyline and circle work without a board.
- Arrange > Link placement to… with nothing selected asks for the item to
  link first, then the reference.

| Tool | Clicks |
| --- | --- |
| Fabrication / assembly / drill view | place at the click |
| Drill / layer stack / BOM table | top-left at the click |
| Horizontal / vertical / aligned dimension | first anchor, second anchor (same view), then the offset (rubber band with the value) |
| Radius / diameter | a hole, arc or circle, then the leader direction |
| Ordinate | datum anchor, measured anchor, then the leader (mostly vertical: x value) |
| Callout (number) | a pad or footprint origin, then the balloon; the next number is shown at the pointer and the tool stays on for the next one |
| Callout (BOM item), Callout (note) | as above; the balloon shows the part's BOM item number, or a note's number (the first note, else a new note list) |
| Note list, text | place at the click |
| Line, rectangle | two corners |
| Polyline (P) | click the points; double-click or Enter finishes |
| Circle (C) | the centre, then a point on the circle |

## Editing shapes, text and dimensions

Select one item to get its handles (one undo step per drag; points snap
to board anchors and the grid; Shift keeps line segments at 0/45/90
degrees):

- Lines, polylines, polygons, arcs: drag a point. Double-click a polyline
  segment to add a point; click a point and press Delete to remove it.
- Rectangles: corner and edge handles; corner radius and size in the
  inspector. Circles: the radius handle.
- Text: drag to move; the round handle above the right end rotates (15
  degree steps, Alt for free rotation); the square on the right edge sets
  a wrap width. Double-click a text, or press Enter with it selected, to
  edit it on the canvas: Enter starts a new line, Ctrl+Enter or a click
  outside keeps it, Escape cancels. `${TITLE}`, `${BOARD_WIDTH}` and the
  other note variables work in texts too.
- Dimensions: drag an end marker onto another pad, hole or outline point
  of the same view (the anchor under the pointer is marked and named);
  drag the text handle along the dimension line.
- Callouts: drag the balloon; drag the anchored end onto another anchor.

## Styles and colours

The inspector's Style section shows what applies to the selection: line
colour, width, pattern (solid, dashed, dotted, dash-dot), fill of closed
shapes, arrows and arrow size of lines, text colour, height, bold,
italic, alignment and line spacing, dimension decimals, table header
fill, opacity. With several items selected, differing values show as
mixed; a change applies to every item it fits. A swatch opens the colour
picker: fixed palette, recent colours, hex entry (#RRGGBB or #RRGGBBAA),
hue/saturation/value/alpha sliders (one undo step per slider drag).
"Reset to document style" drops the item's own settings; Copy / Paste
(Edit > Copy style Ctrl+Alt+C, Paste style Ctrl+Alt+V) transfer a style
between items. Edit > Document styles… sets the defaults per item kind;
items without their own settings follow. Italic text is shown upright on
the canvas (exports are italic).

## Layer colours

The Layers section of a view lists its layers topmost first (drawn last);
drag a row to change the order, untick to hide. "Palette" shows what the
view draws with: Document (the document palette; fabrication and drill
views stay black with it) or a palette of the view's own. Each row has
its colour swatch and opacity; a row with its own colour shows a reset
button ("Reset to palette"), and "Reset all to palette" clears every
row. Changing colours never runs kicad-cli again (plots are cached
uncoloured). In plotted views drill knock-outs are drawn once above all
layers, as in a combined plot.

Every colour a view draws resolves in one order: the view's own colour for
that layer (or native role: pads, holes, outline, bodies, designators,
DNP), then the view's palette, then the document palette, then black. A
native role without its own colour takes its layer's colour (pads: F.Cu
or B.Cu, designators and leaders: the silkscreen, bodies and values: the
fab layer, DNP marks: the courtyard, outline: Edge.Cuts); pad fills are a
light tint of the pad colour (light grey for black), holes and drill
symbols grey. So Black, for a view or for the document, makes everything
it draws black or grey. Older files with `(layer_preset ...)` or a view
`(preset ...)` load as the matching built-in palette and are written in
the new form.

### Palettes

A palette maps layers to colour and opacity: copper by position (front,
inner 1..n, back), technical and user layers by KiCad name (F.SilkS,
Edge.Cuts, User.1 ...), and optionally the native views' roles (outline,
bodies, designators, pads, holes, DNP). Built-in: Black (factory
default), kdraft layer colours, Greyscale, "KiCad Default (paper)" and
"KiCad Classic (paper)" (KiCad's colours adapted for white paper, every
layer at least 3:1 contrast), and KiCad Default and KiCad Classic with
KiCad 10.0.8's own board colours (see NOTICE), listed as "(screen
colours)": several of them, such as Edge.Cuts #D0D2CD and the silkscreen,
are nearly invisible on paper. Own palettes are
made in Settings > Layer colours (duplicate, rename, edit, delete,
export/import as JSON to share between PCs) or imported from KiCad colour
themes.

- **The document palette** (page inspector > Document > Layer palette, or
  Settings > Documents > This document) is stored in the `.kdraft` with
  its colours, so the drawing looks the same on a PC without that
  palette. Every view without its own colours follows it, whichever way it
  was inserted (tool, Insert menu, `item.add_view`, `doc new`, layer
  pages, paste), and a layer added to a view gets its palette colour.
  Changing the palette recolours those views at once, without kicad-cli.
  New documents take the palette of Settings > Documents > Layer palette.
- **Fabrication and drill views stay black line drawings** unless they get
  their own colours; Black stays the factory default, so existing
  documents render exactly as before.
- **Per view**: the view's Layers section has "Palette": Document
  (default) or any palette, stored with the view by value (every layer and
  role, also layers added later); choosing a palette clears the view's
  per-layer colours, choosing Document again follows the document.
- **Importing a KiCad colour theme** (Settings > Layer colours): themes in
  KiCad's `colors` folders of every installed version are listed by name;
  "Browse…" takes any file. kdraft reads the `board` colours (format and
  key names from KiCad 10.0.8's `color_settings.cpp`; `rgb()`, `rgba()`
  and `#hex` colours); keys missing in the file come from KiCad Default.
  "Adapt for white paper" (on by default) darkens colours whose contrast
  against white is below 3:1 (WCAG for graphics), keeping their hue; the
  table then shows the theme's colour beside the one kdraft uses. kdraft
  copies the colours and never writes into KiCad's folders; re-importing
  updates the palette. UNVERIFIED: no theme file written by KiCad itself
  has been read yet (the tests use hand-written files in KiCad's format).

## 3D views

Insert > 3D view places a picture from `kicad-cli pcb render`. Inspector:
Top / Bottom / Isometric / Iso bottom, side, perspective, white or
transparent background, stackup colours, floor and shadows, kicad-cli's
appearance preset (follow plot settings or the PCB editor), quality and
resolution (dpi of the picture on paper, default 300, at most 4096 pixels
a side).

- **Trimmed to the board.** kicad-cli renders the board small inside a
  large empty picture. kdraft cuts the picture to what is drawn (pixels
  that are not transparent) plus 1 mm of paper; the frame, selection,
  snapping, alignment, the "outside the drawing area" check and the
  exports all use the trimmed picture. kdraft always asks for a
  transparent render to find the board; "White background" paints white
  under it. With "Floor and shadows" the floor fills the picture, so it is
  not trimmed.
- **To scale or not.** Top and Bottom without perspective, rotation or
  floor are placed so the board outline box lands exactly at the view's
  scale (label "3D VIEW (TOP)  SCALE 2:1"; the inspector shows the Scale
  section). kdraft finds the outline in the picture from the drawn content
  (the board is centred by kicad-cli and parts overhang it on at most some
  sides; UNVERIFIED with real renders of boards whose parts overhang on
  all four sides). Every other view has no scale: the inspector shows
  "Size" (width on paper) instead, the frame keeps the trimmed picture's
  aspect, and the label reads "3D VIEW (NOT TO SCALE)".
- **Two passes.** A small probe (480 px) finds how much of the picture
  the board uses; the final render zooms in (`--zoom`, fractional zoom
  UNVERIFIED) so the pixels go to the board. The placeholder says
  "Rendering 3D view, pass 1 of 2..." and "pass 2 of 2...". Both passes
  run in the background and are cached per board, view options and
  resolution step: moving a 3D view never renders again, resizing renders
  again only when the size crosses a power of two in pixels.
- Without kicad-cli the view says "3D view needs kicad-cli", also in
  exports and as a check warning (`kdraft check` looks for kicad-cli like
  an export does; `--no-kicad-cli` checks as for an export without it).
- 3D models from libraries resolve (verified on Windows); models
  referenced as `${KIPRJMOD}/...` should resolve too (kdraft passes the
  project folder; not yet confirmed).

## Callouts

A callout is a balloon with a leader to a pad or part origin. What the
balloon shows ("Shows" in the inspector):

- **Number** (default): a label typed by you, normally a number; any
  text works ("12", "A3", "3a").
- **Note**: the number of a note (pick it from the page's notes, or "New
  note…" appends one).
- **BOM item**: the item number of the anchored part's BOM line
  (read-only; only for callouts on a part).
- **Designator**: the reference of the anchored part (R5).

Numbering:

- The callout tool (B) numbers automatically: the next label is one more
  than the highest number among the page's callouts with the same prefix
  as the most recently placed one (1, 2, 3 → 4; A1, A2 → A3). The next
  number is shown at the pointer; the tool stays on until Escape. Page >
  Document "Callouts: Per page | Document" numbers across the whole
  document instead.
- Change a number afterwards: the inspector's Label field (with several
  callouts selected, typing sets all; − and + shift each), double-click or
  Enter on a balloon to edit it in place (Enter commits, Escape cancels),
  or the + and − keys with balloons selected.
- Duplicate (Ctrl+D) keeps the label (several balloons may point at parts
  of the same kind); Edit > Duplicate with next number (Ctrl+Alt+D)
  continues the sequence.
- Arrange > Renumber callouts… (also in the context menu): the selected
  numbered callouts, or all of the page, in rows (top to bottom, then left
  to right), columns, or selection order; start, step and an optional
  prefix; one undo step.
- Switching from Note, BOM item or Designator back to Number keeps the
  number shown.
- "Strict callout numbering" (page inspector, Document) reports gaps in a
  page's sequence as info issues; duplicates are never reported.

Balloon shape (Style section): circle (default), square, hexagon or none
(text only); the balloon grows with the label. Canvas, PDF, SVG and PNG
draw callouts with the same code.

File format: `(callout ... (label "12"))` for numbers, `(target
"note:N1")`, `(target "bom:<key>")` or `(target "refdes")` for derived
content; older files with only `target` read as before. Document options:
`(styles ... (callout_numbering document) (strict_callout_numbering
yes))`; the shape is the style attribute `shape`.

## Placement links

Items on different pages can share one placement: link an item to a
reference and it keeps the same position (and, between views, scale and
rotation) as the reference, wherever either is moved.

- Link: right-click the item(s) > Link placement to > pick the reference
  from the list grouped by page, or "Pick on canvas or in the item tree…"
  then click the reference (on any page; Escape cancels). Arrange > Link
  placement to… does the same. The items stay where they are: the offset is
  their current distance to the reference.
- A link group moves as one: dragging, nudging, aligning or typing a
  position for any member moves the reference and all followers; scale and
  rotation likewise when followed. To move one member alone, change its
  offset in the inspector, hold Alt while dragging it (this edits its
  offset), or unlink it.
- Inspector "Placement link": the reference (click to go there), the
  followed properties as chips, the offset, Unlink; on a reference, the
  list of its followers. Linked items show a chain icon in the item tree and
  "linked" in the selection label.
- Right-click > Select reference / Select linked items; Copy to page,
  linked puts copies at the same place on another page, linked to the
  originals.
- Deleting a reference keeps its followers where they are; the first one
  becomes the reference of the others.
- Views can also follow their reference's **colours** (follow chip
  "colours"): the reference's palette choice and its own colours for the
  layers both views draw, live. On for views made by Layer pages, off for
  links made by hand. Unlinking keeps the colours a view had.
- In the Layers section a linked view whose colours differ from its
  reference says so ("Colours differ from the reference view: Edge.Cuts")
  with "Match reference"; a reference lists layers its linked views draw
  otherwise, with "Match linked views to this one". Arrange (and the
  context menu) > Match layer colours to reference does the same for the
  selection, and `kdraft check` reports such groups (info). A view whose
  own colours are exactly KiCad's screen colours (documents saved before
  palettes were fixed) gets a hint with "Reset to palette"; documents are
  never rewritten on their own.

Layer pages (Insert > Layer pages…) works from any page, with or without
views. The window asks for:

- **Placement reference**: "New placement (centred, scale fitted to the
  sheet)", or any board view, listed by page with name, page and scale.
  Preselected: the selected view, else the only view on the current page,
  else New placement. With a view, the layer views link to it and take its
  page's paper and sheet; with New placement, the first layer view is
  centred in the drawing area at the largest standard scale that fits (as
  `doc new` does) and the other layer views link to it.
- **Layers**: set chips (All copper, Silkscreen, Fab, Mask, Courtyard) and
  single layers; "Add Edge.Cuts to every page".
- **Where**: after the current page (default) or at the end. If the
  current page is empty and untitled ("Page N"), "Use the empty page N as
  the first layer page" (on by default).

A line above Create says what will happen, for example "Page 2 becomes
the F.Cu page, 3 more page(s) after it (In1.Cu, In2.Cu, B.Cu); new
placement: the F.Cu view is centred at 2:1, the others link to it". Run
again with the same reference: only missing layers are added. Every view
of a set takes the reference's colours (palette choice and its own colours
for the common layers such as Edge.Cuts) and keeps following them; each
page's own layer takes the palette's colour. Views of an older set that
keep colours of their own are left alone and counted in the summary.

Example: four copper layers, one per page, always in the same place. Add
a page, Insert > Layer pages…, keep New placement, "All copper" and "Add
Edge.Cuts", Create: pages F.Cu, In1.Cu, In2.Cu and B.Cu, each with a view
of that layer at the same place. Drag or rescale the view on any of these
pages and all move together. From the command line:
`kdraft doc new board.kicad_pcb -o out.kdraft --layer-pages copper`, or
`kdraft cmd run out.kdraft --do view.pages_per_layer --args '{"set":"copper","reference":"new"}'`.

## Keyboard shortcuts

Generated from the catalogue (`kdraft-gui --shortcuts-md`):

| Menu | Action | Shortcut |
| --- | --- | --- |
| File | Open… | Ctrl+O |
| File | New from board… | Ctrl+N |
| File | Save | Ctrl+S |
| File | Save as… | Ctrl+Shift+S |
| File | Export PDF… | Ctrl+E |
| File | Reload board | F5 |
| File | Close document | Ctrl+W |
| File | Settings… | Ctrl+Comma |
| Edit | Undo | Ctrl+Z |
| Edit | Redo | Ctrl+Y, Ctrl+Shift+Z |
| Edit | Rename | F2 |
| Edit | Show / hide | Ctrl+Shift+H |
| Edit | Lock / unlock | Ctrl+Shift+L |
| Edit | Copy style | Ctrl+Alt+C |
| Edit | Paste style | Ctrl+Alt+V |
| Edit | Duplicate with next number | Ctrl+Alt+D |
| Edit | Delete | Delete, Backspace |
| Edit | Duplicate | Ctrl+D |
| Edit | Clear selection | Escape |
| Edit | Select all | Ctrl+A |
| View | Zoom in | Ctrl+Equals, Ctrl+Plus |
| View | Zoom out | Ctrl+Minus |
| View | Zoom to 100 % | Ctrl+0 |
| View | Fit page | Home, Shift+1 |
| View | Fit selection | Shift+2 |
| View | Show grid | Shift+G |
| View | Show rulers | Shift+R |
| View | Snap | Shift+S |
| View | Issues | Ctrl+Shift+I |
| View | Message log | Ctrl+Shift+M |
| View | Command palette… | Ctrl+K, Ctrl+Shift+P |
| View | Check drawing | F7 |
| Insert | Select | V |
| Insert | Hand (pan) | H |
| Insert | Horizontal dimension | D |
| Insert | Vertical dimension | Shift+D |
| Insert | Aligned dimension | Alt+Shift+D |
| Insert | Ordinate dimension | O |
| Insert | Callout (number) | B |
| Insert | Note list | N |
| Insert | Text | T |
| Insert | Line | L |
| Insert | Rectangle | R |
| Insert | Polyline | P |
| Insert | Circle | C |
| Arrange | Bring forward | Ctrl+CloseBracket |
| Arrange | Bring to front | Ctrl+Shift+CloseBracket |
| Arrange | Send backward | Ctrl+OpenBracket |
| Arrange | Send to back | Ctrl+Shift+OpenBracket |
| Arrange | Align left | Alt+A |
| Arrange | Align horizontal centres | Alt+H |
| Arrange | Align right | Alt+D |
| Arrange | Align top | Alt+W |
| Arrange | Align vertical centres | Alt+V |
| Arrange | Align bottom | Alt+S |
| Arrange | Distribute horizontally | Alt+Shift+H |
| Arrange | Distribute vertically | Alt+Shift+V |
| Page | Previous page | PageUp |
| Page | Next page | PageDown |
| Page | Move page up | Alt+Up |
| Page | Move page down | Alt+Down |
| Page | Add page | Ctrl+Shift+N |
| Help | Keyboard shortcuts | Ctrl+Slash |

Canvas gestures: wheel zooms at the cursor; middle drag or Space+drag
pans; Shift+click adds or removes; drag on empty paper makes a marquee
(items wholly inside); arrows nudge by the snap grid (Shift 10x); Alt
while dragging a view's corner handle scales freely (else the standard
scales); double click zooms to an item.

## Files

Open, Save as, Export PDF/PNG/SVG and the drawing sheet use native file
dialogs through `rfd` 0.18 on their own thread (the UI keeps running):
the Windows common item dialog (windows-sys, no extra setup) and the XDG
desktop portal on Linux. With `KDRAFT_TYPED_PATHS=1` (and in tests) a
typed-path prompt is used instead. Files can also be dropped onto the
window. Recent files (10) are in the File menu and on the start screen.
The theme follows the Windows app setting until changed (see Settings).

## Settings

File > Settings… (Ctrl+Comma), the gear in the top strip, the kicad-cli
entry in the status bar or the start screen open the settings page in
place of the drawing; Close or Escape goes back. Sections:

- **General**: theme, rulers, grid, snap (language: later).
- **KiCad**: the kicad-cli in use and where it came from; every install
  found (PATH and `C:\Program Files\KiCad\<version>`) with its version,
  "Auto (newest)", or a custom path (Browse… or typed); Test runs
  `kicad-cli version`. Extra variables: `NAME=VALUE` pairs passed as
  `--define-var` to `pcb export svg` and `pcb render` (both accept it per
  their 10.0.8 help texts), for custom library path variables. Changing
  the install refreshes plots in the background (cache keys include the
  kicad-cli version and arguments).
- **Documents**: defaults for new documents and pages: paper and
  orientation, drawing sheet (used when the project sets none), layer
  palette, 3D resolution and quality, callout numbering, snap grid; and
  the open document's palette.
- **Layer colours**: palettes and KiCad theme import (see Layer colours).
- **Cache**: `.kdraft-cache` next to each document (default) or one folder
  for all, its size, Clear cache. No size limit (no eviction).

Changes apply at once and are saved.

**Where things are stored.** User settings live in one file shared by the
editor and the `kdraft` CLI: `%APPDATA%\kdraft\settings.json` on Windows,
`$XDG_CONFIG_HOME/kdraft/settings.json` or `~/.config/kdraft/settings.json`
elsewhere; `--settings FILE` (CLI and editor) or `KDRAFT_SETTINGS` choose
another file. Sections: `kicad`, `documents`, `palettes` (shared) and `ui`
(the editor's: theme, recent files, window size/position/maximised, panel
widths, dock height and tab, last tool variants per group, and per
document its last page and zoom/pan per page, keyed by the document's
path and pruned with the recent files). The `.kdraft` file holds only the
drawing (plus its own palette colours), never view state, so drawings stay
clean in version control. Unknown keys are kept; writes are atomic; a
broken file is kept as `settings.json.bad` and the defaults are used
(one warning). The window is restored where it was; if that place is on
no monitor any more it opens at the default size, centred. eframe's own
persistence is not used (it is not enabled and would add a second file).

**kicad-cli precedence**, everywhere: `--kicad-cli` on the command line,
then `KDRAFT_KICAD_CLI`, then the `kicad_cli` setting, then the newest
install found (`kdraft kicad list` shows them).

**Reset**: delete the file, or `kdraft settings reset`.

**Privacy**: the file contains local paths (recent files, kicad-cli, cache
folder); nothing is sent anywhere.

## Headless screenshots

`kdraft-gui --screenshot out.png [--size 1400x900] [--ppp 1]
[--theme dark|light] [--settings file] [--script steps.json] [file]`
runs the editor without a window and rasterises egui's meshes in software
(`crates/kdraft-ui/src/shot.rs`). Script steps: `{"click":[x,y]}`,
`{"right_click":[x,y]}`, `{"double_click":[x,y]}`, `{"move":[x,y]}`,
`{"drag":[[x0,y0],[x1,y1]],"hold":true}`, `{"press":[x,y]}`,
`{"release":[x,y]}` (a drag recorded frame by frame, with `move` and
`shot` steps in between), `{"key":"K","mods":"ctrl"}`,
`{"text":"..."}`, `{"cmd":"name","args":{}}`, `{"ui":"fit_page"}`,
`{"wait":ms}`, `{"settle":true}`, `{"shot":"name.png"}`.
`--fake-kicad DIR` answers kicad-cli from recorded outputs (the SVG
plots in `fixtures/kicad-cli-svg` and a stand-in 3D picture), for
screenshots of plotted, coloured and 3D views without KiCad.
`--gallery out.png` draws the icon set; `--bench N` prints frame times;
`--shortcuts-md` prints the table above.

## What to test by hand on Windows

1. Start without arguments: start screen; theme follows Windows; the
   moon/sun button switches and is remembered after a restart.
2. File > Open… (native dialog), New from board…, Save as…, Export PDF…:
   the dialogs open in front of the window, the editor stays responsive
   while they are open, cancel does nothing.
3. Menus: every entry with an icon and shortcut; disabled entries greyed;
   Ctrl+K palette: type "exp pdf", Enter.
4. Pages: click selects, drag reorders, double click renames, right click
   menu. Item tree: click/Ctrl/Shift, F2 rename, eye and lock toggles,
   drag to restack.
5. Canvas: rulers and grid follow zoom/pan; hover highlight; Alt-hover
   gaps; dimensions follow a dragged view without lag; a dimension can be
   clicked on its line or text and dragged to change its offset.
6. Tools: D, click two board corners, click the offset; B for a callout;
   place an assembly view; Escape.
7. Inspector: type `100+5` in X, scrub the X letter, layer preset chips,
   filter, "Show all … layers".
8. Issues: move a view off the sheet; the status bar count changes after
   a moment; open the dock and click the issue.
9. With kicad-cli: BOM tables switch to the schematic BOM after it has
   been fetched in the background (status bar: jobs). Verified on Windows
   with kicad-cli 10.0.8: "BOM from the schematic: 62 lines", inspector
   "BOM: from the schematic".
10. Note anything that looks wrong: clipped text, wrong fonts (glyphs
    missing from IBM Plex Sans show as boxes), sluggish frames, controls
    that do nothing, contrast problems in either theme.

## Known limits

- Never run on Windows in this form; native dialogs and real pointer
  feel unverified.
- Views and tables are one texture each (at most 4096 px per side, or
  the GPU limit); very high zoom blurs them.
- Live annotation text uses egui's glyph atlas; above 600 px font size it
  is not drawn (extreme zoom into one label).
- No multi-page selection; no undo for page selection; free graphics
  cannot be resized on the canvas (inspector only).
- File watching polls the board file once a second; project file and
  drawing sheets are not watched.
