import ComposableArchitecture
import FlowKit
import LangReadrCore
import LangReadrReading
import SwiftUI
import XCTest

/// Reading a story on the Mac, the library beside it. The Mac flow drives
/// the store, since it can't tap.
@MainActor
final class MacFlowSnapshotTests: XCTestCase {
    func testReading() async throws {
        let library = Sample.library()
        let flow = FlowRecorder("reading", LangReadrView(store: library))
        await flow.step("empty-library")
        let reader = try await Sample.importStory(into: library)
        await flow.step("reading")
        reader.send(.binding(.set(\.showsTranslation, true)))
        try await Sample.waitFor { reader.writing.isEmpty && reader.book.annotations.count == 2 }
        await flow.step("translated")
        let paragraph = try XCTUnwrap(reader.book.paragraphs.first)
        let words = reader.words[paragraph.id] ?? []
        let mercado = try XCTUnwrap(words.first { $0.text == "mercado" })
        reader.send(.wordTapped(paragraph: paragraph.id, mercado))
        try await Sample.waitFor { reader.destination?.word?.note != nil }
        await flow.step("word-card")
        reader.send(.destination(.dismiss))
        reader.send(.binding(.set(\.isHighlighting, true)))
        reader.send(.wordTapped(paragraph: paragraph.id, try XCTUnwrap(words.first)))
        reader.send(.wordTapped(paragraph: paragraph.id, mercado))
        await flow.step("passage-picked")
        reader.send(.askTapped)
        await flow.step("ask")
        reader.send(.destination(.presented(.chat(.binding(.set(\.input, "What does “vamos al” mean?"))))))
        reader.send(.destination(.presented(.chat(.sendTapped))))
        try await Sample.waitFor { reader.destination?.chat?.reply == nil }
        await flow.step("replied")
        flow.finish()
    }
}
