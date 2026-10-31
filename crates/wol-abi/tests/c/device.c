/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* device: libwol 1.12 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#include "wol.h"

#define S(x) (x), strlen(x)

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.device"), &ctx, &err) != WOL_OK)
        return 1;
    wol_info *i = NULL;
    if (wol_device_copy_list(ctx, &i, &err) != WOL_OK)
        return 2;
    const wol_info *d = wol_info_get_item(wol_info_get(i, S("devices")), 0);
    double vendor = 0;
    wol_info_get_number(wol_info_get(d, S("vendor")), &vendor);
    printf("%s %d\n", wol_string_get_bytes(wol_info_get_text(wol_info_get(d, S("name"))), NULL), (int)vendor);
    wol_info_release(i);

    wol_file *f = NULL;
    if (wol_device_open(ctx, (uint32_t)vendor, 2, &f, &err) != WOL_OK)
        return 3;
    char buf[32] = {0};
    if (read(wol_file_get_fd(f), buf, sizeof buf - 1) < 0)
        return 4;
    printf("opened %s: %s\n", wol_string_get_bytes(wol_file_get_name(f), NULL), buf);
    wol_file_release(f);
    wol_status st = wol_device_open(ctx, 70000, 2, &f, &err);
    printf("bad id: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}
