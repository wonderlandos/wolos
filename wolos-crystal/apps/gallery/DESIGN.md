# Gallery: design note

App id `org.wolos.gallery`, program `wol-gallery`, icon `wolos-photos` (the crafted print-on-tape icon of `design/icons/apps`). One window, two screens, one current picture. The reference for behaviour is `docs/modules/gallery.md`; this page is the look and the reasons.

## Layout

```
Library (900 x 600; 480 x 360 narrow)                Viewer
+--------------------------------------------------+ +--------------------------------------------------+
| Gallery  9 pictures        [Date|Place] [>] [@]  | | [<] IMG_0377.jpg      Fit [-][+][>][i][^][...]   |  toolbar
+--------------------------------------------------+ +-------------------------------------+------------+
| Today  2 photos                                  | |                                     | Name       |
| [tile] [tile]                                    | |            the picture              | Taken      |
| Yesterday  1 photo                               | |       (fit, or zoomed and panned)   | Size       |  info panel
| [tile]                                           | |                                     | Camera ... |  (I)
| Saturday  3 photos                               | +-------------------------------------+            |
| [tile] [tile] [tile]                             | |  [ ][ ][ ][#][ ][ ][ ]  filmstrip   |            |
+--------------------------------------------------+ +-------------------------------------+------------+
| "IMG_0412.jpg" is in the Trash.          Undo  x |  the notice line, both screens, only when there is one
```

- **Library.** A grid of 160 x 120 tiles in smart sections, newest first. By date: one section per day for the last week ("Today", "Yesterday", "Saturday"), then one per month ("September 2026"); the quiet line beside the heading is the count. By place: one section per cell of a tenth of a degree, named by its coordinates ("38.7° N, 9.1° W", never a town: that needs a map service, which would learn where the person was), with the count and the months; pictures with no place last. The toolbar switches Date and Place with the kit tabs.
- **Viewer.** The picture on the stage (sidebar tone, edge to edge), a filmstrip of up to 13 neighbours under it, the toolbar with the name, "4 of 9", the zoom ("Fit", "100 %") and at most five actions plus a More menu (HIG section 1). The info panel (280 px, sidebar tone) lists name, when taken (or changed, when the camera said nothing), size, camera, exposure, place, folder; its values are selectable. Nothing else: no ratings, no edit tools, no badges.
- **Quiet.** Colour comes only from the picture and the accent: the selected tile and the current filmstrip item get a 2 px accent ring, the active toolbar toggles the accent fill. Text is three sizes (title 17 for headings, body 13, secondary 11).

## Motion

Only the named curves of the style language (SP-3): `fast` 150 ms, `normal` 250 ms, `slow` 400 ms with the theme `ease`, and the springs `gentle`, `snappy`, `bouncy`.

| What moves | Curve | Where |
|---|---|---|
| Tile hover (fill, shadow) and press | `fast ease` | `.tile` |
| Filmstrip item hover, current ring | `fast ease` | `.strip-item` |
| Zoom and pan of the picture (size and position) | spring `snappy` | `.picture` |
| Info panel appearing | `normal ease` | `.info` |
| Buttons, tabs, menus | the kit's own (`fast ease`) | kit sheet |

Under Reduce motion every one of these is off (`@reduce-motion` in `src/styles/gallery.wolcss`): the picture jumps to its zoom, the panel appears.

## States

| State | What shows | Golden |
|---|---|---|
| Loading | "Reading your pictures…" centred, with a line that thumbnails are kept; marked busy for assistive technology | `loading` |
| Empty | "No pictures yet", where to put them, Read again | `empty` |
| No folder | "There is no Pictures folder", what to make, Read again | `missing` |
| Error | "The library could not be read", the one sentence of the cause and what to do, Try again (primary) | `error` |
| Library | by date, by place, narrow, dark | `library-date`, `library-place`, `library-narrow`, `library-dark` |
| Viewer | fit, zoomed, with info, slideshow playing | `viewer`, `viewer-zoomed`, `viewer-info`, `slideshow` |
| After delete | the notice with Undo | `trashed` |

## Keyboard first

Every action has a key; the toolbar buttons are pointer targets named for screen readers (they take no Tab stop, so Return and Space always mean the gallery's own verbs).

| Key | Library | Viewer |
|---|---|---|
| Arrows | move the selection (Up and Down by a row) | previous and next; pan when the picture is larger than the stage |
| Home, End | first, last | first, last |
| Return, Space | open the selection | Space plays or pauses the slideshow |
| Page Up, Page Down | | previous, next (also while zoomed) |
| + or =, -, 0, 1 | | zoom in, zoom out, fit, actual size (or back to fit) |
| I | | info panel |
| F5 | slideshow from the selection | play or pause |
| Backspace, Delete | to the Trash (Undo in the notice) | to the Trash, show the next |
| Ctrl+Z (or Super+Z) | undo the last delete | undo the last delete |
| Ctrl+1, Ctrl+2 | by date, by place | |
| Escape | dismiss the notice | stop the slideshow, else back to the library |

A click on the stage toggles actual size at the point clicked.

## Decisions

- Delete is a move to the shared Trash with Undo, never a confirmation sheet (HIG: prefer undo over confirm).
- Thumbnails are the preview the camera stored in the picture, copied once to `~/.local/share/org.wolos.gallery/thumbnails` (mode 0600, keyed by path, size and change time, pruned on every scan). Pictures without one show the picture itself until the engine decodes and scales.
- The camera block is read by a small pure Crystal reader with every offset bounded; a broken file reads as "nothing there".
