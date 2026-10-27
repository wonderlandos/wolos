/* vptr: one zwlr_virtual_pointer_v1 session for headless compositors (no pointer device to click with).
 * usage: vptr W H cmd...   m X Y (absolute move) | c (left click) | s MS (sleep)
 * build: wayland-scanner client-header/private-code wlr-virtual-pointer-unstable-v1.xml; cc ... -lwayland-client */
#include <linux/input-event-codes.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include "vp.h"

static struct zwlr_virtual_pointer_manager_v1 *mgr;
static struct wl_seat *seat;

static void global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t v) {
	if (!strcmp(iface, zwlr_virtual_pointer_manager_v1_interface.name))
		mgr = wl_registry_bind(r, name, &zwlr_virtual_pointer_manager_v1_interface, 1);
	else if (!strcmp(iface, wl_seat_interface.name))
		seat = wl_registry_bind(r, name, &wl_seat_interface, 1);
}
static void global_remove(void *d, struct wl_registry *r, uint32_t name) {}
static const struct wl_registry_listener rl = {global, global_remove};

static uint32_t now(void) {
	struct timespec t;
	clock_gettime(CLOCK_MONOTONIC, &t);
	return t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

int main(int argc, char **argv) {
	if (argc < 3) return 2;
	uint32_t w = atoi(argv[1]), h = atoi(argv[2]);
	struct wl_display *dpy = wl_display_connect(NULL);
	if (!dpy) return 1;
	wl_registry_add_listener(wl_display_get_registry(dpy), &rl, NULL);
	wl_display_roundtrip(dpy);
	if (!mgr) return 1;
	struct zwlr_virtual_pointer_v1 *p = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(mgr, seat);
	wl_display_roundtrip(dpy);
	usleep(500000); /* let clients bind the new pointer capability */
	for (int i = 3; i < argc; i++) {
		if (!strcmp(argv[i], "m") && i + 2 < argc) {
			zwlr_virtual_pointer_v1_motion_absolute(p, now(), atoi(argv[i + 1]), atoi(argv[i + 2]), w, h);
			zwlr_virtual_pointer_v1_frame(p);
			i += 2;
		} else if (!strcmp(argv[i], "c")) {
			zwlr_virtual_pointer_v1_button(p, now(), BTN_LEFT, WL_POINTER_BUTTON_STATE_PRESSED);
			zwlr_virtual_pointer_v1_frame(p);
			wl_display_flush(dpy);
			usleep(60000);
			zwlr_virtual_pointer_v1_button(p, now(), BTN_LEFT, WL_POINTER_BUTTON_STATE_RELEASED);
			zwlr_virtual_pointer_v1_frame(p);
		} else if (!strcmp(argv[i], "s") && i + 1 < argc) {
			wl_display_flush(dpy);
			usleep(atoi(argv[++i]) * 1000);
		}
		wl_display_roundtrip(dpy);
		usleep(150000);
	}
	zwlr_virtual_pointer_v1_destroy(p);
	wl_display_roundtrip(dpy);
	return 0;
}
