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
| 5 | IPC between apps: typed messages, services, app topics | `wol_message_`, `wol_endpoint_`, `wol_publish` | next: routed by wold, grant per peer app |
| 6 | files and portals: open/save panels, granted file handles (fd passing) | `wol_file_`, `wol_panel_` | 1.5 `wol_panel_open`, `wol_panel_save`, `wol_file_get_fd`, `_get_name`: grant `file-panels`; the desktop opens the person's pick and the descriptor is passed on, so wold never opens a path for an app |
| 7 | settings and app preferences | `wol_prefs_` | 1.4 `_copy_string`, `_set_string`, `_remove`, `_copy_keys`: grant `app-preferences`; per account and executable, kept by wold |
| 9 | search: Spotlight providers and queries | `wol_search_` | |
| 10 | power: battery, power source, keep-awake assertions, Low Power Mode | `wol_power_` | 1.6 `_copy_info` (grant `power-info`), `_keep_awake` and `wol_assertion_` (grant `keep-awake`): a logind inhibitor that lasts as long as the app's connection to wold |
| 11 | network: reachability, interfaces, Wi-Fi state | `wol_network_` | 1.7 `_copy_info`: grant `network-info`; online, and interface names and kinds, never an address or a network name |
| 12 | audio: output, volume, now playing | `wol_audio_` | 1.8 `_copy_info` (grant `audio-info`), `_set_volume`, `_set_muted` (grant `audio-control`), run by the desktop |
| 13 | devices: USB, Bluetooth, iPhone | `wol_device_` | per-device grant |
| 14 | sharing and WolConnect: share sheet, nearby devices | `wol_share_` | |
| 15 | services: the app's background jobs (wold, per account) | `wol_service_` | grant `background` |
| 16 | packages: installed apps, ask to install | `wol_package_` | through wol-admind |
| 18 | answers: one read-only tree for what a service returns | `wol_info_` | 1.4 |
| 17 | accounts: who is signed in, name, picture | `wol_account_` | |

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
- Crystal: `crystal/lib_wol.cr`, `@[Link("wol")] lib LibWol`.
- .NET (C#, F#, Alice): `dotnet/Wol.Native.cs`, P/Invoke on `libwol.so.1`; `Utf8.cs` hands strings over as pointer + length.

Crystal and .NET declarations are generated from `wol.h` (`tests/common/bindgen.rs`) and the test fails when they differ. For a new ABI version: edit `wol.h`, run `WOL_BLESS=1 cargo test -p wol-abi --test bindings`, then add the new calls to a C test and to `services` in Swift, Crystal and C# (`tests/common/services.rs` checks them all). A missing toolchain skips its language; `WOL_BINDINGS_REQUIRED=1` makes that an error.
