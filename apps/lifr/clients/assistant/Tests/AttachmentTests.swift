import ChatKit
import Foundation
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import XCTest

final class AttachmentTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private let photo = ChatAttachment(id: UUID(), kind: .image)
    private let recording = ChatAttachment(id: UUID(), kind: .audio, seconds: 20)

    private func request(_ attachments: [ChatAttachment], onDevice: Bool) -> AssistantRequest {
        AssistantRequest(
            messages: [ChatMessage(id: UUID(), role: .person, text: "", at: now, attachments: attachments)],
            workspace: Workspace(), now: now, timeZone: .gmt,
            attachments: onDevice
                ? attachments.compactMap { attachment in
                    attachment.kind.input.map { ModelAttachment($0, file: URL(filePath: "/tmp/\(attachment.id)")) }
                } : [])
    }

    func testAMessageSavedBeforeAttachmentsReadsAsHavingNone() throws {
        let saved = Data(
            #"{"id":"6F9619FF-8B86-D011-B42D-00C04FC964FF","role":"person","text":"Hi","at":0}"#.utf8)
        let message = try JSONDecoder().decode(ChatMessage.self, from: saved)
        XCTAssertEqual(message.text, "Hi")
        XCTAssertEqual(message.attachments, [])
        XCTAssertNil(message.thinking)
    }

    func testWhatTheModelThoughtAndWhoWroteItAreKeptWithItsReply() throws {
        let thinking = ThreadThinking(text: "Ferns like shade.", duration: .milliseconds(4_200))
        let speed = ThreadSpeed(read: 120, written: 30, reading: .seconds(1), writing: .seconds(2))
        let message = ChatMessage(
            id: UUID(), role: .assistant, text: "A fern.", at: now, thinking: thinking, model: "Qwen", speed: speed)
        let read = try JSONDecoder().decode(ChatMessage.self, from: JSONEncoder().encode(message))
        XCTAssertEqual(read.thinking, thinking)
        XCTAssertEqual(read.model, "Qwen")
        XCTAssertEqual(read.speed, speed)
    }

    func testThinkingKeptWithWhatItRanOnStillReads() throws {
        let old = ModelThinking(text: "Ferns like shade.", duration: .milliseconds(4_200), processor: .gpu)
        let saved = try JSONEncoder().encode(old)
        let read = try JSONDecoder().decode(ThreadThinking.self, from: saved)
        XCTAssertEqual(read, ThreadThinking(text: "Ferns like shade.", duration: .milliseconds(4_200)))
    }

    func testWhatAReplyAskedToChangeIsKeptWithItsDecision() throws {
        let proposal = Proposal(
            actions: [.comment(code: "HOME-0001", text: "Done")], steps: [], decision: .applied)
        let message = ChatMessage(id: UUID(), role: .assistant, text: "Added.", at: now, proposal: proposal)
        let read = try JSONDecoder().decode(ChatMessage.self, from: JSONEncoder().encode(message))
        XCTAssertEqual(read.proposal, proposal)
    }

    func testAttachmentsAreKeptWithTheMessage() throws {
        let message = ChatMessage(
            id: UUID(), role: .person, text: "What's this?", at: now, attachments: [photo, recording])
        let read = try JSONDecoder().decode(ChatMessage.self, from: JSONEncoder().encode(message))
        XCTAssertEqual(read, message)
    }

    func testWhatsAttachedCountsTowardTheModelsRoom() {
        let bare = AssistantPrompt.estimatedTokens(request([], onDevice: true), size: .full)
        let withPhoto = AssistantPrompt.estimatedTokens(request([photo], onDevice: true), size: .full)
        let withBoth = AssistantPrompt.estimatedTokens(request([photo, recording], onDevice: true), size: .full)
        XCTAssertGreaterThanOrEqual(withPhoto - bare, photo.estimatedTokens)
        XCTAssertGreaterThanOrEqual(withBoth - withPhoto, recording.estimatedTokens)
    }

    func testAttachmentsNotOnThisDeviceArentCounted() {
        XCTAssertEqual(AssistantPrompt.attachedTokens(request([photo, recording], onDevice: false)), 0)
    }

    func testALongerRecordingTakesMoreRoom() {
        let short = ChatAttachment(id: UUID(), kind: .audio, seconds: 2)
        XCTAssertLessThan(short.estimatedTokens, recording.estimatedTokens)
    }

    func testAMessageWithOnlyAPhotoStillReachesTheModel() {
        XCTAssertNotEqual(
            AssistantPrompt.prompt(request([photo], onDevice: true)),
            AssistantPrompt.prompt(request([], onDevice: true)))
    }

    func testAFilesTextGoesToTheModelWithTheNewMessage() {
        let notes = ChatAttachment(id: UUID(), name: "notes.md", text: "Repot the fern in spring.")
        XCTAssertTrue(AssistantPrompt.prompt(request([notes], onDevice: false)).contains("Repot the fern in spring."))
    }

    func testAnEarlierFilesTextIsntSentAgain() {
        let notes = ChatAttachment(id: UUID(), name: "notes.md", text: "Repot the fern in spring.")
        let earlier = ChatMessage(id: UUID(), role: .person, text: "Read this", at: now, attachments: [notes])
        let reply = ChatMessage(id: UUID(), role: .assistant, text: "Done.", at: now)
        let next = ChatMessage(id: UUID(), role: .person, text: "And now?", at: now)
        let request = AssistantRequest(
            messages: [earlier, reply, next], workspace: Workspace(), now: now, timeZone: .gmt)
        XCTAssertFalse(AssistantPrompt.prompt(request).contains("Repot the fern in spring."))
    }

    func testAFilesTextIsCutToFitTheModel() {
        let long = ChatAttachment(id: UUID(), name: "long.txt", text: String(repeating: "fern ", count: 2_000))
        let prompt = AssistantPrompt.prompt(request([long], onDevice: false))
        XCTAssertLessThan(prompt.count, long.text?.count ?? 0)
    }

    func testAFilesTextIsKeptWithTheMessageAndSavedPhotosStillRead() throws {
        let notes = ChatAttachment(id: UUID(), name: "notes.md", text: "Repot the fern.")
        let message = ChatMessage(id: UUID(), role: .person, text: "", at: now, attachments: [photo, notes])
        let read = try JSONDecoder().decode(ChatMessage.self, from: JSONEncoder().encode(message))
        XCTAssertEqual(read, message)
        let saved = Data(#"{"id":"6F9619FF-8B86-D011-B42D-00C04FC964FF","kind":"image"}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(ChatAttachment.self, from: saved).kind, .image)
    }

    func testTheScriptedAssistantReadsOnlyWhatItsToldTo() {
        XCTAssertEqual(AssistantClient.scripted().inputs(.defaults), [])
        XCTAssertEqual(AssistantClient.scripted(inputs: [.image]).inputs(.defaults), [.image])
    }

    func testInMemoryAttachmentsAreGoneWithTheirConversation() async throws {
        let attachments = AttachmentsClient.inMemory()
        let conversation = UUID()
        let saved = try await attachments.save(Data([1]), .image, nil, conversation)
        XCTAssertNotNil(attachments.file(saved, conversation))
        await attachments.removeAll(conversation)
        XCTAssertNil(attachments.file(saved, conversation))
    }
}
