# wol-hue: themes from a picture (ADR 0031)

The theme maker from a picture. `wol-hue` turns a picture, or a palette from another tool, into an ordinary WolOS
theme: a folder `$XDG_CONFIG_HOME/wolos/themes/<id>/` with `theme.json` (the 13 tokens, the 16
`terminal` colours and the `wallpaper`) and `look.json` (the colours before the sliders). It then
runs `wol theme set <id>`, the one path that restyles the shell and every app. It is a Crystal
program in `hue/` (package `wolos-hue`, binary `/usr/bin/wol-hue`).

**Why.** ADR 0031 (themes v2) asks for theme-from-wallpaper that always passes the contrast
contract. A good palette needs more than an average colour: the picture's dominant colours by
coverage, vivid accents that a big muted background would drown, a background that really fills
the picture, and a separate path for grey and single-hue pictures. Writing a theme (and nothing
else) keeps one writer for app configuration: `crates/core/src/theme/` renders GTK, Qt, compositor,
terminals and consoles from `theme.json`, so a picture theme looks exactly like a built-in one.
Crystal, because the lead chose it for this tool; no shards, only the standard library, gdk-pixbuf
(already on the desktop) for decoding and the TOML reader of `wopkg/src/wopkg/toml.cr`.

## How it works

1. **Decode** (`hue/src/hue/image.cr`): gdk-pixbuf reads PNG, JPEG, WebP or AVIF (whatever
   loaders are installed). Above 24 MP the decoder downscales while reading (a 50 MP JPEG takes
   about 0.7 s end to end on the forge, `spec/perf_spec.cr`); above 300 MP the picture is refused.
2. **Sample** (`hue/src/hue/extract.cr`): a Catmull-Rom downscale to 400 px on the long side, then
   every opaque pixel (alpha >= 128), with a step so at most ~50 000 are read.
3. **Weight and quantise**: pixels go to OKLab (published matrices); a pixel with OKLCH chroma
   above 0.04 gets 1 to 3 extra copies, graded over 0.16 of chroma; median cut in OKLab (lightness
   weighted 1.2x) splits the largest-volume box until 48 colours exist, sorted by pixel count.
4. **Palette** (`hue/src/hue/palette.cr`), one of eight modes:

   | Mode | What it does |
   |---|---|
   | `normal` | Monochrome pictures (colour on under 6% of the frame, or one hue family) get a grey or tinted ramp; others keep their own accents, folded into hue bands that cover at least 2.5% of the picture |
   | `monochromatic` | One hue family around the picture's tint, slots kept apart by lightness |
   | `analogous` | Six hues within 30 degrees of the most chromatic colour |
   | `pastel`, `colorful`, `muted`, `bright` | The picture's hues with the mode's lightness and chroma |
   | `material` | Fixed neutral surfaces, accents tuned in HSL |

   Background: the darkest (light themes: lightest) of the 12 most common colours, nudged by
   coverage (weight 0.6) and replaced when it is too mid-toned. Foreground: the opposite extreme,
   aiming at 7:1 contrast and never under 4.5:1. Accents 1-6 are matched to the red, green, yellow, blue, magenta and
   cyan hues by a global greedy assignment, then every accent is lifted to 4.5:1 against the
   background.
5. **Tokens** (`hue/src/hue/theme.cr`): surface from colour 0, text from 7, accent from 4,
   success/warning/danger from 2/3/1, the rest mixed. `Theme.check` is the rule set of
   `shared::theme::validate` (`crates/shared/src/theme.rs`); a background that cannot carry readable
   text is moved toward black or white until every rule holds.
6. **Cache** (`hue/src/hue/engine.cr`): `$XDG_CACHE_HOME/wolos/hue/<sha256>-<variant>-<mode>-v1.json`,
   keyed by the file's content.

## Commands

```sh
wol-hue from-wallpaper ~/Pictures/dune.jpg                 # dark, normal: theme picture-dune
wol-hue from-wallpaper dune.jpg --light --mode pastel --id dune-day
wol-hue adjust --temperature 20 --contrast 10              # the active theme; sliders are positions
wol-hue adjust dune-day --reset
wol-hue import base16 ~/schemes/ocean.yaml                 # classic or palette: form
wol-hue import colors ~/theme/colors.toml --wallpaper ~/Pictures/sea.png
wol-hue save "Desert day" --theme dune-day                 # $XDG_DATA_HOME/wolos/hue/looks/
wol-hue list
wol-hue apply "Desert day"                                 # theme look-desert-day
```

`--no-apply` writes the folder without switching; `--json` prints the theme with `dir`, `mode`,
`applied` (and `cached` for pictures); `--no-cache` re-reads the picture. The twelve sliders and
their ranges: vibrance, temperature, tint, shadows, highlights -50..50; saturation -100..100;
contrast, brightness, black point, white point -30..30; hue shift -180..180; gamma 0.5..2. They run
in a fixed order in HSL (`Color.adjust`). Exit codes: 0 done, 1 failed (message says what), 2 usage.

Theme ids are `a-z`, `0-9` and `-` (64 at most), so a name from outside never leaves the themes
folder. The tool writes only its theme folder, its cache and its looks.

## Tests

`make -C hue spec` (forge: `cd wolos/hue && make && make spec`): colour maths, the engine's unit
cases, every mode and variant on ten scenes drawn in code (`spec/support/scenes.cr`), the contrast
contract for all of them, Base16 and colors.toml import, the binary end to end with a stand-in
`wol`, and the 50 MP timing. `spec/fixtures/golden.json` holds 60 palettes (10 scenes x dark/light
x normal, monochromatic, pastel); `make golden` re-records it after a deliberate engine change
(bump `Engine::CACHE_VERSION` with it). `crystal tool format --check src spec` is clean.

Ported code: `color.cr`, `extract.cr`, `palette.cr` and `imports.cr` are MIT (see
[THIRD-PARTY.md](../THIRD-PARTY.md)); the rest is GPL-2.0-or-later. The reference engine's
OKLab matrix has a wrong third row; `Color.legacy_lms = true` reproduces it only to compare against
palettes recorded with it.
