/* SPDX-License-Identifier: GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception */
/* search: libwol 1.9 from C. Run by tests/c_services.rs. */
#include <stdio.h>
#include <string.h>

#include "wol.h"

#define S(x) (x), strlen(x)

static int opened = 0;

static void on_query(wol_search_provider *p, wol_search_reply *r, const wol_string *text, void *ud) {
    (void)p;
    (void)ud;
    printf("asked: %s\n", wol_string_get_bytes(text, NULL));
    wol_error *err = NULL;
    if (wol_search_reply_add(r, S("note-1"), S("A note"), S("Notes"), &err) != WOL_OK)
        printf("add failed\n");
    wol_status st = wol_search_reply_add(r, S("x"), S(""), S(""), &err);
    printf("no title: %d\n", (int)st);
    wol_error_release(err);
}

static void on_open(wol_search_provider *p, const wol_string *key, void *ud) {
    (void)p;
    (void)ud;
    printf("open: %s\n", wol_string_get_bytes(key, NULL));
    opened = 1;
}

int main(void) {
    wol_error *err = NULL;
    wol_context *ctx = NULL;
    if (wol_context_create(S("org.example.search"), &ctx, &err) != WOL_OK)
        return 1;
    wol_info *i = NULL;
    if (wol_search_query(ctx, S("term"), &i, &err) != WOL_OK)
        return 2;
    const wol_info *rows = wol_info_get(i, S("results"));
    printf("found: %zu %s\n", wol_info_get_count(rows),
           wol_string_get_bytes(wol_info_get_text(wol_info_get(wol_info_get_item(rows, 0), S("title"))), NULL));
    wol_info_release(i);

    wol_search_provider *p = NULL;
    if (wol_search_provide(ctx, on_query, on_open, NULL, &p, &err) != WOL_OK)
        return 3;
    wol_search_provider *second = NULL;
    wol_status st = wol_search_provide(ctx, on_query, NULL, NULL, &second, &err);
    printf("twice: %d\n", (int)st);
    wol_error_release(err);
    err = NULL;
    for (int n = 0; n < 100 && !opened; n++)
        if (wol_context_dispatch(ctx, 100, &err) != WOL_OK)
            return 4;
    wol_search_provider_release(p);
    wol_context_release(ctx);
    return 0;
}
