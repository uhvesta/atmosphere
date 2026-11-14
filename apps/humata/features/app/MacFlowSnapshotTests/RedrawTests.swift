import ComposableArchitecture
import HumataAppFeature
import HumataDaemonClient
import HumataProjects
import HumataSession
import SwiftUI
import XCTest

/// A change draws again only the views that show it: typing leaves the
/// chat alone, a reply's words as they stream draw only the reply being
/// written, and a read of the sidebar's list that brings nothing new to
/// show draws no row. Checked by how many times each view was drawn, which
/// the views count in debug builds.
@MainActor
final class RedrawTests: XCTestCase {
    func testTypingDrawsTheBoxAndNotTheChat() async throws {
        #if DEBUG
            var maker = ChatMaker(seed: 11)
            let (store, window, task) = try await opened(maker.history(turns: 200))
            defer {
                store.send(.binding(.set(\.draft, "")))
                task.cancel()
                window.close()
                window.contentViewController = nil
            }
            await quiet(window) { SessionBodyRuns.counts }

            let before = SessionBodyRuns.counts
            var draft = ""
            // Fifty keys, one at a time.
            for character in "Now run the parser's tests again and say who fails" {
                draft.append(character)
                store.send(.binding(.set(\.draft, draft)))
                await drawn(window)
            }
            await settle(window)
            let drew = Self.drew(since: before, SessionBodyRuns.counts)

            XCTAssertGreaterThan(drew(.composer), 0, "the box wasn't drawn as it was typed in")
            XCTAssertEqual(drew(.chatList), 0, "typing drew the chat's list again")
            XCTAssertEqual(drew(.chatRow), 0, "typing drew rows of the chat again")
            XCTAssertEqual(drew(.turnInProgress), 0, "typing drew the end of the chat again")
            XCTAssertEqual(drew(.header), 0, "typing drew the header again")
            XCTAssertEqual(drew(.session), 0, "typing drew the whole session again")
        #else
            throw XCTSkip("Views count how often they're drawn only in debug builds.")
        #endif
    }

    func testAStreamedReplyDrawsOnlyTheReplyBeingWritten() async throws {
        #if DEBUG
            var maker = ChatMaker(seed: 12)
            var history = maker.history(turns: 200)
            history.append(Self.event("asked", .userMessage("Write it all out.")))
            history.append(Self.event("started", .turnStarted))
            let (store, window, task) = try await opened(history)
            defer {
                task.cancel()
                window.close()
                window.contentViewController = nil
            }
            // The reply gets going, which takes the place of the note that
            // the agent is working.
            store.send(.received(Self.event("d-first", .assistantDelta("Here it is: "))))
            await quiet(window) { SessionBodyRuns.counts }
            XCTAssertFalse(store.streaming.isEmpty, "the reply didn't stream")

            let before = SessionBodyRuns.counts
            for word in 0..<60 {
                store.send(.received(Self.event("d\(word)", .assistantDelta("word\(word) "))))
                await drawn(window)
            }
            await settle(window)
            let streamed = Self.drew(since: before, SessionBodyRuns.counts)

            XCTAssertGreaterThan(streamed(.turnInProgress), 0, "the reply wasn't drawn as it streamed")
            XCTAssertEqual(streamed(.chatList), 0, "a streamed word drew the chat's list again")
            XCTAssertEqual(streamed(.chatRow), 0, "a streamed word drew finished rows again")
            XCTAssertEqual(streamed(.composer), 0, "a streamed word drew the box again")
            XCTAssertEqual(streamed(.header), 0, "a streamed word drew the header again")
            XCTAssertEqual(streamed(.session), 0, "a streamed word drew the whole session again")

            // The whole reply arrives: it becomes a row of the chat, so the
            // list is drawn, and the box, which no longer offers to steer.
            let whole = SessionBodyRuns.counts
            store.send(.received(Self.event("said", .assistantMessage(store.streaming))))
            store.send(.received(Self.event("ended", .turnEnded(aborted: false))))
            await settle(window)
            let landed = Self.drew(since: whole, SessionBodyRuns.counts)
            XCTAssertGreaterThan(landed(.chatList), 0, "the finished reply didn't reach the chat's list")
            XCTAssertEqual(landed(.session), 0, "a reply finishing drew the whole session again")
        #else
            throw XCTSkip("Views count how often they're drawn only in debug builds.")
        #endif
    }

    func testAReadOfTheSidebarWithNothingNewToShowDrawsNoRow() async throws {
        #if DEBUG
            let minute = Date(timeIntervalSince1970: 1_794_000_600)
            let project = Project(id: "p1", name: "Parser", repositories: [], created: minute)
            // What a read brings `seconds` later: the session at work is
            // active again and has said more.
            func listed(after seconds: TimeInterval, renamed: String? = nil) -> [Session] {
                (0..<12).map { index in
                    var session = Session(
                        id: "s\(index)", name: "Session \(index)", backend: .scripted, workingDirectory: "/work",
                        projectID: index < 8 ? project.id : nil,
                        lastActive: minute.addingTimeInterval(TimeInterval(-600 * index)))
                    if index == 0 {
                        session.state = .working
                        session.lastActive = minute.addingTimeInterval(seconds)
                        session.lastMessage = "Said \(seconds) seconds in."
                    }
                    if index == 3, let renamed { session.name = renamed }
                    return session
                }
            }
            // Its earlier reply waits in the inbox, as active as it is.
            func inbox(_ sessions: [Session]) -> [InboxEntry] {
                var session = sessions[0]
                session.lastMessage = "Done."
                return [InboxEntry(session: session, reason: .reply, projectName: project.name)]
            }
            let store = Store(initialState: ProjectsFeature.State()) {
                ProjectsFeature()
            } withDependencies: {
                $0.date = .constant(minute.addingTimeInterval(30))
                $0.daemon = .previewValue
                $0.notifications = .previewValue
            }
            let window = Self.window(
                NavigationSplitView {
                    ProjectsView(store: store)
                } detail: {
                    Text("No Session Open")
                })
            defer {
                window.close()
                window.contentViewController = nil
            }
            let first = listed(after: 1)
            store.send(.loaded(projects: [project], sessions: first, inbox: inbox(first)))
            await quiet(window) { SidebarBodyRuns.counts }
            XCTAssertGreaterThan(SidebarBodyRuns.counts[.sessionRow, default: 0], 0, "no session's row was drawn")

            // Reads every two seconds, with the session at work the whole
            // time: nothing the sidebar shows has changed.
            var before = SidebarBodyRuns.counts
            for read in 1...5 {
                let sessions = listed(after: 1 + TimeInterval(2 * read))
                store.send(.loaded(projects: [project], sessions: sessions, inbox: inbox(sessions)))
                await drawn(window)
            }
            await settle(window)
            let unchanged = Self.drew(since: before, SidebarBodyRuns.counts)
            XCTAssertEqual(unchanged(.sessionRow), 0, "a read with nothing new to show drew rows again")
            XCTAssertEqual(unchanged(.list), 0, "a read with nothing new to show drew the list again")
            XCTAssertEqual(unchanged(.inbox), 0, "a read with nothing new to show drew the inbox's row again")
            XCTAssertEqual(unchanged(.sidebar), 0, "a read with nothing new to show drew the sidebar again")

            // One session renamed: its row, and no other.
            before = SidebarBodyRuns.counts
            let renamed = listed(after: 11, renamed: "Renamed")
            store.send(.loaded(projects: [project], sessions: renamed, inbox: inbox(renamed)))
            await settle(window)
            let one = Self.drew(since: before, SidebarBodyRuns.counts)
            XCTAssertEqual(one(.sessionRow), 1, "a session renamed didn't draw its row alone")

            // Another session picked: the rows are as they were.
            before = SidebarBodyRuns.counts
            store.send(.selected(.session("s5")))
            await settle(window)
            let picked = Self.drew(since: before, SidebarBodyRuns.counts)
            XCTAssertEqual(store.selection, .session("s5"))
            XCTAssertEqual(picked(.sessionRow), 0, "picking a session drew rows again")
            XCTAssertEqual(picked(.list), 0, "picking a session worked the list out again")
        #else
            throw XCTSkip("Views count how often they're drawn only in debug builds.")
        #endif
    }

    /// How many more times each view was drawn in `after` than in `before`.
    private static func drew<Body: Hashable>(since before: [Body: Int], _ after: [Body: Int]) -> (Body) -> Int {
        { after[$0, default: 0] - before[$0, default: 0] }
    }

    private static func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    /// `view` in a window under every other one, at the app's size.
    private static func window(_ view: some View) -> NSWindow {
        let window = NSWindow(
            contentRect: CGRect(x: 80, y: 80, width: 1180, height: 760),
            styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopIconWindow)) + 1)
        let hosting = NSHostingController(rootView: view)
        // The window keeps its size, whatever its content asks for.
        hosting.sizingOptions = []
        window.contentViewController = hosting
        window.setContentSize(CGSize(width: 1180, height: 760))
        window.orderFrontRegardless()
        return window
    }

    /// The chat of `history` in a window, opened and at its end.
    private func opened(_ history: [SessionEvent]) async throws -> (StoreOf<SessionFeature>, NSWindow, StoreTask) {
        let session = Session(id: "redraw", name: "long", backend: .scripted, workingDirectory: "/work")
        let store = Store(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = .previewValue
            $0.daemon.watch = { _, _ in
                AsyncThrowingStream { continuation in
                    for event in history { continuation.yield(.event(event)) }
                    continuation.yield(.historyLoaded)
                }
            }
            $0.daemon.markSeen = { _, _ in }
        }
        let window = Self.window(SessionView(store: store))
        let task = store.send(.task)
        let clock = ContinuousClock()
        let start = clock.now
        while !store.historyLoaded || store.items.isEmpty, clock.now - start < .seconds(60) {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(store.historyLoaded, "the chat didn't open")
        await settle(window)
        return (store, window, task)
    }

    /// Lets what a change queued be laid out and drawn.
    private func drawn(_ window: NSWindow) async {
        window.contentView?.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        try? await Task.sleep(for: .milliseconds(10))
    }

    /// Lets queued layout and drawing finish.
    private func settle(_ window: NSWindow) async {
        for _ in 0..<20 {
            window.contentView?.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    /// Waits until nothing more is drawn: what opening the window started,
    /// such as rows measured as they come, has finished.
    private func quiet<Counts: Equatable>(_ window: NSWindow, _ counts: () -> Counts) async {
        for _ in 0..<20 {
            let before = counts()
            await settle(window)
            if counts() == before { return }
        }
    }
}
