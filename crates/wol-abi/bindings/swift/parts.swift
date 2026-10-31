// SPDX-License-Identifier: GPL-2.0-or-later
// parts: the Kit parts, a button and notification buttons from Swift. Run by tests/bindings.rs.
import Wol

var ctx: OpaquePointer?, win: OpaquePointer?, note: OpaquePointer?, err: OpaquePointer?
var changes = 0, closed = 0, pressed = 0, clicks = 0
func text(_ s: OpaquePointer?) -> String { String(cString: wol_string_get_bytes(s, nil)) }
func ok(_ status: Int32) { precondition(status == WOL_OK, "libwol status \(status)") }
func n(_ s: String) -> Int { s.utf8.count }
func refused(_ status: Int32, _ what: String) {
    guard status == WOL_ERR_INVALID_ARGUMENT else { return }
    print("\(what) refused")
    wol_error_release(err)
    err = nil
}

guard wol_version() >= 0x010100 else { fatalError("needs libwol 1.1") }
let app = "org.example.parts", name = "Parts", source = "weather-clear-symbolic", sunny = "Sunny", cloudy = "Cloudy"
let placeholder = "Name", wifiLabel = "Wi-Fi", buttonLabel = "Click Me", badSource = "/etc/../shadow.png"
let heading = "Download done", file = "parts.iso", key = "open", open = "Open"

ok(wol_context_create(app, n(app), &ctx, &err))
var list: OpaquePointer?, row: OpaquePointer?, icon: OpaquePointer?, title: OpaquePointer?, gap: OpaquePointer?
var field: OpaquePointer?, wifi: OpaquePointer?, volume: OpaquePointer?, button: OpaquePointer?, bad: OpaquePointer?
ok(wol_window_create(ctx, name, n(name), &win, &err))
ok(wol_list_create(ctx, &list, &err))
ok(wol_row_create(ctx, 8, &row, &err))
ok(wol_image_create(ctx, source, n(source), 24, &icon, &err))
ok(wol_label_create(ctx, sunny, n(sunny), &title, &err))
ok(wol_spacer_create(ctx, &gap, &err))
ok(wol_text_field_create(ctx, placeholder, n(placeholder), { f, _ in
    var s: OpaquePointer?
    if wol_widget_copy_text(f, &s, nil) == WOL_OK { print("name: \(text(s))"); wol_string_release(s) }
    changes += 1
}, nil, &field, &err))
ok(wol_toggle_create(ctx, wifiLabel, n(wifiLabel), 1, { w, _ in print("wifi: \(wol_toggle_get_on(w))"); changes += 1 }, nil, &wifi, &err))
ok(wol_slider_create(ctx, 0, 100, 50, { w, _ in print("volume: \(wol_slider_get_value(w))"); changes += 1 }, nil, &volume, &err))
ok(wol_button_create(ctx, buttonLabel, n(buttonLabel), { _, _ in print("clicked"); clicks += 1 }, nil, &button, &err))
for part in [icon, title, gap] { ok(wol_widget_add_child(row, part, &err)) }
for part in [row, field, wifi, volume, button] { ok(wol_widget_add_child(list, part, &err)) }
ok(wol_window_set_content(win, list, &err))
ok(wol_window_set_close_handler(win, { _, _ in print("closed"); closed += 1 }, nil, &err))

refused(wol_image_create(ctx, badSource, n(badSource), 24, &bad, &err), "bad image")
refused(wol_slider_set_value(volume, 101, &err), "out of range")
ok(wol_window_show(win, &err))
ok(wol_widget_set_text(title, cloudy, n(cloudy), &err))

ok(wol_notification_create(ctx, heading, n(heading), file, n(file), &note, &err))
ok(wol_notification_add_action(note, key, n(key), open, n(open), &err))
ok(wol_notification_set_action_handler(note, { _, k, _ in print("action: \(text(k))"); pressed += 1 }, nil, &err))
ok(wol_notification_send(note, &err))

for _ in 0..<100 where !(changes >= 3 && closed > 0 && pressed > 0 && clicks > 0) { ok(wol_context_dispatch(ctx, 100, &err)) }
print("changes: \(changes) closed: \(closed) pressed: \(pressed) clicks: \(clicks)")

wol_notification_release(note)
for part in [list, row, icon, title, gap, field, wifi, volume, button] { wol_widget_release(part) }
wol_window_release(win)
wol_context_release(ctx)
