# The WolOS experience: surfaces as data (ADR 0059)

Decision record: [0059](../adr/0059-the-wolos-experience.md). Test: `crates/controller/tests/experience.rs`. The journeys live in `wolos-iso`, `tests/wolvm/journeys/`.

**Concept.** A person does not feel "an app"; they feel the surfaces every app shares: the menu bar, Share, Open With, Quick Look, Search, the clipboard, the colour of the title strip. This page names the 28 surfaces (S01..S28) once, says which package owns each and which journey proves it, and lists what each shipped app provides itself. The test keeps the page, the launcher entries and the journeys in agreement, so a surface cannot be claimed in one place and missing in another.

**Why.** The failure that started this page: a surface was "integrated" once, at session start, and wrong afterwards (the title strip kept the old light/dark variant after the Auto switch). A list in prose rots; a table a test reads does not.

## Surfaces

Tier (ADR D2): 1 = the system provides it for any app, stock or ours; 2 = the app declares it (a desktop-entry key, a menu, a search-provider file, a libwol verb); 3 = native, a toolkit library gives it to apps we write. The Journey column is the acceptance: a file `<name>.yaml` in `tests/wolvm/journeys/`. A journey that has no file yet is listed in `JOURNEYS_NOT_YET` of the test with its owner (the list only shrinks).

| Id | Surface | Tier | Owner | Journey |
|---|---|---|---|---|
| S01 | Launch feedback; the window opens where and how big it was | 1 | E2E-WIN | e2e-window-memory |
| S02 | Menu bar: app menu plus File, Edit, View, Window, Help | 1 | E2E-MENU | e2e-menus |
| S03 | The same shortcuts in every app (Super is Command) | 1 | E2E-MENU | e2e-menus |
| S04 | Share sheet for files, text and links, with app targets | 1 | E2E-SHARE | e2e-share-file |
| S05 | Open With; default apps the person can change | 1 | E2E-OPEN | e2e-default-apps |
| S06 | Drag and drop between apps, onto the Dock | 1 | E2E-OPEN | e2e-default-apps |
| S07 | Clipboard with history; images and files paste | 1 | E2E-CLIP | e2e-clipboard-image |
| S08 | Quick Look: Space on a file | 1 | E2E-QL | e2e-quicklook |
| S09 | Search finds what is inside each app | 2 | E2E-SEARCH | e2e-search-notes |
| S10 | Notifications with buttons, Focus, Notification Center | 1 | E2E-QA | e2e-notifications |
| S11 | Media keys and Now Playing | 2 | E2E-LIB-1 | e2e-media-keys |
| S12 | Resume where you left off, per app | 3 | E2E-STATE | e2e-resume |
| S13 | Services on selected text | 1 | E2E-SERVICES | e2e-services |
| S14 | Spell check in every text field | 1 | E2E-SET | e2e-spell |
| S15 | Emoji and symbols | 1 | E2E-SERVICES | e2e-services |
| S16 | Colour picker: click a pixel, the value is on the clipboard | 1 | E2E-SET | e2e-spell |
| S17 | Printing and PDF export | 2 | E2E-LIB-1 | e2e-print-pdf |
| S18 | Accessibility: roles, names, screen reader | 1 | E2E-A11Y | qa100-p3-a11y-tree |
| S19 | Theme, accent and dark mode follow the system, strip included | 1 | E2E-QA | e2e-strip-follows-auto |
| S20 | Memory Saver rests idle apps | 1 | E2E-QA | e2e-memory-saver |
| S21 | Keyring: one place for credentials, unlocked at login | 2 | E2E-LIB-2 | e2e-secrets |
| S22 | Links open in Atlas; a page can be sent to another app | 1 | E2E-ATLAS | e2e-links-open-in-atlas |
| S23 | Files: Trash with undo, Recents, tags | 2 | E2E-FILES | e2e-files-tags |
| S24 | Dock: badges, jump list, drop to open | 1 | E2E-OPEN | e2e-default-apps |
| S25 | Language, keyboard layout and input method follow Settings | 1 | E2E-QA | A10-keyboard-surfaces |
| S26 | Permissions asked the same way, with a default for bundled apps | 2 | E2E-LIB-1 | e2e-gallery |
| S27 | The app as a product: package, icon, Store, update, removal | 1 | E2E-SET | e2e-gallery |
| S28 | Help: the Help menu, the shortcut sheet, the manual | 1 | E2E-MENU | e2e-menus |

## What each shipped app provides itself

The launcher entry carries `X-WolOS-Surfaces=` with the ids the app provides on its own today (system-provided surfaces work for it anyway and are not listed). An app gains an id in the same change that gains the behaviour: the entry and this table change together, and the test fails when they differ. An empty list is written `X-WolOS-Surfaces=` and means the entry is a launcher.

| Entry | Provides |
|---|---|
| org.wolos.gallery.desktop | S04;S05;S19;S27 |
| org.wolos.store.desktop | S05;S19;S25;S27 |
| org.wolos.plugin-maker.desktop | S19;S25;S27 |
| org.wolos.quicklook.desktop | S08 |
| org.wolos.windows-app.desktop | |
| steam.desktop | S05;S27 |

## Desktop-entry keys

Standard keys are read first (`MimeType`, `Actions`, `x-scheme-handler/*`); a WolOS key exists only where there is no standard one. The test fails on any `X-WolOS-` key of a shipped entry that is not in this table.

| Key | Value | Read by |
|---|---|---|
| X-WolOS-Surfaces | ids `S01`..`S28`, separated by `;` | this page's test |
| X-WolOS-Share | what the app accepts through Share: `text`, `url`, `file`, separated by `;` | the Share sheet (E2E-SHARE) |
| X-WolOS-Service | selection verbs the app offers, separated by `;` | Services (E2E-SERVICES) |

## Keys and tools the image provides for the surfaces

- Ctrl+Super+P picks a colour from the screen (`hyprpicker --autocopy --notify`, S16); it is in `hyprland.lua` and `hyprland.conf`, named in Settings > Keyboard, and `hypr/permissions` lets `/usr/bin/hyprpicker` copy the screen (it is in `security::SCREENCOPY_SYSTEM`). The Services key (S13, proposed Ctrl+Super+S) is added by E2E-SERVICES together with its program, because a bind to a program that does not exist fails the RS-081 rule.
- Spell check (S14): `enchant` is the engine of the text libraries the stock apps use; `hunspell` and one dictionary package per language of `wol_core::region::LANGUAGES` that has a word list (Japanese and Chinese have none). Portuguese is not in the distribution's repository: `wolos-spell-pt` packs the Brazilian and Portuguese lists of the LibreOffice dictionaries project (LGPL-3.0-only or MPL-2.0).
- The PDF tools (`pdftocairo`, `pdftotext`, `pdftoppm`) come with the `poppler` package the document viewer already needs; no second PDF library is added.
- Screen reader (S18): `orca`, `speech-dispatcher` and `espeak-ng`.
- Photos is Gallery: the Dock pin, and the default for PNG and JPEG (`mimeapps.list`); the image viewer stays for the types Gallery does not open.

## Image budget

The image has 21 MiB of room under the release asset limit. Sizes below are compressed package sizes, the dependency closure of the whole image list against the closure with the addition, on the 2026-11-02 package archive (mem 866 sets the room).

| Addition | Compressed | Installed |
|---|---|---|
| hunspell and the 8 dictionary packages | 7.3 MiB | 49 MiB |
| wolos-spell-pt (Brazil, Portugal) | about 1.3 MiB | 5.6 MiB |
| hyprpicker | 0.1 MiB | 0.2 MiB |
| orca, speech-dispatcher, espeak-ng, braille support (21 packages) | 22.7 MiB | 115 MiB |
| wolos-app-gallery with libwolkit and libwol | about 6 MiB | about 20 MiB |
| poppler tools, enchant, libspelling | 0 (already in the image) | 0 |
| **Added** | **about 37 MiB** | **about 190 MiB** |
| glmark2, vkmark, assimp (dropped; mesa-utils stays) | -10.8 MiB | -25 MiB |
| shotwell (dropped; it is in the Store, Gallery is Photos) | -3.3 MiB | -15 MiB |
| eog (dropped; it is in the Store; GIF, WebP and SVG open in the browser until Gallery decodes them) | -3.5 MiB | -10 MiB |
| **Net with the three drops** | **about +19.4 MiB, about 1.6 MiB under the room** | |
| gnote (when Notes ships) | -7.6 MiB | -40 MiB |

Camera and phone import needs Shotwell from the Store until Gallery has it. The ISO size gate in `build.sh` stays at 2047 MiB, one MiB under GitHub's 2 GiB limit for one release asset.

Not added, and why: `mupdf-tools` pulls a second PDF library and a text-recognition engine (39 MiB compressed) for tools `poppler` already has.
