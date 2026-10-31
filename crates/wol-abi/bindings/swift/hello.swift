// SPDX-License-Identifier: GPL-2.0-or-later
// hello: a WolOS app in Swift on libwol (ADR 0020). Built and run by tests/bindings.rs.
import Wol

var ctx: OpaquePointer?, info: OpaquePointer?, sub: OpaquePointer?, err: OpaquePointer?
var seen = false
func text(_ s: OpaquePointer?) -> String { String(cString: wol_string_get_bytes(s, nil)) }
func ok(_ status: Int32) { precondition(status == WOL_OK, "libwol status \(status)") }
func n(_ s: String) -> Int { s.utf8.count }
let app = "org.example.hello", title = "Hello", body = "from hello", topic = "theme"

print("version: \(String(cString: wol_version_string()))")
ok(wol_context_create(app, n(app), &ctx, &err))
ok(wol_system_info_copy(ctx, &info, &err))
print("os: \(text(wol_system_info_get_os_name(info)))")
ok(wol_notification_post(ctx, title, n(title), body, n(body), &err))
print("posted")
ok(wol_subscribe(ctx, topic, n(topic), { e, _ in print("theme: \(text(wol_event_get_json(e)))"); seen = true }, nil, &sub, &err))
for _ in 0..<100 where !seen { ok(wol_context_dispatch(ctx, 100, &err)) }
wol_subscription_cancel(sub)
wol_subscription_release(sub)
wol_system_info_release(info)
wol_context_release(ctx)
