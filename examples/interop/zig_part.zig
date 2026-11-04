//! The Zig part of the polyglot example; see polyglot.h.

export fn zig_name() [*:0]const u8 {
    return "Zig";
}

export fn zig_add(a: i32, b: i32) i32 {
    return a + b;
}
