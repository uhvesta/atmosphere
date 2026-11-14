import Foundation
import HumataAppFeature
import XCTest

final class InstallFilesTests: XCTestCase {
    private func version(_ short: String, _ build: String = "1") -> BundleVersion {
        BundleVersion(short: short, build: build)
    }

    func testVersionsCompareByNumberNotByText() {
        XCTAssertLessThan(version("0.9"), version("0.10"))
        XCTAssertLessThan(version("1.2"), version("1.10"))
        XCTAssertLessThan(version("1.9.9"), version("2"))
    }

    func testAMissingPartCountsAsZero() {
        XCTAssertEqual(version("1.2"), version("1.2.0"))
        XCTAssertLessThan(version("1.2"), version("1.2.1"))
    }

    func testTheBuildNumberDecidesBetweenTheSameVersion() {
        XCTAssertLessThan(version("1.2", "9"), version("1.2", "10"))
        XCTAssertLessThan(version("1.1", "99"), version("1.2", "1"))
    }

    func testAVersionIsReadFromAnInfoPlist() {
        XCTAssertEqual(
            BundleVersion(info: ["CFBundleShortVersionString": "0.3.1", "CFBundleVersion": "12"]),
            version("0.3.1", "12"))
        XCTAssertEqual(BundleVersion(info: ["CFBundleShortVersionString": "0.3"]), version("0.3", "0"))
        XCTAssertNil(BundleVersion(info: ["CFBundleVersion": "12"]))
    }

    func testANewerCopyInstallsOverAnOlderOneOrNone() {
        XCTAssertEqual(InstallFiles.offer(running: version("0.2"), installed: nil), .install(replacing: nil))
        XCTAssertEqual(
            InstallFiles.offer(running: version("0.2"), installed: version("0.1")),
            .install(replacing: version("0.1")))
    }

    func testTheInstalledCopyIsOpenedWhenItIsAsNewOrNewer() {
        XCTAssertEqual(
            InstallFiles.offer(running: version("0.2"), installed: version("0.2")), .openInstalled(version("0.2")))
        XCTAssertEqual(
            InstallFiles.offer(running: version("0.2", "3"), installed: version("0.2", "4")),
            .openInstalled(version("0.2", "4")))
    }

    func testOnlyACopyDirectlyInApplicationsCountsAsInstalled() {
        let applications = URL(filePath: "/Applications", directoryHint: .isDirectory)
        XCTAssertTrue(InstallFiles.isInstalled(URL(filePath: "/Applications/Humata.app"), in: applications))
        XCTAssertFalse(InstallFiles.isInstalled(URL(filePath: "/Users/me/Downloads/Humata.app"), in: applications))
        XCTAssertFalse(InstallFiles.isInstalled(URL(filePath: "/Applications/Tools/Humata.app"), in: applications))
    }

    func testTheCopyGoesInByTheAppsNameNotTheFiles() {
        let applications = URL(filePath: "/Applications", directoryHint: .isDirectory)
        XCTAssertEqual(
            InstallFiles.destination(named: "Humata", in: applications).path(percentEncoded: false),
            "/Applications/Humata.app")
        XCTAssertEqual(
            InstallFiles.destination(named: "", in: applications).path(percentEncoded: false),
            "/Applications/Humata.app")
    }

    func testACopyMacOSRunsFromElsewhereCountsAsInstalledWhenItIsReallyInApplications() {
        let applications = URL(filePath: "/Applications", directoryHint: .isDirectory)
        let translocated = URL(filePath: "/private/var/folders/xx/AppTranslocation/ABCD/d/Humata.app")
        XCTAssertTrue(
            InstallFiles.isInstalled(translocated, in: applications) { _ in URL(filePath: "/Applications/Humata.app") })
        XCTAssertFalse(
            InstallFiles.isInstalled(translocated, in: applications) { _ in
                URL(filePath: "/Users/me/Downloads/Humata.app")
            })
    }

    func testOnlyFilesTheUserCanWriteAndThatArentLockedCanBeReplaced() {
        let user: UInt32 = 501
        let staff: UInt32 = 20
        let own = FileOwnership(owner: user, group: staff, mode: 0o755)
        XCTAssertTrue(InstallFiles.canReplace([own, own], user: user, groups: [staff]))
        // Another user's, writable only by them.
        let others = FileOwnership(owner: 0, group: 0, mode: 0o755)
        XCTAssertFalse(InstallFiles.canReplace([own, others], user: user, groups: [staff]))
        // Writable by a group the user is in.
        let admin = FileOwnership(owner: 0, group: 80, mode: 0o775)
        XCTAssertTrue(InstallFiles.canReplace([admin], user: user, groups: [staff, 80]))
        XCTAssertFalse(InstallFiles.canReplace([admin], user: user, groups: [staff]))
        // Locked, or append-only, even when it's the user's own.
        let locked = FileOwnership(owner: user, group: staff, mode: 0o755, flags: UInt32(UF_IMMUTABLE))
        XCTAssertFalse(InstallFiles.canReplace([locked], user: user, groups: [staff]))
        let appendOnly = FileOwnership(owner: user, group: staff, mode: 0o755, flags: UInt32(SF_APPEND))
        XCTAssertFalse(InstallFiles.canReplace([appendOnly], user: user, groups: [staff]))
    }

    func testOnlyHumataWithAReadableVersionIsTakenForTheInstalledCopy() throws {
        let folder = FileManager.default.temporaryDirectory.appending(path: "InstallFilesTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let identifier = "app.humata"

        XCTAssertEqual(
            InstallFiles.installed(at: folder.appending(path: "Missing.app"), identifier: identifier), .nothing)

        let unreadable = folder.appending(path: "Unreadable.app")
        try FileManager.default.createDirectory(
            at: unreadable.appending(path: "Contents"), withIntermediateDirectories: true)
        XCTAssertEqual(InstallFiles.installed(at: unreadable, identifier: identifier), .other)

        let humata = try bundle(in: folder, named: "Humata.app", identifier: identifier)
        XCTAssertEqual(InstallFiles.installed(at: humata, identifier: identifier), .humata(version("0.3", "7")))
        XCTAssertEqual(InstallFiles.installed(at: humata, identifier: nil), .other)

        let another = try bundle(in: folder, named: "Other.app", identifier: "com.example.other")
        XCTAssertEqual(InstallFiles.installed(at: another, identifier: identifier), .other)
    }

    /// A bundle in `folder` whose Info.plist names `identifier`, at 0.3 (7).
    private func bundle(in folder: URL, named name: String, identifier: String) throws -> URL {
        let bundle = folder.appending(path: name)
        let contents = bundle.appending(path: "Contents")
        try FileManager.default.createDirectory(at: contents, withIntermediateDirectories: true)
        let info: [String: Any] = [
            "CFBundleIdentifier": identifier, "CFBundleShortVersionString": "0.3", "CFBundleVersion": "7",
        ]
        let data = try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0)
        try data.write(to: contents.appending(path: "Info.plist"))
        return bundle
    }

    func testAWriteRefusedOrOutOfSpaceSaysSoWhereverTheCodeIs() {
        let other = { (error: any Error) in InstallerError.copyFailed(from: "a", to: "b", underlying: error) }
        func nested(_ code: POSIXErrorCode) -> NSError {
            NSError(
                domain: NSCocoaErrorDomain, code: CocoaError.fileWriteUnknown.rawValue,
                userInfo: [NSUnderlyingErrorKey: POSIXError(code) as NSError])
        }
        for code in [POSIXErrorCode.EACCES, .EPERM, .EROFS] {
            for error in [POSIXError(code) as NSError, nested(code)] {
                guard case .noPermission = InstallerError.writing(in: "/Applications", error, otherwise: other) else {
                    return XCTFail("\(code) isn't a lack of permission")
                }
            }
        }
        for code in [POSIXErrorCode.ENOSPC, .EDQUOT] {
            for error in [POSIXError(code) as NSError, nested(code)] {
                guard case .diskFull = InstallerError.writing(in: "/Applications", error, otherwise: other) else {
                    return XCTFail("\(code) isn't a full disk")
                }
            }
        }
        guard case .copyFailed = InstallerError.writing(in: "/Applications", POSIXError(.EIO), otherwise: other)
        else { return XCTFail("EIO is neither") }
    }
}
