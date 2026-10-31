// SPDX-License-Identifier: GPL-2.0-or-later
// parts: the Kit parts, a button and notification buttons from C#. Run by tests/bindings.rs.
using System.Runtime.InteropServices;
using Wol;
using static Wol.Native;

unsafe class Parts
{
    static int changes, closed, pressed, clicks;
    static string Text(IntPtr s) => Marshal.PtrToStringUTF8((IntPtr)wol_string_get_bytes(s, null))!;
    static void Ok(int status) { if (status != WOL_OK) throw new Exception($"libwol status {status}"); }

    // Delegates handed to native code stay referenced here for as long as the context lives.
    static readonly wol_click_fn OnClick = (_, _) => { Console.WriteLine("clicked"); clicks++; };
    static readonly wol_change_fn OnName = (field, _) =>
    {
        IntPtr s = 0;
        if (wol_widget_copy_text(field, &s, null) == WOL_OK) { Console.WriteLine($"name: {Text(s)}"); wol_string_release(s); }
        changes++;
    };
    static readonly wol_change_fn OnWifi = (w, _) => { Console.WriteLine($"wifi: {wol_toggle_get_on(w)}"); changes++; };
    static readonly wol_change_fn OnVolume = (w, _) => { Console.WriteLine($"volume: {wol_slider_get_value(w):F1}"); changes++; };
    static readonly wol_window_fn OnClose = (_, _) => { Console.WriteLine("closed"); closed++; };
    static readonly wol_action_fn OnAction = (_, key, _) => { Console.WriteLine($"action: {Text(key)}"); pressed++; };

    static void Refused(int status, string what, IntPtr* err)
    {
        if (status != WOL_ERR_INVALID_ARGUMENT) return;
        Console.WriteLine($"{what} refused");
        wol_error_release(*err);
        *err = 0;
    }

    static void Main()
    {
        if (wol_version() < 0x010100) throw new Exception("needs libwol 1.1");
        IntPtr ctx = 0, win = 0, note = 0, err = 0;
        IntPtr list = 0, row = 0, icon = 0, title = 0, gap = 0, field = 0, wifi = 0, volume = 0, button = 0, bad = 0;
        using Utf8 app = new("org.example.parts"), name = new("Parts"), source = new("weather-clear-symbolic"),
            sunny = new("Sunny"), cloudy = new("Cloudy"), placeholder = new("Name"), wifiLabel = new("Wi-Fi"),
            buttonLabel = new("Click Me"), badSource = new("/etc/../shadow.png"), heading = new("Download done"),
            file = new("parts.iso"), key = new("open"), open = new("Open");

        Ok(wol_context_create(app.Ptr, app.Len, &ctx, &err));
        Ok(wol_window_create(ctx, name.Ptr, name.Len, &win, &err));
        Ok(wol_list_create(ctx, &list, &err));
        Ok(wol_row_create(ctx, 8, &row, &err));
        Ok(wol_image_create(ctx, source.Ptr, source.Len, 24, &icon, &err));
        Ok(wol_label_create(ctx, sunny.Ptr, sunny.Len, &title, &err));
        Ok(wol_spacer_create(ctx, &gap, &err));
        Ok(wol_text_field_create(ctx, placeholder.Ptr, placeholder.Len, OnName, 0, &field, &err));
        Ok(wol_toggle_create(ctx, wifiLabel.Ptr, wifiLabel.Len, 1, OnWifi, 0, &wifi, &err));
        Ok(wol_slider_create(ctx, 0, 100, 50, OnVolume, 0, &volume, &err));
        Ok(wol_button_create(ctx, buttonLabel.Ptr, buttonLabel.Len, OnClick, 0, &button, &err));
        foreach (var part in new[] { icon, title, gap }) Ok(wol_widget_add_child(row, part, &err));
        foreach (var part in new[] { row, field, wifi, volume, button }) Ok(wol_widget_add_child(list, part, &err));
        Ok(wol_window_set_content(win, list, &err));
        Ok(wol_window_set_close_handler(win, OnClose, 0, &err));

        Refused(wol_image_create(ctx, badSource.Ptr, badSource.Len, 24, &bad, &err), "bad image", &err);
        Refused(wol_slider_set_value(volume, 101, &err), "out of range", &err);
        Ok(wol_window_show(win, &err));
        Ok(wol_widget_set_text(title, cloudy.Ptr, cloudy.Len, &err));

        Ok(wol_notification_create(ctx, heading.Ptr, heading.Len, file.Ptr, file.Len, &note, &err));
        Ok(wol_notification_add_action(note, key.Ptr, key.Len, open.Ptr, open.Len, &err));
        Ok(wol_notification_set_action_handler(note, OnAction, 0, &err));
        Ok(wol_notification_send(note, &err));

        for (int i = 0; i < 100 && !(changes >= 3 && closed > 0 && pressed > 0 && clicks > 0); i++)
            Ok(wol_context_dispatch(ctx, 100, &err));
        Console.WriteLine($"changes: {changes} closed: {closed} pressed: {pressed} clicks: {clicks}");

        wol_notification_release(note);
        foreach (var part in new[] { list, row, icon, title, gap, field, wifi, volume, button }) wol_widget_release(part);
        wol_window_release(win);
        wol_context_release(ctx);
    }
}
