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
#define WOL_VERSION_MINOR 1
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

#ifdef __cplusplus
}
#endif

#endif /* WOL_H */
