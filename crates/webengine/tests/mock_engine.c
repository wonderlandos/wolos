/* SPDX-License-Identifier: BSD-2-Clause */
/* A stand-in for libwolwebengine.so.1 (ABI 1) that does no browsing: it lets tests/shim_loader_*.rs
 * drive crates/webengine's loader and view against the C boundary. Built with `cc -shared -fPIC`;
 * with -DCHANNEL it also has the WWE_1.1 host channel, which echoes each posted message back to the
 * host's callback as "echo:<text>" (the real library hands it to the page). */
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>

typedef void (*msg_fn)(void *, const uint8_t *, size_t);
typedef struct { msg_fn fn; void *ctx; int set; } engine;
typedef struct { engine *e; } view;
typedef struct { uint32_t w, h; uint8_t *rgba; } image;

uint32_t wwe_abi_version(void) { return 1; }
const char *wwe_engine_version(void) { return "mock"; }
void *wwe_engine_new(const void *config, char *err, size_t n) {
    static engine e;
    memset(&e, 0, sizeof e);
    (void)config; (void)err; (void)n;
    return &e;
}
void wwe_engine_free(void *e) { (void)e; }
void wwe_engine_spin(void *e) { (void)e; }
void *wwe_view_new(void *e, const void *t, const char *url, char *err, size_t n) {
    static view v;
    (void)t; (void)url; (void)err; (void)n;
    v.e = e;
    return &v;
}
void wwe_view_free(void *v) { (void)v; }
void wwe_view_state(const void *v, void *out) { (void)v; memset(out, 0, 16); }
void wwe_view_resize(void *v, uint32_t w, uint32_t h, float s) { (void)v; (void)w; (void)h; (void)s; }
void wwe_view_input(void *v, const void *ev) { (void)v; (void)ev; }
void wwe_view_focus(void *v, bool f) { (void)v; (void)f; }
void wwe_view_visible(void *v, bool f) { (void)v; (void)f; }
void wwe_view_theme(void *v, bool d) { (void)v; (void)d; }
int wwe_view_screenshot(void *v, uint32_t ms, image *out, char *err, size_t n) { (void)v; (void)ms; (void)out; (void)err; (void)n; return -1; }
void wwe_image_free(image *i) { (void)i; }
void wwe_run_content_process(const char *token) { (void)token; }

#ifdef CHANNEL
int wwe_engine_set_host_channel(void *e, const char *origin, const char *init, msg_fn fn, void *ctx, char *err, size_t n) {
    engine *en = e;
    if (strncmp(origin, "file:///", 8) != 0 || !fn || !init) {
        const char *m = "bad origin";
        size_t k = strlen(m) < n - 1 ? strlen(m) : n - 1;
        memcpy(err, m, k);
        err[k] = 0;
        return -1;
    }
    en->fn = fn;
    en->ctx = ctx;
    en->set = 1;
    return 0;
}
int wwe_view_post_message(void *v, const uint8_t *text, size_t len) {
    engine *en = ((view *)v)->e;
    uint8_t buf[256];
    if (!en->set || len > 200) return -1;
    memcpy(buf, "echo:", 5);
    memcpy(buf + 5, text, len);
    en->fn(en->ctx, buf, len + 5);
    return 0;
}
#endif
