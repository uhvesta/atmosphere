// Command hello prints the platform it was built for.
package main

import (
	"fmt"
	"runtime"
)

func main() {
	fmt.Printf("hello from Go on %s %s\n", runtime.GOOS, runtime.GOARCH)
}
