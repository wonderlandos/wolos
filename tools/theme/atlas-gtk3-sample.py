#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Reads the colours the Atlas engine samples from a WolOS GTK 3 theme with real GTK 3 and compares them with the tokens.
"""Sample a WolOS GTK 3 theme the way the Atlas engine does and compare with the expected tokens.

The engine (`ui/gtk/gtk_util.cc`: GetBgColor, GetFgColor, GetBorderColor, GetSeparatorColor,
GtkStyleContextLookupColor) builds a GtkStyleContext for a CSS selector rooted in `window.background`,
gives every node the class `chromium`, renders the node into a 24 x 24 bitmap and averages it. This
script does the same through PyGObject, for the rows of `crates/core/src/theme/gtk_atlas.rs`.

  WOL_DUMP_GTK3=/tmp/g cargo test -p core dump_for_the_sampler     # the theme files and <id>.json
  python3 -I tools/theme/atlas-gtk3-sample.py /tmp/g wol-dark --adw /path/with/adw-gtk3 [--bare] [--json]

--adw is the directory that holds `adw-gtk3` and `adw-gtk3-dark` (the theme's import is pointed at it).
--no-import drops the adw-gtk3 import (the border rows must hold on their own). --bare drops the Atlas rules from the theme: what the engine would read from adw-gtk3 and the WolOS
colour definitions alone. Needs a display: run it under `broadwayd :7` with BROADWAY_DISPLAY=:7
GDK_BACKEND=broadway, or in a session. Exit 1 when a row differs by more than 1 per channel (2 for a frame: rounded corners).
"""
import argparse
import json
import os
import re
import shutil
import sys
import tempfile


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dump")
    ap.add_argument("theme_id")
    ap.add_argument("--adw", default="/usr/share/themes")
    ap.add_argument("--bare", action="store_true")
    ap.add_argument("--no-import", action="store_true", help="drop the adw-gtk3 import: the rules alone")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    spec = json.load(open(os.path.join(a.dump, a.theme_id + ".json")))
    name = "WolOS-%s-gtk3" % a.theme_id
    home = tempfile.mkdtemp(prefix="atlas-gtk3-")
    try:
        src = os.path.join(a.dump, "themes", name, "gtk-3.0")
        dst = os.path.join(home, "themes", name, "gtk-3.0")
        os.makedirs(dst)
        for f in ("gtk.css", "gtk-dark.css"):
            css = open(os.path.join(src, f)).read()
            css = css.replace("file:///usr/share/themes", "file://" + os.path.abspath(a.adw))
            if a.no_import:
                css = "\n".join(l for l in css.splitlines() if not l.startswith("@import"))
            if a.bare:
                css = css.split("/* Atlas: the colours the engine samples")[0]
            open(os.path.join(dst, f), "w").write(css)
        os.environ["XDG_DATA_HOME"] = home
        os.environ["XDG_DATA_DIRS"] = home + ":" + os.environ.get("XDG_DATA_DIRS", "/usr/share")
        results = sample(spec, name)
    finally:
        shutil.rmtree(home, ignore_errors=True)

    bad = 0
    out = []
    for r, got in zip(spec["rows"], results):
        want = r["expect"]
        # A rounded frame (menus: 9 px) is anti-aliased at the corners: 2 levels of 255 there, 1 elsewhere.
        tol = 2 if r["read"] == "border" else 1
        ok = got is not None and all(abs(int(want[i:i + 2], 16) - int(got[i:i + 2], 16)) <= tol for i in (1, 3, 5))
        bad += not ok
        out.append(dict(r, got=got, ok=ok))
    if a.json:
        print(json.dumps(out, indent=1))
    else:
        for o in out:
            print("%-5s %-38s %-9s want %s got %-8s  %s" % ("ok" if o["ok"] else "DIFF", o["engine"], o["read"],
                                                           o["expect"], o["got"], o["selector"][:60]))
        print("%d of %d rows differ (%s, %s)" % (bad, len(out), a.theme_id, "bare" if a.bare else "with the Atlas rules"))
    return 1 if bad else 0


def sample(spec, theme_name):
    import gi

    gi.require_version("Gtk", "3.0")
    gi.require_version("Gdk", "3.0")
    from gi.repository import GObject, Gtk
    import ctypes

    # Rendering goes through libgtk and libcairo directly: PyGObject's cairo bridge (python3-gi-cairo)
    # is not on every machine, the C libraries are.
    gtk = ctypes.CDLL("libgtk-3.so.0")
    cairo = ctypes.CDLL("libcairo.so.2")
    vp, ci, cd = ctypes.c_void_p, ctypes.c_int, ctypes.c_double
    cairo.cairo_image_surface_create.restype = vp
    cairo.cairo_image_surface_create.argtypes = [ci, ci, ci]
    cairo.cairo_create.restype = vp
    cairo.cairo_create.argtypes = [vp]
    cairo.cairo_surface_flush.argtypes = [vp]
    cairo.cairo_image_surface_get_data.restype = ctypes.POINTER(ctypes.c_ubyte)
    cairo.cairo_image_surface_get_data.argtypes = [vp]
    cairo.cairo_destroy.argtypes = [vp]
    cairo.cairo_surface_destroy.argtypes = [vp]
    for fn in (gtk.gtk_render_background, gtk.gtk_render_frame):
        fn.argtypes = [vp, vp, cd, cd, cd, cd]

    class Surface:
        def __init__(self, w, h):
            self.w, self.h = w, h
            self.s = cairo.cairo_image_surface_create(0, w, h)  # CAIRO_FORMAT_ARGB32
            self.cr = cairo.cairo_create(self.s)

        def render(self, fn, ctx):
            fn(hash(ctx), self.cr, 0.0, 0.0, float(self.w), float(self.h))

        def data(self):
            cairo.cairo_surface_flush(self.s)
            return bytes(cairo.cairo_image_surface_get_data(self.s)[: self.w * self.h * 4])

        def close(self):
            cairo.cairo_destroy(self.cr)
            cairo.cairo_surface_destroy(self.s)

    if not Gtk.init_check()[0]:
        raise SystemExit("no display: start broadwayd and set GDK_BACKEND=broadway BROADWAY_DISPLAY")
    settings = Gtk.Settings.get_default()
    settings.set_property("gtk-theme-name", theme_name)
    settings.set_property("gtk-application-prefer-dark-theme", bool(spec["dark"]))

    states = {
        "active": Gtk.StateFlags.ACTIVE, "hover": Gtk.StateFlags.PRELIGHT, "selected": Gtk.StateFlags.SELECTED,
        "disabled": Gtk.StateFlags.INSENSITIVE, "indeterminate": Gtk.StateFlags.INCONSISTENT,
        "focus": Gtk.StateFlags.FOCUSED,  # focus-within is GTK 4 only; the engine drops it on GTK 3 too
        "backdrop": Gtk.StateFlags.BACKDROP, "link": Gtk.StateFlags.LINK, "visited": Gtk.StateFlags.VISITED,
        "checked": Gtk.StateFlags.CHECKED,
    }

    def node(parent, text):
        """AppendCssNodeToStyleContext (GTK 3 branch): object name, `.class`, `:state`, `(widget name)`."""
        name, obj, classes, state = "", "", [], Gtk.StateFlags.NORMAL
        part = "obj"
        for tok in re.findall(r"[.:()]|[^.:()]+", text):
            if tok in ".:()":
                part = {"(": "name", ")": "none", ".": "class", ":": "pseudo"}[tok]
            elif part == "obj":
                obj = tok
            elif part == "name":
                name = tok
            elif part == "class":
                classes.append(tok)
            elif part == "pseudo":
                state |= states.get(tok, Gtk.StateFlags.NORMAL)
        classes.append("chromium")
        path = parent.get_path().copy() if parent else Gtk.WidgetPath.new()
        path.append_type(GObject.TYPE_NONE)
        if obj:
            path.iter_set_object_name(-1, obj)
        if name:
            path.iter_set_name(-1, name)
        for c in classes:
            path.iter_add_class(-1, c)
        path.iter_set_state(-1, state)
        ctx = Gtk.StyleContext.new()
        ctx.set_path(path)
        ctx.set_state(state)
        if parent:
            ctx.set_parent(parent)
        return ctx

    def context(selector):
        ctx = node(None, "window.background")
        for part in selector.split():
            ctx = node(ctx, part)
        return ctx

    def chain(ctx):
        out = []
        while ctx:
            out.append(ctx)
            ctx = ctx.get_parent()
        return out[::-1]

    strip = Gtk.CssProvider()
    strip.load_from_data(b"* { border-radius: 0px; border-style: none; box-shadow: none; }")

    def average(surface, frame):
        data = surface.data()
        surface.close()
        a = r = g = b = mx = 0
        for i in range(0, len(data), 4):
            pb, pg, pr, pa = data[i], data[i + 1], data[i + 2], data[i + 3]
            a += pa
            r += pr
            g += pg
            b += pb
            mx = max(mx, pa)
        if a == 0:
            return (0, 0, 0, 0)
        n = len(data) // 4
        return (r * 255 // a, g * 255 // a, b * 255 // a, mx if frame else a // n)

    def over(fg, bg):
        al = fg[3] / 255.0
        return tuple(round(fg[i] * al + bg[i] * (1 - al)) for i in range(3)) + (255,)

    def bg_of(ctx, size=24, w=None, h=None):
        w, h = w or size, h or size
        for c in chain(ctx):
            c.add_provider(strip, 4294967295)
        s = Surface(w, h)
        for c in chain(ctx):
            s.render(gtk.gtk_render_background, c)
        return average(s, False)

    def hexed(c):
        return "#%02x%02x%02x" % c[:3]

    res = []
    for r in spec["rows"]:
        read, sel = r["read"], r["selector"]
        try:
            if read == "named":
                ctx = context("entry")
                ok, rgba = ctx.lookup_color(sel)
                res.append(hexed(tuple(round(v * 255) for v in (rgba.red, rgba.green, rgba.blue))) if ok else None)
            elif read == "bg":
                res.append(hexed(bg_of(context(sel))))
            elif read == "fg":
                ctx = context(sel)
                c = ctx.get_color(ctx.get_state())
                fg = (round(c.red * 255), round(c.green * 255), round(c.blue * 255), round(c.alpha * 255))
                res.append(hexed(fg if fg[3] == 255 else over(fg, bg_of(ctx))))
            elif read == "border":
                ctx = context(sel)
                s = Surface(24, 24)
                s.render(gtk.gtk_render_frame, ctx)
                b = average(s, True)
                res.append(hexed(b if b[3] == 255 else over(b, bg_of(ctx))))
            elif read == "separator":
                ctx = context(sel)
                st = ctx.get_state()
                w = ctx.get_property("min-width", st)
                h = ctx.get_property("min-height", st)
                bd, pd = ctx.get_border(st), ctx.get_padding(st)
                w += bd.left + pd.left + pd.right + bd.right
                h += bd.top + pd.top + pd.bottom + bd.bottom
                horizontal = ctx.has_class("horizontal") or "horizontal" in sel
                if horizontal:
                    w, h = 24, max(h, 1)
                else:
                    h, w = 24, max(w, 1)
                s = Surface(w, h)
                s.render(gtk.gtk_render_background, ctx)
                s.render(gtk.gtk_render_frame, ctx)
                res.append(hexed(average(s, False)))
        except Exception as e:  # noqa: BLE001 - one row failing must not hide the others
            print("row %s failed: %s" % (sel, e), file=sys.stderr)
            res.append(None)
    return res


if __name__ == "__main__":
    sys.exit(main())
