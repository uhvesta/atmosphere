import Foundation
import LifrAssistantClient

extension GuidanceClient {
    /// Kept in `Assistant/Guidance.json` in Application Support, readable
    /// only by Lifr, encrypted while the device is locked after a restart,
    /// and left out of backups: it stays on this device.
    package static func onDisk() -> Self {
        let url = { () throws(GuidanceError) -> URL in
            do {
                let support = try FileManager.default.url(
                    for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
                return support.appending(path: "Assistant", directoryHint: .isDirectory)
                    .appending(path: "Guidance.json")
            } catch {
                throw .storageFailed(error.localizedDescription)
            }
        }
        return GuidanceClient(
            load: { try GuidanceFile(url: url()).load() },
            save: { try GuidanceFile(url: url()).save($0) })
    }
}

/// One file holding the person's instructions and their history.
struct GuidanceFile: Sendable {
    let url: URL

    /// Lifr's default when nothing has been saved yet.
    func load() throws(GuidanceError) -> Guidance {
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch CocoaError.fileReadNoSuchFile {
            return Guidance()
        } catch {
            throw .storageFailed(error.localizedDescription)
        }
        do {
            return try JSONDecoder().decode(Guidance.self, from: data)
        } catch {
            throw .storageFailed(error.localizedDescription)
        }
    }

    func save(_ guidance: Guidance) throws(GuidanceError) {
        let folder = url.deletingLastPathComponent()
        do {
            try FileManager.default.createDirectory(
                at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            var excluded = folder
            try excluded.setResourceValues(values)
            try JSONEncoder().encode(guidance).write(
                to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        } catch {
            throw .storageFailed(error.localizedDescription)
        }
    }
}
