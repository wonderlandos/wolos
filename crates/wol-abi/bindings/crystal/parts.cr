# SPDX-License-Identifier: GPL-2.0-or-later
# parts: the Kit parts, a button and notification buttons from Crystal. Run by tests/bindings.rs.
require "./lib_wol"

macro ok(call)
  abort "failed: " + {{call.stringify}} unless {{call}} == LibWol::WOL_OK
end

class State
  class_property changes = 0
  class_property closed = 0
  class_property pressed = 0
  class_property clicks = 0
end

def str(s : LibWol::String*) : String
  String.new(LibWol.wol_string_get_bytes(s, nil))
end

on_click = ->(w : LibWol::Widget*, d : Void*) { puts "clicked"; State.clicks += 1; nil }
on_name = ->(field : LibWol::Widget*, d : Void*) do
  text = Pointer(LibWol::String).null
  if LibWol.wol_widget_copy_text(field, pointerof(text), nil) == LibWol::WOL_OK
    puts "name: #{str(text)}"
    LibWol.wol_string_release(text)
  end
  State.changes += 1
  nil
end
on_wifi = ->(w : LibWol::Widget*, d : Void*) { puts "wifi: #{LibWol.wol_toggle_get_on(w)}"; State.changes += 1; nil }
on_volume = ->(w : LibWol::Widget*, d : Void*) { puts "volume: #{LibWol.wol_slider_get_value(w)}"; State.changes += 1; nil }
on_close = ->(w : LibWol::Window*, d : Void*) { puts "closed"; State.closed += 1; nil }
on_action = ->(n : LibWol::Notification*, key : LibWol::String*, d : Void*) { puts "action: #{str(key)}"; State.pressed += 1; nil }

ctx = Pointer(LibWol::Context).null
err = Pointer(LibWol::Error).null
win = Pointer(LibWol::Window).null
note = Pointer(LibWol::Notification).null
bad = Pointer(LibWol::Widget).null
list = row = icon = title = gap = name = wifi = volume = button = bad
app, win_title, source, sunny, cloudy = "org.example.parts", "Parts", "weather-clear-symbolic", "Sunny", "Cloudy"
ph, wifi_label, ok_label, bad_source = "Name", "Wi-Fi", "Click Me", "/etc/../shadow.png"
heading, file, key, open = "Download done", "parts.iso", "open", "Open"

abort "needs libwol 1.1" if LibWol.wol_version < 0x010100
ok LibWol.wol_context_create(app, app.bytesize, pointerof(ctx), pointerof(err))
ok LibWol.wol_window_create(ctx, win_title, win_title.bytesize, pointerof(win), pointerof(err))
ok LibWol.wol_list_create(ctx, pointerof(list), pointerof(err))
ok LibWol.wol_row_create(ctx, 8, pointerof(row), pointerof(err))
ok LibWol.wol_image_create(ctx, source, source.bytesize, 24, pointerof(icon), pointerof(err))
ok LibWol.wol_label_create(ctx, sunny, sunny.bytesize, pointerof(title), pointerof(err))
ok LibWol.wol_spacer_create(ctx, pointerof(gap), pointerof(err))
ok LibWol.wol_text_field_create(ctx, ph, ph.bytesize, on_name, nil, pointerof(name), pointerof(err))
ok LibWol.wol_toggle_create(ctx, wifi_label, wifi_label.bytesize, 1, on_wifi, nil, pointerof(wifi), pointerof(err))
ok LibWol.wol_slider_create(ctx, 0.0, 100.0, 50.0, on_volume, nil, pointerof(volume), pointerof(err))
ok LibWol.wol_button_create(ctx, ok_label, ok_label.bytesize, on_click, nil, pointerof(button), pointerof(err))
[icon, title, gap].each { |part| ok LibWol.wol_widget_add_child(row, part, pointerof(err)) }
[row, name, wifi, volume, button].each { |part| ok LibWol.wol_widget_add_child(list, part, pointerof(err)) }
ok LibWol.wol_window_set_content(win, list, pointerof(err))
ok LibWol.wol_window_set_close_handler(win, on_close, nil, pointerof(err))

if LibWol.wol_image_create(ctx, bad_source, bad_source.bytesize, 24, pointerof(bad), pointerof(err)) == LibWol::WOL_ERR_INVALID_ARGUMENT
  puts "bad image refused"
  LibWol.wol_error_release(err)
  err = Pointer(LibWol::Error).null
end
if LibWol.wol_slider_set_value(volume, 101.0, pointerof(err)) == LibWol::WOL_ERR_INVALID_ARGUMENT
  puts "out of range refused"
  LibWol.wol_error_release(err)
  err = Pointer(LibWol::Error).null
end
ok LibWol.wol_window_show(win, pointerof(err))
ok LibWol.wol_widget_set_text(title, cloudy, cloudy.bytesize, pointerof(err))

ok LibWol.wol_notification_create(ctx, heading, heading.bytesize, file, file.bytesize, pointerof(note), pointerof(err))
ok LibWol.wol_notification_add_action(note, key, key.bytesize, open, open.bytesize, pointerof(err))
ok LibWol.wol_notification_set_action_handler(note, on_action, nil, pointerof(err))
ok LibWol.wol_notification_send(note, pointerof(err))

100.times { break if State.changes >= 3 && State.closed > 0 && State.pressed > 0 && State.clicks > 0; ok LibWol.wol_context_dispatch(ctx, 100, pointerof(err)) }
puts "changes: #{State.changes} closed: #{State.closed} pressed: #{State.pressed} clicks: #{State.clicks}"

LibWol.wol_notification_release(note)
[list, row, icon, title, gap, name, wifi, volume, button].each { |part| LibWol.wol_widget_release(part) }
LibWol.wol_window_release(win)
LibWol.wol_context_release(ctx)
