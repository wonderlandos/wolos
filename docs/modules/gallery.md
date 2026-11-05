# Gallery, org.wolos.gallery (ADR 0039)

The picture library and viewer that ships with WolOS: a Wolkit app in Crystal (ADR 0039), the first of the bundled apps rewritten on Cryswol and Wolkit (`docs/modules/apps.md`). Source `wolos-crystal/apps/gallery/`, program `wol-gallery`, package `wolos-app-gallery`. The look and the reasons are in the design note, `wolos-crystal/apps/gallery/DESIGN.md`.

## What it does

- **Library.** Reads every PNG and JPEG under the Pictures folder (the folder settings' `XDG_PICTURES_DIR`, else `~/Pictures`) and shows them in smart sections: by date (a section per day for the last week, then per month) or by place (a cell of a tenth of a degree, named by its coordinates; pictures with no place last).
- **Viewer.** One picture at a time with zoom (fit, then 25 % to 800 %), pan, a filmstrip of its neighbours, a quiet info panel (when, size, camera, exposure, place, folder) and a slideshow that goes round every four seconds.
- **Image viewer.** `wol-gallery PICTURE` (or a `file://` address) opens that picture in the viewer, with the other pictures of its folder in the filmstrip; the launcher entry takes `image/jpeg` and `image/png`, and is the default for both ("In the image").
- **Delete with Undo.** Backspace or Delete moves the picture to the shared Trash (the folder the file manager's Trash shows); the notice line offers Undo, which puts it back.
- **Share.** The system share sheet, through `Wolkit::Services.share`.

Every action has a key; the full map is in `DESIGN.md` ("Keyboard first").

## How it is built

| Part | File | What it does |
|---|---|---|
| Camera block | `src/gallery/exif.cr` (`ExifReader.read`, line 72) | Pure Crystal reader of the tagged block in a JPEG (APP1) or PNG (`eXIf`): date taken, place, orientation, camera, exposure, the stored preview. Every offset is bounded; a damaged file reads as "not there" |
| Picture header | `src/gallery/picture.cr` | Format from the first bytes, width and height from the PNG header or the JPEG frame header |
| Scan | `src/gallery/library.cr` (`Library#scan`, line 29) | A bounded walk: no hidden or linked folders, at most 8 levels and 20,000 pictures, 192 KiB read per file |
| Thumbnails | `src/gallery/thumbs.cr` (`Thumbs#fetch`, line 32) | The camera's stored preview copied once to `~/.local/share/org.wolos.gallery/thumbnails/<sha256>.jpg` (mode 0600, keyed by path, size and change time, written to a temporary file and renamed, pruned on every scan) |
| Sections | `src/gallery/sections.cr` (`by_date` line 29, `by_place` line 48) | Pure functions of the photos and "now" |
| Zoom | `src/gallery/zoom.cr` (`Zoom#to`, line 71) | Scale and offset as a value: zoom around a point, clamped pan, steps, fit |
| Trash | `src/gallery/trash.cr` (`Trash#move`, line 33) | The note (`info/<name>.trashinfo`, where it came from and when) is written first, then the file is renamed into `files/`; restore renames it back |
| System | `src/gallery/system.cr` (`Desktop`, line 32) | The one door to the system: trash, restore, share, open with. Specs swap it for a fake |
| App | `src/gallery/app.cr` (`apply` line 93, `trash` line 206, `key` line 285) | State as signals, the verbs, the keyboard map |
| Screens | `src/gallery/views/{main,library,viewer}.cr` | The kit frame (`app_window`, `toolbar`, `tabs`, `empty_state`, `menu`), the grid, the stage, the filmstrip, the info panel, the notice line |
| Look | `src/styles/gallery.wolcss` -> `src/gallery/style/{classes,table}.cr` | Compiled after the kit sheet by `tools/styles.sh` (`make styles`); committed, and `make spec` fails when they drift |

The scan runs on the Wolkit background pool (`FolderSource#load`, `src/gallery/source.cr`) and returns one value to the UI context; nothing blocks the window.

## Testing

`make spec` in `wolos-crystal/apps/gallery` runs three groups:

1. `spec/*_spec.cr`: the pure parts with pictures written from code (`spec/support/fixtures.cr`): both byte orders, PNG and JPEG, a truncation and byte-flip sweep of the reader, the sections with a fixed clock, zoom arithmetic, the scan, the cache and the Trash in temporary folders.
2. `spec/engine/gallery_spec.cr`: the app in the real headless engine with the pinned test fonts (`crates/wolkit/tests/golden/fonts`), driven by clicks and key events, with a golden screenshot and accessibility tree for each state (`spec/golden/`: loading, empty, missing, error, library by date, by place, narrow 480 x 360, dark, viewer, zoomed, info, slideshow, after delete). `WOLKIT_BLESS=1 make spec` rewrites them; look at the PNGs before committing.
3. `tools/styles.sh --check`: the committed look matches the sheet.

The package's `check()` runs the same `make spec` against the installed libwolkit.

## In the image

- **Package.** `wolos-app-gallery` (pkgbuilds, `packages` line after libwolkit and libwol) installs `/usr/bin/wol-gallery`, the launcher entry and `/usr/share/wolos/consent.d/org.wolos.gallery.json` (from `org.wolos.gallery.consent.json` in the app folder). The image profile lists it with the Dock apps.
- **Grants.** The consent file is a vendor default (`consent::DEFAULTS_DIR`, `docs/modules/security.md`): `trash` and `share` for `/usr/bin/wol-gallery`, nothing else. Wold and Settings > Privacy & Security hold them until the person switches one off (the choice is kept in `/etc/wolos/consent.json` under `revoked` and survives updates). `crates/core/tests/consent_defaults.rs` holds the file to the rule for defaults.
- **Default viewer.** Gallery is the default for `image/png` and `image/jpeg` (`desktop/overlay/usr/share/applications/mimeapps.list`, eog listed after it as the fallback), and the Photos pin of the Dock opens it. `image/gif`, `image/webp` and `image/svg+xml` stay with eog: Gallery draws PNG and JPEG only, and a default that cannot show its file is worse than none. Shotwell stays installed until Gallery has import from a camera or phone.
- **Journey.** `e2e-gallery` (wolos-iso `tests/wolvm/journeys/`): a PNG opens from the default (the call Files makes), Delete puts it in the Trash with a notice, Ctrl+Z puts it back, Share opens the sheet; wold refuses nothing.

## Limits and what waits on other packages

- **Pictures.** libwolkit decodes and paints PNG and JPEG since WK-image (`docs/modules/wolkit.md`, `image`): every `image` source (file path, or `icon:<name>`) draws the picture, and a placeholder in the `control` colour shows until the file is decoded. Other formats (GIF, WebP, SVG) are not drawn, so the library does not list them and they stay with the image viewer (see "In the image").
- **Thumbnails** exist only for pictures whose camera stored a preview; the others show the picture itself, scaled by the engine.
- **Trash, Share and Open with** go through libwol 1.15 (`Wol::Trash.move`, `Wol::Share.file`, `Wol::Share.open_with`): the app needs the grants `trash` and `share`. The package ships both as vendor defaults ("In the image"), so the first Delete on a fresh system works; when the person has switched one off in Settings > Privacy & Security, the refusal's sentence shows inline. Undo renames the picture back out of the home Trash (`Gallery::Trash#restore`, the note the desktop wrote is removed); a picture that went to the Trash of its own drive is put back from the Files Trash.
- **Pan** is by keyboard and by clicking (actual size at the point); dragging and wheel panning need pointer drag events in the DSL.
- **Window size**: the grid's row length and the stage size are read from the window when a key is pressed or the zoom changes; a resize does not re-lay the zoom until then.
- At most 4,000 tiles are mounted at once (a window holds 65,536 parts); the library says so when there are more.
