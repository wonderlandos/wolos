/* SPDX-License-Identifier: BSD-2-Clause */
/* hello: a WolOS app in C on libwol (ADR 0020). Built and run by tests/c_hello.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

static int clicks = 0;
static int themes = 0;

static void print_str(const char *label, const wol_string *s) {
    size_t n = 0;
    const char *bytes = wol_string_get_bytes(s, &n);
    printf("%s: %.*s\n", label, (int)n, bytes);
}

static int fail(const char *what, wol_error *e) {
    size_t n = 0;
    const char *m = wol_error_get_message(e, &n);
    printf("%s failed: %d %.*s\n", what, (int)wol_error_get_code(e), (int)n, m);
    wol_error_release(e);
    return 1;
}

static void on_click(wol_widget *button, void *user_data) {
    (void)button;
    int *count = user_data;
    ++*count;
    printf("clicked\n");
}

static void on_theme(const wol_event *event, void *user_data) {
    (void)user_data;
    print_str("event", wol_event_get_topic(event));
    print_str("json", wol_event_get_json(event));
    ++themes;
}

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    printf("version: %s %u\n", wol_version_string(), (unsigned)wol_version());
    if (wol_context_create(S("org.example.hello"), &ctx, &err) != WOL_OK)
        return fail("context", err);

    wol_system_info *info = NULL;
    if (wol_system_info_copy(ctx, &info, &err) != WOL_OK)
        return fail("info", err);
    print_str("os", wol_system_info_get_os_name(info));
    printf("appearance: %s\n",
           wol_system_info_get_appearance(info) == WOL_APPEARANCE_DARK ? "dark" : "light");
    print_str("accent", wol_system_info_get_accent_name(info));
    printf("accent-rgb: %06x\n", (unsigned)wol_system_info_get_accent_rgb(info));
    wol_system_info_release(info);

    if (wol_notification_post(ctx, S("Hello"), S("from C"), &err) != WOL_OK)
        return fail("post", err);
    printf("posted\n");
    if (wol_notification_post(ctx, S("Again"), NULL, 0, &err) == WOL_ERR_PERMISSION_DENIED) {
        size_t n = 0;
        const char *m = wol_error_get_message(err, &n);
        printf("denied: %.*s\n", (int)n, m);
        wol_error_release(err);
        err = NULL;
    }
    if (wol_notification_post(ctx, NULL, 0, NULL, 0, &err) == WOL_ERR_INVALID_ARGUMENT) {
        printf("invalid argument\n");
        wol_error_release(err);
        err = NULL;
    }

    wol_subscription *sub = NULL;
    if (wol_subscribe(ctx, S("theme"), on_theme, NULL, &sub, &err) != WOL_OK)
        return fail("subscribe", err);

    wol_window *win = NULL;
    wol_widget *stack = NULL, *label = NULL, *button = NULL;
    if (wol_window_create(ctx, S("Hello"), &win, &err) != WOL_OK ||
        wol_stack_create(ctx, 8, &stack, &err) != WOL_OK ||
        wol_label_create(ctx, S("Hello, WolOS"), &label, &err) != WOL_OK ||
        wol_button_create(ctx, S("Click Me"), on_click, &clicks, &button, &err) != WOL_OK ||
        wol_widget_add_child(stack, label, &err) != WOL_OK ||
        wol_widget_add_child(stack, button, &err) != WOL_OK ||
        wol_window_set_content(win, stack, &err) != WOL_OK)
        return fail("window", err);
    if (wol_widget_add_child(button, label, &err) == WOL_ERR_INVALID_ARGUMENT) {
        printf("button has no children\n");
        wol_error_release(err);
        err = NULL;
    }
    if (wol_window_show(win, &err) != WOL_OK)
        return fail("show", err);

    printf("fd: %s\n", wol_context_get_fd(ctx) >= 0 ? "yes" : "no");
    for (int i = 0; i < 100 && (clicks == 0 || themes == 0); i++)
        if (wol_context_dispatch(ctx, 100, &err) != WOL_OK)
            return fail("dispatch", err);
    printf("clicks: %d themes: %d\n", clicks, themes);

    wol_subscription_cancel(sub);
    wol_subscription_release(sub);
    wol_widget_release(label);
    wol_widget_release(button);
    wol_widget_release(stack);
    wol_window_release(win);
    wol_context_release(ctx);
    return 0;
}
