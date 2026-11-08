import ComposableArchitecture
import FlowKit
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrBugReport
import LifrConversations
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrModelsClient
import LifrRemindersClient
import LifrRoot
import LifrSearchClient
import LifrSessionClient
import LifrShortcuts
import LifrStoreClient
import LifrSyncClient
import LifrWidgetViews
import SwiftUI
import XCTest

/// The chat on the iPhone: a skill typed and commands, checkpoints, attachments, dark mode, its model, thinking,
/// failing, stopping, Markdown and compacting.
@MainActor
final class PhoneChatFlowTests: PhoneFlowTestCase {
    /// Going back in a chat past a reply whose plan was added, keeping its
    /// tasks; writing again from there, which starts a second version; the
    /// log of every point; then forking the chat from the first reply.
    func testChatCheckpoints() async throws {
        let (store, flow) = app(
            "chat-checkpoints",
            replies: [
                AssistantReply(text: "Start with the urgent tasks, then the ones due this week.", isComplete: true),
                .example,
                AssistantReply(text: "Saturday morning suits the garden; leave the afternoon free.", isComplete: true),
            ])
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        func later() { time.withValue { $0 += 60 } }
        send("What should I do first?", in: id, of: store)
        await flow.step("first", image: false)
        later()
        send("Tidy my weekend", in: id, of: store)
        await flow.step("drafted", image: false)
        chat(.addTapped)
        await flow.step("added", image: false)
        later()

        chat(.backTapped)
        await flow.step("undo-asked")
        let back = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.back)
        chat(.alert(.presented(.go(to: back, undo: false))))
        await flow.step("rewound")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.messages.count, 2)
        XCTAssertNotNil(store.conversations.chats[id: id]?.conversation.forward, "What came after is kept")

        send("Plan Saturday instead", in: id, of: store)
        await flow.step("second-version")
        let conversation = try XCTUnwrap(store.conversations.chats[id: id]?.conversation)
        let asked = try XCTUnwrap(conversation.messages.last { $0.role == .person })
        XCTAssertEqual(conversation.branches(at: asked.id).count, 2)
        chat(.branchTapped(messageID: asked.id, by: -1))
        await flow.step("first-version")
        let shownLast = store.conversations.chats[id: id]?.conversation.messages.last
        XCTAssertEqual(shownLast?.text, AssistantReply.example.text)

        // The Undo toast's gone by now, and the log's sheet covers it.
        if let toast = store.toast { store.send(.toastExpired(toast.id)) }
        chat(.binding(.set(\.showsCheckpoints, true)))
        await flow.step("log")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.checkpoints.count, 3)
        let shown = store.conversations.chats[id: id]?.conversation.messages ?? []
        let reply = try XCTUnwrap(shown.dropFirst().first)
        chat(.forkTapped(messageID: reply.id))
        await flow.step("forked")
        let fork = try XCTUnwrap(store.conversations.openID)
        XCTAssertNotEqual(fork, id)
        XCTAssertEqual(
            store.conversations.chats[id: fork]?.conversation.messages.map(\.text), shown.prefix(2).map(\.text))

        // Searching the list finds the reply on the branch not shown, and
        // opens the chat on it.
        store.send(.conversations(.closed))
        store.send(.conversations(.searched("afternoon")))
        await flow.step("search")
        let found = try XCTUnwrap(store.conversations.found.first)
        XCTAssertEqual(store.conversations.found.map(\.id), [id])
        store.send(.conversations(.foundTapped(found.id, messageID: found.message?.id)))
        await flow.step("found")
        XCTAssertEqual(
            store.conversations.chats[id: id]?.conversation.messages.last?.text,
            "Saturday morning suits the garden; leave the afternoon free.")

        // Finding in the chat goes to the newest message with the words.
        chat(.findTapped)
        chat(.binding(.set(\.findQuery, "saturday")))
        await flow.step("find")
        XCTAssertEqual(store.conversations.chats[id: id]?.findMatches.count, 2)

        // Related words find what the words typed don't, once the person
        // pauses.
        chat(.findClosed)
        store.send(.conversations(.closed))
        store.send(.conversations(.searched("yard work")))
        await clock.advance(by: .seconds(1))
        await flow.step("related")
        XCTAssertEqual(store.conversations.found.map(\.isRelated), [true])
        flow.finish()
    }

    /// Typing a skill's name: "/add " turns into the skill at the start of
    /// the field, the details follow it, and the sent message starts with
    /// it too. Then commands in the / list, and /find with the words to find.
    func testSkillTypedAndCommands() async throws {
        let (store, flow) = app(
            "skill-typed",
            replies: [AssistantReply(text: "Added milk to your list for Saturday.", isComplete: true)])
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        chat(.binding(.set(\.input, "/add")))
        await flow.step("typing")
        XCTAssertNil(store.conversations.chats[id: id]?.picked)
        chat(.binding(.set(\.input, "/add ")))
        await flow.step("picked")
        XCTAssertEqual(store.conversations.chats[id: id]?.picked?.id, "add")
        XCTAssertEqual(store.conversations.chats[id: id]?.input, "")
        chat(.binding(.set(\.input, "buy milk on saturday")))
        await flow.step("details")
        chat(.sendTapped)
        await flow.step("sent")
        let asked = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.first)
        XCTAssertEqual(asked.skill, "add")
        XCTAssertEqual(asked.text, "buy milk on saturday")

        // Skills come first, then the commands that can run now.
        chat(.binding(.set(\.input, "/c")))
        await flow.step("commands")
        chat(.binding(.set(\.input, "/find milk")))
        chat(.sendTapped)
        await flow.step("find")
        XCTAssertEqual(store.conversations.chats[id: id]?.findQuery, "milk")
        flow.finish()
    }

    /// Asking about a photo and a recording, with a model that reads both:
    /// attaching each, sending them, the reply, then a model that can't.
    func testAssistantAttachments() async throws {
        let assistant = AssistantClient.scripted(
            [
                AssistantReply(text: "A Boston fern. Water it when the top of the soil is dry.", isComplete: true)
            ],
            inputs: [.image, .audio]
        )
        .failing([2: .cantRead(.audio)])
        let (store, flow) = app("assistant-attachments", assistant: assistant)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new")
        store.send(.conversations(.chats(.element(id: id, action: .photoPicked(Data([1]))))))
        await flow.step("photo", image: false)
        store.send(.conversations(.chats(.element(id: id, action: .micTapped))))
        await flow.step("recording")
        store.send(.conversations(.chats(.element(id: id, action: .micTapped))))
        store.send(
            .conversations(.chats(.element(id: id, action: .binding(.set(\.input, "What plant is this?"))))))
        await flow.step("attached")
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        await flow.step("replied")
        store.send(.conversations(.chats(.element(id: id, action: .micTapped))))
        store.send(.conversations(.chats(.element(id: id, action: .micTapped))))
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "And this one?"))))))
        // Stopping attaches the recording once it's saved.
        for _ in 0..<100 where store.conversations.chats[id: id]?.canSend != true {
            try await Task.sleep(for: .milliseconds(10))
        }
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        await flow.step("cant-read")
        XCTAssertNotNil(store.conversations.chats[id: id]?.failure)
        flow.finish()
    }

    /// + with a model that reads photos: the camera and recent photos, one
    /// tapped to attach it, the sheet closed with its chip, then the
    /// microphone writing what's said into the message until it's tapped
    /// again.
    func testAddContext() async throws {
        try await addContext("add-context", style: .light)
    }

    func testAddContextInDarkMode() async throws {
        try await addContext("add-context-dark", style: .dark)
    }

    /// + with a model that reads only text: only Add files.
    func testAddContextReadingOnlyText() async throws {
        try await addContextReadingOnlyText("add-context-text", style: .light)
    }

    func testAddContextReadingOnlyTextInDarkMode() async throws {
        try await addContextReadingOnlyText("add-context-text-dark", style: .dark)
    }

    private func addContext(_ name: String, style: UIUserInterfaceStyle, test: String = #function) async throws {
        let (store, flow) = app(
            name, inputs: [.image], photos: .scripted(recentPhotos(), camera: true), style: style)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        await flow.step("new", image: false, testName: test)
        chat(.addContextTapped)
        await flow.step("sheet", testName: test)
        XCTAssertEqual(store.conversations.chats[id: id]?.recentPhotos.count, 4)
        chat(.recentPhotoTapped("photo-1"))
        await flow.step("recent-attached", testName: test)
        XCTAssertEqual(store.conversations.chats[id: id]?.pending.count, 1)
        chat(.binding(.set(\.addsContext, false)))
        await flow.step("attached", testName: test)
        chat(.micTapped)
        await flow.step("dictating", testName: test)
        XCTAssertEqual(store.conversations.chats[id: id]?.dictating, true)
        chat(.micTapped)
        await flow.step("dictated", testName: test)
        XCTAssertEqual(store.conversations.chats[id: id]?.input, "Move my run to Friday")
        flow.finish()
    }

    private func addContextReadingOnlyText(
        _ name: String, style: UIUserInterfaceStyle, test: String = #function
    ) async throws {
        let (store, flow) = app(name, style: style)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new", image: false, testName: test)
        store.send(.conversations(.chats(.element(id: id, action: .addContextTapped))))
        await flow.step("sheet", testName: test)
        XCTAssertEqual(store.conversations.chats[id: id]?.recentPhotos, [])
        flow.finish()
    }

    /// Picking what one chat runs on: a new chat, its Model sheet before
    /// the first message, Gemma thinking Long with a larger window, skill
    /// picking turned off for it, then the chat.
    /// Settings keeps its default.
    /// The chat in dark mode: its composer, the / list, a draft and the
    /// model sheet.
    func testAssistantInDarkMode() async throws {
        let (store, flow) = app("assistant-dark", style: .dark)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "/"))))))
        await flow.step("slash")
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.input, "Tidy my weekend"))))))
        store.send(.conversations(.chats(.element(id: id, action: .sendTapped))))
        await flow.step("replied-with-draft")
        store.send(.conversations(.chats(.element(id: id, action: .modelTapped))))
        await flow.step("model")
    }

    func testChatModel() async throws {
        let (store, flow) = app("chat-model")
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        await flow.step("new")
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(.conversations(.chats(.element(id: id, action: .modelTapped))))
        await flow.step("defaults")
        let sheet = { (action: ChatModelFeature.Action) in
            _ = store.send(.conversations(.chats(.element(id: id, action: .model(.presented(action))))))
        }
        let gemma = ModelCatalog.gemma4E2B.id
        sheet(.binding(.set(\.model, gemma)))
        let long = ModelChoice(model: gemma, thinking: .long, window: 16_384)
        sheet(.binding(.set(\.choice, long)))
        await flow.step("picked")
        sheet(.binding(.set(\.picksSkillsHere, false)))
        flow.scrollToEnd()
        await flow.step("no-skill-picking")
        store.send(.conversations(.chats(.element(id: id, action: .model(.dismiss)))))
        await flow.step("chat")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.context.model, long)
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.context.picksSkills, false)
        flow.finish()
    }

    /// A reply with what the model thought: closed under its chevron with
    /// how long it took on the GPU, then opened.
    func testAssistantThinking() async throws {
        let thinking = ModelThinking(
            text: """
                The person wants to water the plants this weekend. Saturday morning is free, \
                so a task for Saturday at HOME fits.
                """,
            duration: .milliseconds(4_200), processor: .gpu)
        let (store, flow) = app(
            "assistant-thinking",
            replies: [
                AssistantReply(
                    text: "I'd water them on Saturday morning.", plan: nil, isComplete: true, thinking: thinking)
            ])
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        send("When should I water the plants?", in: id, of: store)
        await flow.step("collapsed")
        let reply = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last)
        XCTAssertEqual(reply.thinking, thinking)
        store.send(.conversations(.chats(.element(id: id, action: .thinkingToggled(messageID: reply.id)))))
        await flow.step("expanded")
        flow.finish()
    }

    /// The assistant failing to answer, then Retry writing the reply, the
    /// message edited to write it again beside the old, and a message too
    /// long to answer, which Retry can't help.
    func testAssistantFails() async throws {
        let assistant = AssistantClient.scripted([
            AssistantReply(text: "Start with the urgent tasks, then the ones due this week.", isComplete: true),
            AssistantReply(text: "Keep Saturday for the garden and Sunday free.", isComplete: true),
        ])
        .failing([1: .unreadable, 4: .tooLong])
        let (store, flow) = app("assistant-fails", assistant: assistant)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        send("Plan my week", in: id, of: store)
        await flow.step("unreadable")
        let asked = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last)
        XCTAssertNotNil(store.conversations.chats[id: id]?.failure)

        chat(.retryTapped(messageID: asked.id))
        await flow.step("retried")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.messages.count, 2)

        time.withValue { $0 += 60 }
        chat(.editTapped(messageID: asked.id))
        await flow.step("editing")
        chat(.binding(.set(\.input, "Plan my weekend")))
        chat(.sendTapped)
        await flow.step("edited")
        let edited = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.first)
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.branches(at: edited.id).count, 2)

        send("Add a task for every day of the year", in: id, of: store)
        await flow.step("too-long")
        let long = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last)
        XCTAssertEqual(store.conversations.chats[id: id]?.canRetry(long), false)
        flow.finish()
    }

    /// Stop while a reply is written: Send turns to Stop, and Stop keeps
    /// what the reply had written, then the person can send again.
    func testAssistantStops() async throws {
        var assistant = AssistantClient.scripted([
            AssistantReply(text: "Keep Saturday for the garden.", isComplete: true)
        ])
        let scripted = assistant.reply
        let asked = LockIsolated(0)
        assistant.reply = { request in
            guard
                asked.withValue({
                    $0 += 1
                    return $0
                }) == 1
            else { return scripted(request) }
            return AssistantReplies { continuation in
                let task = Task {
                    continuation.yield(AssistantReply(text: "Start with the urgent tasks, then"))
                    try? await Task.sleep(for: .seconds(3600))
                    continuation.finish()
                }
                continuation.onTermination = { _ in task.cancel() }
            }
        }
        let (store, flow) = app("assistant-stops", assistant: assistant)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        send("Plan my week", in: id, of: store)
        await flow.step("writing")
        XCTAssertEqual(store.conversations.chats[id: id]?.canStop, true)

        chat(.stopTapped)
        await flow.step("stopped")
        XCTAssertEqual(store.conversations.chats[id: id]?.canStop, false)
        XCTAssertEqual(
            store.conversations.chats[id: id]?.conversation.messages.last?.text, "Start with the urgent tasks, then")

        time.withValue { $0 += 60 }
        send("And the weekend?", in: id, of: store)
        await flow.step("sent-again")
        XCTAssertEqual(
            store.conversations.chats[id: id]?.conversation.messages.last?.text, "Keep Saturday for the garden.")
        flow.finish()
    }

    /// A reply in Markdown: while it's written, its code block shows as
    /// far as it's got; then a whole reply with headings, a nested list, a
    /// quote, code with Copy, a rule and a table.
    func testAssistantMarkdown() async throws {
        var assistant = AssistantClient.scripted([
            AssistantReply(text: Self.markdownReply, isComplete: true)
        ])
        let scripted = assistant.reply
        let asked = LockIsolated(0)
        assistant.reply = { request in
            guard
                asked.withValue({
                    $0 += 1
                    return $0
                }) == 1
            else { return scripted(request) }
            return AssistantReplies { continuation in
                let task = Task {
                    continuation.yield(
                        AssistantReply(text: "Start with **Buy soil**.\n\n```swift\nlet due = \"Satur"))
                    try? await Task.sleep(for: .seconds(3600))
                    continuation.finish()
                }
                continuation.onTermination = { _ in task.cancel() }
            }
        }
        let (store, flow) = app("assistant-markdown", assistant: assistant)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        send("Show me some code", in: id, of: store)
        await flow.step("writing")
        store.send(.conversations(.chats(.element(id: id, action: .stopTapped))))
        store.send(.conversations(.closed))
        store.send(.conversations(.newTapped))
        let next = try XCTUnwrap(store.conversations.openID)
        send("How do I plant the seedlings?", in: next, of: store)
        await flow.step("replied")
        XCTAssertEqual(store.conversations.chats[id: next]?.conversation.messages.last?.text, Self.markdownReply)
        flow.finish()
    }

    static let markdownReply = """
        ## Planting the seedlings

        Start with **Buy soil**, then *plant* them. ~~Skip the shed~~ for now; see \
        [the guide](https://example.com/garden) or run `water --morning`.

        ### Steps
        1. Dig a hole
        2. Plant
           - Tomatoes
           - Basil

        > Water them in the morning.

        ```swift
        let due = "Saturday"
        ```

        ---

        | Task | Day |
        |:-----|----:|
        | Buy soil | Sat |
        | Plant | Sun |
        """

    /// A long conversation: the reply written without older messages
    /// offers Compact, which /compact does, summarizing them; the summary is shown where
    /// it ends, and Forget Summary sends the messages again.
    func testAssistantCompacts() async throws {
        let assistant = AssistantClient.scripted([
            AssistantReply(text: "Start with the urgent tasks, then the ones due this week.", isComplete: true),
            AssistantReply(text: "Keep Saturday for the garden and Sunday free.", isComplete: true, leftOut: 1),
        ])
        let (store, flow) = app("assistant-compacts", assistant: assistant)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        send("Plan my week", in: id, of: store)
        await flow.step("planned")
        time.withValue { $0 += 60 }
        send("And the weekend?", in: id, of: store)
        await flow.step("offered")
        XCTAssertEqual(store.conversations.chats[id: id]?.offersCompact, true)

        // /compact does what the offer's button does.
        chat(.binding(.set(\.input, "/compact ")))
        await flow.step("compacted")
        let summary = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.summary)
        XCTAssertEqual(store.conversations.chats[id: id]?.offersCompact, false)

        chat(.forgetSummaryTapped(summary.through))
        await flow.step("forgot")
        XCTAssertNil(store.conversations.chats[id: id]?.conversation.summary)
        flow.finish()
    }

    /// Reading a reply aloud from the button under it, then stopping it.
    func testSpeakingAReply() async throws {
        let said = LockIsolated<[String]>([])
        let speech = SpeechClient.picks(
            VoiceSelection(file: nil),
            speak: { text, _ throws(SpeechError) in
                said.withValue { $0.append(text) }
                try? await Task.never()
            })
        let (store, flow) = app("speak", speech: speech)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        send("What should I do first?", in: id, of: store)
        await flow.step("replied")
        flow.tap("Speak")
        await flow.step("speaking")
        XCTAssertEqual(said.value.count, 1)
        flow.tap("Stop Speaking")
        await flow.step("stopped")
        XCTAssertNil(store.conversations.chats[id: id]?.speaking)
        flow.finish()
    }
}
