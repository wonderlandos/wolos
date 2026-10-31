// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
// services: every libwol call added since 1.2, in Swift (ADR 0020). Built and run by tests/bindings.rs.
import Glibc
import Wol

func text(_ s: OpaquePointer?) -> String { String(cString: wol_string_get_bytes(s, nil)) }
func ok(_ status: Int32) { precondition(status == WOL_OK, "libwol status \(status)") }
func n(_ s: String) -> Int { s.utf8.count }
func field(_ i: OpaquePointer?, _ key: String) -> OpaquePointer? { wol_info_get(i, key, n(key)) }

var ctx: OpaquePointer?, err: OpaquePointer?
let app = "org.example.services"
ok(wol_context_create(app, n(app), &ctx, &err))

let file = "File", open = "open", save = "Open", key = "width", value = "640"
let title = "Copying", body = "1 of 5", shortcut = "cmd+o", icon = "document-open-symbolic", reason = "Exporting"

var menu: OpaquePointer?
ok(wol_menu_create(ctx, file, n(file), { _, _, _ in }, nil, &menu, &err))
ok(wol_menu_add_item(menu, open, n(open), save, n(save), shortcut, n(shortcut), &err))
ok(wol_menu_add_separator(menu, &err))
ok(wol_menu_set_checked(menu, open, n(open), 1, &err))
wol_menu_release(menu)
print("menu")
var bar: OpaquePointer?
ok(wol_toolbar_create(ctx, { _, _, _ in }, nil, &bar, &err))
ok(wol_toolbar_add_button(bar, open, n(open), save, n(save), icon, n(icon), &err))
ok(wol_toolbar_add_spacer(bar, &err))
wol_toolbar_release(bar)
print("toolbar")

var note: OpaquePointer?
ok(wol_notification_create(ctx, title, n(title), body, n(body), &note, &err))
ok(wol_notification_send(note, &err))
ok(wol_notification_replace(note, title, n(title), value, n(value), &err))
ok(wol_notification_withdraw(note, &err))
wol_notification_release(note)
print("note")

var str: OpaquePointer?
ok(wol_clipboard_set_text(ctx, value, n(value), &err))
ok(wol_clipboard_copy_text(ctx, &str, &err))
print("clipboard: \(text(str))")
wol_string_release(str)

ok(wol_prefs_set_string(ctx, key, n(key), value, n(value), &err))
ok(wol_prefs_copy_string(ctx, key, n(key), &str, &err))
print("pref: \(text(str))")
wol_string_release(str)
var keys: OpaquePointer?
ok(wol_prefs_copy_keys(ctx, &keys, &err))
print("keys: \(wol_info_get_count(keys)) \(text(wol_info_get_text(wol_info_get_item(keys, 1))))")
wol_info_release(keys)
ok(wol_prefs_remove(ctx, key, n(key), &err))

var f: OpaquePointer?
ok(wol_panel_open(ctx, save, n(save), &f, &err))
var buf = [UInt8](repeating: 0, count: 64)
let got = read(wol_file_get_fd(f), &buf, 64)
print("panel: \(text(wol_file_get_name(f))) \(String(decoding: buf[0..<got], as: UTF8.self))")
wol_file_release(f)
ok(wol_panel_save(ctx, save, n(save), key, n(key), &f, &err))
wol_file_release(f)

var info: OpaquePointer?
ok(wol_power_copy_info(ctx, &info, &err))
var percent = 0.0
_ = wol_info_get_number(field(field(info, "battery"), "percent"), &percent)
print("battery: \(Int(percent)) \(wol_info_get_bool(field(info, "on_battery")))")
wol_info_release(info)
var held: OpaquePointer?
ok(wol_power_keep_awake(ctx, reason, n(reason), &held, &err))
wol_assertion_release(held)
print("held")
ok(wol_network_copy_info(ctx, &info, &err))
print("online: \(wol_info_get_bool(field(info, "online"))) \(text(wol_info_get_text(field(wol_info_get_item(field(info, "interfaces"), 0), "kind"))))")
wol_info_release(info)
ok(wol_audio_copy_info(ctx, &info, &err))
_ = wol_info_get_number(field(info, "volume"), &percent)
print("volume: \(Int(percent)) \(text(wol_info_get_text(field(field(info, "now_playing"), "title"))))")
wol_info_release(info)
ok(wol_audio_set_volume(ctx, 30, &err))
ok(wol_audio_set_muted(ctx, 1, &err))
var opened = false
let term = "term"
ok(wol_search_query(ctx, term, n(term), &info, &err))
print("found: \(wol_info_get_count(field(info, "results")))")
wol_info_release(info)
var provider: OpaquePointer?
ok(wol_search_provide(ctx, { _, r, t, _ in
    let k = "note-1", title = "A note"
    var e: OpaquePointer?
    _ = wol_search_reply_add(r, k, n(k), title, n(title), nil, 0, &e)
    print("asked: \(text(t))")
}, { _, k, _ in print("open: \(text(k))"); opened = true }, nil, &provider, &err))
for _ in 0..<100 where !opened { ok(wol_context_dispatch(ctx, 100, &err)) }
wol_search_provider_release(provider)
ok(wol_account_copy_info(ctx, &info, &err))
print("account: \(text(wol_info_get_text(field(info, "name"))))")
wol_info_release(info)
var seen = false
let peer = "/usr/bin/peer", hello = "hello"
var door: OpaquePointer?
ok(wol_endpoint_create(ctx, key, n(key), { _, m, _ in
    print("message: \(text(wol_message_get_body(m))) \(text(wol_message_get_from(m)))")
    let pong = "pong"
    var e: OpaquePointer?
    _ = wol_message_reply(m, pong, n(pong), &e)
    seen = true
}, nil, &door, &err))
ok(wol_message_send(ctx, peer, n(peer), key, n(key), hello, n(hello), &str, &err))
print("reply: \(text(str))")
wol_string_release(str)
for _ in 0..<100 where !seen { ok(wol_context_dispatch(ctx, 100, &err)) }
ok(wol_publish(ctx, key, n(key), hello, n(hello), &err))
wol_endpoint_release(door)
ok(wol_device_copy_list(ctx, &info, &err))
print("device: \(text(wol_info_get_text(field(wol_info_get_item(field(info, "devices"), 0), "name"))))")
wol_info_release(info)
ok(wol_device_open(ctx, 1133, 2, &f, &err))
print("opened: \(text(wol_file_get_name(f)))")
wol_file_release(f)
ok(wol_share_text(ctx, title, n(title), body, n(body), &str, &err))
print("shared: \(text(str))")
wol_string_release(str)
ok(wol_service_start(ctx, key, n(key), &err))
ok(wol_service_copy_status(ctx, key, n(key), &info, &err))
print("service: \(text(wol_info_get_text(field(info, "state"))))")
wol_info_release(info)
ok(wol_service_stop(ctx, key, n(key), &err))
wol_context_release(ctx)
