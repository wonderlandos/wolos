/* SPDX-License-Identifier: BSD-2-Clause */
/* media_verbs: libwol 1.16 from C (a player, a recent file, a colour, a printed file). Run by
 * tests/c_services.rs. */
#include <math.h>
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

static int keys = 0;

static void on_key(wol_media *m, wol_media_key key, double seconds, void *ud) {
    (void)m;
    (void)ud;
    printf("key: %d %.1f\n", (int)key, seconds);
    keys++;
}

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.player"), &ctx, &err) != WOL_OK)
        return 1;
    wol_media *m = NULL;
    if (wol_media_publish(ctx, &m, &err) != WOL_OK)
        return 2;
    wol_media *second = NULL;
    wol_status st = wol_media_publish(ctx, &second, &err);
    printf("twice: %d %s\n", (int)st, wol_error_get_message(err, NULL));
    wol_error_release(err);
    err = NULL;
    if (wol_media_set_handler(m, on_key, NULL, &err) != WOL_OK)
        return 3;
    if (wol_media_set_track(m, S("Song"), S("Band"), S("Record"), 215.5, &err) != WOL_OK)
        return 4;
    if (wol_media_set_art(m, S("/home/a/.cache/p/cover.png"), &err) != WOL_OK)
        return 5;
    if (wol_media_set_capabilities(m, WOL_MEDIA_CAN_NEXT | WOL_MEDIA_CAN_SEEK, &err) != WOL_OK)
        return 6;
    st = wol_media_set_state(m, 9, 0.0, &err);
    printf("bad status: %d\n", (int)st);
    wol_error_release(err);
    err = NULL;
    st = wol_media_set_state(m, WOL_MEDIA_PAUSED, -1.0, &err);
    printf("negative position: %d\n", (int)st);
    wol_error_release(err);
    err = NULL;
    st = wol_media_set_state(m, WOL_MEDIA_PAUSED, nan(""), &err);
    printf("nan position: %d\n", (int)st);
    wol_error_release(err);
    err = NULL;
    st = wol_media_set_art(m, S("cover.png"), &err);
    printf("relative art: %d\n", (int)st);
    wol_error_release(err);
    err = NULL;
    st = wol_media_set_capabilities(m, 8, &err);
    printf("unknown ability: %d\n", (int)st);
    wol_error_release(err);
    err = NULL;
    if (wol_media_set_state(m, WOL_MEDIA_PLAYING, 12.5, &err) != WOL_OK)
        return 7;
    for (int n = 0; n < 100 && keys < 2; n++)
        if (wol_context_dispatch(ctx, 100, &err) != WOL_OK)
            return 8;
    wol_media_set_handler(m, NULL, NULL, &err);
    wol_media_release(wol_media_retain(m));
    wol_media_release(m);

    if (wol_recent_add(ctx, S("/home/a/Documents/report.pdf"), &err) != WOL_OK)
        return 9;
    st = wol_recent_add(ctx, S("report.pdf"), &err);
    printf("relative recent: %d\n", (int)st);
    wol_error_release(err);
    err = NULL;

    uint32_t rgb = 0;
    int32_t picked = 0;
    if (wol_color_pick(ctx, &rgb, &picked, &err) != WOL_OK)
        return 10;
    printf("color: %06x %d\n", (unsigned)rgb, (int)picked);
    st = wol_color_pick(ctx, NULL, &picked, &err);
    printf("null rgb: %d\n", (int)st);
    wol_error_release(err);
    err = NULL;

    wol_string *s = NULL;
    if (wol_print_file(ctx, S("/home/a/Documents/report.pdf"), &s, &err) != WOL_OK)
        return 11;
    printf("print: %s\n", wol_string_get_bytes(s, NULL));
    wol_string_release(s);
    st = wol_print_file(ctx, S("/a/../b.pdf"), &s, &err);
    printf("bad print path: %d\n", (int)st);
    wol_error_release(err);
    wol_context_release(ctx);
    return 0;
}
