# desktop/

Overlay defaults for the WolOS session. Copy `overlay/` onto `/`; install `packages.txt`.

| Path | Purpose |
|---|---|
| `usr/local/bin/wol-session` | Creates `~/.config/hypr/hyprland.conf` (sources system file) once; links toolkit defaults; runs `dbus-run-session start-hyprland` |
| `usr/local/share/wolos/hypr/hyprland.conf` | macOS feel: floating, hyprbars traffic lights, spring animations, blur, keybinds, autostart |
| `usr/local/share/wolos/hypr/hypridle.conf` | Lock (wol-lock) at 300 s, DPMS off at 360 s |
| `usr/local/etc/wolos/shell.json` | wol-shell menu bar, status icons, dock, wallpapers |
| `usr/local/bin/wol-screenshot`, `wol-volume` | grim/slurp and wpctl helpers |
| `usr/local/etc/xdg/gtk-{3,4}.0`, `qt6ct`, `foot` | Theme adw-gtk3 + font Inter (image/fetch-theme.sh), icons WolOS |
| `usr/local/etc/fonts/conf.d/60-wolos-inter.conf` | `Inter` -> Inter Variable; default sans |
| `usr/local/etc/xdg/xdg-desktop-portal/hyprland-portals.conf` | Hyprland portal, GTK file chooser |
| `usr/local/etc/wolos/jobs/org.wol.login.json` | wold job: `wol-login` (root) -> `wol-greeter-session` -> `wol-session` |
| `usr/local/bin/wol-greeter-session`, `share/wolos/hypr/login.conf` | Login window: Hyprland + wol-loginwindow as `_wollogin` (uid 911, firstboot) |
| `usr/local/etc/pam.d/wol-login` | PAM service of wol-login |
| `etc/rc.conf.d/wolos` | dbus, seatd (wold: `image/files/usr/local/etc/rc.d/wold`, on by default) |

Super is Command (⌘). A plain ⌘ letter belongs to the app in front: `wol-app` taps the app's own
key (Ctrl+C; a terminal's Ctrl+Shift+C), see `shell::appmenu::send`.

| Keys | Action |
|---|---|
| ⌘C/V/X, ⌘Z / ⇧⌘Z, ⌘A | Copy, Paste, Cut, Undo / Redo, Select All (in the app) |
| ⌘S/O/P, ⌘F / ⌘G, ⌘T/N/R/L | Save, Open, Print, Find / Find Next, New Tab/Window, Reload, Location |
| ⌘W | Close tab (the app's Ctrl+W; a terminal without tabs closes its window) |
| ⌘+ / ⌘− / ⌘0 | Zoom In / Out / Actual Size |
| ⌃⌘Space | Emoji & Symbols |
| ⌘Q / ⌘H / ⌥⌘H / ⌘M | Quit / Hide / Hide Others / Minimize |
| ⌘Space | Spotlight |
| ⌘Tab | App switcher |
| ⌃↑ (⌘↑) | Mission Control |
| ⌃⌘Q | Lock |
| ⌥⌘Esc | Force Quit Applications |
| ⌃F2 / ⌃F3 | Keyboard into the menu bar / the Dock (Esc gives it back) |
| Esc | Also dismisses notification banners |
| ⌘, / ⌘Return | Settings / Terminal |
| ⇧⌘3/4/5 | Screenshot full / region / toolbar (saved to the Desktop) |
| ⌃⌘F, ⌥⌘F | Fullscreen, float a tiled window |
| Ctrl+Left/Right, 3-finger swipe | Switch space |
