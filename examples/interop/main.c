#include <stdio.h>

#include "examples/interop/polyglot.h"

#if defined(__aarch64__)
#define ARCH "arm64"
#elif defined(__x86_64__)
#define ARCH "x86_64"
#else
#define ARCH "unknown"
#endif

int main(void) {
  printf("hello from C on %s, calling %s, %s and %s (2 + 3 = %d)\n", ARCH,
         c_name(), rust_name(), zig_name(), zig_add(2, 3));
  return 0;
}
