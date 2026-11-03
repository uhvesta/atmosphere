const std = @import("std");
const builtin = @import("builtin");

fn platform() [2][]const u8 {
    return .{ @tagName(builtin.os.tag), @tagName(builtin.cpu.arch) };
}

pub fn main() void {
    const p = platform();
    std.debug.print("hello from Zig on {s} {s}\n", .{ p[0], p[1] });
}

test "platform names the build target" {
    const p = platform();
    try std.testing.expect(p[0].len > 0);
    try std.testing.expect(p[1].len > 0);
}
