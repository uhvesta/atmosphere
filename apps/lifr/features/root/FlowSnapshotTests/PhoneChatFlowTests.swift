import ChatFeatures
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
import LifrSettings
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
        chat(.core(.branchTapped(messageID: asked.id, by: -1)))
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
        chat(.core(.findTapped))
        chat(.core(.binding(.set(\.findQuery, "saturday"))))
        await flow.step("find")
        XCTAssertEqual(store.conversations.chats[id: id]?.core.findMatches.count, 2)

        // Related words find what the words typed don't, once the person
        // pauses.
        chat(.core(.findClosed))
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
        chat(.core(.binding(.set(\.input, "/add"))))
        await flow.step("typing")
        XCTAssertNil(store.conversations.chats[id: id]?.picked)
        chat(.core(.binding(.set(\.input, "/add "))))
        await flow.step("picked")
        XCTAssertEqual(store.conversations.chats[id: id]?.picked?.id, "add")
        XCTAssertEqual(store.conversations.chats[id: id]?.input, "")
        chat(.core(.binding(.set(\.input, "buy milk on saturday"))))
        await flow.step("details")
        chat(.core(.sendTapped))
        await flow.step("sent")
        let asked = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.first)
        XCTAssertEqual(asked.skill, "add")
        XCTAssertEqual(asked.text, "buy milk on saturday")

        // Skills come first, then the commands that can run now.
        chat(.core(.binding(.set(\.input, "/c"))))
        await flow.step("commands")
        chat(.core(.binding(.set(\.input, "/find milk"))))
        chat(.core(.sendTapped))
        await flow.step("find")
        XCTAssertEqual(store.conversations.chats[id: id]?.core.findQuery, "milk")
        flow.finish()
    }

    /// Planning with another chat assistant: /export copies a prompt and
    /// says so, then /paste puts the plan in its reply on the draft card,
    /// and a reply that can't be read offers a fix-up prompt.
    func testPlanWithAnotherAssistant() async throws {
        let (store, flow) = app("plan-elsewhere")
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        chat(.core(.binding(.set(\.input, "/ex"))))
        await flow.step("command")
        chat(.pickCommandTapped(.export))
        await flow.step("scope")
        chat(.alert(.presented(.exportPlan(goal: "", scope: .names))))
        await flow.step("copied")
        XCTAssertTrue(copied.value?.contains("HOME: Home") ?? false)
        chat(.alert(.dismiss))

        copied.setValue(
            """
            Here's a plan.

            ```toml
            # Lifr plan v1

            [[project]]
            name = "Spring clean"
            avenue = "HOME"

              [[project.task]]
              title = "Clear the garage"
              due = "saturday"

              [[project.task]]
              title = "Wash the windows"
            ```
            """)
        chat(.core(.binding(.set(\.input, "/paste "))))
        await flow.step("pasted")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.draft?.projects.map(\.name), ["Spring clean"])

        copied.setValue("```toml\n[[task]]\ntitel = \"Wash the car\"\n```")
        chat(.core(.binding(.set(\.input, "/paste "))))
        await flow.step("unreadable")
        XCTAssertNotNil(store.conversations.chats[id: id]?.alert)
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
        let (store, flow) = app("assistant-attachments", readsPhotos: true, assistant: assistant)
        // Photos go to a model that reads them.
        await library.client.select(ModelCatalog.gemma4E4B.id, .assistant)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        // The model's pill names it once the library's read.
        for _ in 0..<100 where store.conversations.chats[id: id]?.core.standard == nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        await flow.step("new")
        store.send(.conversations(.chats(.element(id: id, action: .core(.photoPicked(Data([1])))))))
        await flow.step("photo", image: false)
        store.send(.conversations(.chats(.element(id: id, action: .core(.micTapped)))))
        await flow.step("recording")
        store.send(.conversations(.chats(.element(id: id, action: .core(.micTapped)))))
        store.send(
            .conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, "What plant is this?")))))))
        await flow.step("attached")
        store.send(.conversations(.chats(.element(id: id, action: .core(.sendTapped)))))
        await flow.step("replied")
        store.send(.conversations(.chats(.element(id: id, action: .core(.micTapped)))))
        // A stop before the microphone starts keeps nothing.
        for _ in 0..<100 where store.conversations.chats[id: id]?.recordingLive != true {
            try await Task.sleep(for: .milliseconds(10))
        }
        store.send(.conversations(.chats(.element(id: id, action: .core(.micTapped)))))
        store.send(.conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, "And this one?")))))))
        // Stopping attaches the recording once it's saved.
        for _ in 0..<100 where store.conversations.chats[id: id]?.canSend != true {
            try await Task.sleep(for: .milliseconds(10))
        }
        store.send(.conversations(.chats(.element(id: id, action: .core(.sendTapped)))))
        await flow.step("cant-read")
        XCTAssertNotNil(store.conversations.chats[id: id]?.core.problem)
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

    private func addContext(_ name: String, style: UIUserInterfaceStyle, test: String = #function) async throws {
        let (store, flow) = app(
            name, inputs: [.image], photos: .scripted(recentPhotos(), camera: true), style: style)
        // Photos go to a model that reads them.
        await library.client.select(ModelCatalog.gemma4E4B.id, .assistant)
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        func chat(_ action: ChatFeature.Action) { store.send(.conversations(.chats(.element(id: id, action: action)))) }
        // The model's pill names it once the library's read.
        for _ in 0..<100 where store.conversations.chats[id: id]?.core.standard == nil {
            try await Task.sleep(for: .milliseconds(10))
        }
        await flow.step("new", image: false, testName: test)
        chat(.core(.addContextTapped))
        // The sheet's blur of the chat beneath comes out a little different
        // each run, so these two steps check what's on screen, not pixels.
        await flow.step("sheet", image: false, testName: test)
        XCTAssertEqual(store.conversations.chats[id: id]?.core.recentPhotos.count, 4)
        chat(.core(.recentPhotoTapped("photo-1")))
        await flow.step("recent-attached", image: false, testName: test)
        XCTAssertEqual(store.conversations.chats[id: id]?.core.pendingPhotos.count, 1)
        chat(.core(.binding(.set(\.addsContext, false))))
        await flow.step("attached", testName: test)
        chat(.core(.micTapped))
        await flow.step("dictating", testName: test)
        XCTAssertEqual(store.conversations.chats[id: id]?.core.dictating, true)
        chat(.core(.micTapped))
        await flow.step("dictated", testName: test)
        XCTAssertEqual(store.conversations.chats[id: id]?.input, "Move my run to Friday")
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
        store.send(.conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, "/")))))))
        await flow.step("slash")
        store.send(.conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Tidy my weekend")))))))
        store.send(.conversations(.chats(.element(id: id, action: .core(.sendTapped)))))
        // The glass under the draft comes out a little different from the
        // image saved of it, so this step checks what's on screen, not
        // pixels.
        await flow.step("replied-with-draft", image: false)
        store.send(.conversations(.chats(.element(id: id, action: .core(.modelTapped)))))
        await flow.step("model")
    }

    func testChatModel() async throws {
        let (store, flow) = app("chat-model")
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        await flow.step("new")
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(.conversations(.chats(.element(id: id, action: .core(.modelTapped)))))
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
    /// The experiments turned on in Settings, then a reply with its speed
    /// under it and what the model was sent.
    func testExperiments() async throws {
        var reply = AssistantReply(text: "I'd water them on Saturday morning.", isComplete: true)
        reply.context = ContextUse(used: 1_852, window: 4_096)
        reply.speed = TurnSpeed(read: 1_840, written: 12, reading: .milliseconds(900), writing: .milliseconds(600))
        reply.sent = AssistantSent(
            instructions: "You help the person plan with Lifr.", prompt: "Person: When should I water the plants?",
            photos: 0)
        let (store, flow) = app("experiments", replies: [reply])
        store.isSettingsShown = true
        store.settings.$showsSpeed.withLock { $0 = true }
        store.settings.$showsPrompt.withLock { $0 = true }
        store.settings.$compares.withLock { $0 = true }
        await flow.step("settings", image: false)
        flow.scrollToEnd()
        await flow.step("settings-experiments")
        store.isSettingsShown = false
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        send("When should I water the plants?", in: id, of: store)
        await flow.step("replied")
        let answer = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last)
        store.send(.conversations(.chats(.element(id: id, action: .core(.promptTapped(messageID: answer.id))))))
        await flow.step("prompt")
        flow.finish()
    }

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
        XCTAssertEqual(reply.thinking?.text, thinking.text)
        store.send(.conversations(.chats(.element(id: id, action: .core(.thinkingToggled(messageID: reply.id))))))
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
        XCTAssertNotNil(store.conversations.chats[id: id]?.core.problem)

        chat(.core(.retryTapped(messageID: asked.id)))
        await flow.step("retried")
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.messages.count, 2)

        time.withValue { $0 += 60 }
        chat(.core(.editTapped(messageID: asked.id)))
        await flow.step("editing")
        chat(.core(.binding(.set(\.input, "Plan my weekend"))))
        chat(.core(.sendTapped))
        await flow.step("edited")
        let edited = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.first)
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.branches(at: edited.id).count, 2)

        send("Add a task for every day of the year", in: id, of: store)
        await flow.step("too-long")
        let long = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.messages.last)
        XCTAssertEqual(store.conversations.chats[id: id]?.core.canRetry(long), false)
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

        chat(.core(.stopTapped))
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
        store.send(.conversations(.chats(.element(id: id, action: .core(.stopTapped)))))
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
        chat(.core(.binding(.set(\.input, "/compact "))))
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
        XCTAssertNil(store.conversations.chats[id: id]?.core.speaking)
        flow.finish()
    }
}
