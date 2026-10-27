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
/* 0 ok, -1 bad button. */
int32_t wol_titlebar_button_origin(int32_t button, int32_t *x, int32_t *y);

/* New floating window position in the zone (ax, ay, aw, ah) = output minus reserved areas;
   has_prev != 0 cascades from (px, py). */
int32_t wol_place(int32_t w, int32_t h, int32_t ax, int32_t ay, int32_t aw, int32_t ah,
                  int32_t bar, int32_t has_prev, int32_t px, int32_t py, int32_t *x, int32_t *y);

/* Frontmost pid -> wold QoS boost. 1 sent, 0 dropped/unavailable. */
int32_t wol_qos_foreground(int32_t pid);

/* One rendered frame (CLOCK_MONOTONIC ns, damaged and screen px, render passes). No allocation. */
void wol_frame_record(uint64_t begin_ns, uint64_t end_ns, uint64_t damage_px, uint64_t screen_px,
                      uint32_t passes);

/* Frame stats JSON into buf (NUL-terminated): length, or -1 if buf is null or too small. */
int32_t wol_frame_stats_json(uint8_t *buf, size_t len);

#ifdef __cplusplus
}
#endif
#endif
