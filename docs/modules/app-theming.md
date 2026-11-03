# App theming (`wol theme set <id>`)

Decision record: ADR 0031 (themes and layouts, themes v2), ADR 0032 (Console).

Source: `crates/core/src/theme/{apps,generate,hypr,terminal,console,templates,packs,scaffold,firefox,sourceview,helium,kvantum}.rs`. One palette, every consumer: `generate::consumers` is the full list of files a switch writes. Files go to `$XDG_CONFIG_HOME/wolos/toolkits/` and are linked into `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` (a real user file is never replaced). The image ships the `wol-light` render under `/usr/share/wolos` (toolkit files) and `/usr/share/wolos/toolkits` (console files but starship, whose default is the shipped `starship.toml`); `WOL_REGEN=1 cargo test -p core` rewrites both.

| Toolkit | Output | Live switch |
|---|---|---|
| GTK 4 / libadwaita | `gtk-4.0/gtk.css`: `--*-color`, `--wol-*` tokens, then the Kit scaffold `theme/adwaita.css` (toolbar, source list, cards, controls, sheets) | light/dark + accent preset via portal; palette on app restart |
| GTK 3 (adw-gtk3) | GTK theme `$XDG_DATA_HOME/themes/WolOS-<id>` (`@import` adw-gtk3 + colours + `@wol_*` tokens + Kit scaffold `theme/adw-gtk3.css`); `gtk-3.0/gtk.css` holds no colours | yes: new `gtk-theme` name per theme, GTK 3 reloads |
| Helium (Chromium GTK mode) | `/usr/share/helium/initial_preferences` (linked from `/opt/helium`), `/etc/chromium/policies/managed/wolos.json` (static) | yes: GTK 3 theme + portal `color-scheme`, `accent-color` |
| Firefox | `firefox/wolos-chrome.css`, loaded by `wolos.cfg` from the optional `wolos-browser-config` package, which the image does not ship (AutoConfig, user sheet) | yes: polled, re-registered on change |
| Qt | qt6ct palette + Kvantum `WolOS` | on app restart |
| Compositor | `hypr/wolos/theme.conf` (`hypr::conf`: hyprbars bar, title, inactive look, shadows), linked as `$XDG_CONFIG_HOME/hypr/wolos/theme.conf` and read by the user conf's `hypr/wolos/*.conf` glob after the shipped default `/usr/share/wolos/hypr/wolos/theme.conf`; the window buttons are `/usr/share/wolos/hypr/wolos/lights.conf` | yes: `hyprctl keyword` per line |
| foot | `foot/foot.ini` (terminal palette) | new windows |
| GtkSourceView 5 | `gtksourceview-5/styles/wolos-{light,dark}.xml` in `$XDG_DATA_HOME` | light/dark live; palette on restart |
| zsh | `zsh/colors.zsh` (`WOLOS_PROMPT_*`, read by `/usr/share/wolos/zsh/zshrc`) | new shells |
| starship | `starship/palette.toml`: the shipped starship.toml with `[palettes.wolos]` set (roles + text, surface, ANSI names) | new prompts |
| fish, nushell, bash, PowerShell | `fish/colors.fish`, `nu/colors.nu`, `bash/colors.sh` (`WOLOS_PROMPT_*` hex, `WOLOS_ANSI_*` SGR), `pwsh/colors.ps1` (PSReadLine) | new shells |
| ghostty, zellij | `ghostty/theme` (palette 0-15 + background, foreground, cursor, selection), `zellij/theme.kdl` (theme `wolos`) | new windows / sessions |
| VS Code, Code-OSS | `vscode/wolos.json` (colour theme) | on reload |
| Your own | `$XDG_CONFIG_HOME/wolos/themed/<name>.tpl` -> `toolkits/themed/<name>`: `{{token}}`, `{{color0}}`..`{{color15}}`, ANSI names, `{{background}}`, `{{foreground}}`, `{{cursor}}`, `{{selection}}`, `{{id}}`, `{{name}}`, `{{variant}}`; filters `|strip` (`rrggbb`), `|rgb` (`r,g,b`); a broken template is skipped | every switch |

Terminal palette (`terminal::palette`): a theme may list its own 16 colours (`"terminal": [...]` in theme.json, inherited through `extends`); otherwise red, green, yellow and magenta are the danger, success, warning and accent tokens and the rest a fixed set per variant. Prompt roles (`console::roles`: accent, muted, ok, error, suggest, warn) read >= 4.5:1 on the terminal background (suggest >= 3:1), the same in every shell.

Catalogue: built-ins, `/usr/share/wolos/themes`, `$XDG_CONFIG_HOME/wolos/themes` and the `[[theme]]` files of enabled plugins (`packs`, `ThemeCatalog::installed`; a pack never replaces a known id). Settings > Appearance (`crates/shell/src/ui/settings/appearance.rs`): Light / Dark / Auto, accent, the theme gallery, a "From Plugins" row, the wallpaper and "Theme from your wallpaper" (`generate::from_samples`, saved as the user theme `from-wallpaper`, always passes `check()`). The picker panel (`ui/theme_picker.rs`) shows the same gallery.

Rules (tests): scaffold metrics (`shared::theme::scaffold`) equal the Kit's `kit.css` and `hyprland.conf`; window buttons are 12 px, 22 px apart, first centre 23 px from the window edge in GTK 3, GTK 4, Firefox and hyprbars; every theme's GTK 3 and GTK 4 sheets parse in the real toolkit (`tests/toolkit_css.rs`); every console file parses as its format (TOML, KDL, JSON, ghostty lines) for every theme (`console::tests`); the shipped hyprland.conf has no colour literal but the start-up black (`hypr::tests`); per-app touch-ups key on the app's own classes (Files views, Calculator keys); no stock Adwaita blue survives; GTK 3 CSS has no GTK 4-only syntax; GTK 3 window buttons are drawn on the titlebutton (Chromium renders no image background under GTK 3); every text colour derived from a token reads >= 4.5:1 on its background; the settings portal backend is `gtk` (reads the gsettings `wol theme` sets).
