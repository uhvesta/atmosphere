import ComposableArchitecture
import FlowKit
import HumataAppFeature
import HumataBoard
import HumataDaemonClient
import HumataProjects
import HumataReview
import HumataSearch
import HumataSession
import SwiftUI
import XCTest

/// Humata's window over an in-memory daemon. The Mac flow drives the store,
/// since it can't click.
@MainActor
final class MacFlowSnapshotTests: XCTestCase {
    /// What a flow waited on that never came, by its line.
    private struct TimedOut: Error {
        let line: UInt
    }

    /// An agent edits a file in a project: the edit waits for the person,
    /// is approved, and a command then waits until the session allows
    /// everything.
    func testAnEditThroughAnApproval() async throws {
        let daemon = InMemoryDaemon { _ in
            [
                .think("The notes go in the repository."),
                .write(path: "/code/humata/NOTES.md", contents: "Ship M1."),
                .run(command: "git status", output: "nothing to commit, working tree clean"),
                .say("Wrote the notes and checked the tree is clean."),
            ]
        }
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Release notes", projectID: project.id))
        // The sidebar is read again only when the flow asks, so a refresh
        // never lands between a step settling and its capture.
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("edit", AppView(store: store))

        try await until { store.sidebar.sessions.count == 1 && store.daemon == .running }
        await flow.step("sidebar")

        store.send(.sidebar(.selected(.session(session.id))))
        try await until { store.session != nil }
        store.send(.session(.presented(.binding(.set(\.draft, "Write the release notes for M1")))))
        store.send(.session(.presented(.sendTapped(.enqueue))))
        try await until { store.session?.isWaiting == true }
        try await sidebarShows(.waitingForApproval, store, session)
        await flow.step("waiting")

        guard case .tool(let write)? = store.session?.items.last, let request = write.permission?.requestID else {
            return XCTFail("the write isn't waiting")
        }
        store.send(.session(.presented(.answerPermission(requestID: request, approved: true))))
        try await until { store.session?.isWaiting == true && daemon.files(of: session.id).isEmpty == false }
        try await sidebarShows(.waitingForApproval, store, session)
        await flow.step("approved")

        store.send(.session(.presented(.approvalModePicked(.allowEverything))))
        try await until { store.session?.isWorking == false }
        try await sidebarShows(.idle, store, session)
        await flow.step("done")
        flow.finish()
    }

    /// A session is forked before its second turn: the fork shows under it
    /// in the sidebar with the chat up to there, and goes its own way.
    func testAForkBeforeATurn() async throws {
        let daemon = InMemoryDaemon { text in
            [.write(path: "/\(text).md", contents: text), .say("Wrote \(text).md.")]
        }
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(
                backend: .scripted, name: "Release notes", approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "notes", .enqueue)
        try await daemon.client.send(session.id, "changelog", .enqueue)
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("fork", AppView(store: store))

        try await until { store.sidebar.sessions.count == 1 && store.daemon == .running }
        store.send(.sidebar(.selected(.session(session.id))))
        try await until { store.session?.items.count == 6 }
        await flow.step("session")

        let second = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("changelog") })
        store.send(.session(.presented(.forkTapped(before: second.id, chatOnly: false))))
        try await until { store.sidebar.sessions.count == 2 && store.session?.session.id != session.id }
        let fork = try XCTUnwrap(store.session?.session)
        try await until { store.session?.items.count == 3 }
        XCTAssertEqual(daemon.files(of: fork.id), ["/notes.md": "notes"])
        await flow.step("forked")

        store.send(.session(.presented(.binding(.set(\.draft, "readme")))))
        store.send(.session(.presented(.sendTapped(.enqueue))))
        try await until { store.session?.items.count == 6 }
        XCTAssertEqual(daemon.files(of: fork.id), ["/notes.md": "notes", "/readme.md": "readme"])
        XCTAssertEqual(daemon.files(of: session.id), ["/notes.md": "notes", "/changelog.md": "changelog"])
        await flow.step("went its own way")
        flow.finish()
    }

    /// A session's changes reviewed: a comment on a changed line is sent as
    /// a review, the agent answers in the thread and edits the line, and its
    /// fix is shown. A Markdown file reads rendered, commented on by block.
    func testAReview() async throws {
        let notes = "# Sum\n\nAdds two numbers, `a` and `b`.\n\nCall it from the command line.\n"
        let daemon = InMemoryDaemon { text in
            if text.contains("[thread 1]") {
                return [
                    .write(
                        path: "sum.py",
                        contents: "def total(a, b):\n    \"\"\"Adds a and b.\"\"\"\n    return a + b\n"),
                    .say("[thread 1]\nRenamed it to `total` and said what it does."),
                ]
            }
            return [
                .write(path: "sum.py", contents: "def add(a, b):\n    \"\"\"Adds a and b.\"\"\"\n    return a + b\n"),
                .write(path: "README.md", contents: notes),
                .say("Wrote sum.py and its README."),
            ]
        }
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Sum", approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "Write a sum function", .enqueue)
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("review", AppView(store: store))
        let sum = FileRef(repository: "code", path: "sum.py")

        try await until { store.sidebar.sessions.count == 1 && store.daemon == .running }
        store.send(.sidebar(.selected(.session(session.id))))
        try await until { store.session != nil }
        store.send(.session(.presented(.tabPicked(.review))))
        try await until { store.session?.review.files.count == 2 }
        store.send(.session(.presented(.review(.fileSelected(sum)))))
        await flow.step("changes")

        store.send(.session(.presented(.review(.lineTapped(1, extending: false)))))
        store.send(
            .session(.presented(.review(.binding(.set(\.comment, "Call it `total`; `add` reads like a verb."))))))
        await flow.step("commenting")

        store.send(.session(.presented(.review(.commentSaved))))
        try await until { store.session?.review.pending == 1 }
        await flow.step("commented")

        store.send(.session(.presented(.review(.sendTapped))))
        try await until { store.session?.review.threads.first?.comments.count == 2 }
        await flow.step("answered")

        let thread = try XCTUnwrap(store.session?.review.threads.first)
        store.send(.session(.presented(.review(.showFixTapped(thread.id)))))
        try await until { store.session?.review.range == .turn(2) && store.session?.review.loading == false }
        await flow.step("fix")

        store.send(.session(.presented(.review(.rangePicked(.session)))))
        try await until { store.session?.review.loading == false }
        store.send(.session(.presented(.review(.fileSelected(FileRef(repository: "code", path: "README.md"))))))
        store.send(.session(.presented(.review(.renderedToggled))))
        try await until { store.session?.review.whole?.path == "README.md" }
        store.send(
            .session(
                .presented(.review(.blockTapped(startLine: 3, endLine: 3, quote: "Adds two numbers, `a` and `b`.")))))
        store.send(.session(.presented(.review(.binding(.set(\.comment, "Say what it returns too."))))))
        await flow.step("markdown")
        flow.finish()
    }

    /// A chat in no project gets a folder of its own; the agent writes a
    /// plan there, read rendered in Files, commented on and sent back.
    func testAChatAndItsFiles() async throws {
        let plan = """
            # Reader plan

            Import a book or an article, then read it a page at a time.

            ## Steps

            Tap a word to hear it and see what it means. Translate a sentence by holding it.

            ```
            import → read → tap → chat
            ```
            """
        let daemon = InMemoryDaemon { text in
            if text.contains("Comments on the files") {
                return [
                    .write(path: "plan.md", contents: plan + "\nWords tapped are kept, to review later.\n"),
                    .write(path: "notes/ideas.txt", contents: "Offline first.\nSign in with iCloud, at most.\n"),
                    .say("Kept the tapped words in the plan, and the notes now say iCloud at most."),
                ]
            }
            return [
                .write(path: "plan.md", contents: plan),
                .write(path: "notes/ideas.txt", contents: "Offline first.\nNo accounts.\n"),
                .say("Wrote the plan in plan.md, with some notes."),
            ]
        }
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
            $0.uuid = .incrementing
        }
        let flow = FlowRecorder("chat", AppView(store: store))

        try await until { store.daemon == .running }
        store.send(.sidebar(.newChatTapped))
        store.send(.sidebar(.destination(.presented(.newSession(.binding(.set(\.name, "Reader plan")))))))
        store.send(.sidebar(.destination(.presented(.newSession(.binding(.set(\.backend, .scripted)))))))
        store.send(
            .sidebar(.destination(.presented(.newSession(.binding(.set(\.approvalMode, .allowEverything)))))))
        await flow.step("new chat")

        store.send(.sidebar(.destination(.presented(.newSession(.startTapped)))))
        try await until { store.session != nil }
        store.send(.session(.presented(.binding(.set(\.draft, "Write a plan for the reader app")))))
        store.send(.session(.presented(.sendTapped(.enqueue))))
        try await until { store.session?.isWorking == false && store.session?.items.count ?? 0 >= 2 }
        await flow.step("chat")

        store.send(.session(.presented(.tabPicked(.files))))
        try await until { store.session?.docs.lines != nil }
        await flow.step("files")

        store.send(
            .session(
                .presented(
                    .docs(
                        .blockTapped(
                            startLine: 7, endLine: 7,
                            quote: "Tap a word to hear it and see what it means.")))))
        store.send(.session(.presented(.docs(.binding(.set(\.comment, "Keep the words tapped, to review later."))))))
        await flow.step("commenting")

        store.send(.session(.presented(.docs(.commentSaved))))
        store.send(.session(.presented(.docs(.fileSelected("notes/ideas.txt")))))
        try await until { store.session?.docs.lines?.first == "Offline first." }
        store.send(.session(.presented(.docs(.lineTapped(2, extending: false)))))
        store.send(.session(.presented(.docs(.binding(.set(\.comment, "Sign in with iCloud, at most."))))))
        store.send(.session(.presented(.docs(.commentSaved))))
        await flow.step("drafts")

        store.send(.session(.presented(.docs(.sendTapped))))
        try await until { store.session?.docs.drafts.isEmpty == true }
        store.send(.session(.presented(.tabPicked(.chat))))
        try await until { store.session?.isWorking == false && store.session?.items.count ?? 0 >= 4 }
        await flow.step("sent")
        flow.finish()
    }

    /// A session on a subtask moves it and its task across the board, to
    /// Waiting on you when it asks for an approval, and shows in the inbox.
    func testTheBoard() async throws {
        let daemon = InMemoryDaemon { _ in
            [
                .write(path: "/code/humata/LoginForm.swift", contents: "struct LoginForm {}"),
                .say("Added the form."),
            ]
        }
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let spec = """
            People sign in with **email and a password**.

            - Show an error under the field that's wrong
            - Keep them signed in for 30 days
            """
        let task = try await daemon.client.createItem(
            project.id, ItemFields(title: "Sign in", spec: spec, priority: .high))
        let form = try await daemon.client.createItem(
            project.id, ItemFields(parentID: task.id, title: "Login form", spec: "The form itself."))
        _ = try await daemon.client.createItem(project.id, ItemFields(parentID: task.id, title: "Error copy"))
        _ = try await daemon.client.createItem(project.id, ItemFields(title: "Dark mode", priority: .low))
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("board", AppView(store: store))

        try await until { store.daemon == .running && store.sidebar.projects.count == 1 }
        store.send(.sidebar(.selected(.board(project.id))))
        try await until { store.board?.items.count == 4 }
        await flow.step("board")

        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Login form", itemID: form.id))
        try await daemon.client.send(session.id, "Build the form", .enqueue)
        try await until { daemon.sessions.first?.state == .waitingForApproval }
        store.send(.board(.presented(.refresh)))
        try await until { store.board?.items[id: task.id]?.status == .waitingOnYou }
        await flow.step("waiting on you")

        store.send(.board(.presented(.cardTapped(task.id))))
        try await until { store.board?.destination?.item != nil }
        await flow.step("task")

        store.send(.board(.presented(.destination(.presented(.item(.editTapped))))))
        let written =
            spec + """


                ## Done when

                1. A wrong password says so under the **password** field
                2. `signedIn` survives a relaunch
                """
        let draft = { (previewing: Bool) in
            _ = store.send(
                .board(
                    .presented(
                        .destination(
                            .presented(
                                .item(
                                    .binding(
                                        .set(
                                            \.draft,
                                            ItemFeature.Draft(title: "Sign in", spec: written, previewing: previewing))
                                    )))))))
        }
        draft(false)
        await flow.step("spec writing")
        draft(true)
        await flow.step("spec preview")
        store.send(.board(.presented(.destination(.presented(.item(.cancelEditTapped))))))

        store.send(.board(.presented(.destination(.dismiss))))
        store.send(.sidebar(.refresh))
        try await until { store.sidebar.inbox.count == 1 }
        store.send(.sidebar(.selected(.inbox)))
        try await until { store.sidebar.selection == .inbox }
        await flow.step("inbox")
        flow.finish()
    }

    /// Two hosts: one set up and working, one Humata isn't installed on
    /// yet; adding a third from the SSH config; the Mac's copies of a host's
    /// sessions, one of which it lost; and a session whose host went offline.
    func testHosts() async throws {
        let daemon = InMemoryDaemon(sshHosts: ["build-box", "gpu", "mini"])
        _ = try await daemon.client.addHost("build-box", "build-box")
        _ = try await daemon.client.installHost("build-box")
        _ = try await daemon.client.updateHost(
            "build-box",
            HostUpdate(
                setEnv: ["GH_TOKEN": "secret", "CARGO_HOME": "/opt/cargo"],
                setupScript: "sudo apt-get install -y ripgrep\ncurl -fsSL https://gh.io/copilot-install | bash\n"))
        _ = try await daemon.client.addHost("gpu", "me@10.0.0.7")
        _ = try await daemon.client.updateHost("", HostUpdate(setEnv: ["ANTHROPIC_API_KEY": "secret"]))
        daemon.addSkipped(
            SkippedSession(
                backend: .claude, id: "5e1f2a7c", folder: "~/.humata/claude/sessions/sessions/5e1f2a7c",
                reason:
                    "the session record ~/.humata/claude/sessions/sessions/5e1f2a7c/session.json is damaged, "
                    + "so the session can't be opened. Removing its folder loses the session: "
                    + "expected value at line 1 column 3"))
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .copilot, name: "Profile the build", host: "build-box", projectID: project.id))
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("hosts", AppView(store: store))

        try await until { store.daemon == .running && store.sidebar.sessions.count == 1 }
        store.send(.sidebar(.selected(.hosts)))
        try await until {
            if case .known(.reachable) = store.hosts?.health["build-box"],
                case .known(.unreachable) = store.hosts?.health["gpu"]
            {
                return true
            }
            return false
        }
        await flow.step("hosts")

        store.send(.hosts(.presented(.selected("gpu"))))
        try await until { store.hosts?.selection == "gpu" }
        await flow.step("not installed")

        store.send(.hosts(.presented(.selected(RemoteHost.coordinator(envNames: []).id))))
        try await until {
            store.hosts?.selected?.isCoordinator == true && store.hosts?.editor?.variables.count == 1
                && store.hosts?.skipped.count == 1
        }
        await flow.step("this mac")

        store.send(.hosts(.presented(.addTapped)))
        try await until { store.hosts?.destination?.add?.sshHosts.isEmpty == false }
        store.send(.hosts(.presented(.destination(.presented(.add(.sshHostPicked("mini")))))))
        try await until { store.hosts?.destination?.add?.destination == "mini" }
        await flow.step("add host")

        store.send(.hosts(.presented(.destination(.dismiss))))
        let lost = try await daemon.client.start(
            StartRequest(backend: .claude, name: "Bump the tokio version", host: "build-box", projectID: project.id))
        daemon.lose(session: lost.id)
        store.send(.hosts(.presented(.selected("build-box"))))
        try await until { store.hosts?.recoveries.count == 2 }
        await flow.step("recovery copies")

        store.send(.hosts(.presented(.restoreTapped(lost.id))))
        try await until { store.hosts?.recoveries.allSatisfy(\.onHost) == true }
        daemon.setOffline("build-box", true)
        store.send(.sidebar(.selected(.session(session.id))))
        store.send(.sidebar(.refresh))
        try await until { store.session?.session.hostOffline == true }
        await flow.step("offline session")
        flow.finish()
    }

    /// Search finds a message across sessions, then those with a related
    /// word; a hit opens its session at the reply, offered as a fork point.
    func testSearch() async throws {
        let daemon = InMemoryDaemon { text in
            [.say("The \(text) now run in a transaction, so a failed one leaves the schema as it was.")]
        }
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let first = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Fix the migrations", projectID: project.id))
        try await daemon.client.send(first.id, "migrations", .enqueue)
        try await daemon.client.send(first.id, "seeds", .enqueue)
        let second = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Tidy the schema", projectID: project.id))
        try await daemon.client.send(second.id, "schema checks", .enqueue)
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.relatedWords = RelatedWordsClient { _ in ["migrate", "schema"] }
            $0.calendar = Calendar(identifier: .gregorian)
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("search", AppView(store: store))

        try await until { store.daemon == .running && store.sidebar.sessions.count == 2 }
        store.send(.sidebar(.selected(.search)))
        try await until { store.search != nil }
        store.send(.search(.presented(.binding(.set(\.text, "migrations transaction")))))
        store.send(.search(.presented(.submitted)))
        try await until { store.search?.isSearching == false && store.search?.related.isEmpty == false }
        await flow.step("hits")

        let hit = try XCTUnwrap(store.search?.hits.first { !$0.related && $0.kind == .reply })
        store.send(.search(.presented(.hitTapped(hit))))
        try await until { store.session?.session.id == first.id && store.session?.highlighted == hit.eventID }
        try await until { store.session?.items.isEmpty == false }
        await flow.step("opened at the reply")
        flow.finish()
    }

    /// A session's terminal opens in its folder with ⌘T and runs what's
    /// typed; the host's Terminals list it beside one opened outside any
    /// session.
    func testTerminals() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.addHost("build-box", "build-box")
        _ = try await daemon.client.installHost("build-box")
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .claude, name: "Profile the build", host: "build-box", projectID: project.id))
        let logs = try await daemon.client.openTerminal(HostPlace(host: "build-box", folder: "/var/log"), .standard)
        try await daemon.client.typeInTerminal(logs, Data("echo tailing the build logs\r".utf8))
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("terminals", AppView(store: store))

        try await until { store.daemon == .running && store.sidebar.sessions.count == 1 }
        store.send(.sidebar(.selected(.session(session.id))))
        try await until { store.session != nil }
        store.send(.session(.presented(.newTerminalTapped)))
        try await until { store.session?.terminals.current != nil }
        let terminal = try XCTUnwrap(store.session?.terminals.selected)
        for line in ["pwd", "echo hello from build-box", "cargo --version"] {
            store.send(
                .session(
                    .presented(.terminals(.terminals(.element(id: terminal, action: .typed(Data((line + "\r").utf8))))))
                ))
        }
        try await until {
            String(bytes: store.session?.terminals.current?.screen.data ?? Data(), encoding: .utf8)?
                .contains("command not found") == true
        }
        await flow.step("session terminal")

        store.send(.sidebar(.selected(.hosts)))
        try await until { store.hosts != nil }
        store.send(.hosts(.presented(.selected("build-box"))))
        store.send(.hosts(.presented(.paneSelected(.terminals))))
        try await until { store.hosts?.terminals?.tabs.count == 2 }
        store.send(.hosts(.presented(.terminals(.selected(logs.id)))))
        try await until {
            String(bytes: store.hosts?.terminals?.current?.screen.data ?? Data(), encoding: .utf8)?
                .contains("tailing") == true
        }
        await flow.step("host terminals")

        daemon.addFile(
            "/home/me/notes.md", Data("# Build box\n\nCaches live in `~/.cache/bazel`.\n".utf8), on: "build-box")
        daemon.addFile(
            "/home/me/scripts/warm.sh", Data("#!/bin/sh\n# Warms the cache.\nbazel build //...\n".utf8),
            on: "build-box")
        daemon.addFile("/home/me/.bashrc", Data("export PATH\n".utf8), on: "build-box")
        store.send(.hosts(.presented(.paneSelected(.files))))
        try await until { store.hosts?.files?.root != nil }
        store.send(.hosts(.presented(.files(.folderToggled("scripts")))))
        try await until { store.hosts?.files?.folders["scripts"] != nil }
        store.send(.hosts(.presented(.files(.fileSelected("scripts/warm.sh")))))
        try await until { store.hosts?.files?.contents != nil }
        await flow.step("host files")

        daemon.listen(on: 5173, at: "build-box")
        daemon.listen(on: 8080, at: "build-box")
        store.send(.hosts(.presented(.paneSelected(.ports))))
        for port in ["5173", "8080"] {
            store.send(.hosts(.presented(.ports(.binding(.set(\.port, port))))))
            store.send(.hosts(.presented(.ports(.forwardTapped))))
            try await until { store.hosts?.ports?.forwarding == false && store.hosts?.ports?.port == "" }
        }
        try await until { store.hosts?.ports?.forwards.count == 2 }
        await flow.step("host ports")
        flow.finish()
    }

    /// Claude Code and Copilot sessions started outside Humata are listed,
    /// and one is brought in with its history, into a project.
    func testImportingASession() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let now = Date(timeIntervalSince1970: 1_794_000_600)
        let login = ImportableSession(
            backend: .claude, id: "0a1b-2c3d", workingDirectory: "/code/humata",
            summary: "Fix the login redirect after the session times out", updated: now - 300)
        daemon.addImportable(login)
        daemon.addImportable(
            ImportableSession(
                backend: .copilot, id: "7f8e-9d0c", workingDirectory: "/code/notes",
                summary: "Sort the meeting notes by date", updated: now - 7_200))
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(now)
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("import", AppView(store: store))

        try await until { store.daemon == .running && store.sidebar.projects.count == 1 }
        store.send(.sidebar(.importTapped))
        try await until { store.sidebar.destination?.importSession?.sessions?.count == 2 }
        store.send(.sidebar(.destination(.presented(.importSession(.binding(.set(\.projectID, project.id)))))))
        store.send(.sidebar(.destination(.presented(.importSession(.binding(.set(\.selection, login.id)))))))
        await flow.step("sessions")

        store.send(.sidebar(.destination(.presented(.importSession(.importTapped)))))
        try await until { store.session != nil && store.sidebar.destination == nil }
        await flow.step("imported")
        flow.finish()
    }

    /// An agent asks the person a question and, once done, to move its
    /// task: both wait in the inbox. The reply goes to the session; the
    /// person opens the changes, then approves the move.
    func testAnsweringAnAgentsRequests() async throws {
        let daemon = InMemoryDaemon { _ in
            [.write(path: "/code/humata/Login.swift", contents: "let signIn = \"email\"\n"), .say("Done.")]
        }
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "Sign in"))
        let session = try await daemon.client.start(
            StartRequest(
                backend: .scripted, name: "Sign in", approvalMode: .allowEverything, projectID: project.id,
                itemID: task.id))
        try await daemon.client.send(session.id, "Add sign in", .enqueue)
        daemon.addRequest(
            session.id, .question, "Sign in with **email** or **phone**? I'd pick email: no SMS costs.")
        daemon.addRequest(
            session.id, .move(item: task.id, status: .inReview),
            "Added sign in by email. Check the redirect after the session times out.")
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("requests", AppView(store: store))

        try await until { store.daemon == .running && store.sidebar.inbox.count == 2 }
        store.send(.sidebar(.selected(.inbox)))
        try await until { store.session == nil }
        await flow.step("inbox")

        let question = try XCTUnwrap(store.sidebar.inbox.first?.request)
        store.send(.sidebar(.requestAnswered(question.id, .reply("Email."))))
        try await until { store.sidebar.inbox.count == 1 && store.sidebar.inbox.first?.reason == .move }
        await flow.step("replied")

        store.send(.sidebar(.changesTapped(session.id)))
        try await until { store.session?.tab == .review && store.session?.review.files.isEmpty == false }
        await flow.step("changes")

        let move = try XCTUnwrap(store.sidebar.inbox.first?.request)
        store.send(.sidebar(.requestAnswered(move.id, .approve)))
        try await until { store.sidebar.inbox.isEmpty }
        let status = try await daemon.client.items(project.id).first?.status
        XCTAssertEqual(status, .inReview)
        store.send(.sidebar(.selected(.inbox)))
        try await until { store.session == nil }
        await flow.step("approved")
        flow.finish()
    }

    /// Planning with an agent: it proposes tasks, which are applied or
    /// declined in its chat; a project's skills; and comments on a task's
    /// spec sent to its session.
    func testPlanningWithAnAgent() async throws {
        let daemon = InMemoryDaemon { _ in
            [
                .say("Here's the work as I'd split it."),
                .propose(title: "Login form", spec: "Email and password, with an error under the wrong field."),
                .propose(title: "Dark mode", spec: "Follow the system's appearance."),
                .propose(title: "Confetti", spec: "On every save."),
                .say("Three tasks. The confetti is optional."),
            ]
        }
        let project = try await daemon.client.createProject(
            "humata", [Repository(path: "/code/humata")], "Keep changes small.")
        _ = try await daemon.client.updateProject(
            project.id,
            ProjectUpdate(skills: [
                Skill(name: "Running the tests", text: "bazel test //..."),
                Skill(name: "Releasing", text: "Tag main, then run tools/release.sh."),
            ]))
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Plan", projectID: project.id, planning: true))
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.uuid = .incrementing
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("planning", AppView(store: store))

        try await until { store.daemon == .running && store.sidebar.projects.count == 1 }
        store.send(.sidebar(.projectSettingsTapped(project.id)))
        try await until { store.sidebar.destination?.projectSettings != nil }
        await flow.step("project settings")
        store.send(.sidebar(.destination(.dismiss)))

        store.send(.sidebar(.selected(.session(session.id))))
        try await until { store.session != nil }
        store.send(.session(.presented(.binding(.set(\.draft, "Plan sign in for the app")))))
        store.send(.session(.presented(.sendTapped(.enqueue))))
        try await until { store.session?.isWorking == false && store.session?.proposals.proposals.count == 3 }
        await flow.step("proposals")

        let proposals = try XCTUnwrap(store.session?.proposals.proposals)
        store.send(.session(.presented(.proposals(.applyTapped(proposals[0].id)))))
        store.send(.session(.presented(.proposals(.declineTapped(proposals[2].id)))))
        try await until { store.session?.proposals.deciding.isEmpty == true }
        try await until { store.session?.proposals.proposals[id: proposals[2].id]?.state == .declined }
        await flow.step("decided")

        let board = try await daemon.client.items(project.id)
        let login = try XCTUnwrap(board.first)
        let work = try await daemon.client.startWork(session.id, login.id)
        store.send(.sidebar(.selected(.board(project.id))))
        try await until { store.board?.items.count == 1 }
        store.send(.board(.presented(.cardTapped(login.id))))
        try await until { store.board?.destination?.item?.sessions.contains { $0.id == work.id } == true }
        let block = try XCTUnwrap(MarkdownBlock.blocks([login.spec]).first)
        let comments = { (action: SpecCommentsFeature.Action) in
            _ = store.send(.board(.presented(.destination(.presented(.item(.comments(action)))))))
        }
        comments(.blockTapped(block))
        comments(.binding(.set(\.comment, "Say which field in the error, too.")))
        comments(.commentSaved)
        comments(.blockTapped(block))
        comments(.binding(.set(\.comment, "And lock the account after five tries?")))
        try await until { store.board?.destination?.item?.comments.drafts.count == 1 }
        await flow.step("spec comments")
        flow.finish()
    }

    /// A task's session lands as a pull request; when it merges, the task
    /// is done; and GitHub's open issues come in as tasks.
    func testShippingAsPullRequests() async throws {
        let daemon = InMemoryDaemon { _ in
            [
                .write(path: "/code/humata/LoginForm.swift", contents: "struct LoginForm {}"),
                .say("Added the form."),
            ]
        }
        let project = try await daemon.client.createProject("humata", [Repository(path: "/code/humata")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "Sign in", priority: .high))
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, name: "Sign in", approvalMode: .allowEverything, itemID: task.id))
        try await daemon.client.send(session.id, "Build the form", .enqueue)
        let store = Store(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.date = .constant(Date(timeIntervalSince1970: 1_794_000_600))
            $0.daemon = daemon.client
            $0.daemonService = .previewValue
            $0.notifications = .previewValue
            $0.continuousClock = TestClock()
        }
        let flow = FlowRecorder("shipping", AppView(store: store))

        try await until { store.daemon == .running && store.sidebar.projects.count == 1 }
        store.send(.sidebar(.selected(.session(session.id))))
        try await until { store.session?.isWorking == false }
        store.send(.session(.presented(.landTapped)))
        try await until { store.session?.land != nil }
        await flow.step("land")

        store.send(.session(.presented(.land(.presented(.landTapped)))))
        try await until { store.session?.land?.landed != nil }
        await flow.step("landed")
        store.send(.session(.presented(.land(.dismiss))))

        try daemon.mergePullRequests(of: session.id)
        store.send(.sidebar(.selected(.board(project.id))))
        try await until { store.board?.items.count == 1 }
        store.send(.board(.presented(.cardTapped(task.id))))
        try await until { store.board?.destination?.item?.pullRequests.first?.state == .merged }
        await flow.step("merged")
        store.send(.board(.presented(.destination(.dismiss))))

        daemon.openIssues(
            [
                InMemoryDaemon.Issue(number: 31, title: "Crash when the password is empty"),
                InMemoryDaemon.Issue(number: 34, title: "Remember the last email used"),
            ], in: project.id)
        store.send(.board(.presented(.importIssuesTapped)))
        try await until { store.board?.items.count == 3 && store.board?.destination?.alert != nil }
        await flow.step("imported issues")
        flow.finish()
    }

    private func sidebarShows(
        _ state: SessionState, _ store: StoreOf<AppFeature>, _ session: Session
    ) async throws {
        store.send(.sidebar(.refresh))
        try await until { store.sidebar.sessions[id: session.id]?.state == state }
    }

    private func until(line: UInt = #line, _ condition: () -> Bool) async throws {
        for _ in 0..<500 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        throw TimedOut(line: line)
    }
}
