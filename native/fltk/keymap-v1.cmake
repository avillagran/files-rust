# Patch only the exact pinned source, inside the generated build directory.
function(files_keymap_replace_once input needle replacement out_source)
  string(FIND "${input}" "${needle}" first)
  if(first EQUAL -1)
    message(FATAL_ERROR "files keymap: expected pinned source fragment is missing")
  endif()
  string(LENGTH "${needle}" length)
  math(EXPR next "${first} + ${length}")
  string(SUBSTRING "${input}" ${next} -1 tail)
  string(FIND "${tail}" "${needle}" second)
  if(NOT second EQUAL -1)
    message(FATAL_ERROR "files keymap: pinned source fragment is ambiguous")
  endif()
  string(REPLACE "${needle}" "${replacement}" source "${input}")
  set(${out_source} "${source}" PARENT_SCOPE)
endfunction()

function(files_keymap_source repo upstream stamp out_source)
  string(SHA256 actual "${upstream}")
  if(NOT actual STREQUAL "833c5c9b2cf309e460b9540d87ab72d6bb30e3cae0d93cace065d4b2c2b65f58")
    message(FATAL_ERROR "files keymap: pinned upstream screen source SHA mismatch")
  endif()
  set(original [=[static void wl_keyboard_keymap(void *data, struct wl_keyboard *wl_keyboard,
               uint32_t format, int32_t fd, uint32_t size) {
  struct Fl_Wayland_Screen_Driver::seat *seat =
    (struct Fl_Wayland_Screen_Driver::seat*)data;
  assert(format == WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1);

  char *map_shm = (char*)mmap(NULL, size, PROT_READ,
      wl_keyboard_get_version(wl_keyboard) >= 7 ? MAP_PRIVATE : MAP_SHARED, fd, 0);
  assert(map_shm != MAP_FAILED);

  struct xkb_keymap *xkb_keymap = xkb_keymap_new_from_string(seat->xkb_context, map_shm,
              XKB_KEYMAP_FORMAT_TEXT_V1, XKB_KEYMAP_COMPILE_NO_FLAGS);
  munmap(map_shm, size);
  close(fd);
  if (xkb_keymap) {
    struct xkb_state *xkb_state = xkb_state_new(xkb_keymap);
    xkb_keymap_unref(seat->xkb_keymap);
    if (seat->xkb_state) xkb_state_unref(seat->xkb_state);
    seat->xkb_keymap = xkb_keymap;
    seat->xkb_state = xkb_state;
  }
}]=])
  file(READ "${repo}/native/fltk/wayland-keymap-v1.inc" replacement)
  string(STRIP "${replacement}" replacement)
  files_keymap_replace_once("${upstream}" "${original}" "${replacement}" source)
  files_keymap_replace_once("${source}" "#include <sys/mman.h>"
    "#include <sys/mman.h>\n#include <sys/stat.h>" source)
  # An invalid first keymap must not leave subsequent callbacks decoding NULL.
  # Enter still updates focus/serials; only its pressed-key decoding is skipped.
  files_keymap_replace_once("${source}"
    "      (const char *) p < ((const char *) (keys)->data + (keys)->size);"
    "      seat->xkb_state && (const char *) p < ((const char *) (keys)->data + (keys)->size);" source)
  files_keymap_replace_once("${source}"
    "  seat->serial = serial;\n  static char buf[128];"
    "  seat->serial = serial;\n  if (!seat->xkb_state) return;\n  static char buf[128];" source)
  files_keymap_replace_once("${source}"
    "  xkb_state_update_mask(seat->xkb_state, mods_depressed, mods_latched, mods_locked,"
    "  if (!seat->xkb_state) return;\n  xkb_state_update_mask(seat->xkb_state, mods_depressed, mods_latched, mods_locked," source)
  string(APPEND source "\n// Strong build-input guard for this patched translation unit.\nextern \"C\" int fl_files_keymap_input_${stamp}() { return 1; }\n")
  set(${out_source} "${source}" PARENT_SCOPE)
endfunction()
