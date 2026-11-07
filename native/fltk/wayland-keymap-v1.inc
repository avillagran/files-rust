// Replacement callback for the verified FLTK 1.5.23 generated source copy.
// The generated translation unit retains FLTK's copyright and license.
static void wl_keyboard_keymap(void *data, struct wl_keyboard *wl_keyboard,
               uint32_t format, int32_t fd, uint32_t size) {
  struct Fl_Wayland_Screen_Driver::seat *seat =
    (struct Fl_Wayland_Screen_Driver::seat*)data;
  if (fd < 0) return;
  const uint32_t max_keymap_size = 16u * 1024u * 1024u;
  if (!seat || !seat->xkb_context || format != WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1 ||
      size == 0 || size > max_keymap_size) {
    close(fd);
    return;
  }
  struct stat backing;
  if (fstat(fd, &backing) != 0 || !S_ISREG(backing.st_mode) ||
      backing.st_size < 0 || (uint64_t)backing.st_size < size) {
    close(fd);
    return;
  }
  char *map_shm = (char*)mmap(NULL, size, PROT_READ,
      wl_keyboard_get_version(wl_keyboard) >= 7 ? MAP_PRIVATE : MAP_SHARED, fd, 0);
  close(fd);
  if (map_shm == MAP_FAILED) return;
  if (!memchr(map_shm, '\0', size)) {
    munmap(map_shm, size);
    return;
  }
  struct xkb_keymap *xkb_keymap = xkb_keymap_new_from_string(seat->xkb_context, map_shm,
              XKB_KEYMAP_FORMAT_TEXT_V1, XKB_KEYMAP_COMPILE_NO_FLAGS);
  munmap(map_shm, size);
  if (!xkb_keymap) return;
  struct xkb_state *xkb_state = xkb_state_new(xkb_keymap);
  if (!xkb_state) {
    xkb_keymap_unref(xkb_keymap);
    return;
  }
  // Publish only a complete replacement. Rejected events preserve both objects.
  struct xkb_keymap *old_keymap = seat->xkb_keymap;
  struct xkb_state *old_state = seat->xkb_state;
  seat->xkb_keymap = xkb_keymap;
  seat->xkb_state = xkb_state;
  if (old_state) xkb_state_unref(old_state);
  if (old_keymap) xkb_keymap_unref(old_keymap);
}
