/* SPDX-License-Identifier: GPL-2.0-or-later */
/* C ABI of crates/realtime (librealtime.a). Keep in sync with src/ffi.rs (checked by test). */
#ifndef WOL_REALTIME_H
#define WOL_REALTIME_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

#define WOL_ANIM_OPEN 0
#define WOL_ANIM_CLOSE 1

typedef struct WolVisual {
    float scale;
    float opacity;
} WolVisual;

/* 1 done, 0 running, -1 bad kind. out may be NULL. */
int32_t wol_anim_sample(uint32_t kind, uint64_t start_ms, uint64_t now_ms, WolVisual *out);
uint64_t wol_anim_duration_ms(uint32_t kind);

/* -1 none, 0 close, 1 minimize, 2 zoom, 3 bar. */
int32_t wol_titlebar_hit(int32_t width, int32_t x, int32_t y);
int32_t wol_titlebar_height(void);
/* Left-aligned row (close leftmost): 0 close, 1 minimize, 2 zoom, -1 none; over: 1 over the group. */
int32_t wol_lights_at(int32_t count, int32_t x, int32_t y);
int32_t wol_lights_over(int32_t count, int32_t x, int32_t y);
/* Right-aligned row in a bar `width` px wide (the WolOS default): slot 0 at the right edge holds
   close, then zoom, minimise leftward; a window showing `count` buttons shows slots 0..count.
   at_w: 0 close, 1 minimize, 2 zoom, -1 none. over_w: 1 over the group, gaps included.
   slot_w: the button in a slot (same codes), -1 bad slot. wol_lights_at/over stay left-aligned. */
int32_t wol_lights_at_w(int32_t width, int32_t count, int32_t x, int32_t y);
int32_t wol_lights_over_w(int32_t width, int32_t count, int32_t x, int32_t y);
int32_t wol_lights_slot_w(int32_t slot);
uint32_t wol_lights_hover(uint32_t state, int32_t over, int32_t held);
int32_t wol_lights_glyphs(uint32_t state);
/* Window buttons for a new window: 0 none, 1 close only, 3 all. decoration: 0 no xdg-decoration,
   1 server side, 2 client side, 3 X11, 4 bare X11 (menu, tooltip). class may be NULL. */
int32_t wol_window_lights(const char *class_, int32_t decoration, int32_t transient, int32_t fixed_size);
/* 0 ok, -1 bad button. */
int32_t wol_titlebar_button_origin(int32_t button, int32_t *x, int32_t *y);

/* New floating window position in the zone (ax, ay, aw, ah) = output minus reserved areas;
   has_prev != 0 cascades from (px, py). */
int32_t wol_place(int32_t w, int32_t h, int32_t ax, int32_t ay, int32_t aw, int32_t ah,
                  int32_t bar, int32_t has_prev, int32_t px, int32_t py, int32_t *x, int32_t *y);
/* Client size for a new window: w x h fitted into the zone (title bar `bar` included), min/max
   from the toplevel (0 = unset). */
int32_t wol_fit(int32_t w, int32_t h, int32_t min_w, int32_t min_h, int32_t max_w, int32_t max_h,
                int32_t ax, int32_t ay, int32_t aw, int32_t ah, int32_t bar, int32_t *out_w,
                int32_t *out_h);
/* Dialog position centred on its parent's client rect (px, py, pw, ph), kept in the zone. */
int32_t wol_centre(int32_t w, int32_t h, int32_t px, int32_t py, int32_t pw, int32_t ph,
                   int32_t ax, int32_t ay, int32_t aw, int32_t ah, int32_t bar, int32_t *x,
                   int32_t *y);
/* An open window after the zone changed: client box in/out through x, y, w, h (non-null).
   1 when it changed, 0 when it already fit. */
int32_t wol_refit(int32_t min_w, int32_t min_h, int32_t max_w, int32_t max_h, int32_t ax,
                  int32_t ay, int32_t aw, int32_t ah, int32_t bar, int32_t *x, int32_t *y,
                  int32_t *w, int32_t *h);

/* wol-hypr gates (wm_gates.rs). New floating window: 0 skip (X11 override-redirect or borderless
   popup), 1 only clamp (X11 window that asked for its position; size_hint_flags = the
   WM_NORMAL_HINTS flags), 2 place. */
#define WOL_PLACE_SKIP 0
#define WOL_PLACE_CLAMP_ONLY 1
#define WOL_PLACE 2
int32_t wol_x11_placement(int32_t x11, int32_t override_redirect, int32_t no_borders,
                          uint32_t size_hint_flags);
/* 1 when the plugin may move the window later (cascade origin, refit). */
int32_t wol_x11_managed(int32_t x11, int32_t override_redirect, int32_t no_borders);
/* Fixed axes from client limits (0 = unset): WOL_FIXED_W | WOL_FIXED_H. */
#define WOL_FIXED_W 1u
#define WOL_FIXED_H 2u
uint32_t wol_fixed_axes(int32_t min_w, int32_t min_h, int32_t max_w, int32_t max_h);
/* Limits in/out (non-null) for wol_fit/wol_refit with the fixed axes pinned to cur_w x cur_h.
   Fixed-axes bits, -1 on a null pointer. */
int32_t wol_placement_limits(int32_t cur_w, int32_t cur_h, int32_t *min_w, int32_t *min_h,
                             int32_t *max_w, int32_t *max_h);

/* One watched key: state from the previous call (start 0), returns the next state. action (may
   be NULL): a WOL_LINE_* code in the low byte, | WOL_ACTION_CONSUME to cancel the event. */
#define WOL_KEY_SUPER 0
#define WOL_KEY_TAB 1
#define WOL_KEY_ESC 2
#define WOL_KEY_OTHER 3
#define WOL_KEYF_PRESSED 1u
#define WOL_KEYF_SUPER_HELD 2u
#define WOL_KEYF_SHIFT 4u
#define WOL_KEYF_OTHER_MODS 8u
#define WOL_KEYF_LOCKED 16u
#define WOL_KEYF_SWITCHER_MAPPED 32u
#define WOL_KEYF_SHELL_LAYER_MAPPED 64u
#define WOL_LINE_NONE 0
#define WOL_ACTION_CONSUME 0x100u
uint32_t wol_keys_step(uint32_t state, int32_t key, uint32_t flags, uint32_t *action);
/* Wire text of a WOL_LINE_* code for wol-shell.sock (static, NUL-terminated, "" for none). */
const char *wol_keys_line(int32_t code);

/* Frontmost pid -> wold QoS boost. 1 sent, 0 dropped/unavailable. */
int32_t wol_qos_foreground(int32_t pid);

/* One rendered frame (CLOCK_MONOTONIC ns, damaged and screen px, render passes). No allocation. */
void wol_frame_record(uint64_t begin_ns, uint64_t end_ns, uint64_t damage_px, uint64_t screen_px,
                      uint32_t passes);

/* Frame stats JSON into buf (NUL-terminated): length, or -1 if buf is null or too small. */
int32_t wol_frame_stats_json(uint8_t *buf, size_t len);

/* The motion table (anim.rs; the shell's curve names, docs/modules/hig.md section 7). Curve
   `name`: 0 and the cubic-bezier x1, y1, x2, y2 in out[0..4] (out may be NULL; linear is
   0, 0, 1, 1), -1 for a NULL or unknown name. curve_name: name `index` in table order (static,
   NUL-terminated), NULL past the end. */
int32_t wol_anim_curve(const char *name, float *out);
const char *wol_anim_curve_name(uint32_t index);

#ifdef __cplusplus
}
#endif
#endif
