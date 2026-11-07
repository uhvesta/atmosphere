import ComposableArchitecture
import FlowKit
import LangReadrCore
import LangReadrReading
import SwiftUI
import XCTest

/// Reading a story on the iPhone: one reference per step, the
/// accessibility tree and an image. Words and buttons are tapped; switches
/// and closing sheets go through the store, since a flow's tap can't flip
/// a toolbar toggle or press a sheet's toolbar button.
@MainActor
final class PhoneFlowSnapshotTests: XCTestCase {
    /// Importing pasted text, translating it, looking a word up, and asking
    /// about a passage, then reading the chat on its own.
    func testReading() async throws {
        let library = Sample.library()
        let flow = FlowRecorder("reading", LangReadrView(store: library))
        await flow.step("empty-library")
        flow.tap("Text")
        await flow.step("paste-text")
        let reader = try await Sample.importStory(into: library)
        await flow.step("reading")
        reader.send(.binding(.set(\.showsTranslation, true)))
        try await Sample.waitFor { reader.writing.isEmpty && reader.book.annotations.count == 2 }
        await flow.step("translated")
        flow.tap("mercado")
        try await Sample.waitFor { reader.destination?.word?.note != nil }
        await flow.step("word-card")
        reader.send(.destination(.dismiss))
        reader.send(.binding(.set(\.isHighlighting, true)))
        await flow.step("highlighting")
        flow.tap("Hola")
        flow.tap("mercado")
        await flow.step("passage-picked")
        flow.tap("Ask")
        await flow.step("ask")
        reader.send(.destination(.presented(.chat(.binding(.set(\.input, "What does “vamos al” mean?"))))))
        flow.tap("Send")
        try await Sample.waitFor { reader.destination?.chat?.reply == nil }
        await flow.step("replied")
        flow.tap("Open in chat")
        await flow.step("chat")
        flow.finish()
    }
}
