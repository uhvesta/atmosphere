import AppKit
import ComposableArchitecture
import Darwin
import Foundation
import Security

/// The files and signatures the installer works with: copying this app into
/// Applications, checking who signed a copy, and opening it.
@DependencyClient
package struct InstallerClient: Sendable {
    /// What the installer offers this copy, read as the app opens; nil
    /// when it offers nothing: a build not signed with a Developer ID
    /// certificate, as debug and test builds aren't, or one already in
    /// Applications.
    package var survey: @Sendable () -> InstallSurvey? = { nil }
    /// Copies `source` beside where it goes, on the same volume as
    /// `destination`, and gives where the copy is. Nothing in
    /// `destination`'s folder changes.
    package var stage: @Sendable (_ source: URL, _ destination: URL) async throws -> URL
    /// The team that signed the app at `bundle`, if its signature holds;
    /// nil for one unsigned, signed ad hoc, or changed since it was signed.
    package var team: @Sendable (_ bundle: URL) async -> String? = { _ in nil }
    /// Takes the downloaded mark off every file in `bundle`, so opening it
    /// doesn't ask again whether to trust it.
    package var clearQuarantine: @Sendable (_ bundle: URL) async -> Void
    /// Puts `staged` at `destination` in one step, in place of what's
    /// there, if anything.
    package var swap: @Sendable (_ staged: URL, _ destination: URL) async throws -> Void
    /// Removes a staged copy that won't be swapped in.
    package var discard: @Sendable (_ staged: URL) async -> Void
    /// Opens the app at `bundle` as a new instance, beside this one.
    package var open: @Sendable (_ bundle: URL) async throws -> Void
    /// Quits this copy.
    package var quit: @Sendable () async -> Void
}

extension InstallerClient: TestDependencyKey {
    package static let testValue = InstallerClient()
    package static let previewValue = InstallerClient(
        survey: { nil }, stage: { _, destination in destination }, team: { _ in nil }, clearQuarantine: { _ in },
        swap: { _, _ in }, discard: { _ in }, open: { _ in }, quit: {})
}

extension InstallerClient: DependencyKey {
    package static var liveValue: InstallerClient {
        InstallerClient(
            survey: { LiveInstaller.survey() },
            stage: { try LiveInstaller.stage($0, beside: $1) },
            team: { LiveInstaller.team(of: $0) },
            clearQuarantine: { LiveInstaller.clearQuarantine($0) },
            swap: { try LiveInstaller.swap($0, into: $1) },
            discard: { LiveInstaller.discard($0) },
            open: { try await LiveInstaller.open($0) },
            quit: { await MainActor.run { NSApplication.shared.terminate(nil) } })
    }
}

extension DependencyValues {
    package var installer: InstallerClient {
        get { self[InstallerClient.self] }
        set { self[InstallerClient.self] = newValue }
    }
}

/// Why the app couldn't install itself in Applications.
package enum InstallerError: LocalizedError, CustomStringConvertible {
    /// The person can't write to the folder the app goes in.
    case noPermission(folder: String, underlying: (any Error)?)
    /// The volume has no room for another copy of the app.
    case diskFull(folder: String, underlying: any Error)
    /// Copying the app failed for another reason.
    case copyFailed(from: String, to: String, underlying: any Error)
    /// The copy couldn't be put in place of the one there.
    case replaceFailed(path: String, underlying: any Error)
    /// The installed copy couldn't be opened.
    case openFailed(path: String, underlying: any Error)

    package var errorDescription: String? { description }

    package var description: String {
        switch self {
        case .noPermission(let folder, _):
            "Humata can't write to \(folder), so it wasn't installed and the copy there is as it was. Sign in "
                + "as an administrator, or drag Humata into \(folder) in the Finder and enter an administrator's "
                + "password when asked; or choose Run from Here."
        case .diskFull(let folder, let underlying):
            "There isn't room on the disk holding \(folder) for another copy of Humata "
                + "(\(underlying.localizedDescription)), so it wasn't installed and the copy there is as it was. "
                + "Free some space, then try again."
        case .copyFailed(let from, let to, let underlying):
            "Couldn't copy Humata from \(from) to \(to): \(underlying.localizedDescription) The copy in "
                + "Applications is as it was. Try again; if it fails again, download Humata again, since this "
                + "copy may be damaged."
        case .replaceFailed(let path, let underlying):
            "Couldn't put the new Humata at \(path): \(underlying.localizedDescription) What was there is as it "
                + "was. Quit any other copy of Humata that's open, then try again."
        case .openFailed(let path, let underlying):
            "Humata is at \(path) but didn't open: \(underlying.localizedDescription) Open it from Applications "
                + "yourself, and quit this copy."
        }
    }

    /// `error` from writing in `folder`, as a lack of permission or of
    /// space when it's one of those, else as `otherwise`.
    static func writing(
        in folder: String, _ error: any Error, otherwise: (any Error) -> InstallerError
    ) -> InstallerError {
        let codes = posixCodes(of: error)
        if codes.contains(.EACCES) || codes.contains(.EPERM) || codes.contains(.EROFS) {
            return .noPermission(folder: folder, underlying: error)
        }
        if codes.contains(.ENOSPC) || codes.contains(.EDQUOT) { return .diskFull(folder: folder, underlying: error) }
        if let cocoa = error as? CocoaError {
            switch cocoa.code {
            case .fileWriteNoPermission, .fileReadNoPermission, .fileWriteVolumeReadOnly:
                return .noPermission(folder: folder, underlying: error)
            case .fileWriteOutOfSpace:
                return .diskFull(folder: folder, underlying: error)
            default: break
            }
        }
        return otherwise(error)
    }

    /// The POSIX codes of `error` and the errors under it.
    private static func posixCodes(of error: any Error) -> Set<POSIXErrorCode> {
        var codes: Set<POSIXErrorCode> = []
        var next: NSError? = error as NSError
        while let current = next {
            if current.domain == NSPOSIXErrorDomain, let raw = Int32(exactly: current.code),
                let code = POSIXErrorCode(rawValue: raw)
            {
                codes.insert(code)
            }
            next = current.userInfo[NSUnderlyingErrorKey] as? NSError
        }
        return codes
    }
}

/// The installer's work on the real files and signatures.
private enum LiveInstaller {
    /// What a Developer ID certificate's chain has: Apple's Developer ID
    /// authority over a Developer ID Application leaf.
    private static let developerID =
        "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] "
        + "and certificate leaf[field.1.2.840.113635.100.6.1.13]"

    static func survey() -> InstallSurvey? {
        let bundle = Bundle.main
        guard isRunningDeveloperID(), !InstallFiles.isInstalled(bundle.bundleURL),
            let info = bundle.infoDictionary, let version = BundleVersion(info: info)
        else { return nil }
        let name = info["CFBundleName"] as? String ?? "Humata"
        let destination = InstallFiles.destination(named: name)
        let installed = installedInfo(at: destination)
        // Something else by the name is never replaced.
        if let installed, installed["CFBundleIdentifier"] as? String != bundle.bundleIdentifier { return nil }
        return InstallSurvey(
            source: bundle.bundleURL, destination: destination, version: version,
            installed: installed.flatMap(BundleVersion.init(info:)), team: runningTeam())
    }

    /// The Info.plist of the app at `bundle`, read afresh rather than
    /// from `Bundle`'s cache.
    private static func installedInfo(at bundle: URL) -> [String: Any]? {
        let plist = bundle.appending(path: "Contents/Info.plist")
        guard let data = try? Data(contentsOf: plist) else { return nil }
        return (try? PropertyListSerialization.propertyList(from: data, format: nil)) as? [String: Any]
    }

    /// Whether this process is signed with a Developer ID certificate.
    /// Checked on the running code, which is quick: its pages were checked
    /// as they loaded.
    private static func isRunningDeveloperID() -> Bool {
        var code: SecCode?
        var requirement: SecRequirement?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code,
            SecRequirementCreateWithString(developerID as CFString, [], &requirement) == errSecSuccess,
            let requirement
        else { return false }
        return SecCodeCheckValidity(code, [], requirement) == errSecSuccess
    }

    private static func runningTeam() -> String? {
        var code: SecCode?
        var staticCode: SecStaticCode?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code,
            SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode
        else { return nil }
        return teamIdentifier(of: staticCode)
    }

    private static func teamIdentifier(of code: SecStaticCode) -> String? {
        var information: CFDictionary?
        guard
            SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &information)
                == errSecSuccess,
            let information = information as? [String: Any]
        else { return nil }
        return information[kSecCodeInfoTeamIdentifier as String] as? String
    }

    /// The team of the app at `bundle`, once its whole signature, every
    /// file in it included, is found to hold.
    static func team(of bundle: URL) -> String? {
        var code: SecStaticCode?
        guard SecStaticCodeCreateWithPath(bundle as CFURL, [], &code) == errSecSuccess, let code,
            SecStaticCodeCheckValidity(code, SecCSFlags(rawValue: kSecCSCheckAllArchitectures), nil)
                == errSecSuccess
        else { return nil }
        return teamIdentifier(of: code)
    }

    static func stage(_ source: URL, beside destination: URL) throws -> URL {
        let files = FileManager.default
        let folder = destination.deletingLastPathComponent()
        let folderPath = folder.path(percentEncoded: false)
        // Found out before anything is copied, and before the daemon stops.
        guard files.isWritableFile(atPath: folderPath) else {
            throw InstallerError.noPermission(folder: folderPath, underlying: nil)
        }
        let temporary: URL
        do {
            temporary = try files.url(
                for: .itemReplacementDirectory, in: .userDomainMask, appropriateFor: folder, create: true)
        } catch {
            throw InstallerError.writing(in: folderPath, error) {
                .copyFailed(from: source.path(percentEncoded: false), to: folderPath, underlying: $0)
            }
        }
        let staged = temporary.appending(path: destination.lastPathComponent)
        do {
            try files.copyItem(at: source, to: staged)
        } catch {
            discard(staged)
            throw InstallerError.writing(in: folderPath, error) {
                .copyFailed(
                    from: source.path(percentEncoded: false), to: staged.path(percentEncoded: false), underlying: $0)
            }
        }
        return staged
    }

    static func clearQuarantine(_ bundle: URL) {
        let name = "com.apple.quarantine"
        removexattr(bundle.path(percentEncoded: false), name, XATTR_NOFOLLOW)
        guard let files = FileManager.default.enumerator(at: bundle, includingPropertiesForKeys: nil) else { return }
        for case let file as URL in files {
            removexattr(file.path(percentEncoded: false), name, XATTR_NOFOLLOW)
        }
    }

    static func swap(_ staged: URL, into destination: URL) throws {
        let files = FileManager.default
        let path = destination.path(percentEncoded: false)
        do {
            if files.fileExists(atPath: path) {
                _ = try files.replaceItemAt(destination, withItemAt: staged, backupItemName: nil, options: [])
            } else {
                try files.moveItem(at: staged, to: destination)
            }
        } catch {
            discard(staged)
            let folder = destination.deletingLastPathComponent().path(percentEncoded: false)
            throw InstallerError.writing(in: folder, error) { .replaceFailed(path: path, underlying: $0) }
        }
        // The folder it was staged in, now empty.
        rmdir(staged.deletingLastPathComponent().path(percentEncoded: false))
    }

    /// Removes the staged copy, and the folder made for it once that's
    /// empty: `rmdir` takes only an empty folder.
    static func discard(_ staged: URL) {
        try? FileManager.default.removeItem(at: staged)
        rmdir(staged.deletingLastPathComponent().path(percentEncoded: false))
    }

    static func open(_ bundle: URL) async throws {
        let configuration = NSWorkspace.OpenConfiguration()
        // This copy has the same bundle ID and is still running: without
        // this, macOS brings it forward instead.
        configuration.createsNewApplicationInstance = true
        configuration.activates = true
        do {
            _ = try await NSWorkspace.shared.openApplication(at: bundle, configuration: configuration)
        } catch {
            throw InstallerError.openFailed(path: bundle.path(percentEncoded: false), underlying: error)
        }
    }
}
