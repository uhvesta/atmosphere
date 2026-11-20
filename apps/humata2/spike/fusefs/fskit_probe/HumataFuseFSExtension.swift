// Gate T1 throwaway spike: macOS 26.7+ FSKit extension skeleton.
//
// Demonstrates:
// 1. `FSUnaryFileSystem` + `FSGenericURLResource` (`humata-fuse://main`)
//    so `Humata 2.app` stays clean and no block device is needed.
// 2. App Group Unix domain socket bridge (`fusefs.sock`) to the standalone
//    Rust `humata2_fusefs` process.
// 3. `audit_token_t` caller-PID/UID/GID extraction helper (recording the T1
//    finding that XPC caller tokens in FSKit reflect `fskitd`, so capability
//    roots `$HUMATA_FUSE_ROOT` remain required for same-UID isolation).

import Darwin
import Foundation
import os.log

#if canImport(FSKit)
import FSKit
#endif

/// Extracts `(pid, euid, egid)` from a Darwin `audit_token_t`.
/// Used both on XPC connections (`NSXPCConnection.auditToken` via ObjC/SPI
/// or `xpc_connection_get_audit_token`) and on `LOCAL_PEERTOKEN` Unix sockets.
public struct CallerIdentity: Codable, Sendable, Equatable {
    public let pid: Int32
    public let euid: UInt32
    public let egid: UInt32

    public static func from(auditToken: audit_token_t) -> CallerIdentity {
        // Darwin `audit_token_t.val` layout (bsm/audit.h):
        // [0] auid, [1] euid, [2] egid, [3] ruid, [4] rgid, [5] pid, [6] asid, [7] pidversion
        let euid = auditToken.val.1
        let egid = auditToken.val.2
        let pid = Int32(bitPattern: auditToken.val.5)
        return CallerIdentity(pid: pid, euid: euid, egid: egid)
    }
}

/// Minimal length-prefixed JSON client over the App Group Unix domain socket.
/// Matches the wire framing implemented in `fskit_probe/src/lib.rs`.
public final class AppGroupBridgeClient: @unchecked Sendable {
    public static let defaultGroupID = "group.com.humata2.fusefs"
    public static let socketFileName = "fusefs.sock"

    private let socketPath: String
    private let logger = Logger(subsystem: "com.humata2.fusefs.spike", category: "bridge")

    public init(socketPath: String) {
        self.socketPath = socketPath
    }

    public static func resolveDefaultSocketPath(groupID: String = defaultGroupID) -> String? {
        guard let containerURL = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: groupID
        ) else {
            return nil
        }
        return containerURL.appendingPathComponent(socketFileName).path
    }

    /// Sends a single length-prefixed JSON request and reads one response frame.
    public func roundTrip(_ requestJSON: Data) throws -> Data {
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else {
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
        defer { Darwin.close(fd) }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = socketPath.utf8CString
        let maxLen = MemoryLayout.size(ofValue: addr.sun_path)
        guard pathBytes.count <= maxLen else {
            throw POSIXError(.ENAMETOOLONG)
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { rawBuf in
            pathBytes.withUnsafeBytes { src in
                rawBuf.copyMemory(from: src)
            }
        }

        let addrLen = socklen_t(MemoryLayout<sockaddr_un>.size)
        let connectRes = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                Darwin.connect(fd, sa, addrLen)
            }
        }
        guard connectRes == 0 else {
            logger.error("App Group bridge connect failed: \(errno)")
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .ECONNREFUSED)
        }

        var lenBE = UInt32(requestJSON.count).bigEndian
        let headerWritten = withUnsafeBytes(of: &lenBE) { Darwin.write(fd, $0.baseAddress, 4) }
        guard headerWritten == 4 else {
            throw POSIXError(.EIO)
        }
        let bodyWritten = requestJSON.withUnsafeBytes { Darwin.write(fd, $0.baseAddress, requestJSON.count) }
        guard bodyWritten == requestJSON.count else {
            throw POSIXError(.EIO)
        }

        var respLenBE: UInt32 = 0
        let headerRead = withUnsafeMutableBytes(of: &respLenBE) { Darwin.read(fd, $0.baseAddress, 4) }
        guard headerRead == 4 else {
            throw POSIXError(.EIO)
        }
        let respLen = Int(UInt32(bigEndian: respLenBE))
        var respData = Data(count: respLen)
        let bodyRead = respData.withUnsafeMutableBytes { Darwin.read(fd, $0.baseAddress, respLen) }
        guard bodyRead == respLen else {
            throw POSIXError(.EIO)
        }
        return respData
    }
}

#if canImport(FSKit)
/// Minimal `FSUnaryFileSystem` accepting `FSGenericURLResource` (`humata-fuse://...`).
@available(macOS 15.4, *)
@objc(HumataFuseFSExtension)
public final class HumataFuseFSExtension: FSUnaryFileSystem, FSUnaryFileSystemOperations {
    private let logger = Logger(subsystem: "com.humata2.fusefs.spike", category: "fskit")

    public func probeResource(
        resource: FSResource,
        replyHandler reply: @escaping (FSProbeResult?, Error?) -> Void
    ) {
        guard let urlResource = resource as? FSGenericURLResource,
              urlResource.url.scheme == "humata-fuse" else {
            reply(.notRecognized, nil)
            return
        }
        let name = urlResource.url.host ?? "main"
        let containerID = FSContainerIdentifier(uuid: UUID())
        logger.info("Probed humata-fuse resource: \(name, privacy: .public)")
        reply(.usable(name: "humata-\(name)", containerID: containerID), nil)
    }

    public func loadResource(
        resource: FSResource,
        options: FSTaskOptions,
        replyHandler reply: @escaping (FSVolume?, Error?) -> Void
    ) {
        guard let urlResource = resource as? FSGenericURLResource else {
            reply(nil, POSIXError(.EINVAL))
            return
        }
        let socketPath = AppGroupBridgeClient.resolveDefaultSocketPath()
            ?? "/tmp/humata-fusefs-spike.sock"
        let bridge = AppGroupBridgeClient(socketPath: socketPath)
        let volume = HumataSpikeVolume(
            volumeID: FSVolume.Identifier(uuid: UUID()),
            volumeName: FSFileName(string: urlResource.url.host ?? "main"),
            bridge: bridge
        )
        reply(volume, nil)
    }

    public func unloadResource(
        resource: FSResource,
        options: FSTaskOptions,
        replyHandler reply: @escaping (Error?) -> Void
    ) {
        logger.info("Unloading humata-fuse resource")
        reply(nil)
    }
}

/// Minimal FSVolume skeleton forwarding VFS operations over `AppGroupBridgeClient`.
@available(macOS 15.4, *)
public final class HumataSpikeVolume: FSVolume {
    private let bridge: AppGroupBridgeClient

    public init(
        volumeID: FSVolume.Identifier,
        volumeName: FSFileName,
        bridge: AppGroupBridgeClient
    ) {
        self.bridge = bridge
        super.init(volumeID: volumeID, volumeName: volumeName)
    }

    /// Example ping helper used during T1 bring-up to verify App Group IPC.
    public func pingBridge() throws -> Data {
        let payload = Data("{\"op\":\"ping\"}".utf8)
        return try bridge.roundTrip(payload)
    }
}
#endif
