// SPDX-License-Identifier: GPL-2.0-or-later
// Generated from include/wol.h by crates/wol-abi/tests/bindings.rs; do not edit.
// Regenerate: WOL_BLESS=1 cargo test -p wol-abi --test bindings
using System;
using System.Runtime.InteropServices;

namespace Wol;

public static unsafe class Native
{
    const string Lib = "libwol.so.1";

    public const int WOL_VERSION_MAJOR = 1;
    public const int WOL_VERSION_MINOR = 10;
    public const int WOL_VERSION_PATCH = 0;
    public const int WOL_OK = 0;
    public const int WOL_ERR_INVALID_ARGUMENT = 1;
    public const int WOL_ERR_PERMISSION_DENIED = 2;
    public const int WOL_ERR_UNAVAILABLE = 3;
    public const int WOL_ERR_INTERNAL = 4;
    public const int WOL_APPEARANCE_LIGHT = 0;
    public const int WOL_APPEARANCE_DARK = 1;

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void wol_event_fn(IntPtr @event, IntPtr user_data);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void wol_click_fn(IntPtr button, IntPtr user_data);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void wol_change_fn(IntPtr widget, IntPtr user_data);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void wol_window_fn(IntPtr window, IntPtr user_data);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void wol_action_fn(IntPtr notification, IntPtr key, IntPtr user_data);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void wol_menu_fn(IntPtr menu, IntPtr key, IntPtr user_data);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void wol_toolbar_fn(IntPtr toolbar, IntPtr key, IntPtr user_data);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void wol_search_fn(IntPtr provider, IntPtr reply, IntPtr text, IntPtr user_data);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate void wol_search_open_fn(IntPtr provider, IntPtr key, IntPtr user_data);

    [DllImport(Lib)] public static extern uint wol_version();
    [DllImport(Lib)] public static extern byte* wol_version_string();
    [DllImport(Lib)] public static extern int wol_error_get_code(IntPtr error);
    [DllImport(Lib)] public static extern byte* wol_error_get_message(IntPtr error, nuint* len);
    [DllImport(Lib)] public static extern void wol_error_release(IntPtr error);
    [DllImport(Lib)] public static extern int wol_string_create(byte* utf8, nuint len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern byte* wol_string_get_bytes(IntPtr @string, nuint* len);
    [DllImport(Lib)] public static extern IntPtr wol_string_retain(IntPtr @string);
    [DllImport(Lib)] public static extern void wol_string_release(IntPtr @string);
    [DllImport(Lib)] public static extern int wol_context_create(byte* app_id, nuint len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_context_retain(IntPtr context);
    [DllImport(Lib)] public static extern void wol_context_release(IntPtr context);
    [DllImport(Lib)] public static extern int wol_context_get_fd(IntPtr context);
    [DllImport(Lib)] public static extern int wol_context_dispatch(IntPtr context, int timeout_ms, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_subscribe(IntPtr context, byte* topic, nuint len, wol_event_fn callback, IntPtr user_data, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern void wol_subscription_cancel(IntPtr subscription);
    [DllImport(Lib)] public static extern void wol_subscription_release(IntPtr subscription);
    [DllImport(Lib)] public static extern IntPtr wol_event_get_topic(IntPtr @event);
    [DllImport(Lib)] public static extern IntPtr wol_event_get_json(IntPtr @event);
    [DllImport(Lib)] public static extern IntPtr wol_event_retain(IntPtr @event);
    [DllImport(Lib)] public static extern void wol_event_release(IntPtr @event);
    [DllImport(Lib)] public static extern int wol_system_info_copy(IntPtr context, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_system_info_get_os_name(IntPtr info);
    [DllImport(Lib)] public static extern IntPtr wol_system_info_get_os_version(IntPtr info);
    [DllImport(Lib)] public static extern IntPtr wol_system_info_get_kernel(IntPtr info);
    [DllImport(Lib)] public static extern int wol_system_info_get_appearance(IntPtr info);
    [DllImport(Lib)] public static extern IntPtr wol_system_info_get_accent_name(IntPtr info);
    [DllImport(Lib)] public static extern uint wol_system_info_get_accent_rgb(IntPtr info);
    [DllImport(Lib)] public static extern IntPtr wol_system_info_retain(IntPtr info);
    [DllImport(Lib)] public static extern void wol_system_info_release(IntPtr info);
    [DllImport(Lib)] public static extern int wol_notification_post(IntPtr context, byte* title, nuint title_len, byte* body, nuint body_len, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_window_create(IntPtr context, byte* title, nuint len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_window_set_content(IntPtr window, IntPtr content, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_window_show(IntPtr window, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_window_retain(IntPtr window);
    [DllImport(Lib)] public static extern void wol_window_release(IntPtr window);
    [DllImport(Lib)] public static extern int wol_stack_create(IntPtr context, uint spacing, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_label_create(IntPtr context, byte* text, nuint len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_button_create(IntPtr context, byte* label, nuint len, wol_click_fn on_click, IntPtr user_data, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_widget_add_child(IntPtr parent, IntPtr child, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_widget_retain(IntPtr widget);
    [DllImport(Lib)] public static extern void wol_widget_release(IntPtr widget);
    [DllImport(Lib)] public static extern int wol_row_create(IntPtr context, uint spacing, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_list_create(IntPtr context, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_spacer_create(IntPtr context, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_text_field_create(IntPtr context, byte* placeholder, nuint len, wol_change_fn on_change, IntPtr user_data, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_toggle_create(IntPtr context, byte* label, nuint len, int on, wol_change_fn on_change, IntPtr user_data, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_slider_create(IntPtr context, double min, double max, double value, wol_change_fn on_change, IntPtr user_data, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_image_create(IntPtr context, byte* source, nuint len, uint size, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_widget_set_text(IntPtr widget, byte* text, nuint len, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_widget_copy_text(IntPtr widget, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_toggle_get_on(IntPtr widget);
    [DllImport(Lib)] public static extern int wol_toggle_set_on(IntPtr widget, int on, IntPtr* error);
    [DllImport(Lib)] public static extern double wol_slider_get_value(IntPtr widget);
    [DllImport(Lib)] public static extern int wol_slider_set_value(IntPtr widget, double value, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_window_set_close_handler(IntPtr window, wol_window_fn on_close, IntPtr user_data, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_window_close(IntPtr window, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_notification_create(IntPtr context, byte* title, nuint title_len, byte* body, nuint body_len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_notification_add_action(IntPtr notification, byte* key, nuint key_len, byte* label, nuint label_len, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_notification_set_action_handler(IntPtr notification, wol_action_fn on_action, IntPtr user_data, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_notification_send(IntPtr notification, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_notification_retain(IntPtr notification);
    [DllImport(Lib)] public static extern void wol_notification_release(IntPtr notification);
    [DllImport(Lib)] public static extern int wol_notification_replace(IntPtr notification, byte* title, nuint title_len, byte* body, nuint body_len, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_notification_withdraw(IntPtr notification, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_menu_create(IntPtr context, byte* title, nuint len, wol_menu_fn on_choose, IntPtr user_data, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_menu_add_item(IntPtr menu, byte* key, nuint key_len, byte* label, nuint label_len, byte* shortcut, nuint shortcut_len, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_menu_add_separator(IntPtr menu, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_menu_set_enabled(IntPtr menu, byte* key, nuint key_len, int enabled, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_menu_set_checked(IntPtr menu, byte* key, nuint key_len, int @checked, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_menu_retain(IntPtr menu);
    [DllImport(Lib)] public static extern void wol_menu_release(IntPtr menu);
    [DllImport(Lib)] public static extern int wol_window_add_menu(IntPtr window, IntPtr menu, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_toolbar_create(IntPtr context, wol_toolbar_fn on_choose, IntPtr user_data, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_toolbar_add_button(IntPtr toolbar, byte* key, nuint key_len, byte* label, nuint label_len, byte* icon, nuint icon_len, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_toolbar_add_spacer(IntPtr toolbar, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_toolbar_set_enabled(IntPtr toolbar, byte* key, nuint key_len, int enabled, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_toolbar_retain(IntPtr toolbar);
    [DllImport(Lib)] public static extern void wol_toolbar_release(IntPtr toolbar);
    [DllImport(Lib)] public static extern int wol_window_set_toolbar(IntPtr window, IntPtr toolbar, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_clipboard_copy_text(IntPtr context, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_clipboard_set_text(IntPtr context, byte* text, nuint len, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_info_get(IntPtr info, byte* key, nuint key_len);
    [DllImport(Lib)] public static extern nuint wol_info_get_count(IntPtr info);
    [DllImport(Lib)] public static extern IntPtr wol_info_get_item(IntPtr info, nuint index);
    [DllImport(Lib)] public static extern IntPtr wol_info_get_text(IntPtr info);
    [DllImport(Lib)] public static extern int wol_info_get_number(IntPtr info, double* @out);
    [DllImport(Lib)] public static extern int wol_info_get_bool(IntPtr info);
    [DllImport(Lib)] public static extern IntPtr wol_info_get_json(IntPtr info);
    [DllImport(Lib)] public static extern IntPtr wol_info_retain(IntPtr info);
    [DllImport(Lib)] public static extern void wol_info_release(IntPtr info);
    [DllImport(Lib)] public static extern int wol_prefs_copy_string(IntPtr context, byte* key, nuint key_len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_prefs_set_string(IntPtr context, byte* key, nuint key_len, byte* value, nuint value_len, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_prefs_remove(IntPtr context, byte* key, nuint key_len, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_prefs_copy_keys(IntPtr context, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_panel_open(IntPtr context, byte* title, nuint title_len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_panel_save(IntPtr context, byte* title, nuint title_len, byte* name, nuint name_len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_file_get_fd(IntPtr file);
    [DllImport(Lib)] public static extern IntPtr wol_file_get_name(IntPtr file);
    [DllImport(Lib)] public static extern IntPtr wol_file_retain(IntPtr file);
    [DllImport(Lib)] public static extern void wol_file_release(IntPtr file);
    [DllImport(Lib)] public static extern int wol_power_copy_info(IntPtr context, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_power_keep_awake(IntPtr context, byte* reason, nuint reason_len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_assertion_retain(IntPtr assertion);
    [DllImport(Lib)] public static extern void wol_assertion_release(IntPtr assertion);
    [DllImport(Lib)] public static extern int wol_network_copy_info(IntPtr context, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_audio_copy_info(IntPtr context, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_audio_set_volume(IntPtr context, uint percent, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_audio_set_muted(IntPtr context, int muted, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_search_query(IntPtr context, byte* text, nuint len, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_search_provide(IntPtr context, wol_search_fn on_query, wol_search_open_fn on_open, IntPtr user_data, IntPtr* @out, IntPtr* error);
    [DllImport(Lib)] public static extern int wol_search_reply_add(IntPtr reply, byte* key, nuint key_len, byte* title, nuint title_len, byte* detail, nuint detail_len, IntPtr* error);
    [DllImport(Lib)] public static extern IntPtr wol_search_provider_retain(IntPtr provider);
    [DllImport(Lib)] public static extern void wol_search_provider_release(IntPtr provider);
    [DllImport(Lib)] public static extern int wol_account_copy_info(IntPtr context, IntPtr* @out, IntPtr* error);
}
