/* Unsafe Rust bindings must match the pinned C headers on wasm32 too. */
#include <ghostty/vt.h>
#include <stddef.h>
#define ABI(type, expected) _Static_assert(sizeof(type) == (expected), #type " wasm32 layout changed")
ABI(GhosttyString, 8);
ABI(GhosttyAllocator, 8);
ABI(GhosttyAllocatorVtable, 32);
ABI(GhosttyPointCoordinate, 8);
ABI(GhosttyPoint, 24);
ABI(GhosttyGridRef, 8);
ABI(GhosttySelection, 20);
ABI(GhosttyStyleColor, 4);
ABI(GhosttyStyle, 16);
ABI(GhosttyTerminalScrollViewport, 24);
ABI(GhosttyMouseEncoderSize, 36);
ABI(GhosttyTerminalModeConfig, 4);
ABI(GhosttyClipboardContent, 16);
ABI(GhosttyClipboardWrite, 32);
ABI(GhosttyClipboardRead, 36);
ABI(GhosttyRenderStateRowSelection, 8);
ABI(GhosttyRenderStateCursor, 20);
ABI(GhosttyRenderStateColors, 784);
ABI(GhosttyBuffer, 12);
_Static_assert(offsetof(GhosttyClipboardWrite, name) == 12, "clipboard write offset changed");
_Static_assert(offsetof(GhosttyClipboardRead, name) == 16, "clipboard read offset changed");
