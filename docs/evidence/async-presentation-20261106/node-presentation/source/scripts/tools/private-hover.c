/* Send one pointer motion to an explicitly verified private Wayland socket. */
#include "wlr-virtual-pointer-unstable-v1-client.h"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <wayland-client.h>
static struct zwlr_virtual_pointer_manager_v1 *manager;
static void global(void *d, struct wl_registry *r, uint32_t id, const char *name, uint32_t version) {
    (void)d; (void)version;
    if (!strcmp(name, zwlr_virtual_pointer_manager_v1_interface.name))
        manager = wl_registry_bind(r, id, &zwlr_virtual_pointer_manager_v1_interface, 1);
}
static void removed(void *d, struct wl_registry *r, uint32_t id) { (void)d; (void)r; (void)id; }
static const struct wl_registry_listener listener = {global, removed};
int main(int argc, char **argv) {
    const char *runtime = getenv("XDG_RUNTIME_DIR");
    if (argc != 6 || !runtime || argv[1][0] != '/' || strncmp(argv[1], runtime, strlen(runtime)) ||
        argv[1][strlen(runtime)] != '/') return 2;
    int w = atoi(argv[2]), h = atoi(argv[3]), x = atoi(argv[4]), y = atoi(argv[5]);
    if (w <= 0 || h <= 0 || x < 0 || x >= w || y < 0 || y >= h) return 2;
    struct wl_display *display = wl_display_connect(argv[1]);
    if (!display) return 3;
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !manager) return 4;
    struct zwlr_virtual_pointer_v1 *pointer = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager, NULL);
    struct timespec now; clock_gettime(CLOCK_MONOTONIC, &now);
    zwlr_virtual_pointer_v1_motion_absolute(pointer, (uint32_t)(now.tv_sec * 1000 + now.tv_nsec / 1000000), x, y, w, h);
    zwlr_virtual_pointer_v1_frame(pointer);
    wl_display_roundtrip(display);
    zwlr_virtual_pointer_v1_destroy(pointer);
    wl_display_disconnect(display);
    return 0;
}
