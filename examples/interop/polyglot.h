// The C ABI shared by every language in this example. Each function is
// implemented in a different language and returns that language's name.
#ifndef EXAMPLES_INTEROP_POLYGLOT_H_
#define EXAMPLES_INTEROP_POLYGLOT_H_

#include <stdint.h>

// Implemented in C (c_part.c).
const char *c_name(void);

// Implemented in Rust (rust_part.rs).
const char *rust_name(void);

// Implemented in Zig (zig_part.zig).
const char *zig_name(void);

// Implemented in Zig: adds two numbers, to check arguments cross the ABI.
int32_t zig_add(int32_t a, int32_t b);

#endif  // EXAMPLES_INTEROP_POLYGLOT_H_
