import Darwin
import ExtensionFoundation
import FSKit
import Foundation

@main
struct HumataFSExtension: UnaryFileSystemExtension {
    let fileSystem = HumataFileSystem()
}

final class HumataFileSystem: FSUnaryFileSystem, FSUnaryFileSystemOperations {
    private var activeBridge: BridgeClient?

    func probeResource(
        resource: FSResource,
        replyHandler reply: @escaping (FSProbeResult?, (any Error)?) -> Void
    ) {
        guard let urlResource = resource as? FSGenericURLResource else {
            reply(.notRecognized, nil)
            return
        }
        let url = urlResource.url
        let bridge = BridgeClient(socketPath: BridgeClient.resolveSocketPath(for: url))
        do {
            let response = try bridge.call(.probeResource(resourceURL: url.absoluteString))
            bridge.disconnect()
            guard let probe = response.result, probe.recognition == "usable" else {
                reply(.notRecognized, nil)
                return
            }
            let name = probe.name ?? "humata2"
            let containerID = FSContainerIdentifier(uuid: UUID())
            reply(.usable(name: name, containerID: containerID), nil)
        } catch {
            bridge.disconnect()
            reply(.notRecognized, nil)
        }
    }

    func loadResource(
        resource: FSResource,
        options: FSTaskOptions,
        replyHandler reply: @escaping (FSVolume?, (any Error)?) -> Void
    ) {
        guard let urlResource = resource as? FSGenericURLResource else {
            reply(nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EINVAL)))
            return
        }
        let socketPath = BridgeClient.resolveSocketPath(for: urlResource.url)
        let bridge = BridgeClient(socketPath: socketPath)
        do {
            _ = try bridge.call(.ping)
            self.activeBridge = bridge
            let volumeID = FSVolume.Identifier(uuid: UUID())
            let volume = HumataVolume(
                volumeID: volumeID,
                volumeName: FSFileName(string: "humata2"),
                bridge: bridge
            )
            self.containerStatus = .ready
            reply(volume, nil)
        } catch {
            bridge.disconnect()
            reply(nil, error)
        }
    }

    func unloadResource(
        resource: FSResource,
        options: FSTaskOptions,
        replyHandler reply: @escaping ((any Error)?) -> Void
    ) {
        activeBridge?.disconnect()
        activeBridge = nil
        containerStatus = .notReady(
            status: NSError(domain: NSPOSIXErrorDomain, code: Int(ENOTCONN))
        )
        reply(nil)
    }
}

final class HumataItem: FSItem {
    let nodeID: UInt64
    var isDirectory: Bool

    init(nodeID: UInt64, isDirectory: Bool) {
        self.nodeID = nodeID
        self.isDirectory = isDirectory
        super.init()
    }
}

final class HumataVolume: FSVolume,
    FSVolume.Operations,
    FSVolume.PathConfOperations,
    FSVolume.OpenCloseOperations,
    FSVolume.ReadWriteOperations
{
    private static let vfsRootNodeID: UInt64 = 1

    private let bridge: BridgeClient
    private let itemLock = NSLock()
    private var items: [UInt64: HumataItem] = [:]
    private var openHandles: [UInt64: UInt64] = [:]

    init(volumeID: FSVolume.Identifier, volumeName: FSFileName, bridge: BridgeClient) {
        self.bridge = bridge
        super.init(volumeID: volumeID, volumeName: volumeName)
    }

    // MARK: - Helpers

    private static func fsItemID(for nodeID: UInt64) -> FSItem.Identifier {
        if nodeID == vfsRootNodeID {
            return .rootDirectory
        }
        return FSItem.Identifier(rawValue: nodeID + 1) ?? .invalid
    }

    private static func fsItemType(for kind: String) -> FSItem.ItemType {
        switch kind {
        case "Regular": return .file
        case "Directory": return .directory
        case "Symlink": return .symlink
        default: return .unknown
        }
    }

    private func item(for attr: WireFileAttr) -> HumataItem {
        itemLock.lock()
        defer { itemLock.unlock() }
        if let existing = items[attr.ino] {
            existing.isDirectory = (attr.kind == "Directory")
            return existing
        }
        let created = HumataItem(nodeID: attr.ino, isDirectory: attr.kind == "Directory")
        items[attr.ino] = created
        return created
    }

    private func nodeID(of item: FSItem) throws -> UInt64 {
        guard let hItem = item as? HumataItem else {
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(EINVAL))
        }
        return hItem.nodeID
    }

    private static func makeFSAttributes(from wire: WireFileAttr) -> FSItem.Attributes {
        let attrs = FSItem.Attributes()
        attrs.type = fsItemType(for: wire.kind)
        attrs.mode = wire.mode
        attrs.linkCount = wire.nlink
        attrs.uid = wire.uid
        attrs.gid = wire.gid
        attrs.size = wire.size
        attrs.allocSize = wire.blocks * 512
        attrs.fileID = fsItemID(for: wire.ino)
        attrs.accessTime = wire.atime.timespecValue
        attrs.modifyTime = wire.mtime.timespecValue
        attrs.changeTime = wire.ctime.timespecValue
        attrs.birthTime = wire.ctime.timespecValue
        return attrs
    }

    // MARK: - FSVolume.PathConfOperations

    var maximumLinkCount: Int { 32767 }
    var maximumNameLength: Int { 255 }
    var restrictsOwnershipChanges: Bool { false }
    var truncatesLongNames: Bool { false }
    var maximumFileSizeInBits: Int { 64 }

    // MARK: - FSVolume.CommonOperations

    var supportedVolumeCapabilities: FSVolume.SupportedCapabilities {
        let caps = FSVolume.SupportedCapabilities()
        caps.supportsPersistentObjectIDs = true
        caps.supportsSymbolicLinks = true
        caps.supportsHardLinks = false
        caps.supportsFastStatFS = true
        caps.supports2TBFiles = true
        caps.supports64BitObjectIDs = true
        caps.caseFormat = .sensitive
        return caps
    }

    var volumeStatistics: FSStatFSResult {
        let stat = FSStatFSResult(fileSystemTypeName: "humatafs")
        if let response = try? bridge.call(.volumeStatistics(item: Self.vfsRootNodeID)),
           let s = response.stats
        {
            stat.blockSize = Int(max(512, s.bsize))
            stat.ioSize = Int(max(4096, s.bsize))
            stat.totalBlocks = s.blocks
            stat.availableBlocks = s.bavail
            stat.freeBlocks = s.bfree
            stat.usedBlocks = s.blocks >= s.bfree ? (s.blocks - s.bfree) : 0
            stat.totalFiles = s.files
            stat.freeFiles = s.ffree
        } else {
            stat.blockSize = 4096
            stat.ioSize = 65536
            stat.totalBlocks = 1_048_576
            stat.availableBlocks = 1_048_576
            stat.freeBlocks = 1_048_576
            stat.totalFiles = 1_048_576
            stat.freeFiles = 1_048_576
        }
        return stat
    }

    func mount(options: FSTaskOptions, replyHandler reply: @escaping ((any Error)?) -> Void) {
        reply(nil)
    }

    func unmount(replyHandler reply: @escaping () -> Void) {
        itemLock.lock()
        openHandles.removeAll()
        items.removeAll()
        itemLock.unlock()
        reply()
    }

    func synchronize(flags: FSSyncFlags, replyHandler reply: @escaping ((any Error)?) -> Void) {
        reply(nil)
    }

    func reclaimItem(_ item: FSItem, replyHandler reply: @escaping ((any Error)?) -> Void) {
        if let hItem = item as? HumataItem {
            itemLock.lock()
            if hItem.nodeID != Self.vfsRootNodeID {
                items.removeValue(forKey: hItem.nodeID)
            }
            openHandles.removeValue(forKey: hItem.nodeID)
            itemLock.unlock()
        }
        reply(nil)
    }

    // MARK: - FSVolume.Operations

    func activate(
        options: FSTaskOptions,
        replyHandler reply: @escaping (FSItem?, (any Error)?) -> Void
    ) {
        do {
            let response = try bridge.call(.getAttributes(item: Self.vfsRootNodeID))
            guard let attr = response.attr else {
                reply(nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EIO)))
                return
            }
            reply(item(for: attr), nil)
        } catch {
            reply(nil, error)
        }
    }

    func deactivate(
        options: FSDeactivateOptions,
        replyHandler reply: @escaping ((any Error)?) -> Void
    ) {
        itemLock.lock()
        openHandles.removeAll()
        items.removeAll()
        itemLock.unlock()
        reply(nil)
    }

    func lookupItem(
        named name: FSFileName,
        inDirectory directory: FSItem,
        replyHandler reply: @escaping (FSItem?, FSFileName?, (any Error)?) -> Void
    ) {
        guard let nameStr = name.string else {
            reply(nil, nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EINVAL)))
            return
        }
        do {
            let parentID = try nodeID(of: directory)
            let response = try bridge.call(.lookupItem(parent: parentID, name: nameStr))
            guard let attr = response.attr else {
                reply(nil, nil, NSError(domain: NSPOSIXErrorDomain, code: Int(ENOENT)))
                return
            }
            reply(item(for: attr), name, nil)
        } catch {
            reply(nil, nil, error)
        }
    }

    func createItem(
        named name: FSFileName,
        type: FSItem.ItemType,
        inDirectory directory: FSItem,
        attributes newAttributes: FSItem.SetAttributesRequest,
        replyHandler reply: @escaping (FSItem?, FSFileName?, (any Error)?) -> Void
    ) {
        guard let nameStr = name.string else {
            reply(nil, nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EINVAL)))
            return
        }
        let kind: String
        let defaultMode: UInt32
        switch type {
        case .file:
            kind = "regular"
            defaultMode = 0o644
        case .directory:
            kind = "directory"
            defaultMode = 0o755
        default:
            reply(nil, nil, NSError(domain: NSPOSIXErrorDomain, code: Int(ENOTSUP)))
            return
        }
        let mode = newAttributes.isValid(.mode) ? newAttributes.mode : defaultMode
        do {
            let parentID = try nodeID(of: directory)
            let response = try bridge.call(
                .createItem(
                    parent: parentID,
                    name: nameStr,
                    kind: kind,
                    mode: mode,
                    symlinkTarget: nil
                )
            )
            guard let attr = response.attr else {
                reply(nil, nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EIO)))
                return
            }
            reply(item(for: attr), name, nil)
        } catch {
            reply(nil, nil, error)
        }
    }

    func createSymbolicLink(
        named name: FSFileName,
        inDirectory directory: FSItem,
        attributes newAttributes: FSItem.SetAttributesRequest,
        linkContents contents: FSFileName,
        replyHandler reply: @escaping (FSItem?, FSFileName?, (any Error)?) -> Void
    ) {
        guard let nameStr = name.string, let targetStr = contents.string else {
            reply(nil, nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EINVAL)))
            return
        }
        do {
            let parentID = try nodeID(of: directory)
            let response = try bridge.call(
                .createItem(
                    parent: parentID,
                    name: nameStr,
                    kind: "symlink",
                    mode: 0o777,
                    symlinkTarget: targetStr
                )
            )
            guard let attr = response.attr else {
                reply(nil, nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EIO)))
                return
            }
            reply(item(for: attr), name, nil)
        } catch {
            reply(nil, nil, error)
        }
    }

    func createLink(
        to item: FSItem,
        named name: FSFileName,
        inDirectory directory: FSItem,
        replyHandler reply: @escaping (FSFileName?, (any Error)?) -> Void
    ) {
        reply(nil, NSError(domain: NSPOSIXErrorDomain, code: Int(ENOTSUP)))
    }

    func renameItem(
        _ item: FSItem,
        inDirectory sourceDirectory: FSItem,
        named sourceName: FSFileName,
        to destinationName: FSFileName,
        inDirectory destinationDirectory: FSItem,
        overItem: FSItem?,
        replyHandler reply: @escaping (FSFileName?, (any Error)?) -> Void
    ) {
        guard let srcStr = sourceName.string, let dstStr = destinationName.string else {
            reply(nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EINVAL)))
            return
        }
        do {
            let srcParent = try nodeID(of: sourceDirectory)
            let dstParent = try nodeID(of: destinationDirectory)
            _ = try bridge.call(
                .renameItem(
                    sourceParent: srcParent,
                    sourceName: srcStr,
                    destParent: dstParent,
                    destName: dstStr
                )
            )
            reply(destinationName, nil)
        } catch {
            reply(nil, error)
        }
    }

    func removeItem(
        _ item: FSItem,
        named name: FSFileName,
        fromDirectory directory: FSItem,
        replyHandler reply: @escaping ((any Error)?) -> Void
    ) {
        guard let nameStr = name.string, let hItem = item as? HumataItem else {
            reply(NSError(domain: NSPOSIXErrorDomain, code: Int(EINVAL)))
            return
        }
        do {
            let parentID = try nodeID(of: directory)
            _ = try bridge.call(
                .removeItem(parent: parentID, name: nameStr, isDirectory: hItem.isDirectory)
            )
            reply(nil)
        } catch {
            reply(error)
        }
    }

    func getAttributes(
        _ desiredAttributes: FSItem.GetAttributesRequest,
        of item: FSItem,
        replyHandler reply: @escaping (FSItem.Attributes?, (any Error)?) -> Void
    ) {
        do {
            let id = try nodeID(of: item)
            let response = try bridge.call(.getAttributes(item: id))
            guard let attr = response.attr else {
                reply(nil, NSError(domain: NSPOSIXErrorDomain, code: Int(ENOENT)))
                return
            }
            reply(Self.makeFSAttributes(from: attr), nil)
        } catch {
            reply(nil, error)
        }
    }

    func setAttributes(
        _ newAttributes: FSItem.SetAttributesRequest,
        on item: FSItem,
        replyHandler reply: @escaping (FSItem.Attributes?, (any Error)?) -> Void
    ) {
        do {
            let id = try nodeID(of: item)
            var setAttr = WireSetAttr()
            var consumed: FSItem.Attribute = []
            if newAttributes.isValid(.mode) {
                setAttr.mode = newAttributes.mode
                consumed.insert(.mode)
            }
            if newAttributes.isValid(.uid) {
                setAttr.uid = newAttributes.uid
                consumed.insert(.uid)
            }
            if newAttributes.isValid(.gid) {
                setAttr.gid = newAttributes.gid
                consumed.insert(.gid)
            }
            if newAttributes.isValid(.size) {
                setAttr.size = newAttributes.size
                consumed.insert(.size)
            }
            if newAttributes.isValid(.accessTime) {
                setAttr.atime = WireTimestamp(timespec: newAttributes.accessTime)
                consumed.insert(.accessTime)
            }
            if newAttributes.isValid(.modifyTime) {
                setAttr.mtime = WireTimestamp(timespec: newAttributes.modifyTime)
                consumed.insert(.modifyTime)
            }
            newAttributes.consumedAttributes = consumed
            let response = try bridge.call(.setAttributes(item: id, attr: setAttr))
            guard let attr = response.attr else {
                reply(nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EIO)))
                return
            }
            reply(Self.makeFSAttributes(from: attr), nil)
        } catch {
            reply(nil, error)
        }
    }

    func enumerateDirectory(
        _ directory: FSItem,
        startingAt cookie: FSDirectoryCookie,
        verifier: FSDirectoryVerifier,
        attributes: FSItem.GetAttributesRequest?,
        packer: FSDirectoryEntryPacker,
        replyHandler reply: @escaping (FSDirectoryVerifier, (any Error)?) -> Void
    ) {
        do {
            let dirID = try nodeID(of: directory)
            let startCookie = (cookie == FSDirectoryCookie.initial) ? 0 : cookie.rawValue
            let response = try bridge.call(
                .enumerateDirectory(directory: dirID, cookie: startCookie, maxEntries: nil)
            )
            let entries = response.entries ?? []
            for entry in entries {
                if attributes != nil && (entry.name == "." || entry.name == "..") {
                    continue
                }
                let entryAttrs: FSItem.Attributes?
                if attributes != nil,
                   let attrResp = try? bridge.call(.getAttributes(item: entry.ino)),
                   let wireAttr = attrResp.attr
                {
                    entryAttrs = Self.makeFSAttributes(from: wireAttr)
                } else {
                    entryAttrs = nil
                }
                let packed = packer.packEntry(
                    name: FSFileName(string: entry.name),
                    itemType: Self.fsItemType(for: entry.kind),
                    itemID: Self.fsItemID(for: entry.ino),
                    nextCookie: FSDirectoryCookie(entry.offset),
                    attributes: entryAttrs
                )
                if !packed {
                    break
                }
            }
            reply(FSDirectoryVerifier(1), nil)
        } catch {
            reply(FSDirectoryVerifier.initial, error)
        }
    }

    func readSymbolicLink(
        _ item: FSItem,
        replyHandler reply: @escaping (FSFileName?, (any Error)?) -> Void
    ) {
        do {
            let id = try nodeID(of: item)
            let response = try bridge.call(.readSymbolicLink(item: id))
            guard let target = response.target else {
                reply(nil, NSError(domain: NSPOSIXErrorDomain, code: Int(EINVAL)))
                return
            }
            reply(FSFileName(string: target), nil)
        } catch {
            reply(nil, error)
        }
    }

    // MARK: - FSVolume.OpenCloseOperations

    func openItem(
        _ item: FSItem,
        modes: FSVolume.OpenModes,
        replyHandler reply: @escaping ((any Error)?) -> Void
    ) {
        do {
            let id = try nodeID(of: item)
            let flags = WireOpenFlags(
                read: modes.contains(.read),
                write: modes.contains(.write),
                append: false,
                truncate: false,
                create: false,
                exclusive: false
            )
            let response = try bridge.call(.openItem(item: id, flags: flags))
            if let fh = response.handle {
                itemLock.lock()
                openHandles[id] = fh
                itemLock.unlock()
            }
            reply(nil)
        } catch {
            reply(error)
        }
    }

    func closeItem(
        _ item: FSItem,
        modes: FSVolume.OpenModes,
        replyHandler reply: @escaping ((any Error)?) -> Void
    ) {
        guard modes.isEmpty else {
            reply(nil)
            return
        }
        do {
            let id = try nodeID(of: item)
            itemLock.lock()
            let fh = openHandles.removeValue(forKey: id)
            itemLock.unlock()
            if let fh {
                _ = try bridge.call(.closeItem(item: id, handle: fh))
            }
            reply(nil)
        } catch {
            reply(error)
        }
    }

    // MARK: - FSVolume.ReadWriteOperations

    func read(
        from item: FSItem,
        at offset: off_t,
        length: Int,
        into buffer: FSMutableFileDataBuffer,
        replyHandler reply: @escaping (Int, (any Error)?) -> Void
    ) {
        do {
            let id = try nodeID(of: item)
            itemLock.lock()
            let fh = openHandles[id]
            itemLock.unlock()
            let reqLen = UInt32(min(length, buffer.length))
            let response = try bridge.call(
                .read(item: id, handle: fh, offset: UInt64(max(0, offset)), length: reqLen)
            )
            let bytes = response.data ?? []
            let count = min(bytes.count, buffer.length)
            if count > 0 {
                buffer.withUnsafeMutableBytes { dst in
                    bytes.withUnsafeBytes { src in
                        if let dstBase = dst.baseAddress, let srcBase = src.baseAddress {
                            memcpy(dstBase, srcBase, count)
                        }
                    }
                }
            }
            reply(count, nil)
        } catch {
            reply(0, error)
        }
    }

    func write(
        contents: Data,
        to item: FSItem,
        at offset: off_t,
        replyHandler reply: @escaping (Int, (any Error)?) -> Void
    ) {
        do {
            let id = try nodeID(of: item)
            itemLock.lock()
            let fh = openHandles[id]
            itemLock.unlock()
            let response = try bridge.call(
                .write(
                    item: id,
                    handle: fh,
                    offset: UInt64(max(0, offset)),
                    data: Array(contents)
                )
            )
            reply(Int(response.bytes_written ?? 0), nil)
        } catch {
            reply(0, error)
        }
    }
}
