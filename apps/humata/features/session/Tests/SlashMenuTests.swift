import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import XCTest

@MainActor
final class SlashMenuTests: XCTestCase {
    private func choices(_ names: String...) -> [SlashChoice] { names.map { SlashChoice($0) } }

    func testMatchingPutsPrefixesFirstThenWordsThenContainsThenLettersInOrder() {
        let all = choices("pr-review", "review", "code-review", "previewer", "rv", "deploy")
        XCTAssertEqual(
            SlashChoice.matching("rev", in: all).map(\.id),
            ["review", "pr-review", "code-review", "previewer"])
        XCTAssertEqual(SlashChoice.matching("rv", in: all).map(\.id).first, "rv")
        XCTAssertTrue(SlashChoice.matching("rv", in: all).map(\.id).contains("review"))
        XCTAssertEqual(SlashChoice.matching("zzz", in: all), [])
        XCTAssertEqual(SlashChoice.matching("", in: all).count, all.count)
        XCTAssertEqual(SlashChoice.matching("REV", in: all).first?.id, "review")
    }

    func testAQueryIsOnlyASlashAndANameWithNothingAfterIt() {
        XCTAssertEqual(SlashChoice.query(in: "/"), "")
        XCTAssertEqual(SlashChoice.query(in: "/rel"), "rel")
        XCTAssertNil(SlashChoice.query(in: "/release now"))
        XCTAssertNil(SlashChoice.query(in: "release"))
        XCTAssertNil(SlashChoice.query(in: ""))
    }

    private func session(_ backend: Backend, project: String? = "p1", host: String? = nil) -> Session {
        Session(id: "s1", name: "chat", backend: backend, workingDirectory: "/work", host: host, projectID: project)
    }

    func testEachBackendIsOfferedOnlyTheSkillsItReads() {
        let skills = SkillList(skills: [
            FoundSkill(name: "mine", origin: .claudeUser, hash: "a"),
            FoundSkill(name: "copilots", origin: .copilotUser, hash: "b"),
            FoundSkill(name: "repo", origin: .claudeRepository, projectID: "p1", hash: "c"),
            FoundSkill(name: "github", origin: .githubRepository, projectID: "p1", hash: "d"),
            FoundSkill(name: "other-project", origin: .project, projectID: "p2", hash: "e"),
            FoundSkill(name: "elsewhere", origin: .claudeUser, hash: "f", host: "box"),
        ])
        let claude = SlashChoice.offered(to: session(.claude), skills: skills, plugins: PluginList(), project: nil)
        XCTAssertEqual(Set(claude.map(\.id)), ["mine", "repo"])
        let copilot = SlashChoice.offered(to: session(.copilot), skills: skills, plugins: PluginList(), project: nil)
        XCTAssertEqual(Set(copilot.map(\.id)), ["copilots", "repo", "github"])
        let remote = SlashChoice.offered(
            to: session(.claude, host: "box"), skills: skills, plugins: PluginList(), project: nil)
        XCTAssertEqual(remote.map(\.id), ["elsewhere"])
    }

    func testAClashOffersOnlyThePickedFile() {
        let one = FoundSkill(name: "release", description: "one", origin: .claudeUser, hash: "a")
        let two = FoundSkill(name: "release", description: "two", origin: .claudeRepository, projectID: "p1", hash: "b")
        let skills = SkillList(
            skills: [one, two], clashes: [SkillClash(name: "release", candidates: [one, two], winner: "b")])
        let offered = SlashChoice.offered(to: session(.claude), skills: skills, plugins: PluginList(), project: nil)
        XCTAssertEqual(offered, [SlashChoice("release", summary: "two")])
    }

    func testPluginsOfferTheirSkillsAndCommandsWhenOnForTheAgent() {
        let plugins = PluginList(plugins: [
            Plugin(agent: .claude, pluginID: "review@m", skills: ["audit"], commands: ["ship"]),
            Plugin(agent: .claude, pluginID: "off@m", enabled: false, commands: ["hidden"]),
            Plugin(agent: .copilot, pluginID: "cop@m", commands: ["copilot-only"]),
        ])
        let offered = SlashChoice.offered(to: session(.claude), skills: SkillList(), plugins: plugins, project: nil)
        XCTAssertEqual(offered.map(\.id), ["audit", "ship"])
    }

    private func store(_ daemon: InMemoryDaemon, _ session: Session, draft: String? = nil)
        -> TestStoreOf<SessionFeature>
    {
        var state = SessionFeature.State(session: session)
        if let draft { state.draft = draft }
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = ImmediateClock()
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_000))
        }
        store.exhaustivity = .off
        return store
    }

    private func started(_ daemon: InMemoryDaemon) async throws -> Session {
        try await daemon.client.start(StartRequest(backend: .claude, workingDirectory: "/work", name: "first"))
    }

    func testTypingASlashListsTheSkillsAndNarrowsAsTheNameIsTyped() async throws {
        let daemon = InMemoryDaemon()
        daemon.setSkills(
            SkillList(skills: [
                FoundSkill(name: "release", origin: .claudeUser, hash: "a"),
                FoundSkill(name: "review", origin: .claudeUser, hash: "b"),
            ]))
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertTrue(store.state.slashOpen)
        XCTAssertEqual(store.state.slashChoices.map(\.id), ["release", "review"])

        await store.send(\.binding.draft, "/rev")
        XCTAssertEqual(store.state.slashChoices.map(\.id), ["review"])

        await store.send(\.binding.draft, "/rev now")
        XCTAssertEqual(store.state.slashChoices, [])
    }

    func testReturnPicksTheSelectedSkillInsteadOfSending() async throws {
        let daemon = InMemoryDaemon()
        daemon.setSkills(
            SkillList(skills: [
                FoundSkill(name: "release", origin: .claudeUser, hash: "a"),
                FoundSkill(name: "review", origin: .claudeUser, hash: "b"),
            ]))
        let session = try await started(daemon)
        let store = store(daemon, session)

        await store.send(\.binding.draft, "/re")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.slashMoved(by: 1))
        await store.send(.slashMoved(by: 1))
        XCTAssertEqual(store.state.slashSelected, 0, "moving past the end wraps around")
        await store.send(.slashMoved(by: -1))
        let picked = store.state.slashChoices[store.state.slashSelected].id
        XCTAssertEqual(store.state.slashSelected, 1)
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.slashPicked)
        XCTAssertEqual(store.state.draft, "/\(picked) ")
        XCTAssertEqual(store.state.slashChoices, [])
        XCTAssertEqual(daemon.history(session.id), [], "picking sends nothing")
    }

    func testEscapeClosesTheListUntilTheNameChanges() async throws {
        let daemon = InMemoryDaemon()
        daemon.setSkills(SkillList(skills: [FoundSkill(name: "release", origin: .claudeUser, hash: "a")]))
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/rel")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.slashClosed)
        XCTAssertFalse(store.state.slashOpen)
        XCTAssertEqual(store.state.slashChoices, [])

        await store.send(\.binding.draft, "/rele")
        XCTAssertTrue(store.state.slashOpen)
        XCTAssertEqual(store.state.slashChoices.map(\.id), ["release"])
    }

    private func skilled(_ names: String...) -> InMemoryDaemon {
        let daemon = InMemoryDaemon()
        daemon.setSkills(
            SkillList(skills: names.enumerated().map { FoundSkill(name: $1, origin: .claudeUser, hash: "\($0)") }))
        return daemon
    }

    private func shown(_ store: TestStoreOf<SessionFeature>) -> [String] {
        store.state.slashOpen ? store.state.slashChoices.map(\.id) : []
    }

    func testEscapeHoldsOnlyUntilTheNameIsTypedAgain() async throws {
        let daemon = skilled("release")
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/rel")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.slashClosed)
        await store.send(\.binding.draft, "/re")
        XCTAssertEqual(shown(store), ["release"])
        await store.send(\.binding.draft, "/rel")
        XCTAssertEqual(shown(store), ["release"], "the name it was closed on, typed again, shows the list")
    }

    func testEscapeThenClearingAndTypingASlashOpensTheList() async throws {
        let daemon = skilled("release")
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.slashClosed)
        await store.send(\.binding.draft, "")
        await store.send(\.binding.draft, "/")
        XCTAssertEqual(shown(store), ["release"])
    }

    func testANewNameHighlightsItsBestMatchAndTheSameListKeepsTheHighlight() async throws {
        let daemon = skilled("release", "review", "rename")
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/re")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.slashMoved(by: 1))
        let highlighted = store.state.slashChoices[store.state.slashSelected]
        await store.send(\.binding.draft, "/r")
        XCTAssertEqual(store.state.slashChoices[store.state.slashSelected], highlighted, "same list, same highlight")

        await store.send(\.binding.draft, "/rel")
        XCTAssertEqual(store.state.slashChoices.map(\.id), ["release"])
        XCTAssertEqual(store.state.slashSelected, 0)

        await store.send(\.binding.draft, "/re")
        XCTAssertEqual(store.state.slashSelected, 0, "widening starts again at the best match")
    }

    func testDeletingTheSlashAndTypingItAgainShowsEveryChoiceWithNothingStale() async throws {
        let daemon = skilled("release", "review")
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(\.binding.draft, "/r")
        await store.send(\.binding.draft, "/re")
        await store.send(\.binding.draft, "/")
        await store.send(\.binding.draft, "")
        XCTAssertFalse(store.state.slashOpen)
        await store.send(\.binding.draft, "/")
        XCTAssertEqual(shown(store), ["release", "review"])
        XCTAssertFalse(store.state.slashLoading)
        XCTAssertNil(store.state.slashFailure)
    }

    func testClearingANameAndTypingAnotherShowsItsMatches() async throws {
        let daemon = skilled("release", "review")
        let store = store(daemon, try await started(daemon))

        // The answer may come while typing goes on; a store that isn't
        // exhaustive skips it then, so wait for the look to end instead.
        for draft in ["/", "/r", "/re", "/rel"] { await store.send(\.binding.draft, draft) }
        await store.finish(timeout: .seconds(1))
        for draft in ["/re", "/r", "/", "", "/", "/r", "/re", "/rev"] { await store.send(\.binding.draft, draft) }
        XCTAssertEqual(shown(store), ["review"])
    }

    func testDeletingBackIntoAPickedNameShowsTheListAgain() async throws {
        let daemon = skilled("release", "review")
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/rel")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.slashPicked("release"))
        XCTAssertEqual(store.state.draft, "/release ")
        XCTAssertFalse(store.state.slashOpen)

        await store.send(\.binding.draft, "/release")
        XCTAssertEqual(shown(store), ["release"])
        await store.send(\.binding.draft, "/re")
        XCTAssertEqual(Set(shown(store)), ["release", "review"])
    }

    func testArgumentsCloseTheListAndDeletingThemOpensIt() async throws {
        let daemon = skilled("release")
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/rel")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.slashPicked("release"))
        await store.send(\.binding.draft, "/release foo")
        XCTAssertFalse(store.state.slashOpen)
        await store.send(\.binding.draft, "/release ")
        XCTAssertFalse(store.state.slashOpen)
        await store.send(\.binding.draft, "/release")
        XCTAssertEqual(shown(store), ["release"])
    }

    func testOnlyASlashFirstAndANameOpensTheList() async throws {
        let daemon = skilled("release")
        let store = store(daemon, try await started(daemon))

        for draft in ["hello /x", "hello /", "/\n", "/ ", " /rel"] {
            await store.send(\.binding.draft, draft)
            XCTAssertFalse(store.state.slashOpen, draft)
            XCTAssertEqual(store.state.slashChoices, [], draft)
        }
    }

    func testAFailedLookIsShownAndTypingASlashAgainLooksAgain() async throws {
        let daemon = skilled("release")
        daemon.setSkillsFail(true)
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertTrue(store.state.slashOpen)
        XCTAssertNotNil(store.state.slashFailure)
        XCTAssertFalse(store.state.slashLoading)

        daemon.setSkillsFail(false)
        await store.send(\.binding.draft, "/r")
        await store.finish(timeout: .seconds(1))
        XCTAssertNotNil(store.state.slashFailure, "typing on in the list doesn't look again on every key")

        await store.send(\.binding.draft, "")
        await store.send(\.binding.draft, "/")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertNil(store.state.slashFailure)
        XCTAssertFalse(store.state.slashLoading)
        XCTAssertEqual(shown(store), ["release"])
    }

    func testAnAnswerArrivingAfterTheSlashIsDeletedDoesNotOpenTheList() async throws {
        let daemon = skilled("release")
        let store = store(daemon, try await started(daemon))

        // The answer comes before or after the delete; either way it
        // mustn't open the list.
        await store.send(\.binding.draft, "/")
        await store.send(\.binding.draft, "")
        await store.finish(timeout: .seconds(1))
        XCTAssertFalse(store.state.slashOpen)
        XCTAssertEqual(store.state.slashChoices, [])
        XCTAssertFalse(store.state.slashLoading)

        await store.send(\.binding.draft, "hi")
        XCTAssertFalse(store.state.slashOpen)
        await store.send(\.binding.draft, "/")
        XCTAssertEqual(shown(store), ["release"], "what was found is kept for the next /")
    }

    func testReturnWithNoMatchesSendsTheMessage() async throws {
        let daemon = skilled("release")
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/zzz")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertEqual(store.state.slashChoices, [])
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.delegate.send)
        XCTAssertEqual(store.state.draft, "")
        XCTAssertFalse(store.state.slashOpen)

        await store.send(\.binding.draft, "/")
        XCTAssertEqual(shown(store), ["release"])
    }

    func testTextPutInTheBoxOtherThanByTypingHidesTheList() async throws {
        let daemon = skilled("release")
        let store = store(daemon, try await started(daemon))

        await store.send(\.binding.draft, "/rel")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        await store.send(.quoted("a line"))
        XCTAssertFalse(store.state.slashOpen)
        XCTAssertEqual(store.state.slashChoices, [])
        await store.send(.sendTapped(.enqueue))
        await store.receive(\.delegate.send)
        XCTAssertEqual(store.state.draft, "", "Return sends what's there instead of picking from an old list")
    }

    func testADraftKeptFromBeforeShowsNoListUntilTyping() async throws {
        let daemon = skilled("release")
        let store = store(daemon, try await started(daemon), draft: "/rel")

        XCTAssertEqual(store.state.draft, "/rel")
        XCTAssertFalse(store.state.slashOpen)
        await store.send(\.binding.draft, "/re")
        await store.receive(\.slashLoaded, timeout: .seconds(1))
        XCTAssertEqual(shown(store), ["release"])
    }
}
