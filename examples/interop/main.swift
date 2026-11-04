import Polyglot

#if arch(arm64)
    let arch = "arm64"
#elseif arch(x86_64)
    let arch = "x86_64"
#endif

func name(_ cString: UnsafePointer<CChar>) -> String { String(cString: cString) }

let names = "\(name(c_name())), \(name(rust_name())) and \(name(zig_name()))"
print("hello from Swift on \(arch), calling \(names) (2 + 3 = \(zig_add(2, 3)))")
