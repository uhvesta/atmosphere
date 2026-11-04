// Command polyglot calls C, Rust and Zig through cgo.
package main

// #include "examples/interop/polyglot.h"
import "C"

import (
	"fmt"
	"runtime"
)

func main() {
	fmt.Printf("hello from Go on %s, calling %s, %s and %s (2 + 3 = %d)\n",
		runtime.GOARCH,
		C.GoString(C.c_name()),
		C.GoString(C.rust_name()),
		C.GoString(C.zig_name()),
		int(C.zig_add(2, 3)))
}
