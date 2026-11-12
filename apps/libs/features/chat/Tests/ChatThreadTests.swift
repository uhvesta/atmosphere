import CaptureKit
import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit
import ModelsClient
import XCTest

@testable import ChatFeatures

@MainActor
final class ChatThreadTests: XCTestCase {
    private let qwen = ModelCatalog.qwen35Small.id

    private func notes(_ text: String = "Amigo: friend. Hola: hello.") throws -> PickedFile {
        .text(try FileText(name: "notes.md", text: text, limit: 8_000))
    }

    func testAReplyIsShownAsItsWrittenThenKept() async throws {
        let runs = Runs([ScriptedRuntime.answer("It means friend.", partials: ["It means"])])
        let store = chat(runs)
        await store.send(.binding(.set(\.input, "What's amigo?")))
        await store.send(.sendTapped)
        XCTAssertEqual(store.state.reply, "")
        XCTAssertEqual(store.state.input, "")
        await store.receive(\.progressed)
        XCTAssertEqual(store.state.reply, "It means")
        await store.receive(\.replied)
        await store.settle()
        XCTAssertNil(store.state.reply)
        XCTAssertEqual(store.texts, ["What's amigo?", "It means friend."])
        let turn = try runs.runtime.turns.first.unwrap()
        XCTAssertEqual(turn.instructions, "You help with Spanish.")
        XCTAssertTrue(turn.prompt.contains("What's amigo?"))
        XCTAssertEqual(runs.asked.value, [.chat])
    }

    func testEachChangeIsSentForTheAppToKeep() async throws {
        let store = chat(Runs([ScriptedRuntime.answer("Friend.")]))
        await store.send(.binding(.set(\.input, "Amigo?")))
        await store.send(.sendTapped)
        await store.receive(\.delegate.changed)
        await store.receive(\.replied)
        await store.receive(\.delegate.changed)
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])
    }

    func testStoppingKeepsWhatsWrittenOfTheReply() async throws {
        // Writes a few words, then waits until it's stopped.
        let writing: ScriptedRuntime.Step = { _, partial throws(ModelRuntimeError) in
            partial(.string("It means"))
            try? await Task.sleep(for: .seconds(30))
            return .string("It means")
        }
        let store = chat(Runs([writing]))
        await store.send(.binding(.set(\.input, "Amigo?")))
        await store.send(.sendTapped)
        await store.receive(\.progressed)
        await store.send(.stopTapped)
        await store.settle()
        XCTAssertNil(store.state.reply)
        XCTAssertEqual(store.texts, ["Amigo?", "It means"])
        XCTAssertEqual(store.state.thread.messages.map(\.stopped), [false, true])
    }

    func testStoppingBeforeAnyWordsSaysSoUntilItsAskedAgain() async throws {
        let waiting: ScriptedRuntime.Step = { _, _ throws(ModelRuntimeError) in
            try? await Task.sleep(for: .seconds(30))
            return .string("Friend.")
        }
        let store = chat(Runs([waiting, ScriptedRuntime.answer("Friend.")]))
        await store.send(.binding(.set(\.input, "Amigo?")))
        await store.send(.sendTapped)
        await store.send(.stopTapped)
        await store.settle()
        XCTAssertEqual(store.texts, ["Amigo?"])
        XCTAssertTrue(store.state.stoppedBeforeReplying)
        XCTAssertNil(store.state.problem)

        let asked = try store.state.thread.messages.last.unwrap()
        await store.send(.retryTapped(messageID: asked.id))
        XCTAssertFalse(store.state.stoppedBeforeReplying)
        await store.receive(\.replied)
        await store.settle()
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])
    }

    func testAFailedReplySaysWhyAndRetryAsksAgain() async throws {
        let runs = Runs([ScriptedRuntime.fail(.failed("the model stopped")), ScriptedRuntime.answer("Friend.")])
        let store = chat(runs)
        await store.ask("Amigo?")
        XCTAssertNotNil(store.state.problem)
        XCTAssertEqual(store.texts, ["Amigo?"])
        let asked = try store.state.thread.messages.last.unwrap()
        XCTAssertTrue(store.state.canRetry(asked))

        await store.send(.retryTapped(messageID: asked.id))
        await store.receive(\.replied)
        await store.settle()
        XCTAssertNil(store.state.problem)
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])
    }

    func testAnEmptyReplySaysSoAndRetryAsksAgain() async throws {
        let runs = Runs([ScriptedRuntime.answer(" \n"), ScriptedRuntime.answer("Friend.")])
        let store = chat(runs)
        await store.ask("Amigo?")
        XCTAssertNotNil(store.state.problem)
        XCTAssertNil(store.state.reply)
        XCTAssertEqual(store.texts, ["Amigo?"])
        let asked = try store.state.thread.messages.last.unwrap()
        XCTAssertTrue(store.state.canRetry(asked))

        await store.send(.retryTapped(messageID: asked.id))
        await store.receive(\.replied)
        await store.settle()
        XCTAssertNil(store.state.problem)
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])
    }

    func testRetryingAReplyKeepsTheOldOneOnItsOwnBranch() async throws {
        let store = chat(Runs([ScriptedRuntime.answer("Friend."), ScriptedRuntime.answer("Pal.")]))
        await store.ask("Amigo?")
        let first = try store.state.thread.messages.last.unwrap()
        await store.send(.retryTapped(messageID: first.id))
        await store.receive(\.replied)
        await store.settle()
        XCTAssertEqual(store.texts, ["Amigo?", "Pal."])
        let second = try store.state.thread.messages.last.unwrap()
        let branches = store.state.thread.branches(at: second.id)
        XCTAssertEqual(Set(branches.map(\.text)), ["Friend.", "Pal."])

        let from = try branches.firstIndex { $0.id == second.id }.unwrap()
        let to = try branches.firstIndex { $0.id == first.id }.unwrap()
        await store.send(.branchTapped(messageID: second.id, by: to - from))
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])
    }

    func testEditingAMessageAsksAgainAndKeepsItsFiles() async throws {
        let runs = Runs([ScriptedRuntime.answer("Friend."), ScriptedRuntime.answer("Hello.")])
        let store = chat(runs, files: ["notes.md": try notes()])
        await store.send(.filesPicked(.success([URL(filePath: "/tmp/notes.md")])))
        await store.receive(\.filesRead)
        await store.ask("Amigo?")
        let asked = try store.state.thread.messages.first.unwrap()

        await store.send(.editTapped(messageID: asked.id))
        XCTAssertEqual(store.state.input, "Amigo?")
        await store.send(.binding(.set(\.input, "Hola?")))
        await store.send(.sendTapped)
        await store.receive(\.replied)
        await store.settle()
        XCTAssertNil(store.state.editing)
        XCTAssertEqual(store.texts, ["Hola?", "Hello."])
        XCTAssertEqual(store.state.thread.messages.first?.files.map(\.name), ["notes.md"])
        let edited = try store.state.thread.messages.first.unwrap()
        XCTAssertEqual(Set(store.state.thread.branches(at: edited.id).map(\.text)), ["Amigo?", "Hola?"])
    }

    func testCancellingAnEditLeavesTheChatAsItWas() async throws {
        let store = chat(Runs([ScriptedRuntime.answer("Friend.")]))
        await store.ask("Amigo?")
        await store.send(.editTapped(messageID: try store.state.thread.messages.first.unwrap().id))
        await store.send(.cancelEditTapped)
        XCTAssertNil(store.state.editing)
        XCTAssertEqual(store.state.input, "")
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])
    }

    func testCancellingAnEditPutsBackWhatWasBeingTyped() async throws {
        let store = chat(Runs([ScriptedRuntime.answer("Friend.")]))
        await store.ask("Amigo?")
        await store.send(.binding(.set(\.input, "Buy milk")))
        await store.send(.editTapped(messageID: try store.state.thread.messages.first.unwrap().id))
        XCTAssertEqual(store.state.input, "Amigo?")
        await store.send(.cancelEditTapped)
        XCTAssertEqual(store.state.input, "Buy milk")
    }

    func testWhatTheModelThinksIsShownAndKeptWithTheReply() async throws {
        let runs = Runs([
            ScriptedRuntime.think("It's Spanish.", for: .seconds(2), then: ScriptedRuntime.answer("Friend."))
        ])
        let store = chat(runs, model: ModelChoice(thinking: .long))
        await store.send(.binding(.set(\.input, "Amigo?")))
        await store.send(.sendTapped)
        await store.receive(\.progressed)
        XCTAssertEqual(store.state.replyThinking?.text, "It's Spanish.")
        await store.receive(\.replied)
        await store.settle()
        let reply = try store.state.thread.messages.last.unwrap()
        XCTAssertEqual(reply.thinking, ThreadThinking(text: "It's Spanish.", duration: .seconds(2)))
        XCTAssertFalse(store.state.shownThinking.contains(reply.id))
        await store.send(.thinkingToggled(messageID: reply.id))
        XCTAssertTrue(store.state.shownThinking.contains(reply.id))
        XCTAssertEqual(runs.runtime.turns.first?.thinkingTokens, ThinkingLevel.long.tokens)
    }

    func testAModelThatDoesntThinkIsntAskedTo() async throws {
        let runs = Runs([ScriptedRuntime.answer("Friend.")])
        let store = chat(runs, model: ModelChoice(thinking: .off))
        await store.ask("Amigo?")
        XCTAssertNil(try runs.runtime.turns.first.unwrap().thinking)
        XCTAssertNil(store.state.thread.messages.last?.thinking)
    }

    func testAChatsModelIsKeptAndRunsTheNextReply() async throws {
        let runs = Runs([ScriptedRuntime.answer("Friend.")])
        let store = chat(runs, library: InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]))
        await store.send(.modelTapped)
        await store.send(.modelSheet(.presented(.task)))
        await store.receive(\.modelSheet.presented.loaded)
        XCTAssertEqual(store.state.modelSheet?.models.map(\.id), [.apple, qwen])
        await store.send(.modelSheet(.presented(.binding(.set(\.model, qwen)))))
        await store.receive(\.delegate.changed)
        await store.send(.modelSheet(.presented(.binding(.set(\.choice.window, 16_384)))))
        await store.send(.modelSheet(.dismiss))
        XCTAssertEqual(store.state.model, ModelChoice(model: qwen, window: 16_384))
        await store.ask("Amigo?")
        XCTAssertEqual(runs.choices.value, [ModelChoice(model: qwen, window: 16_384)])
    }

    func testPickingEachDefaultAgainForgetsWhatWasPicked() async throws {
        let store = chat(Runs([]), model: ModelChoice(model: qwen, thinking: .off, window: 16_384))
        await store.send(.modelTapped)
        await store.send(.modelSheet(.presented(.binding(.set(\.model, nil)))))
        await store.send(.modelSheet(.presented(.binding(.set(\.choice.thinking, nil)))))
        await store.send(.modelSheet(.presented(.binding(.set(\.choice.window, nil)))))
        await store.send(.modelSheet(.dismiss))
        XCTAssertEqual(store.state.model, .defaults)
    }

    func testAChatOnAModelThatWasDeletedRunsOnTheDefault() async throws {
        let store = chat(Runs([]), model: ModelChoice(model: qwen))
        await store.send(.modelTapped)
        await store.send(.modelSheet(.presented(.task)))
        await store.receive(\.modelSheet.presented.loaded)
        XCTAssertNil(store.state.modelSheet?.choice.model)
        XCTAssertEqual(store.state.modelSheet?.current?.id, .apple)
    }

    func testTheComposerNamesTheModelTheChatRunsOn() async throws {
        let store = chat(
            Runs([]), model: ModelChoice(model: qwen),
            library: InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]))
        XCTAssertNil(store.state.current)
        await store.send(.task)
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.current?.id, qwen)
    }

    func testAReplySaysHowMuchOfTheWindowItTookUntilTheModelChanges() async throws {
        let store = chat(
            Runs([ScriptedRuntime.answer("Hello.")]),
            library: InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]))
        XCTAssertNil(store.state.context)
        await store.ask("Hola?")
        let context = try store.state.context.unwrap()
        XCTAssertGreaterThan(context.used, 0)
        XCTAssertGreaterThan(context.window, context.used)

        await store.send(.modelTapped)
        await store.send(.modelSheet(.presented(.binding(.set(\.choice.thinking, .long)))))
        await store.send(.modelSheet(.dismiss))
        XCTAssertEqual(store.state.context, context)

        await store.send(.modelTapped)
        await store.send(.modelSheet(.presented(.binding(.set(\.model, qwen)))))
        await store.send(.modelSheet(.dismiss))
        XCTAssertNil(store.state.context)
    }

    func testFindingMovesThroughTheMessagesWithEveryWord() async throws {
        let store = chat(Runs([ScriptedRuntime.answer("Amigo is friend."), ScriptedRuntime.answer("Hola is hello.")]))
        await store.ask("Amigo?")
        await store.ask("Hola?")
        await store.send(.findTapped)
        await store.send(.binding(.set(\.findQuery, "amigo")))
        XCTAssertEqual(store.state.findMatches.count, 2)
        XCTAssertEqual(store.state.currentMatch, store.state.findMatches.last, "The newest match first")
        await store.send(.findMoved(by: 1))
        XCTAssertEqual(store.state.currentMatch, store.state.findMatches.first)
        await store.send(.findClosed)
        XCTAssertFalse(store.state.finds)
        XCTAssertEqual(store.state.findMatches, [])
    }

    func testAReplyIsReadAloudUntilItsStopped() async throws {
        var speech = SpeechClient.inMemory
        speech.speak = { _, _ throws(SpeechError) in try? await Task.sleep(for: .seconds(30)) }
        let store = chat(Runs([ScriptedRuntime.answer("**Amigo** means friend.")]), speech: speech)
        await store.ask("Amigo?")
        let reply = try store.state.thread.messages.last.unwrap()
        await store.send(.speakTapped(messageID: reply.id))
        XCTAssertEqual(store.state.speaking, reply.id)
        await store.send(.speakTapped(messageID: reply.id))
        XCTAssertNil(store.state.speaking)
        XCTAssertEqual(ChatThreadFeature.spoken(reply.text), "Amigo means friend.")
    }

    func testDictationWritesAfterWhatsTypedUntilItsStopped() async throws {
        let store = chat(Runs([]))
        await store.send(.binding(.set(\.input, "Please:")))
        await store.send(.micTapped)
        await store.receive(\.dictated)
        await store.receive(\.dictated)
        XCTAssertEqual(store.state.input, "Please: What does this word mean?")
        XCTAssertTrue(store.state.dictating)
        XCTAssertFalse(store.state.canSend, "Send waits for the last words")

        await store.send(.micTapped)
        await store.receive(\.dictationEnded)
        XCTAssertFalse(store.state.dictating)
        XCTAssertTrue(store.state.canSend)
    }

    func testDictationThatCantStartSaysWhy() async throws {
        let store = chat(Runs([]), dictation: .failing(.speechDenied))
        await store.send(.micTapped)
        await store.receive(\.dictationFailed)
        XCTAssertFalse(store.state.dictating)
        XCTAssertNotNil(store.state.problem)
        XCTAssertEqual(store.state.input, "")
    }

    func testPlusOpensTheFilePickerAndPickingFilesClosesIt() async throws {
        let store = chat(Runs([]))
        await store.send(.addContextTapped)
        XCTAssertTrue(store.state.picksFiles)
        await store.send(.filesPicked(.success([])))
        XCTAssertFalse(store.state.picksFiles)
    }

    func testAFilesTextGoesWithTheMessageAndToTheModel() async throws {
        let runs = Runs([ScriptedRuntime.answer("It means friend.")])
        let store = chat(runs, files: ["notes.md": try notes()])
        await store.send(.filesPicked(.success([URL(filePath: "/tmp/notes.md")])))
        await store.receive(\.filesRead)
        XCTAssertEqual(store.state.pending.map(\.name), ["notes.md"])
        XCTAssertTrue(store.state.canSend, "A file can go on its own")

        await store.send(.sendTapped)
        await store.receive(\.replied)
        await store.settle()
        XCTAssertEqual(store.state.pending, [])
        XCTAssertEqual(store.state.thread.messages.first?.files.map(\.name), ["notes.md"])
        XCTAssertTrue(try runs.runtime.turns.first.unwrap().prompt.contains("Amigo: friend."))
    }

    func testARemovedFileIsntSent() async throws {
        let store = chat(Runs([]), files: ["notes.md": try notes()])
        await store.send(.filesPicked(.success([URL(filePath: "/tmp/notes.md")])))
        await store.receive(\.filesRead)
        await store.send(.removeFileTapped(0))
        XCTAssertEqual(store.state.pending, [])
        XCTAssertFalse(store.state.canSend)
    }

    func testAPhotoOrAnUnreadableFileIsntAttachedAndSaysWhy() async throws {
        let store = chat(Runs([]), files: ["fern.jpg": .photo(Data([1])), "notes.md": try notes()])
        let picked = ["/tmp/fern.jpg", "/tmp/missing.txt", "/tmp/notes.md"].map { URL(filePath: $0) }
        await store.send(.filesPicked(.success(picked)))
        await store.receive(\.filesRead)
        XCTAssertEqual(store.state.pending.map(\.name), ["notes.md"])
        XCTAssertNotNil(store.state.problem)
    }

    func testAFilesTextIsCutToFit() {
        let long = ThreadMessage(
            role: .person, text: "", at: epoch,
            files: [ThreadFile(name: "long.txt", text: String(repeating: "amigo ", count: 1_000))])
        XCTAssertLessThan(ReplyTurn.files(of: long, fitting: 500).count, 600)
        XCTAssertEqual(ReplyTurn.files(of: ThreadMessage(role: .person, text: "Hola", at: epoch), fitting: 500), "")
    }

    func testALongChatSendsItsNewestMessagesThatFit() {
        var thread = ChatThread(id: UUID(999), createdAt: epoch)
        for index in 0..<200 {
            thread.append(
                ThreadMessage(
                    role: index.isMultiple(of: 2) ? .person : .assistant,
                    text: "Message \(index) " + String(repeating: "palabra ", count: 40), at: epoch))
        }
        let turn = ReplyTurn(thread: thread, instructions: "", budget: .appleOnDevice, thinkingTokens: nil).turn
        XCTAssertTrue(turn.prompt.contains("Message 199"))
        XCTAssertFalse(turn.prompt.contains("Message 0 "))
        XCTAssertLessThan(turn.prompt.count, ContextBudget.appleOnDevice.room * 3)
    }

    func testBackToLatestShowsWhatCameAfterAnEarlierPoint() async throws {
        var thread = ChatThread(id: UUID(999), createdAt: epoch)
        thread.append(ThreadMessage(role: .person, text: "Amigo?", at: epoch))
        thread.append(ThreadMessage(role: .assistant, text: "Friend.", at: epoch))
        let earlier = try thread.messages.last.unwrap().id
        thread.append(ThreadMessage(role: .person, text: "Hola?", at: epoch))
        thread.append(ThreadMessage(role: .assistant, text: "Hello.", at: epoch))
        thread.go(to: earlier)
        XCTAssertNotNil(thread.forward)
        let store = chat(Runs([]), thread: thread)
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])

        await store.send(.latestTapped)
        XCTAssertEqual(store.texts, ["Amigo?", "Friend.", "Hola?", "Hello."])
        XCTAssertNil(store.state.thread.forward)
    }
}
