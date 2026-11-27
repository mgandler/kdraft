#!/usr/bin/env python3
"""Records the animations in the README with the editor's headless mode.

    scripts/fetch-fixtures.sh                    # the demo board (once)
    cargo build --release -p kdraft-ui
    python3 scripts/readme-gifs/record.py        # all scenes -> docs/img/*.gif

Needs Python 3 with Pillow, and ffmpeg on PATH. Each scene is a script for
`kdraft-gui --screenshot --script` (see docs/GUI.md, Headless screenshots):
the editor runs without a window, every `shot` step is one frame. The
pointer is not part of those frames; it is drawn afterwards from the
positions the script used, together with the caption bar.

Scenes whose names start with `kicad-` need kicad-cli (plots, 3D renders)
and are not recorded by default; name them to record them. With kicad-cli
installed the other scenes show KiCad's plots instead of the preview.

Without Python on the machine that has KiCad: `--emit DIR` writes the
scripts into DIR/<scene>/, capture.cmd (Windows) runs kdraft-gui on each
of them, and `--assemble DIR` turns the captured frames into GIFs later,
here or elsewhere. The KiCad project is expected in DIR/demo/.

The board is KiCad's demo "RoyalBlue54L Feather" (Lord's Boards,
CERN-OHL-P v2), fetched by scripts/fetch-fixtures.sh.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
W, H = 1200, 760  # logical size of the editor
FPS = 20  # frames per second while something moves
BAR = 38  # caption bar under the frame, px
BOARD = "RoyalBlue54L-Feather"
START = f"../demo/{BOARD}.kicad_pcb"  # seen from a scene's folder
DEMO = ROOT / "fixtures-ext" / "10"

CURSOR = [(0, 0), (0, 17), (4, 13), (7, 20), (10, 19), (7, 12), (12, 12)]


class Rec:
    """Collects script steps and, per shot, what to draw on the frame."""

    def __init__(self, name, start):
        self.name = name
        self.start = start  # file the editor opens
        self.steps = []
        self.frames = []
        self.pointer = (W * 0.55, H * 0.55)
        self.caption = ""
        self.show_pointer = True

    def raw(self, **step):
        self.steps.append(step)

    def shot(self, sec, click=False):
        n = len(self.frames)
        self.raw(shot=f"f{n:04d}.png")
        self.frames.append(
            {
                "file": f"f{n:04d}.png",
                "sec": sec,
                "pointer": list(self.pointer) if self.show_pointer else None,
                "click": click,
                "caption": self.caption,
            }
        )

    def park(self, x, y):
        """Puts the pointer somewhere without recording the way there."""
        self.pointer = (x, y)
        self.raw(move=[x, y])

    def say(self, text):
        self.caption = text

    def hold(self, sec):
        self.shot(sec)

    def settle(self):
        self.raw(settle=True)

    def wait(self, ms):
        self.raw(wait=ms)

    def _path(self, x, y, sec):
        x0, y0 = self.pointer
        n = max(2, round(sec * FPS))
        for k in range(1, n + 1):
            t = k / n
            e = t * t * (3 - 2 * t)  # ease in and out
            yield x0 + (x - x0) * e, y0 + (y - y0) * e

    def move(self, x, y, sec=0.5):
        for p in self._path(x, y, sec):
            self.pointer = p
            self.raw(move=[round(p[0], 1), round(p[1], 1)])
            self.shot(1 / FPS)

    def click(self, x, y, sec=0.5, settle=True):
        self.move(x, y, sec)
        self.raw(click=[x, y])
        if settle:
            self.settle()
        self.shot(0.15, click=True)

    def drag(self, x0, y0, x1, y1, sec=1.0, approach=0.5):
        self.move(x0, y0, approach)
        self.raw(press=[x0, y0])
        self.shot(0.15, click=True)
        for p in self._path(x1, y1, sec):
            self.pointer = p
            self.raw(move=[round(p[0], 1), round(p[1], 1)])
            self.shot(1 / FPS)
        self.raw(release=[x1, y1])
        self.settle()
        self.shot(0.15)

    def key(self, name, mods=""):
        step = {"key": name}
        if mods:
            step["mods"] = mods
        self.steps.append(step)

    def type(self, text, per=0.08):
        for ch in text:
            self.raw(text=ch)
            self.shot(per)

    def cmd(self, command, /, **args):
        self.raw(cmd=command, args=args)

    def ui(self, name):
        self.raw(ui=name)


# --- scenes -----------------------------------------------------------------
#
# Positions are logical pixels in a 1200 x 760 editor with default settings
# and the demo board. PAGE_Y are the page thumbnails, top to bottom.

PAGE_X, PAGE_Y = 46, [107, 180, 253]


def scene_set():
    r = Rec("drawing-set", START)
    r.wait(2500)
    r.settle()
    r.ui("clear_toasts")
    r.park(640, 630)
    r.say("kdraft-gui RoyalBlue54L-Feather.kicad_pcb: a drawing set from the board file")
    r.hold(2.2)
    r.say("Assembly top: reference designators are placed without overlaps")
    r.click(PAGE_X + 90, PAGE_Y[1])
    r.hold(1.0)
    r.click(590, 330)
    r.ui("fit_selection")
    r.settle()
    r.hold(2.4)
    r.key("Escape")
    r.ui("fit_page")
    r.settle()
    r.say("Assembly bottom, seen from the bottom")
    r.click(PAGE_X + 90, PAGE_Y[2])
    r.hold(1.6)
    r.say("Drill drawing with one symbol per tool, drill table, layer stack")
    r.key("PageDown")
    r.settle()
    r.hold(2.4)
    r.say("Bill of materials (from the schematic when kicad-cli is installed)")
    r.key("PageDown")
    r.settle()
    r.hold(2.4)
    r.say("Fabrication drawing with board dimensions and notes")
    r.click(PAGE_X + 90, PAGE_Y[0])
    r.hold(2.0)
    return r


def _open(r, pages_down=0):
    """Common start: document open, toasts gone, `pages_down` pages on."""
    r.wait(2500)
    r.settle()
    r.ui("clear_toasts")
    for _ in range(pages_down):
        r.key("PageDown")
    r.settle()


def _zoom_to_view(r):
    r.raw(click=[590, 330])
    r.ui("fit_selection")
    r.settle()
    r.wait(600)
    r.settle()
    r.park(700, 610)


def scene_annotate():
    r = Rec("annotate", START)
    _open(r, 1)
    _zoom_to_view(r)
    r.say("Callouts (B): pick a part, place the balloon. The numbers count up.")
    r.hold(1.2)
    r.key("B")
    for part, balloon in [((486, 425), (455, 565)), ((584, 390), (590, 565)), ((660, 447), (725, 565))]:
        r.click(*part, sec=0.6, settle=False)
        r.hold(0.25)
        r.click(*balloon, sec=0.5, settle=False)
        r.hold(0.5)
    r.key("Escape")
    r.key("Escape")
    r.say("Dimensions (D) snap to pads, holes and outline points")
    r.key("D")
    r.click(343, 313, sec=0.7, settle=False)
    r.hold(0.3)
    r.click(840, 313, sec=0.9, settle=False)
    r.hold(0.3)
    r.click(600, 258, sec=0.6, settle=False)
    r.hold(1.2)
    r.key("Escape")
    r.key("V")
    r.say("They are attached to the board: move the view and everything follows")
    r.drag(770, 480, 700, 440, sec=1.3, approach=0.7)
    r.hold(0.4)
    r.drag(700, 440, 790, 500, sec=1.3, approach=0.1)
    r.hold(2.2)
    return r


ALIGN_TOP, DISTRIBUTE_H = (1007, 95), (1093, 95)


def scene_arrange():
    r = Rec("arrange", START)
    _open(r, 3)
    # Before the first frame: a smaller drill drawing, and the three items
    # scattered so that there is something to arrange.
    r.raw(click=[500, 375])
    r.cmd("item.set_scale", scale=2)
    r.cmd("item.move", delta=[-64.5, 10])
    r.raw(click=[720, 256])
    r.cmd("item.move", delta=[-111, 75])
    r.raw(click=[745, 385])
    r.cmd("item.move", delta=[0, -20])
    r.key("Escape")
    r.settle()
    r.wait(600)
    r.settle()
    r.park(600, 640)
    r.say("Select several items (drag a box, or Ctrl+A)")
    r.hold(1.4)
    r.drag(322, 222, 853, 568, sec=1.0, approach=0.6)
    r.hold(0.6)
    r.say("Align top")
    r.click(*ALIGN_TOP, sec=0.7)
    r.hold(1.3)
    r.say("Distribute horizontally: equal gaps")
    r.click(*DISTRIBUTE_H, sec=0.5)
    r.hold(1.6)
    r.say("Pages are reordered by dragging")
    r.drag(130, 246, 130, 80, sec=1.2, approach=0.8)
    r.hold(2.4)
    return r


PALETTE = (1057, 625)
PALETTE_ROW = {"kdraft layer colours": 461, "KiCad Default (paper)": 515}
THEME_BUTTON = (1179, 19)


def scene_colours():
    r = Rec("colours", START)
    _open(r, 1)
    _zoom_to_view(r)
    r.say("Layer colours per view or per document")
    r.hold(1.2)
    r.click(*PALETTE, sec=0.7)
    r.hold(0.7)
    r.click(1040, PALETTE_ROW["kdraft layer colours"], sec=0.5)
    r.wait(500)
    r.settle()
    r.hold(2.2)
    r.say("KiCad's own themes, adapted for white paper; or import a theme from KiCad")
    r.click(*PALETTE, sec=0.6)
    r.hold(0.7)
    r.click(1040, PALETTE_ROW["KiCad Default (paper)"], sec=0.5)
    r.wait(500)
    r.settle()
    r.hold(2.2)
    r.say("Light and dark")
    r.click(*THEME_BUTTON, sec=0.9)
    r.wait(500)
    r.settle()
    r.hold(2.4)
    return r


def _renders(r):
    """Waits for kicad-cli: `settle` gives up after 30 s, a render may not."""
    for _ in range(4):
        r.settle()
        r.wait(400)
    r.settle()


def scene_kicad_3d():
    r = Rec("kicad-3d", START)
    _open(r, 0)
    r.cmd("page.add", title="3D views")
    r.cmd("item.add_view", kind="render3d")
    _renders(r)
    r.raw(click=[592, 395])
    r.ui("fit_selection")
    _renders(r)
    r.ui("clear_toasts")
    r.show_pointer = False
    r.say("3D views are rendered by KiCad (kicad-cli pcb render)")
    r.hold(2.6)
    r.say("Camera presets: isometric")
    r.cmd("view.set_render3d", preset="isometric")
    _renders(r)
    r.hold(2.6)
    r.say("Camera presets: isometric, bottom side")
    r.cmd("view.set_render3d", preset="isometric_bottom")
    _renders(r)
    r.hold(2.6)
    return r


def scene_kicad_layers():
    r = Rec("kicad-layers", START)
    _open(r, 0)
    r.cmd("view.pages_per_layer")
    r.cmd("document.set_layer_palette", name="KiCad Default (paper)")
    _renders(r)
    for _ in range(12):
        r.key("PageUp")
    r.key("PageDown")
    _renders(r)
    r.ui("clear_toasts")
    r.say("Insert > Layer pages...: one page per copper layer, plotted by KiCad")
    r.hold(2.0)
    for _ in range(3):
        r.key("PageDown")
        _renders(r)
        r.hold(1.3)
    for _ in range(3):
        r.key("PageUp")
    _renders(r)
    r.say("The views are linked: move the first one and the others follow")
    r.hold(0.8)
    r.park(700, 560)
    r.drag(592, 395, 660, 340, sec=1.2, approach=0.7)
    _renders(r)
    r.hold(1.0)
    for _ in range(2):
        r.key("PageDown")
        _renders(r)
        r.hold(1.5)
    return r


SCENES = {
    "drawing-set": scene_set,
    "annotate": scene_annotate,
    "arrange": scene_arrange,
    "colours": scene_colours,
    "kicad-3d": scene_kicad_3d,
    "kicad-layers": scene_kicad_layers,
}


# --- recording ---------------------------------------------------------------


def gui_binary():
    env = os.environ.get("KDRAFT_GUI")
    if env:
        return Path(env)
    exe = ".exe" if os.name == "nt" else ""
    for profile in ("release", "debug"):
        p = ROOT / "target" / profile / f"kdraft-gui{exe}"
        if p.exists():
            return p
    sys.exit("kdraft-gui not built: cargo build --release -p kdraft-ui")


def prepare(work):
    """An empty scene folder, and the demo project next to it in demo/.

    A demo/ folder that is already there is used as it is, so the complete
    KiCad project (schematic, 3D models) can be put there for the scenes
    that run kicad-cli.
    """
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    demo = work.parent / "demo"
    if (demo / f"{BOARD}.kicad_pcb").exists():
        return
    demo.mkdir(parents=True, exist_ok=True)
    for ext in ("kicad_pcb", "kicad_pro"):
        src = DEMO / f"royalblue54L_feather__{BOARD}.{ext}"
        if not src.exists():
            sys.exit(f"{src} is missing: run scripts/fetch-fixtures.sh")
        shutil.copy(src, demo / f"{BOARD}.{ext}")


def emit(rec, work):
    (work / "steps.json").write_text(json.dumps(rec.steps))
    meta = {"start": rec.start, "size": [W, H], "frames": rec.frames}
    (work / "frames.json").write_text(json.dumps(meta, indent=1))


def capture(rec, work, extra):
    cmd = [
        str(gui_binary()),
        "--screenshot",
        "last.png",
        "--size",
        f"{W}x{H}",
        "--theme",
        "light",
        "--settings",
        "settings.json",
        "--script",
        "steps.json",
        *extra,
        rec.start,
    ]
    subprocess.run(cmd, cwd=work, check=True)


def assemble(work, out, only=None):
    from PIL import Image, ImageDraw, ImageFont

    meta = json.loads((work / "frames.json").read_text())
    font = ImageFont.truetype(str(ROOT / "assets/fonts/IBMPlexSans-Regular.ttf"), 17)
    done = work / "out"
    done.mkdir(exist_ok=True)
    lines = ["ffconcat version 1.0"]
    last = None
    for i, f in enumerate(meta["frames"]):
        if only is not None and i not in only:
            continue
        src = Image.open(work / f["file"]).convert("RGB")
        if src.size != (W, H):
            src = src.resize((W, H), Image.LANCZOS)
        im = Image.new("RGB", (W, H + BAR), (27, 31, 40))
        im.paste(src, (0, 0))
        d = ImageDraw.Draw(im, "RGBA")
        if f["pointer"]:
            x, y = f["pointer"]
            if f["click"]:
                d.ellipse([x - 13, y - 13, x + 13, y + 13], fill=(37, 99, 235, 70), outline=(37, 99, 235, 230), width=2)
            pts = [(x + px * 1.15, y + py * 1.15) for px, py in CURSOR]
            d.polygon(pts, fill=(255, 255, 255, 255))
            d.line(pts + [pts[0]], fill=(20, 20, 20, 255), width=2, joint="curve")
        if f["caption"]:
            d.text((16, H + BAR / 2), f["caption"], font=font, fill=(240, 242, 246, 255), anchor="lm")
        name = f"p{i:04d}.png"
        im.save(done / name)
        lines += [f"file {name}", f"duration {f['sec']:.3f}"]
        last = name
    if only is not None:
        return
    lines.append(f"file {last}")  # the concat demuxer needs the last file twice
    (done / "frames.txt").write_text("\n".join(lines) + "\n")
    out.parent.mkdir(parents=True, exist_ok=True)
    vf = (
        "split[a][b];[a]palettegen=max_colors=160:stats_mode=diff[p];"
        "[b][p]paletteuse=dither=none:diff_mode=rectangle"
    )
    subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-y", "-f", "concat", "-i", "frames.txt",
         "-vf", vf, "-fps_mode", "vfr", "-loop", "0", str(out.resolve())],
        cwd=done,
        check=True,
    )
    secs = sum(f["sec"] for f in meta["frames"])
    print(f"{out}: {len(meta['frames'])} frames, {secs:.1f} s, {out.stat().st_size / 1e6:.2f} MB")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("scenes", nargs="*", help=f"from {', '.join(SCENES)}; default: those not named kicad-*")
    ap.add_argument("--work", default=str(ROOT / "target" / "readme-gifs"), help="scratch folder")
    ap.add_argument("--out", default=str(ROOT / "docs" / "img"), help="where the GIFs go")
    ap.add_argument("--emit", metavar="DIR", help="only write the scripts and frame lists into DIR/<scene>/")
    ap.add_argument("--assemble", metavar="DIR", help="only build GIFs from frames captured in DIR/<scene>/")
    ap.add_argument("--fake-kicad", metavar="DIR", help="pass --fake-kicad DIR to the editor (rehearsal only)")
    ap.add_argument("--peek", metavar="N,N,...", help="only post-process these frame numbers, no GIF")
    a = ap.parse_args()
    names = a.scenes or [n for n in SCENES if not n.startswith("kicad-")]
    for n in names:
        if n not in SCENES:
            sys.exit(f"unknown scene {n}; there are: {', '.join(SCENES)}")
    for n in names:
        rec = SCENES[n]()
        if a.emit:
            work = Path(a.emit) / n
            work.mkdir(parents=True, exist_ok=True)
            emit(rec, work)
            continue
        if a.assemble:
            work = Path(a.assemble) / n
            emit(rec, work)
        else:
            work = Path(a.work) / n
            prepare(work)
            emit(rec, work)
            capture(rec, work, ["--fake-kicad", a.fake_kicad] if a.fake_kicad else [])
        only = {int(x) for x in a.peek.split(",")} if a.peek else None
        assemble(work, Path(a.out) / f"{n}.gif", only)


if __name__ == "__main__":
    main()
