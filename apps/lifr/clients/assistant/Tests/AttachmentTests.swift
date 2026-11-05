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
                ? attachments.map { ModelAttachment($0.kind, file: URL(filePath: "/tmp/\($0.id)")) } : [])
    }

    func testAMessageSavedBeforeAttachmentsReadsAsHavingNone() throws {
        let saved = Data(
            #"{"id":"6F9619FF-8B86-D011-B42D-00C04FC964FF","role":"person","text":"Hi","at":0}"#.utf8)
        let message = try JSONDecoder().decode(ChatMessage.self, from: saved)
        XCTAssertEqual(message.text, "Hi")
        XCTAssertEqual(message.attachments, [])
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

    func testTheScriptedAssistantReadsOnlyWhatItsToldTo() {
        XCTAssertEqual(AssistantClient.scripted().inputs(), [])
        XCTAssertEqual(AssistantClient.scripted(inputs: [.image]).inputs(), [.image])
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
