import Darwin
import Foundation

/// Wire types mirroring `libs/fusefs/fskit/src/protocol.rs` and `libs/fusefs/vfs/src/types.rs`.

struct WireTimestamp: Codable {
    let secs: Int64
    let nanos: UInt32

    var timespecValue: timespec {
        timespec(tv_sec: Int(secs), tv_nsec: Int(nanos))
    }

    init(secs: Int64, nanos: UInt32) {
        self.secs = secs
        self.nanos = nanos
    }

    init(timespec ts: timespec) {
        self.secs = Int64(ts.tv_sec)
        self.nanos = UInt32(max(0, ts.tv_nsec))
    }
}

struct WireFileAttr: Codable {
    let ino: UInt64
    let kind: String
    let size: UInt64
    let blocks: UInt64
    let atime: WireTimestamp
    let mtime: WireTimestamp
    let ctime: WireTimestamp
    let mode: UInt32
    let nlink: UInt32
    let uid: UInt32
    let gid: UInt32
    let generation: UInt64
}

struct WireDirEntry: Codable {
    let ino: UInt64
    let kind: String
    let name: String
    let offset: UInt64
}

struct WireStatFs: Codable {
    let blocks: UInt64
    let bfree: UInt64
    let bavail: UInt64
    let files: UInt64
    let ffree: UInt64
    let bsize: UInt32
    let namelen: UInt32
    let frsize: UInt32
}

struct WireSetAttr: Codable {
    var mode: UInt32?
    var uid: UInt32?
    var gid: UInt32?
    var size: UInt64?
    var atime: WireTimestamp?
    var mtime: WireTimestamp?
    var expected_generation: UInt64?
}

struct WireOpenFlags: Codable {
    let read: Bool
    let write: Bool
    let append: Bool
    let truncate: Bool
    let create: Bool
    let exclusive: Bool
}

struct WireProbeResult: Codable {
    let recognition: String
    let name: String?
    let container_id: String?
}

struct WireEnvelope: Encodable {
    let cap_id: String?
    let request: WireRequest

    init(request: WireRequest, capID: String? = nil) {
        self.cap_id = capID
        self.request = request
    }
}

enum WireRequest: Encodable {
    case ping
    case probeResource(resourceURL: String)
    case lookupItem(parent: UInt64, name: String)
    case getAttributes(item: UInt64)
    case setAttributes(item: UInt64, attr: WireSetAttr)
    case enumerateDirectory(directory: UInt64, cookie: UInt64, maxEntries: Int?)
    case readSymbolicLink(item: UInt64)
    case createItem(parent: UInt64, name: String, kind: String, mode: UInt32, symlinkTarget: String?)
    case removeItem(parent: UInt64, name: String, isDirectory: Bool)
    case renameItem(sourceParent: UInt64, sourceName: String, destParent: UInt64, destName: String)
    case openItem(item: UInt64, flags: WireOpenFlags)
    case closeItem(item: UInt64, handle: UInt64)
    case read(item: UInt64, handle: UInt64?, offset: UInt64, length: UInt32)
    case write(item: UInt64, handle: UInt64?, offset: UInt64, data: [UInt8])
    case synchronize(item: UInt64, handle: UInt64?, datasync: Bool)
    case volumeStatistics(item: UInt64?)

    private enum CodingKeys: String, CodingKey {
        case op
        case resource_url
        case parent
        case name
        case item
        case attr
        case directory
        case cookie
        case max_entries
        case kind
        case mode
        case symlink_target
        case is_directory
        case source_parent
        case source_name
        case dest_parent
        case dest_name
        case flags
        case handle
        case offset
        case length
        case data
        case datasync
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .ping:
            try c.encode("ping", forKey: .op)
        case let .probeResource(resourceURL):
            try c.encode("probe_resource", forKey: .op)
            try c.encode(resourceURL, forKey: .resource_url)
        case let .lookupItem(parent, name):
            try c.encode("lookup_item", forKey: .op)
            try c.encode(parent, forKey: .parent)
            try c.encode(name, forKey: .name)
        case let .getAttributes(item):
            try c.encode("get_attributes", forKey: .op)
            try c.encode(item, forKey: .item)
        case let .setAttributes(item, attr):
            try c.encode("set_attributes", forKey: .op)
            try c.encode(item, forKey: .item)
            try c.encode(attr, forKey: .attr)
        case let .enumerateDirectory(directory, cookie, maxEntries):
            try c.encode("enumerate_directory", forKey: .op)
            try c.encode(directory, forKey: .directory)
            try c.encode(cookie, forKey: .cookie)
            try c.encodeIfPresent(maxEntries, forKey: .max_entries)
        case let .readSymbolicLink(item):
            try c.encode("read_symbolic_link", forKey: .op)
            try c.encode(item, forKey: .item)
        case let .createItem(parent, name, kind, mode, symlinkTarget):
            try c.encode("create_item", forKey: .op)
            try c.encode(parent, forKey: .parent)
            try c.encode(name, forKey: .name)
            try c.encode(kind, forKey: .kind)
            try c.encode(mode, forKey: .mode)
            try c.encodeIfPresent(symlinkTarget, forKey: .symlink_target)
        case let .removeItem(parent, name, isDirectory):
            try c.encode("remove_item", forKey: .op)
            try c.encode(parent, forKey: .parent)
            try c.encode(name, forKey: .name)
            try c.encode(isDirectory, forKey: .is_directory)
        case let .renameItem(sourceParent, sourceName, destParent, destName):
            try c.encode("rename_item", forKey: .op)
            try c.encode(sourceParent, forKey: .source_parent)
            try c.encode(sourceName, forKey: .source_name)
            try c.encode(destParent, forKey: .dest_parent)
            try c.encode(destName, forKey: .dest_name)
        case let .openItem(item, flags):
            try c.encode("open_item", forKey: .op)
            try c.encode(item, forKey: .item)
            try c.encode(flags, forKey: .flags)
        case let .closeItem(item, handle):
            try c.encode("close_item", forKey: .op)
            try c.encode(item, forKey: .item)
            try c.encode(handle, forKey: .handle)
        case let .read(item, handle, offset, length):
            try c.encode("read", forKey: .op)
            try c.encode(item, forKey: .item)
            try c.encodeIfPresent(handle, forKey: .handle)
            try c.encode(offset, forKey: .offset)
            try c.encode(length, forKey: .length)
        case let .write(item, handle, offset, data):
            try c.encode("write", forKey: .op)
            try c.encode(item, forKey: .item)
            try c.encodeIfPresent(handle, forKey: .handle)
            try c.encode(offset, forKey: .offset)
            try c.encode(data, forKey: .data)
        case let .synchronize(item, handle, datasync):
            try c.encode("synchronize", forKey: .op)
            try c.encode(item, forKey: .item)
            try c.encodeIfPresent(handle, forKey: .handle)
            try c.encode(datasync, forKey: .datasync)
        case let .volumeStatistics(item):
            try c.encode("volume_statistics", forKey: .op)
            try c.encodeIfPresent(item, forKey: .item)
        }
    }
}

struct WireResponse: Decodable {
    let status: String
    let result: WireProbeResult?
    let attr: WireFileAttr?
    let entries: [WireDirEntry]?
    let next_cookie: UInt64?
    let eof: Bool?
    let target: String?
    let handle: UInt64?
    let data: [UInt8]?
    let bytes_written: UInt32?
    let stats: WireStatFs?
    let errno: Int32?
    let message: String?
}

/// Framed Unix-domain socket client speaking the `fusefs_fskit` protocol
/// (4-byte big-endian length prefix + JSON payload).
final class BridgeClient: @unchecked Sendable {
    private static let maxFrameBytes = 16 * 1024 * 1024

    private let lock = NSLock()
    private let socketPath: String
    private var fd: Int32 = -1
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    init(socketPath: String) {
        self.socketPath = socketPath
    }

    deinit {
        disconnect()
    }

    static func resolveSocketPath(for resourceURL: URL) -> String {
        if let components = URLComponents(url: resourceURL, resolvingAgainstBaseURL: false),
           let queryPath = components.queryItems?.first(where: { $0.name == "socket" })?.value,
           !queryPath.isEmpty
        {
            return queryPath
        }

        let appGroup = Bundle.main.object(forInfoDictionaryKey: "HumataAppGroup") as? String
        if let appGroup, !appGroup.isEmpty {
            if let container = FileManager.default.containerURL(
                forSecurityApplicationGroupIdentifier: appGroup
            ) {
                return container.appendingPathComponent("fskit.sock").path
            }
            let fallback = FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent("Library/Group Containers")
                .appendingPathComponent(appGroup)
                .appendingPathComponent("fskit.sock")
            return fallback.path
        }

        return FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".humata2/fskit.sock")
            .path
    }

    func disconnect() {
        lock.lock()
        defer { lock.unlock() }
        closeLocked()
    }

    func call(_ request: WireRequest) throws -> WireResponse {
        lock.lock()
        defer { lock.unlock() }

        let payload = try encoder.encode(WireEnvelope(request: request))
        do {
            return try sendAndReceiveLocked(payload: payload)
        } catch {
            // Reconnect once in case humata2_fusefs restarted.
            closeLocked()
            return try sendAndReceiveLocked(payload: payload)
        }
    }

    private func sendAndReceiveLocked(payload: Data) throws -> WireResponse {
        if fd < 0 {
            try connectLocked()
        }
        guard payload.count <= Self.maxFrameBytes else {
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(EMSGSIZE))
        }

        var lenBE = UInt32(payload.count).bigEndian
        let header = Data(bytes: &lenBE, count: 4)
        try writeAllLocked(header)
        try writeAllLocked(payload)

        let respHeader = try readExactLocked(count: 4)
        let respLen = respHeader.withUnsafeBytes { ptr -> Int in
            Int(UInt32(bigEndian: ptr.load(as: UInt32.self)))
        }
        guard respLen <= Self.maxFrameBytes else {
            closeLocked()
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(EIO))
        }
        let respData = try readExactLocked(count: respLen)
        let response = try decoder.decode(WireResponse.self, from: respData)
        if response.status == "error" {
            let code = Int(response.errno ?? EIO)
            throw NSError(
                domain: NSPOSIXErrorDomain,
                code: code,
                userInfo: response.message.map { [NSLocalizedDescriptionKey: $0] }
            )
        }
        return response
    }

    private func connectLocked() throws {
        let sock = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard sock >= 0 else {
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(Darwin.errno))
        }

        var noSigPipe: Int32 = 1
        _ = Darwin.setsockopt(
            sock,
            SOL_SOCKET,
            SO_NOSIGPIPE,
            &noSigPipe,
            socklen_t(MemoryLayout<Int32>.size)
        )

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let maxLen = MemoryLayout.size(ofValue: addr.sun_path)
        let pathBytes = Array(socketPath.utf8)
        guard pathBytes.count < maxLen else {
            Darwin.close(sock)
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(ENAMETOOLONG))
        }

        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            for (idx, byte) in pathBytes.enumerated() {
                raw[idx] = byte
            }
            raw[pathBytes.count] = 0
        }

        let rc = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { saPtr in
                Darwin.connect(sock, saPtr, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard rc == 0 else {
            let err = Darwin.errno
            Darwin.close(sock)
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(err))
        }
        self.fd = sock
    }

    private func closeLocked() {
        if fd >= 0 {
            Darwin.close(fd)
            fd = -1
        }
    }

    private func writeAllLocked(_ data: Data) throws {
        try data.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            var written = 0
            while written < data.count {
                let n = Darwin.write(fd, base.advanced(by: written), data.count - written)
                if n < 0 {
                    if Darwin.errno == EINTR { continue }
                    let err = Darwin.errno
                    closeLocked()
                    throw NSError(domain: NSPOSIXErrorDomain, code: Int(err))
                }
                if n == 0 {
                    closeLocked()
                    throw NSError(domain: NSPOSIXErrorDomain, code: Int(EPIPE))
                }
                written += n
            }
        }
    }

    private func readExactLocked(count: Int) throws -> Data {
        var buffer = Data(count: count)
        try buffer.withUnsafeMutableBytes { raw in
            guard let base = raw.baseAddress else { return }
            var total = 0
            while total < count {
                let n = Darwin.read(fd, base.advanced(by: total), count - total)
                if n < 0 {
                    if Darwin.errno == EINTR { continue }
                    let err = Darwin.errno
                    closeLocked()
                    throw NSError(domain: NSPOSIXErrorDomain, code: Int(err))
                }
                if n == 0 {
                    closeLocked()
                    throw NSError(domain: NSPOSIXErrorDomain, code: Int(ECONNRESET))
                }
                total += n
            }
        }
        return buffer
    }
}
