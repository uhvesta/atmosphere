import ComposableArchitecture
import Foundation
import HumataAppFeature
import XCTest

private let copySource = URL(filePath: "/Users/me/Downloads/Humata.app")
private let copyDestination = URL(filePath: "/Applications/Humata.app")
private let stagedCopy = URL(filePath: "/private/var/folders/xx/TemporaryItems/Humata.app")
private let appTeam = "TEAM123456"
private let copyVersion = BundleVersion(short: "0.2", build: "5")

@MainActor
final class InstallerFeatureTests: XCTestCase {
    /// Applications and the daemon as the installer leaves them.
    private struct Disk {
        /// The version of the copy in Applications; nil when there's none.
        var installed: BundleVersion?
        /// Whether the copy in Applications still has the downloaded mark.
        var installedQuarantined = false
        /// Copies staged and not yet swapped in or discarded.
        var staged: Set<URL> = []
        var quarantined: Set<URL> = []
        var daemonRunning = true
        var opened: [URL] = []
        var quit = false
    }

    private func survey(installed: BundleVersion?) -> InstallSurvey {
        InstallSurvey(
            source: copySource, destination: copyDestination, version: copyVersion, installed: installed,
            team: appTeam)
    }

    /// The installer over `disk`. A copy is staged with the downloaded mark,
    /// signed by `stagedTeam`; `copyFails` makes copying fail.
    private func makeStore(
        installed: BundleVersion?, stagedTeam: String? = appTeam, copyFails: Bool = false
    ) -> (TestStoreOf<InstallerFeature>, LockIsolated<Disk>) {
        let disk = LockIsolated(Disk(installed: installed))
        let client = InstallerClient(
            survey: { nil },
            stage: { _, _ in
                if copyFails {
                    throw InstallerError.diskFull(folder: "/Applications", underlying: CocoaError(.fileWriteOutOfSpace))
                }
                disk.withValue {
                    $0.staged.insert(stagedCopy)
                    $0.quarantined.insert(stagedCopy)
                }
                return stagedCopy
            },
            team: { _ in stagedTeam },
            clearQuarantine: { url in disk.withValue { _ = $0.quarantined.remove(url) } },
            swap: { staged, _ in
                disk.withValue {
                    $0.installed = copyVersion
                    $0.installedQuarantined = $0.quarantined.contains(staged)
                    $0.staged.remove(staged)
                }
            },
            discard: { url in disk.withValue { _ = $0.staged.remove(url) } },
            open: { url in disk.withValue { $0.opened.append(url) } },
            quit: { disk.withValue { $0.quit = true } })
        let store = TestStore(initialState: InstallerFeature.State(survey: survey(installed: installed))) {
            InstallerFeature()
        } withDependencies: {
            $0.installer = client
            $0.daemonService.stop = { disk.withValue { $0.daemonRunning = false } }
        }
        return (store, disk)
    }

    /// Sends Install and follows it through to opening the new copy.
    private func installThrough(_ store: TestStoreOf<InstallerFeature>) async {
        await store.send(.installTapped) { $0.phase = .working(.copying) }
        await store.receive(\.stepped) { $0.phase = .working(.stoppingDaemon) }
        await store.receive(\.stepped) { $0.phase = .working(.replacing) }
        await store.receive(\.stepped) { $0.phase = .working(.opening) }
        await store.finish()
    }

    func testANewerCopyReplacesTheInstalledOneAndOpensFromApplications() async {
        let (store, disk) = makeStore(installed: BundleVersion(short: "0.1", build: "9"))

        await installThrough(store)

        XCTAssertEqual(disk.value.installed, copyVersion)
        XCTAssertFalse(disk.value.daemonRunning)
        XCTAssertEqual(disk.value.opened, [copyDestination])
        XCTAssertTrue(disk.value.quit)
        XCTAssertTrue(disk.value.staged.isEmpty)
    }

    func testACopyInstallsWhereThereIsNone() async {
        let (store, disk) = makeStore(installed: nil)

        await installThrough(store)

        XCTAssertEqual(disk.value.installed, copyVersion)
        XCTAssertEqual(disk.value.opened, [copyDestination])
        XCTAssertTrue(disk.value.quit)
    }

    func testAnOlderCopyOpensTheInstalledOneAndCopiesNothing() async {
        let newer = BundleVersion(short: "0.3", build: "1")
        let (store, disk) = makeStore(installed: newer)
        XCTAssertEqual(store.state.survey.offer, .openInstalled(newer))

        // Installing over a newer copy isn't on offer.
        await store.send(.installTapped)
        await store.send(.openInstalledTapped) { $0.phase = .working(.opening) }
        await store.finish()

        XCTAssertEqual(disk.value.installed, newer)
        XCTAssertTrue(disk.value.daemonRunning)
        XCTAssertTrue(disk.value.staged.isEmpty)
        XCTAssertEqual(disk.value.opened, [copyDestination])
        XCTAssertTrue(disk.value.quit)
    }

    func testTheSameVersionOpensTheInstalledOne() {
        let (store, _) = makeStore(installed: copyVersion)
        XCTAssertEqual(store.state.survey.offer, .openInstalled(copyVersion))
    }

    func testAFailedCopySaysWhyAndLeavesTheInstalledAppAndDaemonAlone() async {
        let older = BundleVersion(short: "0.1", build: "1")
        let (store, disk) = makeStore(installed: older, copyFails: true)
        let failure = InstallerError.diskFull(folder: "/Applications", underlying: CocoaError(.fileWriteOutOfSpace))

        await store.send(.installTapped) { $0.phase = .working(.copying) }
        await store.receive(\.failed) { $0.phase = .failed(failure.localizedDescription) }
        await store.finish()

        XCTAssertEqual(disk.value.installed, older)
        XCTAssertTrue(disk.value.daemonRunning)
        XCTAssertTrue(disk.value.opened.isEmpty)
        XCTAssertFalse(disk.value.quit)
    }

    func testACopyFromTheAppsOwnTeamHasItsQuarantineCleared() async {
        let (store, disk) = makeStore(installed: nil, stagedTeam: appTeam)

        await installThrough(store)

        XCTAssertEqual(disk.value.installed, copyVersion)
        XCTAssertFalse(disk.value.installedQuarantined)
    }

    func testACopySignedByAnotherTeamOrNoneKeepsItsQuarantine() async {
        for team in ["OTHERTEAM1", nil] as [String?] {
            let (store, disk) = makeStore(installed: nil, stagedTeam: team)

            await installThrough(store)

            XCTAssertEqual(disk.value.installed, copyVersion)
            XCTAssertTrue(disk.value.installedQuarantined)
        }
    }

    func testRunFromHereClosesTheInstallerAndLeavesApplicationsAlone() async {
        var state = AppFeature.State()
        state.installer = InstallerFeature.State(survey: survey(installed: nil))
        let opened = LockIsolated<[URL]>([])
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.installer.open = { url in opened.withValue { $0.append(url) } }
        }

        await store.send(.installer(.presented(.runFromHereTapped)))
        await store.receive(\.installer.dismiss) { $0.installer = nil }
        await store.finish()

        XCTAssertTrue(opened.value.isEmpty)
    }
}
