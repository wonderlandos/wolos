# libwol: the app API (ADR 0020)

One API for apps from any language: `wol.h` + `libwol.so.1`. Code: `crates/wol-abi` (C ABI over Rust), `crates/wol-lua` (Lua widgets), wire contracts in `shared::portal` (app -> wold) and `shared::view` (app <-> Kit host), wold side in `api::portal`.

## Rules (header comment has the full text)

| Rule | Means |
|---|---|
| Opaque handles | no struct layout is public, ever |
| `_create` / `_copy` | you own a reference: `_release` it |
| `_get_` | borrowed, lives as long as its argument |
| Strings | UTF-8 + length in; NUL-terminated + length out |
| Errors | `wol_status` return; `wol_error **` filled only on failure, with a sentence to show |
| Threads | retain/release anywhere; the rest one thread at a time per context |
| Callbacks | `void *user_data`; run only inside `wol_context_dispatch`; must not unwind |
| Event loop | poll `wol_context_get_fd`, then `wol_context_dispatch(ctx, 0, &err)` |
| Stability | symbols versioned by `abi/wol.map`; never removed or changed, only added in a new node |

Adding API: implement + test, declare in `include/wol.h`, add the symbol to a new node in `abi/wol.map` and to `abi/wol-<version>.symbols`, snapshot the header as `abi/wol-<version>.h` at release. `cargo test -p wol-abi` checks all of it, links `libwol.so.1` with `link.sh` and runs `tests/c/hello.c` (1.0), `tests/c/parts.c` (1.1) and `tests/c/menus.c` (1.2) against fake wold and Kit hosts. Version defines are the only lines a newer header changes.

Build: `cargo build --release -p wol-abi`, then `crates/wol-abi/link.sh target/release/libwol.a <dir>` (the map alone decides exports). Install `libwol.so.1` and `wol.h`.

## Security

- libwol has no privilege. In-process: reading the person's own settings and world-readable facts. Everything else is a `shared::portal::Call` to wold, decided with the kernel's peer identity and the consent grants (Settings > Privacy & Security, `/etc/wolos/consent.json`).
- No telemetry. libwol opens the wold socket and the Kit host socket (`$XDG_RUNTIME_DIR/wol-kit.sock`) and nothing else.
- Every argument is checked (NULL, length, UTF-8, control characters, ranges). Panics stop at the boundary as `WOL_ERR_INTERNAL`.
- A call that needs the person's session (clipboard, panels, audio) is checked by wold first, then asked of the shell of the caller's account (`Event::SessionAsk`, `Call::SessionAnswer`). wold matches each answer to a waiting ask of that account; a call waits at most a few seconds, and at most 64 wait at once.
- Decoders: `shared/tests/portal_fuzz.rs` (every `cargo test`) and `fuzz/` (`cargo +nightly fuzz run ipc_decode`).

## Surface: what is in, what comes, in order

| # | Family | Prefix | State |
|---|---|---|---|
| 1 | core: version, errors, strings, context, event loop, subscriptions | `wol_version`, `wol_error_`, `wol_string_`, `wol_context_`, `wol_subscribe`, `wol_event_` | 1.0 |
| 2 | system facts, appearance, accent | `wol_system_info_` | 1.0 (change events: topic `theme`) |
| 3 | notifications | `wol_notification_` | 1.0 post; 1.1 buttons (`_create`, `_add_action`, `_set_action_handler`, `_send`); 1.2 `_replace`, `_withdraw` (only the process that posted it) |
| 4 | app, windows, Kit parts | `wol_window_`, `wol_stack_`, `wol_label_`, `wol_button_`, `wol_widget_`, `wol_row_`, `wol_list_`, `wol_spacer_`, `wol_text_field_`, `wol_toggle_`, `wol_slider_`, `wol_image_`, `wol_menu_`, `wol_toolbar_` | 1.0 stack, label, button; 1.1 row, list, spacer, text field, toggle, slider, image, `set_text`/`copy_text`, value getters and setters, close handler and `wol_window_close`; 1.2 menus (`wol_window_add_menu`) and toolbars (`wol_window_set_toolbar`) |
| 8 | clipboard | `wol_clipboard_` | 1.3 `_copy_text`, `_set_text`: grants `clipboard-read`, `clipboard-write` |
| 5 | IPC between apps: typed messages, services, app topics | `wol_message_`, `wol_endpoint_`, `wol_publish` | 1.11 `wol_endpoint_create` and `wol_publish` (grant `ipc`), `wol_message_send` (grant `ipc:<peer path>` per peer app): routed by wold within one account; the sender is the executable the kernel reports |
| 6 | files and portals: open/save panels, granted file handles (fd passing) | `wol_file_`, `wol_panel_` | 1.5 `wol_panel_open`, `wol_panel_save`, `wol_file_get_fd`, `_get_name`: grant `file-panels`; the desktop opens the person's pick and the descriptor is passed on, so wold never opens a path for an app; 1.15 `wol_file_trash`: grant `trash`; the desktop moves the file the app names into the person's Trash as them, with the standard information file (the Files Trash lists it and puts it back), and answers its name there for Undo; never the home folder, a folder holding it, or something already in the Trash |
| 7 | settings and app preferences | `wol_prefs_` | 1.4 `_copy_string`, `_set_string`, `_remove`, `_copy_keys`: grant `app-preferences`; per account and executable, kept by wold |
| 9 | search: Search providers and queries | `wol_search_` | 1.9 `_query` (grant `search`); `_provide`, `_reply_add` (grant `search-provider`): wold asks only that account's providers, for 200 ms, each as its own process |
| 10 | power: battery, power source, keep-awake assertions, Low Power Mode | `wol_power_` | 1.6 `_copy_info` (grant `power-info`), `_keep_awake` and `wol_assertion_` (grant `keep-awake`): a logind inhibitor that lasts as long as the app's connection to wold |
| 11 | network: reachability, interfaces, Wi-Fi state | `wol_network_` | 1.7 `_copy_info`: grant `network-info`; online, and interface names and kinds, never an address or a network name |
| 12 | audio: output, volume, now playing | `wol_audio_` | 1.8 `_copy_info` (grant `audio-info`), `_set_volume`, `_set_muted` (grant `audio-control`), run by the desktop |
| 13 | devices: USB, Bluetooth, iPhone | `wol_device_` | 1.12 `_copy_list` (grant `devices`), `_open` (grant `device:<vendor>:<product>` per device): wold opens the USB node and passes the descriptor, never for a keyboard, disk or hub |
| 14 | sharing and WolConnect: share sheet, nearby devices | `wol_share_`, `wol_open_with` | 1.13 `_text`: grant `share`; the person sees the text and chooses copy or save. 1.15 `_file`: grant `share`; the share sheet for a file: copy a link to it, save a copy, open it with another app, or cancel. 1.15 `wol_open_with`: grant `share`; the chooser of this computer's apps for the file's kind. The app learns only what was done, never where a copy went or which app opened it. Nearby devices: next, once wold finds them |
| 15 | services: the app's background jobs (wold, per account) | `wol_service_` | 1.14 `_start`, `_stop`, `_copy_status`: grant `background`; wold supervises the caller's own executable as the account, never a program the app names |
| 16 | packages: installed apps, ask to install | `wol_package_` | through wol-admind |
| 18 | answers: one read-only tree for what a service returns | `wol_info_` | 1.4 |
| 17 | accounts: who is signed in, name, picture | `wol_account_` | 1.10 `_copy_info`: grant `account-info`; the caller's own account, from the kernel's uid. Accounts have no picture yet |
| 19 | a player, a recent file, a colour, a printed file | `wol_media_`, `wol_recent_`, `wol_color_`, `wol_print_` | 1.16 `wol_media_publish` and its setters (`_set_handler`, `_set_track`, `_set_art`, `_set_state`, `_set_capabilities`): grant `media`; the desktop gives the player the name `org.mpris.MediaPlayer2.wol.<program>.instance<pid>` on the session bus, see "Players" below. 1.16 `wol_recent_add`: grant `recents`; the person's recent files, as this app. 1.16 `wol_color_pick`: grant `color-picker`; the person clicks a colour on the screen and the app gets that colour only. 1.16 `wol_print_file`: grant `print`; the print dialog for a file, nothing prints until the person chooses |

## Players (libwol 1.16)

`wol_media_publish` makes a player that other programs can control: the media keys, Control Center and `playerctl` see it as an MPRIS player. Who owns what:

- **The desktop owns the bus name, never the app.** The shell serves `org.mpris.MediaPlayer2.wol.<id>.instance<pid>` on the session bus on the app's behalf, one bus connection per player (`shell::app_media`). `<id>` is the file name of the executable the kernel reported to wold, with every character outside `A-Z a-z 0-9 _ -` made `_` (at most 48 bytes, never starting with a digit). The `.instance<pid>` suffix is MPRIS' own convention for one player per process, and Memory Saver reads the playing process from it, so a playing app is never frozen; `playerctl -p wol.<id>` still finds it, and Control Center shows the program's icon. An app cannot take another name, and cannot pose as another app: the Identity and DesktopEntry come from the `.desktop` entry whose `Exec` program resolves, through `PATH` and links, to the very executable the kernel reported (`shell::app_media::entry_of`), never from its file name alone, so a program called `firefox` in `/tmp` is only `firefox`; the recent files' app name uses the same lookup.
- **The app gives the state, whole, every time** (`shared::portal::MediaState`): status, title, artist, album (one line of at most 256 bytes each), length, position, a picture path and its abilities (next, previous, seek; play and pause always work). The desktop counts the seconds while it plays, so the app sends state when something changes, not on every tick; a position that differs from the desktop's count by more than 1.5 s is announced as `Seeked`. A picture is shown as `file://` (its real path) only if the file is a PNG, JPEG, WebP or GIF of at most 4 MiB inside the person's home folder, a mounted drive (`/run/media`, `/media`), `/usr/share` or `$XDG_RUNTIME_DIR`, with no hidden part in its path (so not under `~/.cache`; a cover written to `$XDG_RUNTIME_DIR` is the place for one that is needed only for now), as named and with every link followed; no other URL is ever offered.
- **One player per process, for as long as its connection lives.** `media.publish` is a held call like `search.provide`: releasing the player, the process ending or wold restarting ends it (`Event::MediaGone`), and the name goes with it. An account has at most 8 players on the bus and 16 holds in wold.
- **Remote controls go to the owner only.** A control on the bus becomes `Call::MediaKey` (only the shell may send it: `Need::Shower`), wold checks that process is a registered player of that account and publishes `Event::MediaKey` to that one process (`Audience::Process`); libwol calls the handler (`wol_media_fn`) inside `wol_context_dispatch`. The desktop sends only what the abilities allow: Next and Previous need their flag, Seek and SetPosition need `WOL_MEDIA_CAN_SEEK`; `OpenUri` does nothing.

Wire (`shared::portal`, `shared::events`): `media.publish {}` (grant `media`), `media.state {state}` (grant `media`; refused unless the caller published), `media.key {player, key, value_us}` (the shell), events `media-state {uid, pid, app, state}` and `media-gone {uid, pid}` to the account's desktop, `media-key {uid, pid, key, value_us}` to one process. `recent.add {path}`, `color.pick {}` and `print.file {path}` become `Ask::RecentAdd {path, app}` (app is the kernel's word), `Ask::ColorPick` and `Ask::PrintFile {path}` for the shell, which answers as the person: a recent file or a file to print must be inside the person's home folder or on a mounted drive (`/run/media`, `/media`) with no hidden part (a name starting with `.`) in its path, judged as named without touching the disk and again on the real path with every link followed, so a link from `~/Documents` into `~/.ssh` is refused; hidden, outside and missing get the same sentence ("That file cannot be added to the recent files." or "...cannot be printed."), so neither call tells an app what exists; a colour comes from `hyprpicker -f hex` as `#rrggbb`; a print file must also be a plain PDF, PostScript, text, PNG or JPEG file and opens `GtkPrintDialog` (Save as PDF is its own printer).

## Event topics (`wol_subscribe`)

JSON is the `shared::events` line: `theme` `{"event":"theme","id":"wol-dark-blue"}`, `workspace` `{"event":"workspace","name":"2"}`, `active-window` `{"event":"active-window","class":"…","title":"…","pid":7}`, `open-window`, `close-window`, `job`. Notifications go only to the shell; a pressed notification button goes only to the process that posted it, and runs the handler set with `wol_notification_set_action_handler`.

Menus and toolbars are data on the window (`ToHost::Show`), drawn by the Kit host: a menu bar under the title row and toolbar buttons in it. `cmd` in a shortcut is the Command key, which WolOS hands to the app in front as Control (`shell::appmenu`). What the person chooses comes back as a key; libwol runs the handler only for an enabled key the app added.

Kit parts: a change made through the API redraws the window the part is in (if shown); a change the person made is recorded first, then the part's change callback runs, so getters inside it read the new value.

## Kit host and notifications in the shell

- `wol-shell` listens on `$XDG_RUNTIME_DIR/wol-kit.sock` (0600, own account) and starts one `wol-kit-host` per app connection: the app's windows carry its own window class, and a broken app takes down only its host. Protocol rules: `shell::app_host` (hello first, at most 16 windows, 256 KiB lines); drawing: `shell::ui::app_view` (a new tree of the same shape updates parts in place).
- `shell::app_notices` subscribes to wold as the account's notification shower and posts each app notification to the session notification server (banners, Notification Center, Focus); a pressed button goes back to wold as `notification.answer`.

## Lua widgets (`crates/wol-lua`)

A widget is one file in `~/.config/wolos/widgets/<id>.lua` (id: lowercase letters, digits, `-`), reloaded when it changes. On the desktop it is kind `lua:<id>` (`wol widgets add lua:<id>`); it renders at its `refresh` on the desktop widget timer and after a click (`shell::ui::lua_widgets`). Example: `crates/wol-lua/widgets/clock.lua`.

```lua
return wol.widget {
  name = "Counter", refresh = 0, permissions = { "notifications" },
  render = function()
    return wol.row { spacing = 8, wol.label { "Hello" }, wol.spacer(),
      wol.button { label = "Ping", on_click = function() wol.notify("Ping", "from Lua") end } }
  end,
}
```

| API | Gives |
|---|---|
| `wol.stack{spacing=, ...}`, `wol.row{...}`, `wol.label{text, role=}`, `wol.button{label=, on_click=}`, `wol.spacer()` | view tree parts (roles: display, large, title, body, secondary, caption) |
| `wol.now()` | `{year, month, day, weekday (1 = Monday), hour, minute, second}` |
| `wol.system()` | `{os_name, os_version, kernel, appearance, accent, accent_rgb}` |
| `wol.notify(title, body)` | only with `permissions = {"notifications"}`; returns ok, message |

Sandbox: string, table, math, utf8; no io, os, require, debug, load, bytecode; 16 MiB; 5 M instructions per load, render or click. Services appear after the file loads.

## Other languages

All call the same `wol.h`; strings go as pointer + byte length, so no language needs NUL-terminated buffers. Bindings live in `crates/wol-abi/bindings/`, each with a `hello` (notification, event) and a `parts` (Kit parts, button, notification buttons) that `tests/bindings.rs` builds and runs against the same fake wold and Kit host as the C programs:

- Swift: `swift/module.modulemap` over `wol.h` (`import Wol`); no generated code, the header is the binding.
- Crystal: `crystal/lib_wol.cr`, `@[Link("wol")] lib LibWol`. Apps use the Cryswol shard (`wolos-crystal/shards/cryswol/`, `require "cryswol"`), a typed layer with exceptions, blocks and handles that release themselves over the same generated declarations, a copy of this file; see [cryswol.md](cryswol.md).
- .NET (C#, F#, Alice): `dotnet/Wol.Native.cs`, P/Invoke on `libwol.so.1`; `Utf8.cs` hands strings over as pointer + length.

Crystal and .NET declarations are generated from `wol.h` (`tests/common/bindgen.rs`) and the test fails when they differ. For a new ABI version: edit `wol.h`, run `WOL_BLESS=1 cargo test -p wol-abi --test bindings`, then add the new calls to a C test, to `services` in Swift, Crystal and C# (`tests/common/services.rs` checks them all), and their Cryswol wrapper and spec ([cryswol.md](cryswol.md), "How to add a verb"). The library ships in the `libwol` package (`wolos-pkgbuilds/libwol/`, pkgver = the ABI version). A missing toolchain skips its language; `WOL_BINDINGS_REQUIRED=1` makes that an error.
