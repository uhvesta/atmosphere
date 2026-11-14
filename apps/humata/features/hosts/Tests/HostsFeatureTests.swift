import ComposableArchitecture
import Foundation
import HumataDaemonClient
import HumataHosts
import XCTest

@MainActor
final class HostsFeatureTests: XCTestCase {
    func testAHostAddedFromHostsIsInstalledThenChecked() async throws {
        let daemon = InMemoryDaemon(sshHosts: ["build-box", "gpu"])
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.addTapped)
        await store.send(\.destination.add.task)
        await store.receive(\.destination.add.sshHostsLoaded)
        XCTAssertEqual(store.state.destination?.add?.sshHosts, ["build-box", "gpu"])
        await store.send(\.destination.add.sshHostPicked, "gpu")
        XCTAssertEqual(store.state.destination?.add?.name, "gpu")
        await store.send(\.destination.add.addTapped)
        await store.receive(\.destination.add.delegate.added)
        XCTAssertNil(store.state.destination)
        XCTAssertEqual(store.state.selection, "gpu")
        await store.receive(\.installTapped)
        XCTAssertEqual(store.state.health["gpu"], .installing)
        await store.receive(\.installed)
        guard case .known(.reachable(let status)) = store.state.health["gpu"] else {
            return XCTFail("added from Hosts, gpu isn't reachable: \(String(describing: store.state.health["gpu"]))")
        }
        XCTAssertFalse(status.githubToken)
    }

    func testAHostReachedByItsOwnCommandIsOnlyChecked() async throws {
        let installs = LockIsolated(0)
        let own = RemoteHost(name: "box", sshDestination: nil)
        var state = HostsFeature.State()
        state.destination = .add(AddHostFeature.State())
        let store = TestStore(initialState: state) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = InMemoryDaemon().client
            $0.daemon.installHost = { _ in
                installs.withValue { $0 += 1 }
                return "0.0.0"
            }
            $0.daemon.checkHost = { _ in .unreachable("its command didn't answer") }
        }
        store.exhaustivity = .off

        await store.send(.destination(.presented(.add(.delegate(.added(own))))))
        XCTAssertEqual(store.state.selection, "box")
        await store.receive(\.checked)
        await store.finish()
        XCTAssertEqual(installs.value, 0)
    }

    func testRestartingTheDaemonFromHostsAsksFirst() async throws {
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off

        // Asked, it only asks: the window is told once the person confirms.
        await store.send(.restartDaemonTapped)
        XCTAssertNotNil(store.state.destination?.alert)
        await store.finish()

        await store.send(.destination(.presented(.alert(.confirmRestartDaemon))))
        await store.receive(\.delegate.restartDaemon)
        XCTAssertNil(store.state.destination)
    }

    func testOnlyAnEnvironmentProblemOrAFailedSetupMarksAHost() {
        let start = Date(timeIntervalSince1970: 1_794_000_600)
        // A host for Claude Code alone: no Copilot, no GitHub token.
        var status = HostStatus(
            programVersion: "0.0.0", platform: "Linux x86_64", freeDiskBytes: 120_000_000_000,
            copilot: .failure(ProgramProblem("copilot isn't on the PATH")), githubToken: false,
            claude: .success("2.1.322 (Claude Code)"))
        XCTAssertFalse(status.needsAttention)
        status.claude = .failure(ProgramProblem("claude isn't on the PATH"))
        XCTAssertFalse(status.needsAttention)
        status.setup = SetupResult(succeeded: true, output: "", time: start)
        XCTAssertFalse(status.needsAttention)

        status.setup = SetupResult(succeeded: false, output: "apt-get: not found", time: start)
        XCTAssertTrue(status.needsAttention)
        status.setup = nil
        status.environmentProblem = "the login shell didn't answer"
        XCTAssertTrue(status.needsAttention)
    }

    func testLostSessionsThatCantBeReadSayWhyAndCanBeReadAgain() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        let fails = LockIsolated(false)
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            let read = $0.daemon.recoveries
            $0.daemon.recoveries = {
                if fails.value { throw DaemonError.disconnected }
                return try await read()
            }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.recoveriesLoaded)
        XCTAssertEqual(store.state.recoveries.ids, [session.id])
        XCTAssertNil(store.state.recoveriesFailure)

        fails.setValue(true)
        await store.send(.recoveriesRetryTapped)
        await store.receive(\.recoveriesFailed)
        XCTAssertNotNil(store.state.recoveriesFailure)
        XCTAssertEqual(store.state.recoveries.ids, [session.id], "the list is left as it was")

        fails.setValue(false)
        await store.send(.recoveriesRetryTapped)
        await store.receive(\.recoveriesLoaded)
        XCTAssertNil(store.state.recoveriesFailure)
        XCTAssertEqual(store.state.recoveries.ids, [session.id])
    }

    func testSkippedSessionsThatCantBeReadOnThisMacSayWhy() async throws {
        let fails = LockIsolated(true)
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = InMemoryDaemon().client
            let read = $0.daemon.skipped
            $0.daemon.skipped = { host in
                if fails.value { throw DaemonError.disconnected }
                return try await read(host)
            }
        }
        store.exhaustivity = .off

        await store.send(.selected(""))
        await store.receive(\.skippedFailed)
        XCTAssertNotNil(store.state.skippedFailure)

        fails.setValue(false)
        await store.send(.skippedRetryTapped)
        await store.receive(\.skippedLoaded)
        XCTAssertNil(store.state.skippedFailure)
    }

    func testAnUnreachableHostsSkippedSessionsSayNothing() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        daemon.setOffline("far", true)
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.skipped = { _ in
                throw DaemonError.refused(.hostUnavailable, message: "ssh couldn't connect to far.")
            }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.checked)
        guard case .known(.unreachable) = store.state.health["far"] else {
            return XCTFail("far answered: \(String(describing: store.state.health["far"]))")
        }
        await store.finish()
        XCTAssertNil(store.state.skippedFailure)

        // Reachable again, a read that still fails is worth saying.
        daemon.setOffline("far", false)
        await store.send(.checkTapped("far"))
        await store.receive(\.skippedFailed)
        XCTAssertNotNil(store.state.skippedFailure)
    }

    func testThisMacShowsItsCommandsUnlinkedUntilLinked() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.selected(""))
        await store.receive(\.commandLinksLoaded)
        let checked = try XCTUnwrap(store.state.commandLinks)
        XCTAssertTrue(checked.linkable)
        XCTAssertEqual(checked.links.map(\.state), [.missing, .missing])

        await store.send(.linkCommandsTapped)
        XCTAssertTrue(store.state.linking)
        await store.receive(\.commandLinksLoaded)
        XCTAssertFalse(store.state.linking)
        XCTAssertEqual(store.state.commandLinks?.links.map(\.state), [.linked, .linked])
        XCTAssertEqual(store.state.commandLinks?.linkable, false)

        await store.send(.selected("gpu"))
        XCTAssertNil(store.state.commandLinks)
    }

    func testTheListChecksEveryHost() async throws {
        let daemon = InMemoryDaemon()
        for name in ["a", "b"] {
            _ = try await daemon.client.addHost(name, "me@\(name)")
        }
        _ = try await daemon.client.installHost("b")
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.selection, "a")
        await store.receive(\.checked)
        await store.receive(\.checked)
        guard case .known(.unreachable) = store.state.health["a"], case .known(.reachable) = store.state.health["b"]
        else {
            return XCTFail("\(store.state.health)")
        }
    }

    func testShowingTheListAgainKeepsWhatWasKnownUntilTheCheckAnswers() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("b", "me@b")
        _ = try await daemon.client.installHost("b")
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.checked)

        await store.send(.task)
        await store.receive(\.loaded)
        guard case .known(.reachable) = store.state.health["b"] else {
            return XCTFail("\(store.state.health)")
        }
        await store.receive(\.checked)
    }

    func testVariablesAreSentButTheirValuesNeverShown() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        _ = try await daemon.client.updateHost("far", HostUpdate(setEnv: ["OLD": "1", "KEEP": "2"]))
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        let editor = try XCTUnwrap(store.state.editor)
        XCTAssertEqual(editor.variables.map(\.name), ["KEEP", "OLD"])
        XCTAssertEqual(editor.variables.map(\.value), ["", ""])
        XCTAssertFalse(editor.canSave)

        let old = try XCTUnwrap(editor.variables.first { $0.name == "OLD" })
        await store.send(\.editor.removeVariableTapped, old.id)
        await store.send(\.editor.addVariableTapped)
        let added = try XCTUnwrap(store.state.editor?.variables.last)
        var token = added
        token.name = "GH_TOKEN"
        token.value = "secret"
        var variables = store.state.editor?.variables ?? []
        variables[id: token.id] = token
        await store.send(\.editor.binding.variables, variables)
        await store.send(\.editor.binding.setupScript, "apt-get install -y ripgrep\n")
        await store.send(\.editor.saveTapped)
        await store.receive(\.editor.delegate.saved)
        XCTAssertEqual(daemon.env(of: "far"), ["KEEP": "2", "GH_TOKEN": "secret"])
        XCTAssertEqual(store.state.editor?.variables.map(\.value), ["", ""])
        XCTAssertEqual(store.state.hosts[id: "far"]?.setupScript, "apt-get install -y ripgrep\n")
        await store.receive(\.checked)
        guard case .known(.reachable(let status)) = store.state.health["far"] else {
            return XCTFail("far isn't reachable after saving")
        }
        XCTAssertTrue(status.githubToken)
        XCTAssertEqual(status.setup?.succeeded, true)
    }

    func testThisMacsVariablesAreEditedLikeAHostsWithNoSetupScript() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.coordinatorLoaded)
        // With no remote hosts, This Mac is what there is to edit.
        XCTAssertEqual(store.state.selected?.isCoordinator, true)

        await store.send(\.editor.addVariableTapped)
        var variables = store.state.editor?.variables ?? []
        let row = try XCTUnwrap(variables.last)
        variables[id: row.id]?.name = "GH_TOKEN"
        variables[id: row.id]?.value = "secret"
        await store.send(\.editor.binding.variables, variables)
        await store.send(\.editor.saveTapped)
        await store.receive(\.editor.delegate.saved)
        XCTAssertEqual(daemon.env(of: ""), ["GH_TOKEN": "secret"])
        XCTAssertEqual(store.state.coordinator.envNames, ["GH_TOKEN"])
        XCTAssertEqual(store.state.editor?.variables.map(\.value), [""])
        XCTAssertTrue(store.state.hosts.isEmpty)
    }

    func testAProviderIsSavedWithItsVariablesNameAndTheKeyOnlyAmongTheVariables() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.coordinatorLoaded)
        XCTAssertEqual(store.state.selected?.isCoordinator, true)
        await store.send(\.editor.addProviderTapped, .openCodeGo)
        let provider = try XCTUnwrap(store.state.editor?.providers.first)
        XCTAssertEqual(provider.keyVariable, HostProvider.Kind.openCodeGo.keyVariable)
        XCTAssertEqual(store.state.editor?.keyIsSet(for: provider), false)
        XCTAssertEqual(store.state.editor?.offersAddKeyVariable(for: provider), true)

        await store.send(\.editor.addKeyVariableTapped, provider.id)
        XCTAssertEqual(store.state.editor?.offersAddKeyVariable(for: provider), false)
        var variables = store.state.editor?.variables ?? []
        let row = try XCTUnwrap(variables.last)
        XCTAssertEqual(row.name, provider.keyVariable)
        variables[id: row.id]?.value = "sk-secret"
        await store.send(\.editor.binding.variables, variables)
        XCTAssertEqual(store.state.editor?.keyIsSet(for: provider), true)

        await store.send(\.editor.saveTapped)
        await store.receive(\.editor.delegate.saved)
        XCTAssertEqual(store.state.coordinator.providers.map(\.keyVariable), [provider.keyVariable])
        XCTAssertEqual(store.state.coordinator.providers(for: .local).map(\.id), [provider.id])
        XCTAssertEqual(store.state.coordinator.providers, [provider])
        XCTAssertEqual(daemon.env(of: "")[provider.keyVariable], "sk-secret")
        XCTAssertNil(store.state.editor?.update)

        // Removed, the next save takes it away.
        await store.send(\.editor.removeProviderTapped, provider.id)
        await store.send(\.editor.saveTapped)
        await store.receive(\.editor.delegate.saved)
        XCTAssertTrue(store.state.coordinator.providers.isEmpty)
    }

    func testAProviderNeedsANameAWebAddressAndAVariablesName() {
        var state = HostEditorFeature.State(host: RemoteHost(name: "far", sshDestination: "me@far"))
        state.providers = [HostProvider(id: "p", kind: .openAI)]
        XCTAssertTrue(state.canSave)
        state.providers[id: "p"]?.keyVariable = "sk-typed-a-key-here"
        XCTAssertFalse(state.canSave)
        state.providers[id: "p"]?.keyVariable = "OPENAI_API_KEY"
        state.providers[id: "p"]?.baseURL = "api.openai.com/v1"
        XCTAssertFalse(state.canSave)
        state.providers[id: "p"]?.baseURL = " https://api.openai.com/v1 "
        state.providers[id: "p"]?.name = " "
        XCTAssertFalse(state.canSave)
        state.providers[id: "p"]?.name = "Mine"
        XCTAssertTrue(state.canSave)
        XCTAssertEqual(state.update?.providers?.first?.baseURL, "https://api.openai.com/v1")
    }

    func testAKeyPastedAsTheProvidersVariableIsAProblem() throws {
        var state = HostEditorFeature.State(host: RemoteHost(name: "far", sshDestination: "me@far"))
        state.providers = [HostProvider(id: "p", kind: .openAI)]
        let provider = try XCTUnwrap(state.providers[id: "p"])
        XCTAssertNil(state.problem(with: provider))

        state.providers[id: "p"]?.keyVariable = "sk1234abcd5678efgh"
        let pasted = try XCTUnwrap(state.providers[id: "p"])
        XCTAssertNotNil(state.problem(with: pasted))
        XCTAssertFalse(state.canSave)

        state.providers[id: "p"]?.keyVariable = String(repeating: "K", count: 65)
        let long = try XCTUnwrap(state.providers[id: "p"])
        XCTAssertNotNil(state.problem(with: long))
    }

    func testAVariableNeedsAName() {
        var state = HostEditorFeature.State(host: RemoteHost(name: "far", sshDestination: "me@far"))
        state.variables = [HostEditorFeature.Variable(id: 0, name: "1BAD", value: "x")]
        XCTAssertFalse(state.canSave)
        state.variables = [HostEditorFeature.Variable(id: 0, name: "GOOD_1", value: "x")]
        XCTAssertTrue(state.canSave)
        state.variables = [HostEditorFeature.Variable(id: 0, name: "EMPTY")]
        XCTAssertFalse(state.canSave)
    }

    func testWhatWasChangedOnAHostIsStillThereAfterLookingAtAnother() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.addHost("near", "me@near")
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.selected("far"))
        await store.send(\.editor.binding.setupScript, "apt-get install -y ripgrep\n")

        await store.send(.selected("near"))
        XCTAssertEqual(store.state.editor?.host.id, "near")
        XCTAssertNil(store.state.editor?.update)

        await store.send(.selected("far"))
        XCTAssertEqual(store.state.editor?.host.id, "far")
        XCTAssertEqual(store.state.editor?.setupScript, "apt-get install -y ripgrep\n")

        // Saved, there's nothing left to bring back.
        await store.send(\.editor.saveTapped)
        await store.receive(\.editor.delegate.saved)
        await store.send(.selected("near"))
        await store.send(.selected("far"))
        XCTAssertNil(store.state.editor?.update)
        XCTAssertEqual(store.state.editor?.setupScript, "apt-get install -y ripgrep\n")
        await store.skipReceivedActions(strict: false)
    }

    func testASaveThatAnswersAfterAnotherHostIsPickedLeavesThatHostsEditorAlone() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.addHost("near", "me@near")
        let (saveAsked, asked) = AsyncStream.makeStream(of: Void.self)
        let (saveAnswers, answer) = AsyncStream.makeStream(of: Void.self)
        var client = daemon.client
        let update = client.updateHost
        client.updateHost = { host, changes in
            asked.yield()
            for await _ in saveAnswers { break }
            return try await update(host, changes)
        }
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.selected("far"))
        await store.send(\.editor.binding.setupScript, "apt-get install -y ripgrep\n")
        await store.send(\.editor.saveTapped)
        for await _ in saveAsked { break }

        await store.send(.selected("near"))
        answer.yield()
        await store.receive(\.editor.delegate.saved)
        XCTAssertEqual(store.state.editor?.host.id, "near")
        XCTAssertNil(store.state.editor?.update)
        XCTAssertEqual(store.state.hosts[id: "far"]?.setupScript, "apt-get install -y ripgrep\n")

        // Saved, far has nothing left to bring back.
        await store.send(.selected("far"))
        XCTAssertNil(store.state.editor?.update)
        XCTAssertEqual(store.state.editor?.setupScript, "apt-get install -y ripgrep\n")
        await store.skipReceivedActions(strict: false)
    }

    func testASaveThatFailsAfterAnotherHostIsPickedSaysSoAndKeepsWhatWasChanged() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.addHost("near", "me@near")
        let (saveAsked, asked) = AsyncStream.makeStream(of: Void.self)
        let (saveAnswers, answer) = AsyncStream.makeStream(of: Void.self)
        var client = daemon.client
        client.updateHost = { _, _ in
            asked.yield()
            for await _ in saveAnswers { break }
            throw DaemonError.unexpected("no answer")
        }
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.selected("far"))
        await store.send(\.editor.binding.setupScript, "apt-get install -y ripgrep\n")
        await store.send(\.editor.saveTapped)
        for await _ in saveAsked { break }

        await store.send(.selected("near"))
        answer.yield()
        await store.receive(\.editor.delegate.saveFailed)
        XCTAssertNotNil(store.state.destination?.alert)
        XCTAssertEqual(store.state.editor?.host.id, "near")
        XCTAssertNil(store.state.editor?.alert)
        XCTAssertNil(store.state.editor?.update)

        await store.send(.selected("far"))
        XCTAssertEqual(store.state.editor?.setupScript, "apt-get install -y ripgrep\n")
        XCTAssertEqual(store.state.editor?.canSave, true)
        await store.skipReceivedActions(strict: false)
    }

    func testAHostsLateListingDoesntShowUnderTheHostPickedAfterIt() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        daemon.addFile("/home/me/far.txt", Data("far\n".utf8), on: "far")
        daemon.addFile("/Users/me/mac.txt", Data("mac\n".utf8))
        let (farAsked, asked) = AsyncStream.makeStream(of: Void.self)
        let (farAnswers, answer) = AsyncStream.makeStream(of: Void.self)
        var client = daemon.client
        let list = client.listFolder
        client.listFolder = { place, directory in
            if place.host == "far" {
                asked.yield()
                for await _ in farAnswers { break }
            }
            return try await list(place, directory)
        }
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.selected("far"))
        await store.send(\.files.task)
        for await _ in farAsked { break }

        await store.send(.selected(""))
        await store.send(\.files.task)
        await store.receive(\.files.folderRead)
        XCTAssertEqual(store.state.files?.rows.map(\.path), ["mac.txt"])

        answer.yield()
        await store.finish()
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.files?.root, "/Users/me")
        XCTAssertEqual(store.state.files?.rows.map(\.path), ["mac.txt"])
    }

    func testACheckThatAnswersDuringAnInstallLeavesItInstalling() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("gpu", "me@gpu")
        let (installAsked, asked) = AsyncStream.makeStream(of: Void.self)
        let (installAnswers, answer) = AsyncStream.makeStream(of: Void.self)
        let installs = LockIsolated(0)
        var client = daemon.client
        let install = client.installHost
        client.installHost = { host in
            installs.withValue { $0 += 1 }
            asked.yield()
            for await _ in installAnswers { break }
            return try await install(host)
        }
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.checked)
        await store.send(.installTapped("gpu"))
        for await _ in installAsked { break }

        // A check asked before the install answers while it runs.
        await store.send(.checked("gpu", .unreachable("Humata's program isn't on gpu yet.")))
        XCTAssertEqual(store.state.health["gpu"], .installing)
        await store.send(.checkTapped("gpu"))
        XCTAssertEqual(store.state.health["gpu"], .installing)
        await store.send(.installTapped("gpu"))
        XCTAssertEqual(installs.value, 1)

        answer.yield()
        await store.receive(\.installed)
        guard case .known(.reachable) = store.state.health["gpu"] else {
            return XCTFail("installed, gpu still isn't reachable: \(String(describing: store.state.health["gpu"]))")
        }
        // Left while it installs, the list checks the host again when it shows.
        var leaving = store.state
        leaving.health["gpu"] = .installing
        XCTAssertNil(leaving.settled().health["gpu"])
    }

    func testRemovingAHostAsksFirst() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.removeTapped("far"))
        let hosts = try await daemon.client.hosts()
        XCTAssertEqual(hosts.map(\.name), ["far"])
        await store.send(.destination(.presented(.alert(.confirmRemove("far")))))
        await store.receive(\.removed)
        XCTAssertEqual(store.state.hosts, [])
        // With the last remote host gone, This Mac is what's left selected.
        XCTAssertEqual(store.state.selected?.isCoordinator, true)
        let left = try await daemon.client.hosts()
        XCTAssertEqual(left, [])
    }

    func testASessionThatCouldntBeOpenedIsListedAndRemovedAfterAsking() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let damaged = SkippedSession(
            backend: .claude, id: "5e1f", folder: "/home/me/.humata/claude/sessions/5e1f",
            reason: "the session record is damaged")
        daemon.addSkipped(damaged, on: "far")
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.skippedLoaded)
        XCTAssertEqual(store.state.skipped.elements, [damaged])

        // This Mac skipped none.
        await store.send(.selected(store.state.coordinator.id))
        XCTAssertEqual(store.state.skipped, [])
        await store.receive(\.skippedLoaded)
        XCTAssertEqual(store.state.skipped, [])

        await store.send(.selected("far"))
        await store.receive(\.skippedLoaded)
        await store.send(.removeSkippedTapped(damaged.id))
        let stillThere = try await daemon.client.skipped("far")
        XCTAssertEqual(stillThere, [damaged])
        await store.send(.destination(.presented(.alert(.confirmRemoveSkipped(damaged.id)))))
        await store.receive(\.skippedRemoved)
        XCTAssertEqual(store.state.skipped, [])
        let left = try await daemon.client.skipped("far")
        XCTAssertEqual(left, [])
    }

    func testSessionsAHostLostAreRestoredFromTheirCopies() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let first = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        let second = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.recoveriesLoaded)
        XCTAssertEqual(Set(store.state.recoveries.ids), [first.id, second.id])
        XCTAssertTrue(store.state.recoveries.allSatisfy(\.onHost))

        daemon.lose(host: "far")
        await store.send(.selected("far"))
        await store.receive(\.recoveriesLoaded)
        XCTAssertTrue(store.state.recoveries.allSatisfy { !$0.onHost })

        await store.send(.restoreTapped(first.id))
        XCTAssertEqual(store.state.restoring, [first.id])
        await store.receive(\.restored)
        await store.receive(\.recoveriesLoaded)
        XCTAssertEqual(store.state.restoring, [])
        XCTAssertEqual(store.state.recoveries[id: first.id]?.onHost, true)
        XCTAssertEqual(store.state.recoveries[id: second.id]?.onHost, false)

        await store.send(.restoreAllTapped)
        await store.receive(\.restored)
        await store.receive(\.recoveriesLoaded)
        XCTAssertTrue(store.state.recoveries.allSatisfy(\.onHost))
        let sessions = try await daemon.client.sessions(nil)
        XCTAssertEqual(sessions.filter { $0.host == "far" }.count, 2)
    }

    func testASessionMovedToThisMacLeavesItsHostsCopies() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.recoveriesLoaded)
        await store.send(.moveTapped(session.id, to: nil))
        await store.receive(\.restored)
        await store.receive(\.recoveriesLoaded)
        XCTAssertEqual(store.state.recoveries, [])
        let sessions = try await daemon.client.sessions(nil)
        XCTAssertNil(sessions.first { $0.id == session.id }?.host)
    }

    func testARestoreOntoAnUnreachableHostSaysWhy() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("far", "me@far")
        _ = try await daemon.client.installHost("far")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", host: "far"))
        daemon.lose(host: "far")
        daemon.setOffline("far", true)
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.recoveriesLoaded)
        await store.send(.restoreTapped(session.id))
        await store.receive(\.restoreFailed)
        XCTAssertNotNil(store.state.destination?.alert)
        XCTAssertEqual(store.state.restoring, [])
        XCTAssertEqual(store.state.recoveries[id: session.id]?.onHost, false)
    }

    func testARestartThatWasRefusedSaysWhy() async {
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off

        XCTAssertNil(store.state.destination)
        await store.send(.restartRefused("Humata didn't start the daemon that answers."))
        XCTAssertNotNil(store.state.destination?.alert)
    }

    func testThisMacsSkippedSessionsAreReadAgainAfterTheDaemonRestarts() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        // With no hosts, This Mac is selected.
        await store.send(.task)
        await store.receive(\.skippedLoaded)
        XCTAssertEqual(store.state.selection, store.state.coordinator.id)
        XCTAssertEqual(store.state.skipped, [])

        // What the daemon couldn't open is what it found when it started.
        let damaged = SkippedSession(
            backend: .claude, id: "5e1f", folder: "/Users/me/.humata/claude/sessions/5e1f",
            reason: "the session record is damaged")
        daemon.addSkipped(damaged)
        await store.send(.daemonRestarted)
        await store.receive(\.skippedLoaded)
        XCTAssertEqual(store.state.skipped.elements, [damaged])
    }

    func testAnInstallThatFailsSaysSoAndIsNotCalledUnreachable() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("gpu", "me@gpu")
        daemon.setOffline("gpu", true)
        let store = TestStore(initialState: HostsFeature.State()) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.checked)
        await store.send(.installTapped("gpu"))
        await store.receive(\.installFailed)
        guard case .installFailed = store.state.health["gpu"] else {
            return XCTFail("the install failed, but gpu is \(String(describing: store.state.health["gpu"]))")
        }

        // Tried again once the host is up, it installs.
        daemon.setOffline("gpu", false)
        await store.send(.installTapped("gpu"))
        await store.receive(\.installed)
        guard case .known(.reachable) = store.state.health["gpu"] else {
            return XCTFail("installed, gpu isn't reachable: \(String(describing: store.state.health["gpu"]))")
        }
    }

    func testAHostThatDoesntAnswerAfterItsInstallIsUnreachable() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("gpu", "me@gpu")
        var state = HostsFeature.State()
        state.hosts = [RemoteHost(name: "gpu", sshDestination: "me@gpu")]
        let store = TestStore(initialState: state) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.checkHost = { _ in throw DaemonError.unexpected("it stopped answering") }
        }
        store.exhaustivity = .off

        await store.send(.installTapped("gpu"))
        await store.receive(\.installed)
        guard case .known(.unreachable) = store.state.health["gpu"] else {
            return XCTFail("installed and not answering, gpu is \(String(describing: store.state.health["gpu"]))")
        }
    }

    func testAHostAddedFromAnotherPaneOpensOnItsSettings() async throws {
        let daemon = InMemoryDaemon()
        let added = try await daemon.client.addHost("gpu", "me@gpu")
        var state = HostsFeature.State()
        state.pane = .files
        state.destination = .add(AddHostFeature.State())
        let store = TestStore(initialState: state) {
            HostsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.destination(.presented(.add(.delegate(.added(added))))))
        XCTAssertEqual(store.state.selection, "gpu")
        XCTAssertEqual(store.state.pane, .settings)
        await store.receive(\.installed)
    }

    func testARefusedHostSaysWhy() async {
        let store = TestStore(initialState: AddHostFeature.State()) {
            AddHostFeature()
        } withDependencies: {
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off

        await store.send(\.binding.name, "no spaces")
        await store.send(\.binding.destination, "me@far")
        await store.send(.addTapped)
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.alert)
        XCTAssertFalse(store.state.adding)
    }
}
