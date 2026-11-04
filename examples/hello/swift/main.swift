import Foundation

let platform: String
#if os(macOS)
platform = "macOS"
#elseif os(Linux)
platform = "Linux"
#else
platform = "unknown"
#endif

#if arch(arm64)
let arch = "arm64"
#elseif arch(x86_64)
let arch = "x86_64"
#else
let arch = "unknown"
#endif

let greeting = String(format: "hello from Swift on %@ %@", platform, arch)
print(greeting)
print("uuid:", UUID().uuidString.count == 36 ? "ok" : "bad")
