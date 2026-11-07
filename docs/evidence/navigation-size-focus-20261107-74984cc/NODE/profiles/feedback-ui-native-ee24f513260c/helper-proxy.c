#include <sys/stat.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <stdlib.h>
int main(int argc, char **argv) {
  if (argc > 1 && (!strcmp(argv[1], "measure_directory") || !strcmp(argv[1], "measure_directory_allocated"))) {
    const char *gate = getenv("FILES_UI_SIZE_GATE");
    struct stat st; struct timespec delay = {0, 10000000};
    if (!gate || gate[0] != '/') return 2;
    for (int i = 0; i < 1500; i++) {
      if (lstat(gate, &st) == 0) { if (!S_ISREG(st.st_mode) || st.st_uid != getuid()) return 3; break; }
      if (i == 1499) return 4;
      nanosleep(&delay, 0);
    }
  }
  execv(REAL_HELPER, argv);
  return 5;
}
