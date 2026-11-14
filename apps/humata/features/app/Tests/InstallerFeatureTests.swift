import ComposableArchitecture
import Foundation
import HumataAppFeature
import HumataDaemonClient
import XCTest

private let copySource = URL(filePath: "/Users/me/Downloads/Humata.app")
private let copyDestination = URL(filePath: "/Applications/Humata.app")
private let stagedCopy = URL(filePath: "/private/var/folders/xx/TemporaryItems/Humata.app")
private let appTeam = "TEAM123456"
private let copyVersion = BundleVersion(short: "0.2", build: "5")
private let swapFailure = CocoaError(.fileWriteUnknown)
private let openFailure = CocoaError(.fileReadUnknown)

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
        var stops = 0
        var starts = 0
        var quitCopies: [RunningCopy] = []
        var opened: [URL] = []
        var quit = false
    }

    /// What fails, and what else is open, in the installer's world.
    private struct Fakes {
        var stagedTeam: String? = appTeam
        var copyFails = false
        var stopFails = false
        var swapFails = false
        var openFails = false
        /// Other copies of Humata that are open.
        var others: [RunningCopy] = []
        /// The sessions the daemon lists.
        var sessions: [Session] = []
    }

    private func survey(installed: BundleVersion?) -> InstallSurvey {
        InstallSurvey(
            source: copySource, destination: copyDestination, version: copyVersion, installed: installed,
            team: appTeam)
    }

    /// The installer over `disk`. A copy is staged with the downloaded mark,
    /// signed by `fakes.stagedTeam`; the rest of `fakes` says what fails.
    private func makeStore(
        installed: BundleVersion?, _ fakes: Fakes = Fakes()
    ) -> (TestStoreOf<InstallerFeature>, LockIsolated<Disk>) {
        let disk = LockIsolated(Disk(installed: installed))
        var client = InstallerClient()
        client.stage = { _, _ in
            if fakes.copyFails {
                throw InstallerError.diskFull(folder: "/Applications", underlying: CocoaError(.fileWriteOutOfSpace))
            }
            disk.withValue {
                $0.staged.insert(stagedCopy)
                $0.quarantined.insert(stagedCopy)
            }
            return stagedCopy
        }
        client.team = { _ in fakes.stagedTeam }
        client.clearQuarantine = { url in disk.withValue { _ = $0.quarantined.remove(url) } }
        client.swap = { staged, _ in
            if fakes.swapFails {
                throw InstallerError.replaceFailed(path: "/Applications", underlying: swapFailure)
            }
            disk.withValue {
                $0.installed = copyVersion
                $0.installedQuarantined = $0.quarantined.contains(staged)
                $0.staged.remove(staged)
            }
        }
        client.discard = { url in disk.withValue { _ = $0.staged.remove(url) } }
        client.open = { url in
            if fakes.openFails {
                throw InstallerError.openFailed(path: "/Applications", underlying: openFailure)
            }
            disk.withValue { $0.opened.append(url) }
        }
        client.quit = { disk.withValue { $0.quit = true } }
        client.otherCopies = { fakes.others }
        client.quitCopies = { copies in disk.withValue { $0.quitCopies = copies } }
        let store = TestStore(initialState: InstallerFeature.State(survey: survey(installed: installed))) {
            InstallerFeature()
        } withDependencies: {
            $0.installer = client
            $0.daemon.sessions = { _ in fakes.sessions }
            $0.daemonService.stop = {
                if fakes.stopFails { throw DaemonServiceError.stillRunning(seconds: 60) }
                return disk.withValue {
                    $0.stops += 1
                    defer { $0.daemonRunning = false }
                    return $0.daemonRunning
                }
            }
            $0.daemonService.start = { disk.withValue { $0.starts += 1 } }
        }
        return (store, disk)
    }

    /// Sends Install and follows it through to swapping the copy in.
    private func installThroughSwap(_ store: TestStoreOf<InstallerFeature>) async {
        await store.send(.installTapped) { $0.phase = .working(.checking) }
        await store.receive(\.checked) { $0.phase = .working(.copying) }
        await store.receive(\.stepped) { $0.phase = .working(.stoppingDaemon) }
        await store.receive(\.daemonStopped) { $0.daemonStopped = true }
        await store.receive(\.stepped) { $0.phase = .working(.replacing) }
        await store.receive(\.swapped) { $0.survey.installed = copyVersion }
    }

    /// Sends Install and follows it through to opening the new copy.
    private func installThrough(_ store: TestStoreOf<InstallerFeature>) async {
        await installThroughSwap(store)
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
        let (store, disk) = makeStore(installed: older, Fakes(copyFails: true))
        let failure = InstallerError.diskFull(folder: "/Applications", underlying: CocoaError(.fileWriteOutOfSpace))

        await store.send(.installTapped) { $0.phase = .working(.checking) }
        await store.receive(\.checked) { $0.phase = .working(.copying) }
        await store.receive(\.failed) { $0.phase = .failed(failure.localizedDescription) }
        await store.finish()

        XCTAssertEqual(disk.value.installed, older)
        XCTAssertTrue(disk.value.daemonRunning)
        XCTAssertTrue(disk.value.opened.isEmpty)
        XCTAssertFalse(disk.value.quit)
    }

    func testACopyFromTheAppsOwnTeamHasItsQuarantineCleared() async {
        let (store, disk) = makeStore(installed: nil, Fakes(stagedTeam: appTeam))

        await installThrough(store)

        XCTAssertEqual(disk.value.installed, copyVersion)
        XCTAssertFalse(disk.value.installedQuarantined)
    }

    func testACopySignedByAnotherTeamOrNoneKeepsItsQuarantine() async {
        for team in ["OTHERTEAM1", nil] as [String?] {
            let (store, disk) = makeStore(installed: nil, Fakes(stagedTeam: team))

            await installThrough(store)

            XCTAssertEqual(disk.value.installed, copyVersion)
            XCTAssertTrue(disk.value.installedQuarantined)
        }
    }

    func testASwapThatFailsOverAnInstalledCopyLeavesItForItsOwnDaemonAndOffersToOpenIt() async {
        let older = BundleVersion(short: "0.1", build: "1")
        let (store, disk) = makeStore(installed: older, Fakes(swapFails: true))
        let failure = InstallerError.stoppedDaemon(
            restarted: false,
            underlying: InstallerError.replaceFailed(path: "/Applications", underlying: swapFailure))

        await store.send(.installTapped) { $0.phase = .working(.checking) }
        await store.receive(\.checked) { $0.phase = .working(.copying) }
        await store.receive(\.stepped) { $0.phase = .working(.stoppingDaemon) }
        await store.receive(\.daemonStopped) { $0.daemonStopped = true }
        await store.receive(\.stepped) { $0.phase = .working(.replacing) }
        await store.receive(\.failed) { $0.phase = .failed(failure.localizedDescription) }
        await store.finish()

        XCTAssertEqual(disk.value.installed, older)
        XCTAssertTrue(disk.value.staged.isEmpty)
        // This copy's daemon isn't started over the installed copy's.
        XCTAssertEqual(disk.value.starts, 0)
        XCTAssertTrue(store.state.opensInstalled)
        XCTAssertFalse(store.state.offersRunFromHere)
    }

    func testASwapThatFailsWithNothingInstalledStartsTheDaemonAgain() async {
        let (store, disk) = makeStore(installed: nil, Fakes(swapFails: true))
        let failure = InstallerError.stoppedDaemon(
            restarted: true,
            underlying: InstallerError.replaceFailed(path: "/Applications", underlying: swapFailure))

        await store.send(.installTapped) { $0.phase = .working(.checking) }
        await store.receive(\.checked) { $0.phase = .working(.copying) }
        await store.receive(\.stepped) { $0.phase = .working(.stoppingDaemon) }
        await store.receive(\.daemonStopped) { $0.daemonStopped = true }
        await store.receive(\.stepped) { $0.phase = .working(.replacing) }
        await store.receive(\.daemonStopped) { $0.daemonStopped = false }
        await store.receive(\.failed) { $0.phase = .failed(failure.localizedDescription) }
        await store.finish()

        XCTAssertNil(disk.value.installed)
        XCTAssertTrue(disk.value.staged.isEmpty)
        XCTAssertEqual(disk.value.starts, 1)
        XCTAssertTrue(store.state.offersRunFromHere)
    }

    func testADaemonThatWontStopLeavesApplicationsAsItWas() async {
        let older = BundleVersion(short: "0.1", build: "1")
        let (store, disk) = makeStore(installed: older, Fakes(stopFails: true))
        let failure = DaemonServiceError.stillRunning(seconds: 60)

        await store.send(.installTapped) { $0.phase = .working(.checking) }
        await store.receive(\.checked) { $0.phase = .working(.copying) }
        await store.receive(\.stepped) { $0.phase = .working(.stoppingDaemon) }
        await store.receive(\.failed) { $0.phase = .failed(failure.localizedDescription) }
        await store.finish()

        XCTAssertEqual(disk.value.installed, older)
        XCTAssertTrue(disk.value.staged.isEmpty)
        XCTAssertTrue(disk.value.opened.isEmpty)
        XCTAssertFalse(store.state.daemonStopped)
    }

    func testAnInstalledCopyThatWontOpenIsOfferedToOpenAgain() async {
        let (store, disk) = makeStore(installed: nil, Fakes(openFails: true))
        let failure = InstallerError.stoppedDaemon(
            restarted: false,
            underlying: InstallerError.openFailed(path: "/Applications", underlying: openFailure))

        await installThroughSwap(store)
        await store.receive(\.stepped) { $0.phase = .working(.opening) }
        await store.receive(\.failed) { $0.phase = .failed(failure.localizedDescription) }
        await store.finish()

        XCTAssertEqual(disk.value.installed, copyVersion)
        XCTAssertEqual(store.state.survey.offer, .openInstalled(copyVersion))
        XCTAssertTrue(store.state.opensInstalled)
        XCTAssertFalse(disk.value.quit)
    }

    func testOtherOpenCopiesAreQuitOnlyOnceThePersonSaysSo() async {
        let others = [RunningCopy(id: 42, path: "/Users/me/Desktop/Humata.app")]
        let (store, disk) = makeStore(installed: nil, Fakes(others: others))

        await store.send(.installTapped) { $0.phase = .working(.checking) }
        await store.receive(\.checked) { $0.phase = .confirming(others: others, busy: false) }
        XCTAssertEqual(disk.value.stops, 0)
        XCTAssertTrue(disk.value.staged.isEmpty)

        await store.send(.confirmTapped) { $0.phase = .working(.copying) }
        await store.receive(\.stepped) { $0.phase = .working(.quittingOthers) }
        await store.receive(\.stepped) { $0.phase = .working(.stoppingDaemon) }
        await store.receive(\.daemonStopped) { $0.daemonStopped = true }
        await store.receive(\.stepped) { $0.phase = .working(.replacing) }
        await store.receive(\.swapped) { $0.survey.installed = copyVersion }
        await store.receive(\.stepped) { $0.phase = .working(.opening) }
        await store.finish()

        XCTAssertEqual(disk.value.quitCopies, others)
        XCTAssertEqual(disk.value.installed, copyVersion)
        XCTAssertTrue(disk.value.quit)
    }

    func testCancellingLeavesTheOtherCopiesApplicationsAndTheDaemonAlone() async {
        let others = [RunningCopy(id: 42, path: "/Users/me/Desktop/Humata.app")]
        let older = BundleVersion(short: "0.1", build: "1")
        let (store, disk) = makeStore(installed: older, Fakes(others: others))

        await store.send(.installTapped) { $0.phase = .working(.checking) }
        await store.receive(\.checked) { $0.phase = .confirming(others: others, busy: false) }
        await store.send(.cancelTapped) { $0.phase = .offering }
        await store.finish()

        XCTAssertTrue(disk.value.quitCopies.isEmpty)
        XCTAssertEqual(disk.value.stops, 0)
        XCTAssertEqual(disk.value.installed, older)
        XCTAssertTrue(disk.value.staged.isEmpty)
    }

    func testASessionInATurnIsAskedAboutBeforeTheDaemonStops() async {
        let busy = Session(id: "s1", name: "busy", backend: .scripted, workingDirectory: "/work", state: .working)
        let (store, disk) = makeStore(installed: nil, Fakes(sessions: [busy]))

        await store.send(.installTapped) { $0.phase = .working(.checking) }
        await store.receive(\.checked) { $0.phase = .confirming(others: [], busy: true) }
        XCTAssertEqual(disk.value.stops, 0)

        await store.send(.confirmTapped) { $0.phase = .working(.copying) }
        await store.receive(\.stepped) { $0.phase = .working(.stoppingDaemon) }
        await store.receive(\.daemonStopped) { $0.daemonStopped = true }
        await store.receive(\.stepped) { $0.phase = .working(.replacing) }
        await store.receive(\.swapped) { $0.survey.installed = copyVersion }
        await store.receive(\.stepped) { $0.phase = .working(.opening) }
        await store.finish()

        XCTAssertEqual(disk.value.stops, 1)
        XCTAssertEqual(disk.value.installed, copyVersion)
    }

    func testRunningFromHereIsntOfferedBesideANewerInstalledCopy() async {
        let (store, disk) = makeStore(installed: BundleVersion(short: "0.3", build: "1"))
        XCTAssertFalse(store.state.offersRunFromHere)

        await store.send(.runFromHereTapped)
        await store.finish()

        XCTAssertTrue(disk.value.daemonRunning)
        XCTAssertEqual(disk.value.starts, 0)
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
