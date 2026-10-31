# SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
# services: every libwol call added since 1.2, in Crystal (ADR 0020). Built and run by tests/bindings.rs.
require "./lib_wol"

macro ok(call)
  abort "failed: " + {{call.stringify}} unless {{call}} == LibWol::WOL_OK
end

def str(s : LibWol::String*) : String
  String.new(LibWol.wol_string_get_bytes(s, nil))
end

def field(i : LibWol::Info*, key : String) : LibWol::Info*
  LibWol.wol_info_get(i, key, key.bytesize)
end

app = "org.example.services"
ctx = Pointer(LibWol::Context).null
err = Pointer(LibWol::Error).null
ok LibWol.wol_context_create(app, app.bytesize, pointerof(ctx), pointerof(err))

file, open, save, key, value, title, body = "File", "open", "Open", "width", "640", "Copying", "1 of 5"
shortcut, icon, reason = "cmd+o", "document-open-symbolic", "Exporting"

on_menu = ->(m : LibWol::Menu*, k : LibWol::String*, d : Void*) { nil }
on_tool = ->(t : LibWol::Toolbar*, k : LibWol::String*, d : Void*) { nil }
menu = Pointer(LibWol::Menu).null
ok LibWol.wol_menu_create(ctx, file, file.bytesize, on_menu, nil, pointerof(menu), pointerof(err))
ok LibWol.wol_menu_add_item(menu, open, open.bytesize, save, save.bytesize, shortcut, shortcut.bytesize, pointerof(err))
ok LibWol.wol_menu_add_separator(menu, pointerof(err))
ok LibWol.wol_menu_set_checked(menu, open, open.bytesize, 1, pointerof(err))
LibWol.wol_menu_release(menu)
puts "menu"
bar = Pointer(LibWol::Toolbar).null
ok LibWol.wol_toolbar_create(ctx, on_tool, nil, pointerof(bar), pointerof(err))
ok LibWol.wol_toolbar_add_button(bar, open, open.bytesize, save, save.bytesize, icon, icon.bytesize, pointerof(err))
ok LibWol.wol_toolbar_add_spacer(bar, pointerof(err))
LibWol.wol_toolbar_release(bar)
puts "toolbar"

note = Pointer(LibWol::Notification).null
ok LibWol.wol_notification_create(ctx, title, title.bytesize, body, body.bytesize, pointerof(note), pointerof(err))
ok LibWol.wol_notification_send(note, pointerof(err))
ok LibWol.wol_notification_replace(note, title, title.bytesize, value, value.bytesize, pointerof(err))
ok LibWol.wol_notification_withdraw(note, pointerof(err))
LibWol.wol_notification_release(note)
puts "note"

text = Pointer(LibWol::String).null
ok LibWol.wol_clipboard_set_text(ctx, value, value.bytesize, pointerof(err))
ok LibWol.wol_clipboard_copy_text(ctx, pointerof(text), pointerof(err))
puts "clipboard: #{str(text)}"
LibWol.wol_string_release(text)

ok LibWol.wol_prefs_set_string(ctx, key, key.bytesize, value, value.bytesize, pointerof(err))
ok LibWol.wol_prefs_copy_string(ctx, key, key.bytesize, pointerof(text), pointerof(err))
puts "pref: #{str(text)}"
LibWol.wol_string_release(text)
keys = Pointer(LibWol::Info).null
ok LibWol.wol_prefs_copy_keys(ctx, pointerof(keys), pointerof(err))
puts "keys: #{LibWol.wol_info_get_count(keys)} #{str(LibWol.wol_info_get_text(LibWol.wol_info_get_item(keys, 1)))}"
LibWol.wol_info_release(keys)
ok LibWol.wol_prefs_remove(ctx, key, key.bytesize, pointerof(err))

f = Pointer(LibWol::File).null
ok LibWol.wol_panel_open(ctx, save, save.bytesize, pointerof(f), pointerof(err))
buf = Bytes.new(64)
n = LibC.read(LibWol.wol_file_get_fd(f), buf.to_unsafe.as(Void*), 64)
puts "panel: #{str(LibWol.wol_file_get_name(f))} #{String.new(buf[0, n])}"
LibWol.wol_file_release(f)
ok LibWol.wol_panel_save(ctx, save, save.bytesize, key, key.bytesize, pointerof(f), pointerof(err))
LibWol.wol_file_release(f)

info = Pointer(LibWol::Info).null
ok LibWol.wol_power_copy_info(ctx, pointerof(info), pointerof(err))
percent = 0.0
LibWol.wol_info_get_number(field(field(info, "battery"), "percent"), pointerof(percent))
puts "battery: #{percent.to_i} #{LibWol.wol_info_get_bool(field(info, "on_battery"))}"
LibWol.wol_info_release(info)
held = Pointer(LibWol::Assertion).null
ok LibWol.wol_power_keep_awake(ctx, reason, reason.bytesize, pointerof(held), pointerof(err))
LibWol.wol_assertion_release(held)
puts "held"
LibWol.wol_context_release(ctx)
