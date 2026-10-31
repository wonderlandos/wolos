/* SPDX-License-Identifier: GPL-2.0-or-later */
/* menus: libwol 1.2 menus, toolbars, notification replace and withdraw from C. Run by
 * tests/c_menus.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

static int chosen = 0;

static int fail(const char *what, wol_error *e) {
    size_t n = 0;
    const char *m = wol_error_get_message(e, &n);
    printf("%s failed: %d %.*s\n", what, (int)wol_error_get_code(e), (int)n, m);
    wol_error_release(e);
    return 1;
}

static void on_menu(wol_menu *menu, const wol_string *key, void *user_data) {
    size_t n = 0;
    const char *b = wol_string_get_bytes(key, &n);
    printf("menu: %.*s\n", (int)n, b);
    ++chosen;
    /* A change from inside the callback redraws the shown window. */
    wol_menu_set_enabled(menu, S("save"), 1, NULL);
    (void)user_data;
}

static void on_tool(wol_toolbar *toolbar, const wol_string *key, void *user_data) {
    (void)toolbar;
    size_t n = 0;
    const char *b = wol_string_get_bytes(key, &n);
    printf("tool: %.*s %s\n", (int)n, b, (const char *)user_data);
    ++chosen;
}

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_version() < 0x010200) {
        printf("needs libwol 1.2\n");
        return 1;
    }
    if (wol_context_create(S("org.example.menus"), &ctx, &err) != WOL_OK)
        return fail("context", err);

    wol_window *win = NULL;
    wol_widget *label = NULL;
    wol_menu *file = NULL;
    wol_toolbar *tools = NULL;
    if (wol_window_create(ctx, S("Editor"), &win, &err) != WOL_OK ||
        wol_label_create(ctx, S("Hello"), &label, &err) != WOL_OK ||
        wol_window_set_content(win, label, &err) != WOL_OK ||
        wol_menu_create(ctx, S("File"), on_menu, NULL, &file, &err) != WOL_OK ||
        wol_menu_add_item(file, S("open"), S("Open"), S("cmd+o"), &err) != WOL_OK ||
        wol_menu_add_separator(file, &err) != WOL_OK ||
        wol_menu_add_item(file, S("save"), S("Save"), NULL, 0, &err) != WOL_OK ||
        wol_menu_set_enabled(file, S("save"), 0, &err) != WOL_OK ||
        wol_menu_set_checked(file, S("open"), 1, &err) != WOL_OK ||
        wol_window_add_menu(win, file, &err) != WOL_OK ||
        wol_toolbar_create(ctx, on_tool, "ud", &tools, &err) != WOL_OK ||
        wol_toolbar_add_button(tools, S("back"), S("Back"), S("go-previous-symbolic"), &err) != WOL_OK ||
        wol_toolbar_add_spacer(tools, &err) != WOL_OK ||
        wol_toolbar_add_button(tools, S("share"), S("Share"), NULL, 0, &err) != WOL_OK ||
        wol_window_set_toolbar(win, tools, &err) != WOL_OK ||
        wol_window_show(win, &err) != WOL_OK)
        return fail("window", err);

    if (wol_menu_add_item(file, S("BAD KEY"), S("x"), NULL, 0, &err) == WOL_ERR_INVALID_ARGUMENT) {
        printf("bad key refused\n");
        wol_error_release(err);
        err = NULL;
    }

    wol_notification *note = NULL;
    if (wol_notification_withdraw(NULL, &err) == WOL_ERR_INVALID_ARGUMENT) {
        printf("null refused\n");
        wol_error_release(err);
        err = NULL;
    }
    if (wol_notification_create(ctx, S("Copying"), S("1 of 5"), &note, &err) != WOL_OK ||
        wol_notification_replace(note, S("Copying"), S("0 of 5"), &err) != WOL_OK ||
        wol_notification_send(note, &err) != WOL_OK ||
        wol_notification_replace(note, S("Copying"), S("2 of 5"), &err) != WOL_OK ||
        wol_notification_withdraw(note, &err) != WOL_OK ||
        wol_notification_withdraw(note, &err) != WOL_OK)
        return fail("notification", err);
    printf("withdrawn\n");

    for (int i = 0; i < 100 && chosen < 2; i++)
        if (wol_context_dispatch(ctx, 100, &err) != WOL_OK)
            return fail("dispatch", err);
    printf("chosen: %d\n", chosen);

    wol_notification_release(note);
    wol_toolbar_release(tools);
    wol_menu_release(file);
    wol_widget_release(label);
    wol_window_release(win);
    wol_context_release(ctx);
    return 0;
}
