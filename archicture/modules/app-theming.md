# App theming (`wol theme set <id>`)

Source: `crates/core/src/theme/{apps,scaffold,firefox,sourceview,helium,kvantum}.rs`. Files go to `$XDG_CONFIG_HOME/wolos/toolkits/` and are linked into `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME` (a real user file is never replaced). The image ships the `wol-light` render under `/usr/local/share/wolos` (`WOL_REGEN=1 cargo test -p core` rewrites it).

| Toolkit | Output | Live switch |
|---|---|---|
| GTK 4 / libadwaita | `gtk-4.0/gtk.css`: `--*-color`, `--wol-*` tokens, then the Kit scaffold `theme/adwaita.css` (toolbar, source list, cards, controls, sheets) | light/dark + accent preset via portal; palette on app restart |
| GTK 3 (adw-gtk3) | GTK theme `$XDG_DATA_HOME/themes/WolOS-<id>` (`@import` adw-gtk3 + colours + `@wol_*` tokens + Kit scaffold `theme/adw-gtk3.css`); `gtk-3.0/gtk.css` holds no colours | yes: new `gtk-theme` name per theme, GTK 3 reloads |
| Helium (Chromium GTK mode) | `/usr/share/helium/initial_preferences` (linked from `/opt/helium`), `/etc/chromium/policies/managed/wolos.json` (static) | yes: GTK 3 theme + portal `color-scheme`, `accent-color` |
| Firefox | `firefox/wolos-chrome.css`, loaded by `/usr/local/lib/firefox/wolos.cfg` (AutoConfig, user sheet) | yes: polled, re-registered on change |
| Qt | qt6ct palette + Kvantum `WolOS` | on app restart |
| foot | `foot/foot.ini` | new windows |
| GtkSourceView 5 | `gtksourceview-5/styles/wolos-{light,dark}.xml` in `$XDG_DATA_HOME` | light/dark live; palette on restart |
| zsh | `zsh/colors.zsh` (`WOLOS_PROMPT_*`, read by `/usr/share/wolos/zsh/zshrc`) | new shells |

Rules (tests): scaffold metrics (`shared::theme::scaffold`) equal the Kit's `kit.css` and `hyprland.conf`; window buttons are 12 px, 22 px apart, first centre 23 px from the window edge in GTK 3, GTK 4, Firefox and hyprbars; every theme's GTK 3 and GTK 4 sheets parse in the real toolkit (`tests/toolkit_css.rs`); per-app touch-ups key on the app's own classes (Files views, Calculator keys); no stock Adwaita blue survives; GTK 3 CSS has no GTK 4-only syntax; GTK 3 window buttons are drawn on the titlebutton (Chromium renders no image background under GTK 3); every text colour derived from a token reads >= 4.5:1 on its background; the settings portal backend is `gtk` (reads the gsettings `wol theme` sets).
