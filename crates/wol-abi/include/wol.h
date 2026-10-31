/* SPDX-License-Identifier: BSD-2-Clause */
/* Copyright (c) 2026 Julia Klee */
/*
 * wol.h: the WolOS app API (libwol.so.1). ADR 0020, archicture/modules/abi.md.
 *
 * Stability: a declaration here never changes or goes away once shipped; new ones are only
 * added (symbol versions WOL_1.0, WOL_1.1, ...). Check wol_version() at run time before using
 * a newer one.
 *
 * Rules:
 * - Handles are opaque. Never look inside one.
 * - Ownership: *_create and *_copy hand you a reference: release it with the matching
 *   *_release. *_retain adds one and returns its argument. *_get_* results are borrowed from
 *   their argument and live as long as it does. Release functions accept NULL.
 * - Strings are UTF-8 with an explicit length in bytes. NULL with length 0 is the empty string.
 *   Strings libwol returns are also NUL-terminated (the NUL is not counted in the length).
 * - Calls that can fail return a wol_status. When `error` is not NULL and the call fails,
 *   *error receives an error object you release; on success *error is untouched. Pass a
 *   pointer to NULL.
 * - Threads: retain and release are safe from any thread. Every other call on a context and
 *   the objects made from it must come from one thread at a time. Callbacks run only inside
 *   wol_context_dispatch, on the thread that calls it, and must not unwind.
 * - Event loop: poll wol_context_get_fd for reading (or call dispatch with a timeout), then
 *   call wol_context_dispatch. Any language's loop can drive libwol this way.
 * - Security: libwol holds no privilege. Anything done for the app is asked of the system
 *   service, which checks the app's permission (System Settings > Privacy & Security). Every
 *   argument is checked; a bad one gives WOL_ERR_INVALID_ARGUMENT, never undefined behaviour.
 */
#ifndef WOL_H
#define WOL_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Version ---- */

#define WOL_VERSION_MAJOR 1
#define WOL_VERSION_MINOR 7
#define WOL_VERSION_PATCH 0

/* Runtime version: major << 16 | minor << 8 | patch. */
uint32_t wol_version(void);
/* Runtime version as static text, "1.1.0". */
const char *wol_version_string(void);

/* ---- Status and errors ---- */

typedef int32_t wol_status;
#define WOL_OK 0
#define WOL_ERR_INVALID_ARGUMENT 1
#define WOL_ERR_PERMISSION_DENIED 2
#define WOL_ERR_UNAVAILABLE 3
#define WOL_ERR_INTERNAL 4

typedef struct wol_error wol_error;

wol_status wol_error_get_code(const wol_error *error);
/* A sentence to show the person, borrowed from `error`. `len` may be NULL. */
const char *wol_error_get_message(const wol_error *error, size_t *len);
void wol_error_release(wol_error *error);

/* ---- Strings ---- */

typedef struct wol_string wol_string;

wol_status wol_string_create(const char *utf8, size_t len, wol_string **out, wol_error **error);
/* Bytes borrowed from `string`, NUL-terminated. `len` may be NULL. */
const char *wol_string_get_bytes(const wol_string *string, size_t *len);
wol_string *wol_string_retain(wol_string *string);
void wol_string_release(wol_string *string);

/* ---- Context and event loop ---- */

typedef struct wol_context wol_context;

/* `app_id` is reverse-DNS, e.g. "org.example.hello". */
wol_status wol_context_create(const char *app_id, size_t len, wol_context **out, wol_error **error);
wol_context *wol_context_retain(wol_context *context);
void wol_context_release(wol_context *context);
/* Readable when something is waiting for wol_context_dispatch. Owned by the context. */
int wol_context_get_fd(const wol_context *context);
/* Waits up to `timeout_ms` (-1 = until something arrives, 0 = not at all), then runs callbacks. */
wol_status wol_context_dispatch(wol_context *context, int32_t timeout_ms, wol_error **error);

/* ---- Events ---- */

typedef struct wol_event wol_event;
typedef struct wol_subscription wol_subscription;
typedef void (*wol_event_fn)(const wol_event *event, void *user_data);

/* Topics: "active-window", "open-window", "close-window", "workspace", "theme", "job". */
wol_status wol_subscribe(wol_context *context, const char *topic, size_t len, wol_event_fn callback,
                         void *user_data, wol_subscription **out, wol_error **error);
/* Stops delivery at once; still release the subscription. */
void wol_subscription_cancel(wol_subscription *subscription);
void wol_subscription_release(wol_subscription *subscription);
const wol_string *wol_event_get_topic(const wol_event *event);
/* The event as one JSON object with an "event" key (documented per topic). */
const wol_string *wol_event_get_json(const wol_event *event);
/* Keep an event past its callback. */
const wol_event *wol_event_retain(const wol_event *event);
void wol_event_release(const wol_event *event);

/* ---- System ---- */

typedef struct wol_system_info wol_system_info;
typedef int32_t wol_appearance;
#define WOL_APPEARANCE_LIGHT 0
#define WOL_APPEARANCE_DARK 1

wol_status wol_system_info_copy(wol_context *context, wol_system_info **out, wol_error **error);
const wol_string *wol_system_info_get_os_name(const wol_system_info *info);
const wol_string *wol_system_info_get_os_version(const wol_system_info *info);
const wol_string *wol_system_info_get_kernel(const wol_system_info *info);
wol_appearance wol_system_info_get_appearance(const wol_system_info *info);
/* Accent id ("violet", "blue", ...) and its colour now, 0xRRGGBB. */
const wol_string *wol_system_info_get_accent_name(const wol_system_info *info);
uint32_t wol_system_info_get_accent_rgb(const wol_system_info *info);
wol_system_info *wol_system_info_retain(wol_system_info *info);
void wol_system_info_release(wol_system_info *info);

/* ---- Notifications (permission: notifications) ---- */

wol_status wol_notification_post(wol_context *context, const char *title, size_t title_len,
                                 const char *body, size_t body_len, wol_error **error);

/* ---- Windows and Kit parts ---- */

typedef struct wol_window wol_window;
typedef struct wol_widget wol_widget;
typedef void (*wol_click_fn)(wol_widget *button, void *user_data);

wol_status wol_window_create(wol_context *context, const char *title, size_t len, wol_window **out,
                             wol_error **error);
/* A part goes in one stack or one window. NULL clears the window. */
wol_status wol_window_set_content(wol_window *window, wol_widget *content, wol_error **error);
/* Shows the window, or brings it up to date with its parts. */
wol_status wol_window_show(wol_window *window, wol_error **error);
wol_window *wol_window_retain(wol_window *window);
void wol_window_release(wol_window *window);

/* Spacing: 0, 4, 8, 12, 16 or 24. */
wol_status wol_stack_create(wol_context *context, uint32_t spacing, wol_widget **out, wol_error **error);
wol_status wol_label_create(wol_context *context, const char *text, size_t len, wol_widget **out,
                            wol_error **error);
wol_status wol_button_create(wol_context *context, const char *label, size_t len, wol_click_fn on_click,
                             void *user_data, wol_widget **out, wol_error **error);
wol_status wol_widget_add_child(wol_widget *parent, wol_widget *child, wol_error **error);
wol_widget *wol_widget_retain(wol_widget *widget);
void wol_widget_release(wol_widget *widget);

/* ---- 1.1: more Kit parts (WOL_1.1; check wol_version() >= 0x010100) ----
 * A change made through these calls redraws the window the part is in, if it is shown.
 * Change callbacks run after the person changed the value (read it with the getters). */

typedef void (*wol_change_fn)(wol_widget *widget, void *user_data);
typedef void (*wol_window_fn)(wol_window *window, void *user_data);

/* Side by side. Spacing: 0, 4, 8, 12, 16 or 24. */
wol_status wol_row_create(wol_context *context, uint32_t spacing, wol_widget **out, wol_error **error);
/* Rows with separators, scrolling when long. */
wol_status wol_list_create(wol_context *context, wol_widget **out, wol_error **error);
/* Takes the free space in a row or stack. */
wol_status wol_spacer_create(wol_context *context, wol_widget **out, wol_error **error);
/* One line of text; starts empty. on_change may be NULL. */
wol_status wol_text_field_create(wol_context *context, const char *placeholder, size_t len,
                                 wol_change_fn on_change, void *user_data, wol_widget **out,
                                 wol_error **error);
/* label may be empty. on is 0 or 1. on_change may be NULL. */
wol_status wol_toggle_create(wol_context *context, const char *label, size_t len, int32_t on,
                             wol_change_fn on_change, void *user_data, wol_widget **out,
                             wol_error **error);
/* min < max, min <= value <= max. on_change may be NULL. */
wol_status wol_slider_create(wol_context *context, double min, double max, double value,
                             wol_change_fn on_change, void *user_data, wol_widget **out,
                             wol_error **error);
/* source: an icon name ("weather-clear-symbolic") or an absolute path to a .png, .svg or .jpg.
 * size: 16, 24, 32, 48, 64, 96 or 128. */
wol_status wol_image_create(wol_context *context, const char *source, size_t len, uint32_t size,
                            wol_widget **out, wol_error **error);
/* Label text, button label or text field text. */
wol_status wol_widget_set_text(wol_widget *widget, const char *text, size_t len, wol_error **error);
wol_status wol_widget_copy_text(const wol_widget *widget, wol_string **out, wol_error **error);
/* 1 when on; 0 when off or not a toggle. */
int32_t wol_toggle_get_on(const wol_widget *widget);
wol_status wol_toggle_set_on(wol_widget *widget, int32_t on, wol_error **error);
/* 0 when not a slider. */
double wol_slider_get_value(const wol_widget *widget);
wol_status wol_slider_set_value(wol_widget *widget, double value, wol_error **error);
/* Called when the person closes the window. NULL removes it. */
wol_status wol_window_set_close_handler(wol_window *window, wol_window_fn on_close, void *user_data,
                                        wol_error **error);
/* Takes the window off screen; wol_window_show brings it back. */
wol_status wol_window_close(wol_window *window, wol_error **error);

/* ---- 1.1: notifications with buttons (permission: notifications) ---- */

typedef struct wol_notification wol_notification;
/* key is borrowed for the call. */
typedef void (*wol_action_fn)(wol_notification *notification, const wol_string *key, void *user_data);

/* Nothing is shown until wol_notification_send. */
wol_status wol_notification_create(wol_context *context, const char *title, size_t title_len,
                                   const char *body, size_t body_len, wol_notification **out,
                                   wol_error **error);
/* Up to 3 buttons. key: 1-32 of [a-z0-9_-], unique; key "default" is a click on the
 * notification itself. */
wol_status wol_notification_add_action(wol_notification *notification, const char *key, size_t key_len,
                                       const char *label, size_t label_len, wol_error **error);
wol_status wol_notification_set_action_handler(wol_notification *notification, wol_action_fn on_action,
                                               void *user_data, wol_error **error);
/* The handler runs inside wol_context_dispatch while the notification is alive. */
wol_status wol_notification_send(wol_notification *notification, wol_error **error);
wol_notification *wol_notification_retain(wol_notification *notification);
void wol_notification_release(wol_notification *notification);

/* ---- 1.2: replace and withdraw (WOL_1.2; check wol_version() >= 0x010200) ---- */

/* New text. Already sent: it changes on screen, buttons kept. Not sent yet: used when sent. Only
 * the process that posted a notification can change it. */
wol_status wol_notification_replace(wol_notification *notification, const char *title, size_t title_len,
                                    const char *body, size_t body_len, wol_error **error);
/* Takes it off the screen and out of Notification Center. Nothing happens if it was never sent. */
wol_status wol_notification_withdraw(wol_notification *notification, wol_error **error);

/* ---- 1.2: menus and toolbars ----
 * What the person chooses comes back as a key, inside wol_context_dispatch.
 * Keys: 1-32 of [a-z0-9_-], unique in their menu or toolbar. Labels: one line, up to 64 bytes.
 * A change redraws the window, if it is shown. */

typedef struct wol_menu wol_menu;
typedef struct wol_toolbar wol_toolbar;
/* key is borrowed for the call. */
typedef void (*wol_menu_fn)(wol_menu *menu, const wol_string *key, void *user_data);
typedef void (*wol_toolbar_fn)(wol_toolbar *toolbar, const wol_string *key, void *user_data);

wol_status wol_menu_create(wol_context *context, const char *title, size_t len, wol_menu_fn on_choose,
                           void *user_data, wol_menu **out, wol_error **error);
/* shortcut: "cmd+s", "cmd+shift+n", "alt+f4"; NULL with 0 for none. Up to 48 entries. */
wol_status wol_menu_add_item(wol_menu *menu, const char *key, size_t key_len, const char *label,
                             size_t label_len, const char *shortcut, size_t shortcut_len,
                             wol_error **error);
wol_status wol_menu_add_separator(wol_menu *menu, wol_error **error);
wol_status wol_menu_set_enabled(wol_menu *menu, const char *key, size_t key_len, int32_t enabled,
                                wol_error **error);
wol_status wol_menu_set_checked(wol_menu *menu, const char *key, size_t key_len, int32_t checked,
                                wol_error **error);
wol_menu *wol_menu_retain(wol_menu *menu);
void wol_menu_release(wol_menu *menu);
/* Adds it to the window's menu bar, after the others. A menu goes on one window; up to 8. */
wol_status wol_window_add_menu(wol_window *window, wol_menu *menu, wol_error **error);

wol_status wol_toolbar_create(wol_context *context, wol_toolbar_fn on_choose, void *user_data,
                              wol_toolbar **out, wol_error **error);
/* icon: an icon name, or NULL with 0 to show the label. The label is the tooltip either way. */
wol_status wol_toolbar_add_button(wol_toolbar *toolbar, const char *key, size_t key_len,
                                  const char *label, size_t label_len, const char *icon, size_t icon_len,
                                  wol_error **error);
/* Pushes what follows to the far side. Up to 16 buttons and spacers. */
wol_status wol_toolbar_add_spacer(wol_toolbar *toolbar, wol_error **error);
wol_status wol_toolbar_set_enabled(wol_toolbar *toolbar, const char *key, size_t key_len, int32_t enabled,
                                   wol_error **error);
wol_toolbar *wol_toolbar_retain(wol_toolbar *toolbar);
void wol_toolbar_release(wol_toolbar *toolbar);
/* A window has one toolbar, a toolbar goes on one window. NULL removes it. */
wol_status wol_window_set_toolbar(wol_window *window, wol_toolbar *toolbar, wol_error **error);

/* ---- 1.3: clipboard (WOL_1.3) ---- */

/* The text on the clipboard. Permission: clipboard-read. */
wol_status wol_clipboard_copy_text(wol_context *context, wol_string **out, wol_error **error);
/* Up to 48 KiB, no NUL byte. Permission: clipboard-write. */
wol_status wol_clipboard_set_text(wol_context *context, const char *text, size_t len,
                                  wol_error **error);

/* ---- 1.4: answers, preferences (WOL_1.4) ---- */

/*
 * wol_info: a service's answer, one read-only tree. A getter takes NULL and answers NULL or 0, so
 * lookups chain: wol_info_get_text(wol_info_get(info, "name", 4)). What a getter returns is
 * borrowed from its argument. The keys of each answer are listed with the call; more can appear
 * in a later version, and your code must ignore the ones it does not know.
 */
typedef struct wol_info wol_info;

/* Field `key` of an object; NULL when there is none. */
const wol_info *wol_info_get(const wol_info *info, const char *key, size_t key_len);
/* Items in a list; 0 for anything else. */
size_t wol_info_get_count(const wol_info *info);
const wol_info *wol_info_get_item(const wol_info *info, size_t index);
/* The text of a string; NULL for anything else. */
const wol_string *wol_info_get_text(const wol_info *info);
/* 1 and *out set for a number; 0 for anything else. */
int wol_info_get_number(const wol_info *info, double *out);
/* 1 for true; 0 for anything else. */
int wol_info_get_bool(const wol_info *info);
/* The whole value as JSON. */
const wol_string *wol_info_get_json(const wol_info *info);
wol_info *wol_info_retain(wol_info *info);
void wol_info_release(wol_info *info);

/*
 * Preferences of this app, for this account; no app can reach another's. Permission:
 * app-preferences. A key is 1 to 64 letters, digits, '.', '_' or '-'; a value up to 4 KiB, no
 * NUL; 256 keys per app.
 */
/* *out is NULL, and the call succeeds, when the key is not set. */
wol_status wol_prefs_copy_string(wol_context *context, const char *key, size_t key_len,
                                 wol_string **out, wol_error **error);
wol_status wol_prefs_set_string(wol_context *context, const char *key, size_t key_len,
                                const char *value, size_t value_len, wol_error **error);
wol_status wol_prefs_remove(wol_context *context, const char *key, size_t key_len,
                            wol_error **error);
/* A list of strings. */
wol_status wol_prefs_copy_keys(wol_context *context, wol_info **out, wol_error **error);

/* ---- 1.5: files and panels (WOL_1.5) ---- */

/*
 * The person picks; the app gets that one file, already open, and no path. Permission:
 * file-panels. These calls wait for the person, up to ten minutes. *out is NULL, and the call
 * succeeds, when the person cancels.
 */
typedef struct wol_file wol_file;

/* An open file, for reading. `title` may be empty. */
wol_status wol_panel_open(wol_context *context, const char *title, size_t title_len,
                          wol_file **out, wol_error **error);
/* A file to write, emptied. `name` is the suggestion: a file name, no '/'. */
wol_status wol_panel_save(wol_context *context, const char *title, size_t title_len,
                          const char *name, size_t name_len, wol_file **out, wol_error **error);
/* The descriptor, owned by `file`: dup it to keep it past wol_file_release. */
int wol_file_get_fd(const wol_file *file);
/* The file's name, without its folder. */
const wol_string *wol_file_get_name(const wol_file *file);
wol_file *wol_file_retain(wol_file *file);
void wol_file_release(wol_file *file);

/* ---- 1.6: power (WOL_1.6) ---- */

/*
 * Permission: power-info. Keys: on_battery (bool), low_power (bool), and on a machine with a
 * battery: battery { percent (number), charging (bool), full (bool), minutes_left (number, when
 * known) }.
 */
wol_status wol_power_copy_info(wol_context *context, wol_info **out, wol_error **error);

/*
 * Keep this computer from sleeping until the assertion is released or the process ends.
 * Permission: keep-awake. `reason` is one line the person can see. An account holds at most 16.
 */
typedef struct wol_assertion wol_assertion;

wol_status wol_power_keep_awake(wol_context *context, const char *reason, size_t reason_len,
                                wol_assertion **out, wol_error **error);
wol_assertion *wol_assertion_retain(wol_assertion *assertion);
void wol_assertion_release(wol_assertion *assertion);

/* ---- 1.7: network (WOL_1.7) ---- */

/*
 * Permission: network-info. Keys: online (bool), interfaces (list of { name, kind: "ethernet",
 * "wifi" or "other", up (bool) }). No address, hardware address or network name: an app does not
 * learn where the person is.
 */
wol_status wol_network_copy_info(wol_context *context, wol_info **out, wol_error **error);

#ifdef __cplusplus
}
#endif

#endif /* WOL_H */
