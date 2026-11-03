# Plugin Maker and Theme Studio (ADR 0031)

Plugin Maker (`wol-plugin-maker`) is the app where a person or an AI agent makes a shell plugin or a theme and sees it live, without turning anything on in their own desktop.

Decision: [ADR 0031](../adr/0031-shell-plugins-themes-layouts.md), bullet "Plugin Maker / Theme Studio". Code: `crates/shell/src/ui/maker/` (`mod.rs` app and pages, `editor.rs`, `preview.rs`, `theme_studio.rs`, `publish.rs`), `crates/shell/src/bin/wol-plugin-maker.rs`, sentences in `crates/shell/src/copy_maker.rs`. Desktop entry: `desktop/overlay/usr/share/applications/org.wolos.plugin-maker.desktop` (icon `wolos-plugin-maker`, `design/icons/apps/`). The plugin API it edits is [plugins.md](plugins.md).

## Why

A plugin is a folder of text files, and `wol plugin new|check|run|pack` already covers making one from a terminal. What the terminal cannot show is how the plugin looks where it lands: an item in the menu bar, a card on the desktop, a Dock entry, a Control Center tile. Turning the plugin on to see it means a half-written plugin runs on the real desktop with real permissions. The app closes that gap: the same checks as the CLI, the same plugin host as the desktop, in a sandbox. The Theme Studio does the same for themes: sliders over the 13 tokens with the contrast contract measured while they move, so an unreadable theme is visible before it is saved.

## How

Launch: `wol-plugin-maker` (start page), `wol-plugin-maker <plugin folder>`, or `wol-plugin-maker --theme [theme.json]` (Theme Studio). It is one window per session: a second launch opens its argument in the first window (`gtk::Application` id `org.wolos.plugin-maker`). Settings > Appearance has a "Make a theme" row (`crates/shell/src/ui/settings/appearance.rs`, `--theme`) and Store > Plugins a Create Plugin button (`crates/shell/src/ui/store_plugins.rs`); both start the app through `ui::maker::launch`.

| Part | What it does | Code |
|---|---|---|
| Start page | Open Plugin (a folder chooser) or Create Plugin: an id and the kinds to add as switches (the ten kinds of `maker::KINDS`: menu bar, widget, Dock, tile, panel, search, toggle, hotkey, web app, theme; `[[look]]` and `[[hypr]]` are planned, not offered, see ADR 0031's note on planned kinds), written by `wol_core::plugins::scaffold`, the function `wol plugin new` calls, into `~/.local/share/wolos/plugins/<id>/`. An existing folder is never written into. | `editor::create`, `Maker::start_page` |
| Editor | `plugin.toml` and every `.lua`, `.json`, `.toml` and `.conf` file of the folder in a monospace text view (a plain `GtkTextView`: the workspace has no GtkSourceView binding, see below). Save writes each changed file through a fresh file in the same folder, synced, then renamed over the old one. | `editor::Editor`, `editor::save_atomic` |
| Problems | Under the editor. The same function as `wol plugin check`: `wol_core::plugins::check_full` (the folder check: manifest, id, files named, symlinks, size, theme contrast; a `[[look]]` or `[[hypr]]` section is refused as planned, `shared::plugins::Kind::planned_sentence`) with the Lua step `wol_lua::plugin::lua_problems` (every script loaded with the host's deadlines and run once). The app's services act on nothing; the CLI's run commands. Unsaved text is checked in a scratch copy of the folder, so the list follows the typing; it reruns 400 ms after the last keystroke (`editor::CHECK_AFTER`), off the GTK thread, and only the newest result is shown (`editor::Turns`). | `editor::check_folder`, `editor::check_draft` |
| Preview | The plugin loaded through the shell's own plugin host (`ui::plugins::Host`) and drawn on four surfaces: a menu bar strip with its items in the slots the default layout gives them, a Dock strip, its desktop cards, its Control Center tiles. Deadlines and the three-strike rule are the desktop's: a failing entry shows its sentence where it sits, and three failures in a row turn the plugin off in the preview. Reloads after every save. | `preview::Preview`, `preview::surfaces` |
| Sandbox | The preview runs no command and reads nothing from the web (`wol.run` and `wol.http_get` answer with a sentence); state, toggles and the enable list live in `$XDG_RUNTIME_DIR/wol-plugin-maker-<pid>/`, removed with the window. Nothing in the preview touches the person's `plugins.json`, toggles or plugin state. | `preview::PreviewServices`, `preview::sandbox` |
| Check | The folder as saved, now. | `Editor::check` |
| Create Package | `wol plugin pack <folder> --yes --json`: the CLI's own pack (`.wolplug`, the format in `crates/core/src/package.rs`), written next to the folder; then the SHA-256 and size of the file. | `publish::pack` |
| Share | A sheet with the plugin index request as text: where to put the file, the index entry (format 1, `wopkg::index::plugins::Entry`) with the file's SHA-256 and size, and the `gh pr create` command for the `wolos-plugins` index repository. Copy Steps puts it on the clipboard. The app sends nothing; the index CI runs `wol plugin check` again and signs the package. | `publish::steps`, `copy::maker::publish_steps` |
| Theme Studio | Starts on the theme in use, or a `theme.json` opened (a theme that extends another is resolved against the installed catalogue). Two sliders per token group (Surfaces, Text, Accent, Status Colors): lightness moves OKLab L by up to 0.3 either way, color scales OKLab chroma from grey to twice as vivid; hue and alpha are kept. A translucent token is written as `#rrggbbaa`. | `theme_studio::shift`, `theme_studio::apply` |
| Contrast | One badge per rule of `shared::theme::CONTRAST_RULES`, measured the way `shared::theme::validate` measures it (a translucent background over the surface, the surface over white); the line under them is `validate`'s verdict. Save Theme refuses a theme `validate` does not pass. | `theme_studio::badges`, `theme_studio::problems` |
| Previews of a theme | The theme gallery card (`kit::widgets::theme_preview`, as Settings > Appearance and the picker draw themes) and the terminal palette (`wol_core::theme::terminal::palette`, the 16 colours every terminal gets). | `theme_studio::Studio` |
| Use Wallpaper Colors | The desktop picture decoded small off the GTK thread, `wol_core::theme::generate::from_samples` with the same variant; the sliders go back to the middle. | `theme_studio::samples_of` |
| Save Theme | `$XDG_CONFIG_HOME/wolos/themes/<id>/theme.json` (id from the name, never `wol-*`), written atomically, so Settings > Appearance and `wol theme list` show it. | `theme_studio::save_user_theme` |

Rules it keeps: every read, check, save and pack runs off the GTK thread (`ui::maker::off_thread`); every sentence is in `copy_maker.rs` and passes `copy::lint`; widgets are Kit parts except the text view, the share text's monospace label and the terminal strip's drawing area.

Not in v0.12: syntax colouring (the editor stays a plain text view; GtkSourceView would be a new C dependency with its own theme work), and refresh timers in the preview (an entry renders on load, on each click and on each save, not every `refresh` seconds).

The Theme Studio and [hue.md](hue.md) overlap on purpose for now: `wol-hue` is the command-line palette engine and the Studio is the window; in a later release the Studio will call `wol-hue` for palette extraction instead of its own wallpaper sampling.

## Tests

- `crates/shell/src/ui/maker/editor.rs` tests: Create Plugin writes exactly the CLI scaffold and it checks clean; drafts give the folder check's own sentences and leave the folder and the temp folder clean; atomic save; only the newest edit is current; 400 ms.
- `crates/shell/src/ui/maker/preview.rs` tests: a scaffolded plugin with a menu bar item, a card, a Dock entry and a tile renders on all four surfaces without a display; a failing entry shows its sentence and the third failure turns the plugin off, in the sandbox only; a slow entry stops at the host deadline; the sandbox runs nothing.
- `crates/shell/src/ui/maker/theme_studio.rs` tests: OKLab round trip and a known value, shifts keep hue and alpha, a slider moves only its group, the badges fail exactly the rules `validate` fails, a saved theme loads from the catalogue and an unreadable one is not saved.
- `crates/shell/src/ui/maker/publish.rs` tests: the pack is the CLI verb; the index entry pins the file hash and reads back as an index entry.
- Goldens `maker-editor`, `maker-preview`, `maker-theme` (light and dark): `crates/shell/tests/golden_cases/maker.rs`.
