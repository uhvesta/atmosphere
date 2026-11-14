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
}
