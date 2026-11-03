/* SPDX-License-Identifier: BSD-2-Clause */
/* libwolwebengine.so.1: the C boundary over the pinned Rust browser engine (ADR 0042).
 * Built once per pin by the webengine-wolos package; crates/webengine (Rust) loads it with dlopen and
 * is the only supported caller. Every call is made on the thread that called wwe_engine_new, except
 * the waker, which the engine calls from any thread. One engine per process (one app per process).
 * Strings are UTF-8 and NUL-terminated unless a length is given. Errors are written into err/errlen
 * (NUL-terminated, truncated). The source is crates/webengine/src/engine/lib.rs; the exports come
 * only from crates/webengine/engine/wolwebengine.map. */
#ifndef WOLWEBENGINE_H
#define WOLWEBENGINE_H
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#define WWE_ABI 1

/* wwe_config.flags */
#define WWE_MULTIPROCESS 1u   /* content in a child process (re-executes the host with --content-process) */
#define WWE_SANDBOX 2u        /* the engine's own content sandbox (multiprocess only) */

/* wwe_config.features: what the page may use (the policy's features) */
#define WWE_FEATURE_WEBGL2 1u
#define WWE_FEATURE_WEBGPU 2u
#define WWE_FEATURE_SERVICE_WORKER 4u
#define WWE_FEATURE_INDEXEDDB 8u
#define WWE_FEATURE_NOTIFICATIONS 16u
#define WWE_FEATURE_WEBRTC 32u

/* wwe_allow_fn kind */
#define WWE_LOAD_NAVIGATION 0u
#define WWE_LOAD_RESOURCE 1u

typedef bool (*wwe_allow_fn)(void *ctx, const uint8_t *url, size_t len, uint32_t kind);
typedef void (*wwe_wake_fn)(void *ctx);
/* Clipboard read: copy at most cap bytes of the text into buf; return the full length, or -1. */
typedef intptr_t (*wwe_clip_get_fn)(void *ctx, uint8_t *buf, size_t cap);
typedef void (*wwe_clip_set_fn)(void *ctx, const uint8_t *text, size_t len);

typedef struct {
    uint32_t abi;               /* WWE_ABI */
    uint32_t flags;
    uint32_t features;
    uint32_t heap_mb;           /* 0: the engine's default */
    const char *profile_dir;    /* required: every file the engine writes goes under it */
    const char *user_agent;     /* NULL: the engine's */
    wwe_allow_fn allow;         /* required: called for every navigation and resource load */
    wwe_wake_fn wake;           /* NULL: poll with wwe_engine_spin */
    wwe_clip_get_fn clip_get;   /* NULL: no clipboard read */
    wwe_clip_set_fn clip_set;   /* NULL: no clipboard write */
    void *ctx;
} wwe_config;

#define WWE_TARGET_HEADLESS 0u
#define WWE_TARGET_WAYLAND 1u
typedef struct {
    uint32_t kind;
    uint32_t width, height;     /* device pixels */
    float scale;
    void *wl_display;           /* WWE_TARGET_WAYLAND: the wolkit surface node's display and */
    void *wl_surface;           /* subsurface (wolkit_surface / Engine::create_subsurface) */
} wwe_target;

#define WWE_INPUT_MOUSE_MOVE 1u
#define WWE_INPUT_MOUSE_BUTTON 2u
#define WWE_INPUT_WHEEL 3u
#define WWE_INPUT_KEY 4u
#define WWE_INPUT_IME_PREEDIT 5u
#define WWE_INPUT_IME_COMMIT 6u
#define WWE_INPUT_IME_DISMISS 7u
#define WWE_INPUT_MOUSE_LEAVE 8u
/* mods */
#define WWE_MOD_SHIFT 1u
#define WWE_MOD_CTRL 2u
#define WWE_MOD_ALT 4u
#define WWE_MOD_META 8u
typedef struct {
    uint32_t kind;
    uint32_t button;            /* 0 left, 1 middle, 2 right */
    uint32_t pressed;           /* MOUSE_BUTTON, KEY */
    uint32_t mods;
    uint32_t repeat;
    float x, y;                 /* device pixels in the view */
    float dx, dy;               /* WHEEL, device pixels */
    const char *key;            /* KEY: a key value ("a", "Enter"); IME: the text */
    const char *code;           /* KEY: a code value ("KeyA"), or NULL */
} wwe_input;

#define WWE_LOAD_NONE 0u
#define WWE_LOAD_STARTED 1u
#define WWE_LOAD_HEAD_PARSED 2u
#define WWE_LOAD_COMPLETE 3u
typedef struct {
    uint32_t load;
    uint32_t crashed;           /* content crashes seen */
    uint64_t frames;            /* new frames painted */
} wwe_state;

typedef struct {
    uint32_t width, height;
    uint8_t *rgba;              /* width*height*4, rows top to bottom; free with wwe_image_free */
} wwe_image;

typedef struct wwe_engine wwe_engine;
typedef struct wwe_view wwe_view;

uint32_t wwe_abi_version(void);
/* The pinned engine's version string (static). */
const char *wwe_engine_version(void);
wwe_engine *wwe_engine_new(const wwe_config *config, char *err, size_t errlen);
void wwe_engine_free(wwe_engine *engine);
/* Run what is pending (no blocking). */
void wwe_engine_spin(wwe_engine *engine);
wwe_view *wwe_view_new(wwe_engine *engine, const wwe_target *target, const char *url, char *err, size_t errlen);
void wwe_view_free(wwe_view *view);
void wwe_view_state(const wwe_view *view, wwe_state *out);
void wwe_view_resize(wwe_view *view, uint32_t width, uint32_t height, float scale);
void wwe_view_input(wwe_view *view, const wwe_input *event);
void wwe_view_focus(wwe_view *view, bool focused);
/* Hidden: the engine throttles the view (timers, animations, painting). */
void wwe_view_visible(wwe_view *view, bool visible);
void wwe_view_theme(wwe_view *view, bool dark);
/* Spins until the rendering is up to date and captured, at most timeout_ms. 0 ok, -1 error. */
int wwe_view_screenshot(wwe_view *view, uint32_t timeout_ms, wwe_image *out, char *err, size_t errlen);
void wwe_image_free(wwe_image *image);
/* In a content process (argv: --content-process TOKEN): runs the content until the engine ends it;
 * the host then exits. */
void wwe_run_content_process(const char *token);
#endif
