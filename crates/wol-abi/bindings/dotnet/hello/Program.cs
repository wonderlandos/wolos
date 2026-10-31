// SPDX-License-Identifier: GPL-2.0-or-later
// hello: a WolOS app in C# on libwol (ADR 0020). Built and run by tests/bindings.rs.
using System.Runtime.InteropServices;
using Wol;
using static Wol.Native;

unsafe class Hello
{
    static bool seen;
    static string Text(IntPtr s) => Marshal.PtrToStringUTF8((IntPtr)wol_string_get_bytes(s, null))!;
    static void Ok(int status) { if (status != WOL_OK) throw new Exception($"libwol status {status}"); }

    static void Main()
    {
        IntPtr ctx = 0, info = 0, sub = 0, err = 0;
        using Utf8 app = new("org.example.hello"), title = new("Hello"), body = new("from hello"), topic = new("theme");
        wol_event_fn onEvent = (e, _) => { Console.WriteLine($"theme: {Text(wol_event_get_json(e))}"); seen = true; };

        Console.WriteLine($"version: {Marshal.PtrToStringUTF8((IntPtr)wol_version_string())}");
        Ok(wol_context_create(app.Ptr, app.Len, &ctx, &err));
        Ok(wol_system_info_copy(ctx, &info, &err));
        Console.WriteLine($"os: {Text(wol_system_info_get_os_name(info))}");
        Ok(wol_notification_post(ctx, title.Ptr, title.Len, body.Ptr, body.Len, &err));
        Console.WriteLine("posted");
        Ok(wol_subscribe(ctx, topic.Ptr, topic.Len, onEvent, 0, &sub, &err));
        for (int i = 0; i < 100 && !seen; i++) Ok(wol_context_dispatch(ctx, 100, &err));
        wol_subscription_cancel(sub);
        wol_subscription_release(sub);
        wol_system_info_release(info);
        wol_context_release(ctx);
        GC.KeepAlive(onEvent);
    }
}
