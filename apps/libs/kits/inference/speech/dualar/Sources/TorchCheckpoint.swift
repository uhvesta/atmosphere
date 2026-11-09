import Foundation

/// A PyTorch checkpoint, the zip `torch.save` writes, read without
/// Python: its pickled dictionary of tensors, each found in the file by
/// where its bytes start. Only what a saved state dictionary uses is
/// understood; anything else is refused rather than run.
struct TorchCheckpoint: Sendable {
    /// A tensor's place in the file.
    struct Tensor: Hashable, Sendable {
        var shape: [Int]
        var type: ScalarType
        /// Where its first byte is, from the start of the file.
        var offset: Int

        var count: Int { shape.reduce(1, *) }
        var byteCount: Int { count * type.size }
    }

    enum ScalarType: String, Hashable, Sendable {
        case float32 = "FloatStorage"
        case float16 = "HalfStorage"
        case bfloat16 = "BFloat16Storage"

        var size: Int { self == .float32 ? 4 : 2 }
    }

    /// What's wrong with a file that isn't a checkpoint this can read.
    enum Failure: Error, Hashable, Sendable, CustomStringConvertible {
        case unreadable(reason: String)
        case notAZip(reason: String)
        case compressed(entry: String)
        case missingEntry(String)
        case unsupportedPickle(opcode: UInt8, at: Int)
        case unexpected(reason: String)

        var description: String {
            switch self {
            case .unreadable(let reason): "it can't be read: \(reason)"
            case .notAZip(let reason): "it isn't a zip: \(reason)"
            case .compressed(let entry): "its \(entry) is compressed, which torch.save never does"
            case .missingEntry(let name): "it has no \(name)"
            case .unsupportedPickle(let opcode, let position):
                "its pickle uses opcode 0x\(String(opcode, radix: 16)) at byte \(position), which a state "
                    + "dictionary doesn't"
            case .unexpected(let reason): "its pickle isn't a dictionary of tensors: \(reason)"
            }
        }
    }

    /// The tensors by name, as the state dictionary has them.
    let tensors: [String: Tensor]
    /// The file's bytes, mapped rather than read.
    let data: Data

    init(url: URL) throws(Failure) {
        let data: Data
        do {
            data = try Data(contentsOf: url, options: .alwaysMapped)
        } catch {
            throw .unreadable(reason: error.localizedDescription)
        }
        try self.init(data: data)
    }

    init(data: Data) throws(Failure) {
        self.data = data
        let entries = try Zip.entries(of: data)
        guard let pickle = entries.first(where: { $0.key.hasSuffix("/data.pkl") }) else {
            throw .missingEntry("data.pkl")
        }
        let prefix = String(pickle.key.dropLast("data.pkl".count))
        let storages = try Unpickler(data[pickle.value]).stateDictionary()
        var tensors: [String: Tensor] = [:]
        for (name, stored) in storages {
            guard let range = entries[prefix + "data/" + stored.key] else {
                throw .missingEntry("data/\(stored.key) for \(name)")
            }
            let tensor = Tensor(
                shape: stored.shape, type: stored.type, offset: range.lowerBound + stored.offset * stored.type.size)
            guard stored.isContiguous, tensor.offset + tensor.byteCount <= range.upperBound else {
                throw .unexpected(reason: "\(name) isn't laid out in one piece in its storage")
            }
            tensors[name] = tensor
        }
        self.tensors = tensors
    }

    /// `tensor`'s bytes.
    func bytes(of tensor: Tensor) -> Data {
        data[(data.startIndex + tensor.offset)..<(data.startIndex + tensor.offset + tensor.byteCount)]
    }
}

/// The few parts of the zip format `torch.save` writes: stored entries,
/// with zip64 records for large files.
private enum Zip {
    /// Each entry's name and where its bytes are.
    static func entries(of data: Data) throws(TorchCheckpoint.Failure) -> [String: Range<Int>] {
        let bytes = Bytes(data)
        let searchStart = max(0, bytes.count - 65_557)
        guard
            let end = stride(from: bytes.count - 22, through: searchStart, by: -1).first(where: {
                bytes.uint32($0) == 0x0605_4b50
            })
        else { throw .notAZip(reason: "no end of central directory") }
        var count = Int(bytes.uint16(end + 10))
        var directory = Int(bytes.uint32(end + 16))
        if bytes.uint32(end - 20) == 0x0706_4b50 {
            let record = Int(bytes.uint64(end - 12))
            guard record + 56 <= bytes.count, bytes.uint32(record) == 0x0606_4b50 else {
                throw .notAZip(reason: "a damaged zip64 record")
            }
            count = Int(bytes.uint64(record + 32))
            directory = Int(bytes.uint64(record + 48))
        }
        var entries: [String: Range<Int>] = [:]
        var position = directory
        for _ in 0..<count {
            guard position + 46 <= bytes.count, bytes.uint32(position) == 0x0201_4b50 else {
                throw .notAZip(reason: "a damaged central directory")
            }
            let method = bytes.uint16(position + 10)
            var size = Int(bytes.uint32(position + 24))
            let nameLength = Int(bytes.uint16(position + 28))
            let extraLength = Int(bytes.uint16(position + 30))
            let commentLength = Int(bytes.uint16(position + 32))
            var local = Int(bytes.uint32(position + 42))
            let name = String(bytes: bytes.slice(position + 46, nameLength), encoding: .utf8) ?? ""
            if size == 0xFFFF_FFFF || local == 0xFFFF_FFFF {
                (size, local) = zip64(
                    bytes, extra: position + 46 + nameLength, length: extraLength, size: size, local: local)
            }
            guard method == 0 else { throw .compressed(entry: name) }
            guard local + 30 <= bytes.count, bytes.uint32(local) == 0x0403_4b50 else {
                throw .notAZip(reason: "\(name) has no header")
            }
            let start = local + 30 + Int(bytes.uint16(local + 26)) + Int(bytes.uint16(local + 28))
            guard start + size <= bytes.count else { throw .notAZip(reason: "\(name) runs past the end") }
            entries[name] = start..<(start + size)
            position += 46 + nameLength + extraLength + commentLength
        }
        return entries
    }

    /// The sizes and offset a zip64 extra field holds in place of those
    /// too large for the central directory's four bytes.
    private static func zip64(_ bytes: Bytes, extra: Int, length: Int, size: Int, local: Int) -> (Int, Int) {
        var position = extra
        while position + 4 <= extra + length {
            let id = bytes.uint16(position)
            let fieldLength = Int(bytes.uint16(position + 2))
            if id == 0x0001 {
                var field = position + 4
                var size = size
                var local = local
                if size == 0xFFFF_FFFF {
                    // The uncompressed size comes first, then the compressed.
                    size = Int(bytes.uint64(field))
                    field += 16
                }
                if local == 0xFFFF_FFFF { local = Int(bytes.uint64(field)) }
                return (size, local)
            }
            position += 4 + fieldLength
        }
        return (size, local)
    }
}

/// Little-endian reads from a file's bytes, past the end reading zero.
private struct Bytes {
    let data: Data
    var count: Int { data.count }

    init(_ data: Data) { self.data = data }

    func byte(_ index: Int) -> UInt8 {
        index >= 0 && index < data.count ? data[data.startIndex + index] : 0
    }

    func uint16(_ index: Int) -> UInt16 { UInt16(byte(index)) | UInt16(byte(index + 1)) << 8 }
    func uint32(_ index: Int) -> UInt32 { UInt32(uint16(index)) | UInt32(uint16(index + 2)) << 16 }
    func uint64(_ index: Int) -> UInt64 { UInt64(uint32(index)) | UInt64(uint32(index + 4)) << 32 }

    func slice(_ start: Int, _ length: Int) -> Data {
        let start = min(max(start, 0), data.count)
        return data[(data.startIndex + start)..<(data.startIndex + min(start + length, data.count))]
    }
}

/// A tensor as the pickle describes it: which storage, and where in it.
struct StoredTensor: Hashable, Sendable {
    var key: String
    var type: TorchCheckpoint.ScalarType
    var offset: Int
    var shape: [Int]
    var strides: [Int]

    /// Whether its elements follow one another, last dimension fastest.
    var isContiguous: Bool {
        var expected = 1
        for (size, stride) in zip(shape, strides).reversed() where size != 1 {
            guard stride == expected else { return false }
            expected *= size
        }
        return true
    }
}

/// Python's pickle, as far as `torch.save` uses it for a state
/// dictionary: an OrderedDict of names to tensors rebuilt from storages.
/// It calls nothing; the two functions it knows by name it only records.
struct Unpickler {
    private indirect enum Value {
        case none
        case bool(Bool)
        case int(Int)
        case string(String)
        case tuple([Value])
        case global(module: String, name: String)
        case dictionary([(String, Value)])
        case storage(key: String, type: TorchCheckpoint.ScalarType)
        case tensor(StoredTensor)
        case mark
    }

    private let bytes: [UInt8]

    init(_ data: Data) { bytes = [UInt8](data) }

    func stateDictionary() throws(TorchCheckpoint.Failure) -> [(String, StoredTensor)] {
        guard case .dictionary(let items) = try run() else {
            throw .unexpected(reason: "it isn't a dictionary")
        }
        return try items.map { name, value throws(TorchCheckpoint.Failure) in
            guard case .tensor(let tensor) = value else { throw .unexpected(reason: "\(name) isn't a tensor") }
            return (name, tensor)
        }
    }

    // swiftlint:disable:next cyclomatic_complexity function_body_length
    private func run() throws(TorchCheckpoint.Failure) -> Value {
        var stack: [Value] = []
        var memo: [Int: Value] = [:]
        var position = 0

        func read(_ count: Int) throws(TorchCheckpoint.Failure) -> ArraySlice<UInt8> {
            guard position + count <= bytes.count else { throw .unexpected(reason: "it ends early") }
            defer { position += count }
            return bytes[position..<(position + count)]
        }
        func integer(_ count: Int) throws(TorchCheckpoint.Failure) -> Int {
            try read(count).reversed().reduce(0) { $0 << 8 | Int($1) }
        }
        func text(_ count: Int) throws(TorchCheckpoint.Failure) -> String {
            guard let text = String(bytes: try read(count), encoding: .utf8) else {
                throw .unexpected(reason: "a string isn't UTF-8")
            }
            return text
        }
        func line() throws(TorchCheckpoint.Failure) -> String {
            guard let end = bytes[position...].firstIndex(of: 0x0A) else { throw .unexpected(reason: "it ends early") }
            defer { position = end + 1 }
            return String(bytes: bytes[position..<end], encoding: .utf8) ?? ""
        }
        func pop() throws(TorchCheckpoint.Failure) -> Value {
            guard let value = stack.popLast() else { throw .unexpected(reason: "its stack runs out") }
            return value
        }
        func popToMark() throws(TorchCheckpoint.Failure) -> [Value] {
            guard let mark = stack.lastIndex(where: { if case .mark = $0 { true } else { false } }) else {
                throw .unexpected(reason: "it has no mark")
            }
            defer { stack.removeSubrange(mark...) }
            return Array(stack[(mark + 1)...])
        }

        while position < bytes.count {
            let start = position
            let opcode = bytes[position]
            position += 1
            switch opcode {
            case 0x80: _ = try read(1)  // PROTO
            case 0x95: _ = try read(8)  // FRAME
            case 0x2E: return try pop()  // STOP
            case 0x28: stack.append(.mark)  // MARK
            case 0x4E: stack.append(.none)  // NONE
            case 0x88: stack.append(.bool(true))  // NEWTRUE
            case 0x89: stack.append(.bool(false))  // NEWFALSE
            case 0x4B: stack.append(.int(try integer(1)))  // BININT1
            case 0x4D: stack.append(.int(try integer(2)))  // BININT2
            case 0x4A: stack.append(.int(Int(Int32(truncatingIfNeeded: try integer(4)))))  // BININT
            case 0x8A:  // LONG1
                let length = try integer(1)
                let value = try read(length)
                var number = value.reversed().reduce(0) { $0 << 8 | Int($1) }
                if let last = value.last, last & 0x80 != 0 { number -= 1 << (8 * length) }
                stack.append(.int(number))
            case 0x58: stack.append(.string(try text(try integer(4))))  // BINUNICODE
            case 0x8C: stack.append(.string(try text(try integer(1))))  // SHORT_BINUNICODE
            case 0x29: stack.append(.tuple([]))  // EMPTY_TUPLE
            case 0x85: stack.append(.tuple([try pop()]))  // TUPLE1
            case 0x86:  // TUPLE2
                let second = try pop()
                stack.append(.tuple([try pop(), second]))
            case 0x87:  // TUPLE3
                let third = try pop()
                let second = try pop()
                stack.append(.tuple([try pop(), second, third]))
            case 0x74: stack.append(.tuple(try popToMark()))  // TUPLE
            case 0x7D: stack.append(.dictionary([]))  // EMPTY_DICT
            case 0x71: memo[try integer(1)] = stack.last  // BINPUT
            case 0x72: memo[try integer(4)] = stack.last  // LONG_BINPUT
            case 0x94: memo[memo.count] = stack.last  // MEMOIZE
            case 0x68, 0x6A:  // BINGET, LONG_BINGET
                let key = try integer(opcode == 0x68 ? 1 : 4)
                guard let value = memo[key] else { throw .unexpected(reason: "it refers to \(key) before setting it") }
                stack.append(value)
            case 0x63: stack.append(.global(module: try line(), name: try line()))  // GLOBAL
            case 0x93:  // STACK_GLOBAL
                guard case .string(let name) = try pop(), case .string(let module) = try pop() else {
                    throw .unexpected(reason: "a global without a name")
                }
                stack.append(.global(module: module, name: name))
            case 0x51: stack.append(try storage(try pop()))  // BINPERSID
            case 0x52:  // REDUCE
                let arguments = try pop()
                stack.append(try call(try pop(), arguments))
            case 0x73, 0x75:  // SETITEM, SETITEMS
                let items = opcode == 0x73 ? Array([try pop(), try pop()].reversed()) : try popToMark()
                guard case .dictionary(var dictionary) = try pop() else {
                    throw .unexpected(reason: "items set on something that isn't a dictionary")
                }
                for pair in stride(from: 0, to: items.count - 1, by: 2) {
                    guard case .string(let key) = items[pair] else {
                        throw .unexpected(reason: "a key that isn't a name")
                    }
                    dictionary.append((key, items[pair + 1]))
                }
                stack.append(.dictionary(dictionary))
            default: throw .unsupportedPickle(opcode: opcode, at: start)
            }
        }
        throw .unexpected(reason: "it has no end")
    }

    /// A persistent ID: ("storage", type, key, device, size).
    private func storage(_ id: Value) throws(TorchCheckpoint.Failure) -> Value {
        guard case .tuple(let parts) = id, parts.count >= 3, case .string("storage") = parts[0],
            case .global("torch", let typeName) = parts[1], case .string(let key) = parts[2]
        else { throw .unexpected(reason: "a persistent ID that isn't a storage") }
        guard let type = TorchCheckpoint.ScalarType(rawValue: typeName) else {
            throw .unexpected(reason: "its tensors are \(typeName), not 32- or 16-bit floats")
        }
        return .storage(key: key, type: type)
    }

    private func call(_ function: Value, _ arguments: Value) throws(TorchCheckpoint.Failure) -> Value {
        guard case .global(let module, let name) = function, case .tuple(let arguments) = arguments else {
            throw .unexpected(reason: "it calls something that isn't a function")
        }
        switch (module, name) {
        case ("collections", "OrderedDict"):
            return .dictionary([])
        case ("torch._utils", "_rebuild_tensor_v2"):
            guard arguments.count >= 4, case .storage(let key, let type) = arguments[0],
                case .int(let offset) = arguments[1], case .tuple(let shape) = arguments[2],
                case .tuple(let strides) = arguments[3]
            else { throw .unexpected(reason: "a tensor without its storage, offset, shape and strides") }
            return .tensor(
                StoredTensor(
                    key: key, type: type, offset: offset, shape: try integers(shape), strides: try integers(strides)))
        default:
            throw .unexpected(reason: "it calls \(module).\(name)")
        }
    }

    private func integers(_ values: [Value]) throws(TorchCheckpoint.Failure) -> [Int] {
        try values.map { value throws(TorchCheckpoint.Failure) in
            guard case .int(let number) = value else { throw .unexpected(reason: "a size that isn't a number") }
            return number
        }
    }
}
