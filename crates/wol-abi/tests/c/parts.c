/* SPDX-License-Identifier: BSD-2-Clause */
/* parts: the libwol 1.1 Kit parts and notification buttons from C. Run by tests/c_hello.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

static int changes = 0, closed = 0, pressed = 0, clicks = 0;

static int fail(const char *what, wol_error *e) {
    size_t n = 0;
    const char *m = wol_error_get_message(e, &n);
    printf("%s failed: %d %.*s\n", what, (int)wol_error_get_code(e), (int)n, m);
    wol_error_release(e);
    return 1;
}

static void on_click(wol_widget *button, void *user_data) {
    (void)button;
    (void)user_data;
    printf("clicked\n");
    ++clicks;
}

static void on_name(wol_widget *field, void *user_data) {
    (void)user_data;
    wol_string *s = NULL;
    if (wol_widget_copy_text(field, &s, NULL) == WOL_OK) {
        size_t n = 0;
        const char *b = wol_string_get_bytes(s, &n);
        printf("name: %.*s\n", (int)n, b);
        wol_string_release(s);
    }
    ++changes;
}

static void on_wifi(wol_widget *toggle, void *user_data) {
    (void)user_data;
    printf("wifi: %d\n", (int)wol_toggle_get_on(toggle));
    ++changes;
}

static void on_volume(wol_widget *slider, void *user_data) {
    (void)user_data;
    printf("volume: %.1f\n", wol_slider_get_value(slider));
    ++changes;
}

static void on_close(wol_window *window, void *user_data) {
    (void)window;
    (void)user_data;
    printf("closed\n");
    ++closed;
}

static void on_action(wol_notification *n, const wol_string *key, void *user_data) {
    (void)n;
    (void)user_data;
    size_t len = 0;
    const char *b = wol_string_get_bytes(key, &len);
    printf("action: %.*s\n", (int)len, b);
    ++pressed;
}

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_version() < 0x010100) {
        printf("needs libwol 1.1\n");
        return 1;
    }
    if (wol_context_create(S("org.example.parts"), &ctx, &err) != WOL_OK)
        return fail("context", err);

    wol_window *win = NULL;
    wol_widget *list = NULL, *row = NULL, *icon = NULL, *title = NULL, *gap = NULL, *name = NULL,
               *wifi = NULL, *volume = NULL, *button = NULL;
    if (wol_window_create(ctx, S("Parts"), &win, &err) != WOL_OK ||
        wol_list_create(ctx, &list, &err) != WOL_OK ||
        wol_row_create(ctx, 8, &row, &err) != WOL_OK ||
        wol_image_create(ctx, S("weather-clear-symbolic"), 24, &icon, &err) != WOL_OK ||
        wol_label_create(ctx, S("Sunny"), &title, &err) != WOL_OK ||
        wol_spacer_create(ctx, &gap, &err) != WOL_OK ||
        wol_text_field_create(ctx, S("Name"), on_name, NULL, &name, &err) != WOL_OK ||
        wol_toggle_create(ctx, S("Wi-Fi"), 1, on_wifi, NULL, &wifi, &err) != WOL_OK ||
        wol_slider_create(ctx, 0, 100, 50, on_volume, NULL, &volume, &err) != WOL_OK ||
        wol_button_create(ctx, S("Click Me"), on_click, NULL, &button, &err) != WOL_OK ||
        wol_widget_add_child(row, icon, &err) != WOL_OK ||
        wol_widget_add_child(row, title, &err) != WOL_OK ||
        wol_widget_add_child(row, gap, &err) != WOL_OK ||
        wol_widget_add_child(list, row, &err) != WOL_OK ||
        wol_widget_add_child(list, name, &err) != WOL_OK ||
        wol_widget_add_child(list, wifi, &err) != WOL_OK ||
        wol_widget_add_child(list, volume, &err) != WOL_OK ||
        wol_widget_add_child(list, button, &err) != WOL_OK ||
        wol_window_set_content(win, list, &err) != WOL_OK ||
        wol_window_set_close_handler(win, on_close, NULL, &err) != WOL_OK)
        return fail("window", err);
    wol_widget *bad = NULL;
    if (wol_image_create(ctx, S("/etc/../shadow.png"), 24, &bad, &err) == WOL_ERR_INVALID_ARGUMENT) {
        printf("bad image refused\n");
        wol_error_release(err);
        err = NULL;
    }
    if (wol_slider_set_value(volume, 101, &err) == WOL_ERR_INVALID_ARGUMENT) {
        printf("out of range refused\n");
        wol_error_release(err);
        err = NULL;
    }
    if (wol_window_show(win, &err) != WOL_OK)
        return fail("show", err);
    /* A change through the API redraws the shown window. */
    if (wol_widget_set_text(title, S("Cloudy"), &err) != WOL_OK)
        return fail("set_text", err);

    wol_notification *note = NULL;
    if (wol_notification_create(ctx, S("Download done"), S("parts.iso"), &note, &err) != WOL_OK ||
        wol_notification_add_action(note, S("open"), S("Open"), &err) != WOL_OK ||
        wol_notification_set_action_handler(note, on_action, NULL, &err) != WOL_OK ||
        wol_notification_send(note, &err) != WOL_OK)
        return fail("notification", err);

    for (int i = 0; i < 100 && (changes < 3 || closed == 0 || pressed == 0 || clicks == 0); i++)
        if (wol_context_dispatch(ctx, 100, &err) != WOL_OK)
            return fail("dispatch", err);
    printf("changes: %d closed: %d pressed: %d clicks: %d\n", changes, closed, pressed, clicks);

    wol_notification_release(note);
    wol_widget *parts[] = {list, row, icon, title, gap, name, wifi, volume, button};
    for (size_t i = 0; i < sizeof parts / sizeof parts[0]; i++)
        wol_widget_release(parts[i]);
    wol_window_release(win);
    wol_context_release(ctx);
    return 0;
}
