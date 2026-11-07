/* Send input only after verifying the private runtime, socket and compositor peer. */
#define _GNU_SOURCE
#include "wlr-virtual-pointer-unstable-v1-client.h"
#include <stdint.h>
#include <linux/input-event-codes.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
static struct zwlr_virtual_pointer_manager_v1 *manager;
static void global(void *d, struct wl_registry *r, uint32_t id, const char *name, uint32_t version) {
    (void)d; (void)version;
    if (!strcmp(name, zwlr_virtual_pointer_manager_v1_interface.name))
        manager = wl_registry_bind(r, id, &zwlr_virtual_pointer_manager_v1_interface, 1);
}
static void removed(void *d, struct wl_registry *r, uint32_t id) { (void)d; (void)r; (void)id; }
static const struct wl_registry_listener listener = {global, removed};
static int open_runtime(const char *path) {
    char *copy = strdup(path), *save = NULL;
    if (!copy) return -1;
    int dir = open("/", O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (dir < 0) { free(copy); return -1; }
    for (char *part = strtok_r(copy, "/", &save); part; part = strtok_r(NULL, "/", &save)) {
        if (!strcmp(part, ".") || !strcmp(part, "..")) { close(dir); free(copy); return -1; }
        int next = openat(dir, part, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
        close(dir);
        if (next < 0) { free(copy); return -1; }
        dir = next;
    }
    free(copy);
    return dir;
}
int main(int argc, char **argv) {
    const char *runtime = getenv("XDG_RUNTIME_DIR");
    if ((argc != 5) || !runtime || runtime[0] != '/' || strchr(argv[1], '/') ||
        !strcmp(argv[1], ".") || !strcmp(argv[1], "..")) return 2;
    char *end = NULL;
    long expected = strtol(argv[2], &end, 10);
    if (!end || *end || expected <= 0) return 2;
    int w = atoi(argv[3]), h = atoi(argv[4]);
    if (w <= 0 || h <= 0) return 2;
    int dir = open_runtime(runtime);
    if (dir < 0) return 3;
    struct stat ds, ss;
    if (fstat(dir, &ds) || ds.st_uid != getuid() || (ds.st_mode & 0777) != 0700 ||
        fstatat(dir, argv[1], &ss, AT_SYMLINK_NOFOLLOW) || !S_ISSOCK(ss.st_mode) || ss.st_uid != getuid()) {
        close(dir); return 3;
    }
    struct sockaddr_un addr = {.sun_family = AF_UNIX};
    /* AF_UNIX connect has no no-follow/dirfd form. Pin the parent as cwd,
       and reject a changed socket or unexpected peer before sending input. */
    int n = snprintf(addr.sun_path, sizeof(addr.sun_path), "%s", argv[1]);
    if (n <= 0 || (size_t)n >= sizeof(addr.sun_path)) { close(dir); return 3; }
    if (fchdir(dir)) { close(dir); return 3; }
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0 || connect(fd, (struct sockaddr *)&addr, sizeof(addr))) {
        if (fd >= 0) close(fd);
        close(dir); return 3;
    }
    struct ucred peer;
    socklen_t peer_len = sizeof(peer);
    struct stat after;
    if (getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &peer, &peer_len) ||
        peer.uid != getuid() || peer.pid != expected ||
        fstatat(dir, argv[1], &after, AT_SYMLINK_NOFOLLOW) || !S_ISSOCK(after.st_mode) ||
        after.st_dev != ss.st_dev || after.st_ino != ss.st_ino) {
        close(fd); close(dir); return 3;
    }
    close(dir);
    struct wl_display *display = wl_display_connect_to_fd(fd);
    if (!display) { close(fd); return 3; }
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !manager) return 4;
    struct zwlr_virtual_pointer_v1 *pointer = zwlr_virtual_pointer_manager_v1_create_virtual_pointer(manager, NULL);
    char line[128];
    puts("READY"); fflush(stdout);
    while (fgets(line, sizeof line, stdin)) {
        int x, y, click;
        char extra;
        if (sscanf(line, "%d %d %d %c", &x, &y, &click, &extra) != 3 ||
            x < 0 || x >= w || y < 0 || y >= h || (click != 0 && click != 1)) return 2;
        struct timespec now; clock_gettime(CLOCK_MONOTONIC, &now);
        uint32_t t = (uint32_t)(now.tv_sec * 1000 + now.tv_nsec / 1000000);
        zwlr_virtual_pointer_v1_motion_absolute(pointer, t, x, y, w, h);
        zwlr_virtual_pointer_v1_frame(pointer);
        if (wl_display_roundtrip(display) < 0) return 4;
        if (click) {
            zwlr_virtual_pointer_v1_button(pointer, t, BTN_LEFT, WL_POINTER_BUTTON_STATE_PRESSED);
            zwlr_virtual_pointer_v1_frame(pointer);
            if (wl_display_roundtrip(display) < 0) return 4;
            zwlr_virtual_pointer_v1_button(pointer, t + 1, BTN_LEFT, WL_POINTER_BUTTON_STATE_RELEASED);
            zwlr_virtual_pointer_v1_frame(pointer);
            if (wl_display_roundtrip(display) < 0) return 4;
        }
        puts("OK"); fflush(stdout);
    }
    zwlr_virtual_pointer_v1_destroy(pointer);
    wl_display_disconnect(display);
    return 0;
}
