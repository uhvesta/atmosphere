import ComposableArchitecture
import HumataDaemonClient
import XCTest

@testable import HumataSession

/// The subagents a session's agent starts: their steps stay out of the
/// session's chat, each has a chat of its own, and the list says how each
/// is going.
@MainActor
final class SubagentTests: XCTestCase {
    private func event(
        _ id: String, _ kind: SessionEvent.Kind, under call: String? = nil, at seconds: Double = 0
    ) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: seconds), kind: kind, parentCallID: call)
    }

    private func state(_ backend: Backend = .scripted) -> SessionFeature.State {
        SessionFeature.State(session: Session(id: "s1", name: "first", backend: backend, workingDirectory: "/work"))
    }

    /// A turn that starts two subagents: one finishes, the other is still
    /// going at the end of the history.
    private var history: [SessionEvent] {
        let explore = Subagent(agentType: "Explore", description: "Find the parser", status: .running)
        let tester = Subagent(agentType: "general-purpose", description: "Run the tests", status: .running)
        let search = #"{"command":"rg parser"}"#
        return [
            event("e1", .userMessage("find the parser and the tests")),
            event("e2", .turnStarted),
            event("e3", .toolStarted(callID: "a", tool: "Agent", argumentsJSON: "{}")),
            event("e4", .subagent(explore), under: "a"),
            event("e5", .toolStarted(callID: "b", tool: "Agent", argumentsJSON: "{}")),
            event("e6", .subagent(tester), under: "b"),
            event("e7", .toolStarted(callID: "a1", tool: "shell", argumentsJSON: search), under: "a", at: 1),
            event("e8", .assistantDelta("It's in"), under: "a", at: 2),
            event("e9", .toolCompleted(callID: "a1", success: true, output: "src/parser.rs"), under: "a", at: 2),
            event("e10", .assistantMessage("It's in src/parser.rs"), under: "a", at: 3),
            event(
                "e11", .usage(model: "small", inputTokens: 100, outputTokens: 20, premiumRequests: 0), under: "a",
                at: 3),
            event("e12", .reasoning("Which tests?"), under: "b", at: 3),
            event(
                "e13",
                .subagent(
                    Subagent(
                        agentType: "Explore", description: "Find the parser", status: .completed, totalTokens: 1500,
                        toolUses: 1, duration: .seconds(4))),
                under: "a", at: 4),
            event("e14", .toolCompleted(callID: "a", success: true, output: "src/parser.rs"), at: 4),
            event("e15", .usage(model: "big", inputTokens: 10, outputTokens: 2, premiumRequests: 0), at: 5),
        ]
    }

    private func texts(_ items: [ChatItem]) -> [String] {
        items.compactMap { item in
            switch item {
            case .person(_, let text, _), .reply(_, let text), .thinking(_, let text): text
            case .tool(let call): call.subject ?? call.tool
            default: nil
            }
        }
    }

    func testASubagentsStepsStayOutOfTheSessionsChat() {
        var state = state()
        state.receive(history)
        XCTAssertEqual(texts(state.items), ["find the parser and the tests", "Agent", "Agent"])
        XCTAssertEqual(state.streaming, "")
    }

    func testAnAgentCallKeepsItsRawResultOnceItsRowShowsTheSubagent() {
        var state = state()
        state.receive(history)
        let results = state.items.compactMap { item -> String?? in
            guard case .tool(let call) = item, state.subagents[id: call.callID] != nil else { return nil }
            return .some(call.output)
        }
        // The first returned; the second still runs, so has nothing to show.
        XCTAssertEqual(results, ["src/parser.rs", nil])
    }

    func testASubagentsWordsLeaveTheReplyBeingWrittenAlone() {
        var state = state()
        state.receive(Array(history.prefix(6)))
        state.receive(event("d1", .assistantDelta("Looking")))
        state.receive(event("d2", .assistantDelta("its words"), under: "a"))
        state.receive(event("m1", .assistantMessage("its message"), under: "a"))
        XCTAssertEqual(state.streaming, "Looking")
    }

    func testTheListShowsEachSubagentAsLastReported() {
        var state = state()
        state.receive(history)
        XCTAssertEqual(state.subagents.map(\.id), ["a", "b"])
        let found = state.subagents[id: "a"]
        XCTAssertEqual(found?.agentType, "Explore")
        XCTAssertEqual(found?.description, "Find the parser")
        XCTAssertEqual(found?.status, .completed)
        XCTAssertEqual(found?.tokens, 1500)
        XCTAssertEqual(found?.toolUses, 1)
        XCTAssertEqual(found?.duration, .seconds(4))
        let running = state.subagents[id: "b"]
        XCTAssertEqual(running?.status, .running)
        XCTAssertNil(running?.duration)
    }

    func testASubagentThatReportsNoTotalsShowsWhatItsStepsAddUpTo() {
        var state = state()
        state.receive(Array(history.prefix(11)))
        let found = state.subagents[id: "a"]
        XCTAssertEqual(found?.tokens, 120)
        XCTAssertEqual(found?.toolUses, 1)
        XCTAssertEqual(found?.latestStep, "It's in src/parser.rs")
    }

    func testASubagentStillRunningWhenItsTurnEndsReadsAsStopped() {
        var state = state()
        state.receive(history)
        state.receive(event("e16", .turnEnded(aborted: true), at: 9))
        XCTAssertEqual(state.subagents[id: "b"]?.status, .stopped)
        XCTAssertEqual(state.subagents[id: "b"]?.duration, .seconds(3))
        XCTAssertEqual(state.subagents[id: "a"]?.status, .completed)
    }

    func testEventsOneAtATimeAddUpAsTheWholeHistoryDoes() {
        let history = history + [event("e16", .turnEnded(aborted: false), at: 9)]
        var each = state()
        for (index, event) in history.enumerated() {
            each.receive(event)
            var upTo = state()
            upTo.events = IdentifiedArray(uniqueElements: history.prefix(index + 1))
            XCTAssertEqual(each.subagents, upTo.subagents, "after \(event.id)")
            XCTAssertEqual(each.items, upTo.items, "after \(event.id)")
            XCTAssertEqual(each.usage, upTo.usage, "after \(event.id)")
        }
    }

    func testASubagentsUsageCountsOnceInTheSessionsTotal() {
        // Every backend counts a subagent apart from its main agent.
        for backend in [Backend.copilot, .claude] {
            var state = state(backend)
            state.receive(history)
            XCTAssertEqual(state.usage, Usage(model: "big", inputTokens: 110, outputTokens: 22), "\(backend)")
            XCTAssertEqual(state.subagentUsage, Usage(model: "small", inputTokens: 100, outputTokens: 20), "\(backend)")
        }
    }

    func testOpeningASubagentShowsItsOwnChat() async {
        var initial = state()
        initial.receive(history)
        let store = TestStore(initialState: initial) { SessionFeature() }
        XCTAssertEqual(store.state.subagentItems, [])

        await store.send(.subagentTapped(callID: "a")) { $0.shownSubagent = "a" }
        XCTAssertEqual(texts(store.state.subagentItems), ["rg parser", "It's in src/parser.rs"])
        await store.send(.subagentTapped(callID: "b")) { $0.shownSubagent = "b" }
        XCTAssertEqual(texts(store.state.subagentItems), ["Which tests?"])
        // A call that started no subagent opens nothing.
        await store.send(.subagentTapped(callID: "a1"))
        await store.send(.subagentClosed) { $0.shownSubagent = nil }
    }

    func testAScriptedSubagentShowsUnderItsCallWithItsOwnChat() async throws {
        let daemon = InMemoryDaemon { _ in
            [
                .delegate(
                    type: "Explore", description: "Find the parser",
                    steps: [.run(command: "rg parser", output: "src/parser.rs"), .say("It's in src/parser.rs")],
                    result: "src/parser.rs", tokens: 1200),
                .say("Found it."),
            ]
        }
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", name: "first"))
        try await daemon.client.send(session.id, "where's the parser?", .enqueue)
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off

        let watching = await store.send(.task)
        await store.receive(\.receivedAll, timeout: .seconds(1))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(texts(store.state.items), ["where's the parser?", "Find the parser", "Found it."])
        let subagent = try XCTUnwrap(store.state.subagents.first)
        XCTAssertEqual(store.state.subagents.count, 1)
        XCTAssertEqual(subagent.agentType, "Explore")
        XCTAssertEqual(subagent.status, .completed)
        XCTAssertEqual(subagent.tokens, 1200)
        XCTAssertEqual(subagent.toolUses, 1)

        await store.send(.subagentTapped(callID: subagent.id))
        XCTAssertEqual(texts(store.state.subagentItems), ["rg parser", "It's in src/parser.rs"])
        await watching.cancel()
    }
}
