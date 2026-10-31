# SPDX-License-Identifier: GPL-2.0-or-later
# hello: a WolOS app in Crystal on libwol (ADR 0020). Built and run by tests/bindings.rs.
require "./lib_wol"

macro ok(call)
  abort "failed: " + {{call.stringify}} unless {{call}} == LibWol::WOL_OK
end

class State
  class_property seen = false
end

def str(s : LibWol::String*) : String
  String.new(LibWol.wol_string_get_bytes(s, nil))
end

app, title, body, topic = "org.example.hello", "Hello", "from hello", "theme"
ctx = Pointer(LibWol::Context).null
info = Pointer(LibWol::SystemInfo).null
sub = Pointer(LibWol::Subscription).null
err = Pointer(LibWol::Error).null
on_event = ->(e : LibWol::Event*, data : Void*) { puts "theme: #{str(LibWol.wol_event_get_json(e))}"; State.seen = true; nil }

puts "version: #{String.new(LibWol.wol_version_string)}"
ok LibWol.wol_context_create(app, app.bytesize, pointerof(ctx), pointerof(err))
ok LibWol.wol_system_info_copy(ctx, pointerof(info), pointerof(err))
puts "os: #{str(LibWol.wol_system_info_get_os_name(info))}"
ok LibWol.wol_notification_post(ctx, title, title.bytesize, body, body.bytesize, pointerof(err))
puts "posted"
ok LibWol.wol_subscribe(ctx, topic, topic.bytesize, on_event, nil, pointerof(sub), pointerof(err))
100.times { break if State.seen; ok LibWol.wol_context_dispatch(ctx, 100, pointerof(err)) }
LibWol.wol_subscription_cancel(sub)
LibWol.wol_subscription_release(sub)
LibWol.wol_system_info_release(info)
LibWol.wol_context_release(ctx)
