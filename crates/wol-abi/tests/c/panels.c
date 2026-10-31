/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* panels: libwol 1.5 from C. Run by tests/c_services.rs. */
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "wol.h"

#define S(x) (x), strlen(x)

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.panels"), &ctx, &err) != WOL_OK)
        return 1;
    wol_file *f = NULL;
    if (wol_panel_open(ctx, S("Open"), &f, &err) != WOL_OK)
        return 2;
    printf("name: %s\n", wol_string_get_bytes(wol_file_get_name(f), NULL));
    char buf[64];
    ssize_t n = read(wol_file_get_fd(f), buf, sizeof buf);
    printf("read: %.*s\n", (int)n, buf);
    wol_file_release(f);

    f = NULL;
    if (wol_panel_open(ctx, S("Nothing"), &f, &err) != WOL_OK)
        return 3;
    printf("cancelled: %s\n", f ? "file" : "null");

    if (wol_panel_save(ctx, S("Save"), S("out.txt"), &f, &err) != WOL_OK)
        return 4;
    printf("wrote: %s\n", wol_string_get_bytes(wol_file_get_name(f), NULL));
    wol_file_release(f);

    wol_status st = wol_panel_save(ctx, S("Save"), S("../out.txt"), &f, &err);
    printf("bad name: %d\n", (int)st);
    wol_error_release(err);
    printf("%s", wol_file_get_fd(NULL) == -1 ? "" : "null fd\n");
    wol_context_release(ctx);
    return 0;
}
