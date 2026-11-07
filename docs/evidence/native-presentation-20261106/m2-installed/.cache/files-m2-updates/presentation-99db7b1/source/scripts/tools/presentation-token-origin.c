/* Private test client: emit a fresh xdg-activation token to an inherited pipe.
 * The token is never printed or written to a persistent file. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>
#include "xdg-shell-client.h"
#include "xdg-activation-client.h"

static struct wl_display *display;
static struct wl_compositor *compositor;
static struct wl_shm *shm;
static struct wl_seat *seat;
static struct wl_keyboard *keyboard;
static struct wl_surface *surface;
static struct xdg_wm_base *wm;
static struct xdg_surface *xdgsurface;
static struct xdg_toplevel *toplevel;
static struct xdg_activation_v1 *activation;
static int output_fd, requested, done;

static void token_done(void *data, struct xdg_activation_token_v1 *token, const char *value) {
    (void)data;
    size_t n = strlen(value), sent = 0;
    if (n == 0 || n > 4096) { done = -1; return; }
    while (sent < n) {
        ssize_t r = write(output_fd, value + sent, n - sent);
        if (r < 0 && errno == EINTR) continue;
        if (r <= 0) { done = -1; return; }
        sent += (size_t)r;
    }
    xdg_activation_token_v1_destroy(token);
    close(output_fd);
    done = 1;
}
static const struct xdg_activation_token_v1_listener token_listener = { .done = token_done };
static void keymap(void *d, struct wl_keyboard *k, uint32_t f, int32_t fd, uint32_t n) {
    (void)d; (void)k; (void)f; (void)n; close(fd);
}
static void enter(void *d, struct wl_keyboard *k, uint32_t serial, struct wl_surface *s, struct wl_array *keys) {
    (void)d; (void)k; (void)serial; (void)s; (void)keys;
}
static void leave(void *d, struct wl_keyboard *k, uint32_t serial, struct wl_surface *s) {
    (void)d; (void)k; (void)serial; (void)s;
}
static void key(void *d, struct wl_keyboard *k, uint32_t serial, uint32_t when, uint32_t keycode, uint32_t state) {
    (void)d; (void)k; (void)when; (void)keycode;
    if (state != WL_KEYBOARD_KEY_STATE_PRESSED || requested) return;
    requested = 1;
    struct xdg_activation_token_v1 *token = xdg_activation_v1_get_activation_token(activation);
    xdg_activation_token_v1_add_listener(token, &token_listener, NULL);
    xdg_activation_token_v1_set_serial(token, serial, seat);
    xdg_activation_token_v1_set_surface(token, surface);
    xdg_activation_token_v1_commit(token);
}
static void modifiers(void *d, struct wl_keyboard *k, uint32_t s, uint32_t a, uint32_t b, uint32_t c, uint32_t g) {
    (void)d; (void)k; (void)s; (void)a; (void)b; (void)c; (void)g;
}
static void repeat(void *d, struct wl_keyboard *k, int32_t rate, int32_t delay) {
    (void)d; (void)k; (void)rate; (void)delay;
}
static const struct wl_keyboard_listener keyboard_listener = {
    .keymap=keymap, .enter=enter, .leave=leave, .key=key, .modifiers=modifiers, .repeat_info=repeat
};
static void capabilities(void *d, struct wl_seat *s, uint32_t caps) {
    (void)d;
    if ((caps & WL_SEAT_CAPABILITY_KEYBOARD) && !keyboard) {
        keyboard = wl_seat_get_keyboard(s); wl_keyboard_add_listener(keyboard, &keyboard_listener, NULL);
    }
}
static void seat_name(void *d, struct wl_seat *s, const char *name) { (void)d; (void)s; (void)name; }
static const struct wl_seat_listener seat_listener = { .capabilities=capabilities, .name=seat_name };
static void ping(void *d, struct xdg_wm_base *w, uint32_t serial) { (void)d; xdg_wm_base_pong(w, serial); }
static const struct xdg_wm_base_listener wm_listener = { .ping=ping };
static void configure(void *d, struct xdg_surface *s, uint32_t serial) {
    (void)d; xdg_surface_ack_configure(s, serial);
}
static const struct xdg_surface_listener surface_listener = { .configure=configure };
static void top_configure(void *d, struct xdg_toplevel *t, int32_t w, int32_t h, struct wl_array *a) {
    (void)d; (void)t; (void)w; (void)h; (void)a;
}
static void top_close(void *d, struct xdg_toplevel *t) { (void)d; (void)t; done = -1; }
static const struct xdg_toplevel_listener top_listener = { .configure=top_configure, .close=top_close };
static void global(void *d, struct wl_registry *r, uint32_t name, const char *interface, uint32_t version) {
    (void)d; (void)version;
    if (!strcmp(interface, wl_compositor_interface.name)) compositor = wl_registry_bind(r, name, &wl_compositor_interface, 1);
    else if (!strcmp(interface, wl_shm_interface.name)) shm = wl_registry_bind(r, name, &wl_shm_interface, 1);
    else if (!strcmp(interface, wl_seat_interface.name)) { seat = wl_registry_bind(r, name, &wl_seat_interface, 1); wl_seat_add_listener(seat, &seat_listener, NULL); }
    else if (!strcmp(interface, xdg_wm_base_interface.name)) { wm = wl_registry_bind(r, name, &xdg_wm_base_interface, 1); xdg_wm_base_add_listener(wm, &wm_listener, NULL); }
    else if (!strcmp(interface, xdg_activation_v1_interface.name)) activation = wl_registry_bind(r, name, &xdg_activation_v1_interface, 1);
}
static void remove_global(void *d, struct wl_registry *r, uint32_t n) { (void)d; (void)r; (void)n; }
static const struct wl_registry_listener registry_listener = { .global=global, .global_remove=remove_global };
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    output_fd = atoi(argv[1]);
    display = wl_display_connect(NULL);
    if (!display) return 3;
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &registry_listener, NULL);
    wl_display_roundtrip(display);
    if (!compositor || !shm || !seat || !wm || !activation) return 4;
    surface = wl_compositor_create_surface(compositor);
    xdgsurface = xdg_wm_base_get_xdg_surface(wm, surface);
    xdg_surface_add_listener(xdgsurface, &surface_listener, NULL);
    toplevel = xdg_surface_get_toplevel(xdgsurface);
    xdg_toplevel_add_listener(toplevel, &top_listener, NULL);
    xdg_toplevel_set_title(toplevel, "Files private presentation origin");
    xdg_toplevel_set_app_id(toplevel, "files-private-presentation-origin");
    wl_surface_commit(surface);
    wl_display_roundtrip(display);
    int fd = memfd_create("files-private-origin", MFD_CLOEXEC);
    if (fd < 0 || ftruncate(fd, 256 * 128 * 4) != 0) return 5;
    void *pixels = mmap(NULL, 256 * 128 * 4, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (pixels == MAP_FAILED) return 6;
    memset(pixels, 0x70, 256 * 128 * 4);
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, 256 * 128 * 4);
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(pool, 0, 256, 128, 256 * 4, WL_SHM_FORMAT_XRGB8888);
    wl_surface_attach(surface, buffer, 0, 0); wl_surface_damage(surface, 0, 0, 256, 128); wl_surface_commit(surface);
    while (done >= 0 && wl_display_dispatch(display) >= 0) {}
    if (!done) close(output_fd);
    return done == 1 ? 0 : 7;
}
