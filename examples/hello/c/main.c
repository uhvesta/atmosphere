#include <stdio.h>

#if defined(__APPLE__)
#define OS "macOS"
#elif defined(__linux__)
#define OS "Linux"
#else
#define OS "unknown"
#endif

#if defined(__aarch64__)
#define ARCH "arm64"
#elif defined(__x86_64__)
#define ARCH "x86_64"
#else
#define ARCH "unknown"
#endif

int main(void) {
  printf("hello from C on %s %s\n", OS, ARCH);
  return 0;
}
