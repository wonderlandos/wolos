/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* clipboard: libwol 1.3 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

static void show(const char *what, wol_error *e) {
    size_t n = 0;
    const char *m = wol_error_get_message(e, &n);
    printf("%s: %d %.*s\n", what, (int)wol_error_get_code(e), (int)n, m);
    wol_error_release(e);
}

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.clip"), &ctx, &err) != WOL_OK)
        return 1;
    if (wol_clipboard_set_text(ctx, S("hello"), &err) != WOL_OK) {
        show("set", err);
        return 1;
    }
    wol_string *s = NULL;
    if (wol_clipboard_copy_text(ctx, &s, &err) != WOL_OK) {
        show("copy", err);
        return 1;
    }
    size_t n = 0;
    const char *b = wol_string_get_bytes(s, &n);
    printf("text: %.*s\n", (int)n, b);
    wol_string_release(s);

    if (wol_clipboard_copy_text(ctx, NULL, &err) == WOL_ERR_INVALID_ARGUMENT) {
        show("null out", err);
        err = NULL;
    }
    static char big[48 * 1024 + 1];
    memset(big, 'x', sizeof big);
    if (wol_clipboard_set_text(ctx, big, sizeof big, &err) == WOL_ERR_INVALID_ARGUMENT) {
        printf("too long refused\n");
        wol_error_release(err);
        err = NULL;
    }
    if (wol_clipboard_set_text(ctx, S("denied"), &err) == WOL_ERR_PERMISSION_DENIED) {
        show("denied", err);
        err = NULL;
    }
    wol_context_release(ctx);
    return 0;
}
