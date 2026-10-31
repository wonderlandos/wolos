// SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception
// services: every libwol call added since 1.2, in C# (ADR 0020). Built and run by tests/bindings.rs.
using System.Runtime.InteropServices;
using Wol;
using static Wol.Native;

unsafe class Services
{
    static string Text(IntPtr s) => Marshal.PtrToStringUTF8((IntPtr)wol_string_get_bytes(s, null))!;
    static void Ok(int status) { if (status != WOL_OK) throw new Exception($"libwol status {status}"); }
    static IntPtr Field(IntPtr i, string key)
    {
        using Utf8 k = new(key);
        return wol_info_get(i, k.Ptr, k.Len);
    }

    static void Main()
    {
        IntPtr ctx = 0, err = 0;
        using Utf8 app = new("org.example.services"), file = new("File"), open = new("open"), save = new("Open"),
            key = new("width"), value = new("640"), title = new("Copying"), body = new("1 of 5"),
            shortcut = new("cmd+o"), icon = new("document-open-symbolic"), reason = new("Exporting");
        Ok(wol_context_create(app.Ptr, app.Len, &ctx, &err));

        wol_menu_fn onMenu = (_, _, _) => { };
        wol_toolbar_fn onTool = (_, _, _) => { };
        IntPtr menu = 0, bar = 0;
        Ok(wol_menu_create(ctx, file.Ptr, file.Len, onMenu, 0, &menu, &err));
        Ok(wol_menu_add_item(menu, open.Ptr, open.Len, save.Ptr, save.Len, shortcut.Ptr, shortcut.Len, &err));
        Ok(wol_menu_add_separator(menu, &err));
        Ok(wol_menu_set_checked(menu, open.Ptr, open.Len, 1, &err));
        wol_menu_release(menu);
        Console.WriteLine("menu");
        Ok(wol_toolbar_create(ctx, onTool, 0, &bar, &err));
        Ok(wol_toolbar_add_button(bar, open.Ptr, open.Len, save.Ptr, save.Len, icon.Ptr, icon.Len, &err));
        Ok(wol_toolbar_add_spacer(bar, &err));
        wol_toolbar_release(bar);
        Console.WriteLine("toolbar");

        IntPtr note = 0;
        Ok(wol_notification_create(ctx, title.Ptr, title.Len, body.Ptr, body.Len, &note, &err));
        Ok(wol_notification_send(note, &err));
        Ok(wol_notification_replace(note, title.Ptr, title.Len, value.Ptr, value.Len, &err));
        Ok(wol_notification_withdraw(note, &err));
        wol_notification_release(note);
        Console.WriteLine("note");

        IntPtr str = 0;
        Ok(wol_clipboard_set_text(ctx, value.Ptr, value.Len, &err));
        Ok(wol_clipboard_copy_text(ctx, &str, &err));
        Console.WriteLine($"clipboard: {Text(str)}");
        wol_string_release(str);

        Ok(wol_prefs_set_string(ctx, key.Ptr, key.Len, value.Ptr, value.Len, &err));
        Ok(wol_prefs_copy_string(ctx, key.Ptr, key.Len, &str, &err));
        Console.WriteLine($"pref: {Text(str)}");
        wol_string_release(str);
        IntPtr keys = 0;
        Ok(wol_prefs_copy_keys(ctx, &keys, &err));
        Console.WriteLine($"keys: {wol_info_get_count(keys)} {Text(wol_info_get_text(wol_info_get_item(keys, 1)))}");
        wol_info_release(keys);
        Ok(wol_prefs_remove(ctx, key.Ptr, key.Len, &err));

        IntPtr f = 0;
        Ok(wol_panel_open(ctx, save.Ptr, save.Len, &f, &err));
        using (var h = new Microsoft.Win32.SafeHandles.SafeFileHandle((IntPtr)wol_file_get_fd(f), false))
        using (var r = new StreamReader(new FileStream(h, FileAccess.Read)))
            Console.WriteLine($"panel: {Text(wol_file_get_name(f))} {r.ReadToEnd()}");
        wol_file_release(f);
        Ok(wol_panel_save(ctx, save.Ptr, save.Len, key.Ptr, key.Len, &f, &err));
        wol_file_release(f);

        IntPtr info = 0;
        Ok(wol_power_copy_info(ctx, &info, &err));
        double percent = 0;
        wol_info_get_number(Field(Field(info, "battery"), "percent"), &percent);
        Console.WriteLine($"battery: {(int)percent} {wol_info_get_bool(Field(info, "on_battery"))}");
        wol_info_release(info);
        IntPtr held = 0;
        Ok(wol_power_keep_awake(ctx, reason.Ptr, reason.Len, &held, &err));
        wol_assertion_release(held);
        Console.WriteLine("held");
        Ok(wol_network_copy_info(ctx, &info, &err));
        Console.WriteLine($"online: {wol_info_get_bool(Field(info, "online"))} {Text(wol_info_get_text(Field(wol_info_get_item(Field(info, "interfaces"), 0), "kind")))}");
        wol_info_release(info);
        Ok(wol_audio_copy_info(ctx, &info, &err));
        wol_info_get_number(Field(info, "volume"), &percent);
        Console.WriteLine($"volume: {(int)percent} {Text(wol_info_get_text(Field(Field(info, "now_playing"), "title")))}");
        wol_info_release(info);
        Ok(wol_audio_set_volume(ctx, 30, &err));
        Ok(wol_audio_set_muted(ctx, 1, &err));
        using Utf8 term = new("term"), rowKey = new("note-1"), rowTitle = new("A note");
        Ok(wol_search_query(ctx, term.Ptr, term.Len, &info, &err));
        Console.WriteLine($"found: {wol_info_get_count(Field(info, "results"))}");
        wol_info_release(info);
        bool opened = false;
        wol_search_fn onQuery = (_, r, t, _) =>
        {
            IntPtr e = 0;
            wol_search_reply_add(r, rowKey.Ptr, rowKey.Len, rowTitle.Ptr, rowTitle.Len, null, 0, &e);
            Console.WriteLine($"asked: {Text(t)}");
        };
        wol_search_open_fn onOpen = (_, row, _) => { Console.WriteLine($"open: {Text(row)}"); opened = true; };
        IntPtr provider = 0;
        Ok(wol_search_provide(ctx, onQuery, onOpen, 0, &provider, &err));
        for (int i = 0; i < 100 && !opened; i++) Ok(wol_context_dispatch(ctx, 100, &err));
        wol_search_provider_release(provider);
        GC.KeepAlive(onQuery);
        GC.KeepAlive(onOpen);
        Ok(wol_account_copy_info(ctx, &info, &err));
        Console.WriteLine($"account: {Text(wol_info_get_text(Field(info, "name")))}");
        wol_info_release(info);
        wol_context_release(ctx);
        GC.KeepAlive(onMenu);
        GC.KeepAlive(onTool);
    }
}
