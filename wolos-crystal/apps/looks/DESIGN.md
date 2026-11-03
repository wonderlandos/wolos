# Looks: design note

App id `org.wolos.looks`, program `wol-looks`, icon `wolos-settings` until the brand adds `wolos-looks` to `design/icons/apps`. One window, four pages, one current candidate. The reference for behaviour is `docs/modules/looks.md`; this page is the look and the reasons.

## Layout

```
Make (1160 x 720; 900 x 600 wraps the sliders below; 480 x 360 uses tabs, not the sidebar)
+--------------------------------------------------------------------------------------+
| Looks  sunset                              [Choose Picture...] [Save...] [Apply]     |  toolbar
+------------+-------------------------------------------------------------+-----------+
| Looks      | Picture                                                     | Variant   |
| > Make     | [thumb][thumb][thumb][thumb]...                             | Dark Light|
|   Saved    | Mode   dark on the left, light on the right of each card    | Adjust    |
|   History  | [Natural][Monochrome][Analogous][Pastel]                    | Vibrance  |
|   Schedule | [Material][Colourful][Muted][Bright]                        | Contrast  |
|            | In your apps  dark natural                                  | Temperature|
|            | [Terminal][Settings][Document][Browser]                     | Shadows   |
| On the     |                                                             | Highlights|
| desktop:.. |                                                             | Reset     |
+------------+-------------------------------------------------------------+-----------+
|  "sunset Natural" is on.                                       Undo  x                |  the notice line, only when there is one
```

- **Make.** The picture strip (the wallpaper folders, then the Pictures folder, 96 x 64 thumbnails), eight mode cards each showing the dark and the light palette side by side (the 16 terminal colours, then a small window in the tokens), the five sliders, and the per-app strip: a terminal, Settings, a document and a browser-like frame drawn with the candidate's tokens. Choosing a mode, a variant or a slider position redraws all of them at once; nothing reaches the desktop until Apply.
- **Saved.** One row per look (its 16 colours, name, kind), favourites first, Apply and Favourite on each row, Import, Export, Rename and Delete on the page's own bar.
- **History.** The last twenty looks applied, newest first, with "Apply again".
- **Schedule.** Off, with Night Shift, or at set hours; a day look and a night look chosen from the saved ones.
- **Quiet.** Colour comes only from the pictures and the accent: the chosen picture, card and row get a 2 px accent ring, the chosen tab the accent fill. Text is three sizes (title 17, body 13, secondary 11).

## Why the previews are pictures

Wolkit's style language takes theme tokens and no literal colours (WK-001), and a candidate palette is by definition not the active theme. So each preview is a PNG written from code (`src/looks/raster.cr`) and shown as an image; its pixels are the candidate's tokens, resolved by `Hue::Theme.tokens`, the function `wol-hue` writes `theme.json` with, so what is previewed is what `wol theme set` receives. The specs read the files back and compare pixels. Text in the previews is drawn as bars, the way a wireframe is, because real text would need the font of each candidate app.

Sizes: pictures are a few kilobytes each; every change writes a new generation (`cards-3.png`) and removes the generation before the previous one, so the file on screen and the one being swapped in both exist.

## Nothing is an overlay

The Kit backend of the engine lowers overlays, menus and sheets to nothing and scroll views to plain columns, and takes at most 200 parts in a window. So Looks has no sheet, menu or dropdown: naming a look is a row under the toolbar, the day and night looks are chosen with Previous and Next, the notice is a row at the bottom. Every page is kept under 200 parts (the saved list shows 18 rows, the history 20; a spec counts the parts), and works the same on the native backend.

## Motion

Only the named curves of the style language (SP-3): `fast` 150 ms, `normal` 250 ms, with the theme `ease`.

| What moves | Curve | Where |
|---|---|---|
| Sidebar row, picture, mode card and look row hover and selection | `fast ease` | `.nav`, `.pic`, `.mode`, `.look-row` |
| Mode card shadow | `fast ease` | `.mode` |
| App previews appearing | `normal ease` | `.preview` |
| Buttons, tabs | the kit's own (`fast ease`) | kit sheet |

Under Reduce motion every one of these is off (`@reduce-motion` in `src/styles/looks.wolcss`). The only timer is the minute tick of the schedule, which moves nothing.

## States

| State | What shows | Golden |
|---|---|---|
| No picture | "Start from a picture" and how | `make-empty` |
| Reading | "Reading the picture…", busy for assistive technology | `make-reading` |
| Cannot read | the one sentence and "Choose another" | `make-failed` |
| Ready | pictures, modes, sliders, previews | `make`, `make-900`, `make-narrow`, `make-dark` |
| Naming | the inline name row | `make-save` |
| Applied | the notice with Undo | `applied` |
| Saved | rows, empty page, after delete | `saved`, `saved-empty`, `saved-deleted` |
| History | rows, empty page | `history`, `history-empty` |
| Schedule | off/Night Shift, hours | `schedule`, `schedule-hours` |

## Keyboard first

Every action has a key; the toolbar buttons are pointer targets named for screen readers (they take no Tab stop, so Return and the letters always mean Looks' own verbs). While the name row is open only Escape is a command; Return saves.

| Key | Make | Saved | History | Schedule |
|---|---|---|---|---|
| Arrows, Home, End | move between modes (Up and Down by a row) | move between looks | move between entries | |
| Return | apply the candidate | apply the look | apply again | |
| D, L | dark, light | | | cycle the day, night look |
| 1 to 5, then - and + | choose a slider, move it by 5 | 1, 2: all, favourites | | 1, 2, 3: when |
| 0 | reset the sliders | | | |
| Page Up, Page Down | previous, next picture | | | |
| F | | favourite | | |
| F2, Delete | | rename, delete (with Undo) | | |
| Ctrl+O | choose a picture | | | |
| Ctrl+S | name and save the candidate | | | |
| Ctrl+I, Ctrl+E | | import, export | | |
| Ctrl+1 to Ctrl+4 | the four pages | | | |
| Ctrl+Z | undo the last apply or delete | | | |
| Escape | close the name row, else dismiss the notice | | | |

## Decisions

- Apply writes `themes/<id>/{theme.json,look.json}` exactly as `wol-hue` does and runs `wol theme set -- <id>`; the id is `looks-` and the look's name as a slug, built by the app and checked against the engine's id rule. libwol has no theme verb yet.
- Deleting a look is a delete with Undo (the look is kept in memory for the notice), never a confirmation (HIG: prefer undo over confirm).
- A look file is the one `wol-hue save` writes, so the command line sees what the app saves and the app sees what the command line saves.
- A look keeps the colours before the sliders and the slider positions, so a slider is a position, never a step.
- The history keeps full looks, so "Apply again" works after a look is deleted or renamed.
- Everything that comes in (`.wollook`, Base16, colors.toml, a handed picture) is size-limited, parsed into types and named by the store's rule before it is used.
