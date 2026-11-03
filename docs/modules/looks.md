# Looks (org.wolos.looks)

The app that makes a theme for the whole desktop from a picture: a Wolkit app in Crystal (ADR 0039) on the `wol-hue` palette engine (ADR 0031, [hue.md](hue.md)). Source `wolos-crystal/apps/looks/`, program `wol-looks`, package `wolos-app-looks`. The look and the reasons are in the design note, `wolos-crystal/apps/looks/DESIGN.md`.

## Why

`wol-hue` turns a picture into a theme, but a command line cannot show eight possible results next to each other, move a slider and watch every one of them change, or look at a terminal, Settings, a document and a browser in the colours that are about to be applied. Looks is that window. It adds no palette code: the picture reading, the eight modes, the twelve sliders, the contrast contract and the theme writer are `hue/src/hue/*.cr`, required by path, so what the window previews is what `wol-hue` would write.

## What it does

- **Pick a picture.** The wallpaper folders (`/usr/share/wolos/wallpapers`, `~/.local/share/wolos/wallpapers`) and the Pictures folder as 96 x 64 thumbnails (kept in `~/.cache/org.wolos.looks/thumbnails`), Previous and Next (Page Up, Page Down), or Choose Picture… (Ctrl+O), the system file panel: the app gets the open file, never a path, and keeps a copy under `~/.local/share/org.wolos.looks/pictures/`.
- **Eight modes, light and dark, side by side.** One card per mode of the engine (`normal`, `monochromatic`, `analogous`, `pastel`, `material`, `colorful`, `muted`, `bright`; shown as Natural, Monochrome, Analogous, Pastel, Material, Colourful, Muted, Bright). Each card is the dark palette on the left and the light one on the right: the 16 terminal colours, then a small window in the 13 tokens.
- **Five sliders with instant preview.** Vibrance, contrast, temperature, shadows, highlights (the engine's ranges); a move recolours all 16 candidates and the previews in 80 to 150 ms.
- **A per-app strip.** A terminal, a Settings page, a document and a browser-like frame drawn with the chosen candidate's tokens.
- **Apply to the whole desktop.** Apply (Return) writes `~/.config/wolos/themes/looks-<name>/{theme.json,look.json}` as `wol-hue` does and runs `wol theme set -- looks-<name>`; Undo in the notice switches back to the previous theme.
- **Save, favourite, rename, delete.** Looks are the files `wol-hue save` writes (`~/.local/share/wolos/hue/looks/<name>.json`), so the command line sees them; favourites and the history are in `~/.local/share/org.wolos.looks/state.json`. Delete has Undo.
- **History.** The last 20 applied looks, each with "Apply again", even if the look was since renamed or deleted.
- **Import and export.** `.wollook` (below), Base16 YAML and colors.toml through the engine's readers; Export writes a `.wollook` through the save panel.
- **Schedule.** A day look and a night look, switched with Night Shift (the Night Light toggle's flag file, `wolos/toggles/org.wolos.nightlight` in the state home) or at set hours (default 19:00 to 07:00). A look chosen by hand stays until the phase changes. The open window checks every minute; `wol-looks --scheduled` makes no window and runs the same rule, for a user timer.
- **Keyboard first, Reduce Motion respected.** Every action has a key (DESIGN.md has the table); the window's styles turn every transition off under Reduce motion.

### The .wollook file

```json
{"format": "wollook", "version": 1, "name": "Dune day", "variant": "light", "mode": "pastel",
 "colors": ["#f8eff3", "... 16 in all"], "accent": null, "adjustments": {"vibrance": 10.0}}
```

The colours before the sliders and the slider positions, so the receiver can still move them. The wallpaper path is left out (it names a file on this computer). Everything read is at most 64 KiB, parsed into types, checked by `Hue::Look#validate!` and given a name that matches the store's rule; a newer `version` is refused.

## How it is built

| Part | File | What it does |
|---|---|---|
| Engine link | `src/looks/engine_link.cr` | Requires `hue/src/hue/{engine,imports,look}.cr` by path |
| Studio | `src/looks/studio.cr` (`Analysis.read`, `Candidate`) | Reads a picture once (decode at most 1200 px a side, sample, quantise), then every mode and variant is the engine's palette step; sliders and `Hue::Theme.tokens` on top |
| Previews | `src/looks/raster.cr`, `src/looks/previews.cr` | Anti-aliased shapes in the candidate's tokens, written as PNG files by generation |
| Store | `src/looks/store.cr` | Saved looks, favourites, history, schedule state; atomic writes |
| File format | `src/looks/wollook.cr` | `.wollook`, Base16 and colors.toml in; `.wollook` out |
| Applier | `src/looks/applier.cr` (`Desktop#apply`) | Theme folder, then the one process `wol theme set -- <id>` |
| Schedule | `src/looks/schedule.cr`, `Tick` in `store.cr` | The phase rule and the quiet run |
| Pictures | `src/looks/pictures.cr`, `source.cr` | Folder listing, thumbnails, kept copies |
| System | `src/looks/system.cr` | The file panels (Wolkit services), swapped for a fake in specs |
| App | `src/looks/app.cr` | State as signals, the verbs, the keyboard map |
| Screens | `src/looks/views/{main,make,saved,history,schedule}.cr` | The kit frame and the four pages |
| Look | `src/styles/looks.wolcss` -> `src/looks/style/{classes,table}.cr` | Compiled after the kit sheet by `tools/styles.sh`; `make spec` fails when they drift |

### Decisions and limits

- **Previews are pictures.** Wolkit colours are theme tokens (WK-001) and a candidate is not the active theme, so each preview is a PNG written from code. libwolkit does not paint images yet (the goldens show empty frames, as the Gallery's do); the specs read the PNG files back and compare pixels, and the Kit backend, which does show images, draws them.
- **One tree for two backends.** The Kit backend of the engine shows at most 12 levels and 200 parts, gives clicks only to buttons, draws no selected state, no overlays, menus or sheets, and its window takes the size of its first content. So every action is a button, the chosen mode, variant, tab and look carry a » in the label, the first frame already holds the whole page with neutral pictures, the name form is a row (not a sheet), and the specs assert at most 200 parts and 10 levels on every page (`LooksSpec.fits_kit?`). Under the Kit backend there are no key events either, so the keyboard map works on the native backend only.
- **No libwol theme verb.** `wol.h` has the `theme` event topic and no call to change it, so Apply starts `wol` with a fixed argument list; the id is built by the app (`looks-` and a slug) and checked against the engine's id rule before it is used (RCR-020).
- **The Night Shift "clock".** The tree has the Night Light toggle (a flag file and hyprsunset) and no schedule for it, so the schedule is Looks' own: follow the flag, or set hours.
- **Images in a vector wallpaper** are drawn at most 1200 px a side (`Hue::Picture.load(path, max_side)`); at 3840 px a wallpaper SVG took 9 s to decode in the VM.

## Measured

On the forge, in a VM (4 vCPU, 4 GB, installed v0.12 image, Kit backend), `LOOKS_TRACE=1`:

| What | Result |
|---|---|
| Process start to window mapped (compositor client list) | 0.75 to 1.5 s (first start after boot 2.2 s to build the view) |
| View built | 110 to 215 ms |
| A 480 x 270 PNG: decode, sample, quantise, 16 candidates, 12 pictures | 0.5 s |
| A 3840 x 2160 wallpaper SVG at 1200 px | 4 s (was 9.4 s at full size) |
| Previews redrawn after a mode, variant or slider change | 80 to 150 ms |
| Apply: theme folder and `wol theme set` | 0.9 to 1.2 s |
| `wol-looks --scheduled` | 1.0 s |

## Tests

`make spec` in `wolos-crystal/apps/looks`: the pure parts in `spec/*_spec.cr` (the studio on the ten scenes of `hue/spec/support/scenes.cr` for every mode and variant with the contrast contract, the raster by pixel, the store, the `.wollook` and import readers, the schedule rule, the applier with a stand-in `wol` on the PATH, the pictures and thumbnails), then `spec/engine/looks_spec.cr`: the app in the headless engine with the pinned fonts, driven with key events and clicks, with a golden screenshot and accessibility tree for each state (`spec/golden/`: empty, reading, failed, ready at 1160 x 720, 900 x 600, 480 x 360, the one-column layout for the Kit backend and dark; naming; applied; saved, empty and after delete; history; schedule), then the look against its sheet. `WOLKIT_BLESS=1 make spec` rewrites the goldens. Also `cargo test -p controller --test rules_crystal` (RCR-001 to RCR-044 and ameba cover the app).

The shard writes PNGs with wrong chunk checksums (the Gallery's note, CR-9), so the goldens open in a few image tools only after the checksums are rewritten; the byte comparison is unaffected.

## Packaging

`wolos-app-looks` in wolos-pkgbuilds, built like `wolos-app-gallery`; drafts are in `wolos-crystal/apps/looks/packaging/` (`PKGBUILD`, the contract test `app_looks.rs`, and the user timer that runs `wol-looks --scheduled`).
