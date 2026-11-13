import ComposableArchitecture
import HumataBoard
import HumataDaemonClient
import HumataReview
import XCTest

@MainActor
final class SpecCommentsTests: XCTestCase {
    private let spec = "# Login\n\nEmail and password.\n\nRemember me for a month."

    private func store(_ daemon: InMemoryDaemon) -> TestStoreOf<SpecCommentsFeature> {
        let store = TestStore(initialState: SpecCommentsFeature.State(title: "Login")) {
            SpecCommentsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    func testCommentsReachTheSessionAsOneMessageInTheSpecsOrder() async throws {
        let heard = LockIsolated<[String]>([])
        let daemon = InMemoryDaemon { message in
            heard.withValue { $0.append(message) }
            return [.say("Noted.")]
        }
        let session = try await daemon.client.start(StartRequest(backend: .scripted))
        let blocks = MarkdownBlock.blocks(spec.components(separatedBy: "\n"))
        let store = store(daemon)

        await store.send(.blockTapped(blocks[2]))
        await store.send(\.binding.comment, "A week is enough.")
        await store.send(.commentSaved)
        await store.send(.blockTapped(blocks[1]))
        await store.send(\.binding.comment, "  Add a magic link.  ")
        await store.send(.commentSaved)
        await store.send(.sendTapped(to: session.id))
        await store.receive(\.sent)

        XCTAssertTrue(store.state.drafts.isEmpty)
        let message = try XCTUnwrap(heard.value.first)
        XCTAssertEqual(heard.value.count, 1)
        let link = try XCTUnwrap(message.range(of: "Add a magic link."))
        let week = try XCTUnwrap(message.range(of: "A week is enough."))
        XCTAssertLessThan(link.lowerBound, week.lowerBound)
        XCTAssertTrue(message.contains("> Remember me for a month."))
    }

    func testAnEmptyCommentIsNotKept() async throws {
        let daemon = InMemoryDaemon()
        let blocks = MarkdownBlock.blocks(spec.components(separatedBy: "\n"))
        let store = store(daemon)

        await store.send(.blockTapped(blocks[1]))
        await store.send(\.binding.comment, "   ")
        await store.send(.commentSaved)
        XCTAssertTrue(store.state.drafts.isEmpty)
        XCTAssertEqual(store.state.picked, blocks[1])
    }

    func testCommentsStayWhenTheSessionIsGone() async throws {
        let daemon = InMemoryDaemon()
        let blocks = MarkdownBlock.blocks(spec.components(separatedBy: "\n"))
        let store = store(daemon)

        await store.send(.blockTapped(blocks[1]))
        await store.send(\.binding.comment, "Add a magic link.")
        await store.send(.commentSaved)
        await store.send(.sendTapped(to: "no-such-session"))
        await store.receive(\.failed)

        XCTAssertEqual(store.state.drafts.map(\.text), ["Add a magic link."])
        XCTAssertFalse(store.state.sending)
        XCTAssertNotNil(store.state.alert)
    }

    func testTappingAnotherBlockMovesAHalfWrittenCommentThere() async throws {
        let blocks = MarkdownBlock.blocks(spec.components(separatedBy: "\n"))
        let store = store(InMemoryDaemon())

        await store.send(.blockTapped(blocks[1]))
        await store.send(\.binding.comment, "Add a magic link.")
        await store.send(.blockTapped(blocks[2]))
        XCTAssertEqual(store.state.picked, blocks[2])
        XCTAssertEqual(store.state.comment, "Add a magic link.")
    }

    func testTappingThePickedBlockClosesOnlyAnEmptyComment() async throws {
        let blocks = MarkdownBlock.blocks(spec.components(separatedBy: "\n"))
        let store = store(InMemoryDaemon())

        await store.send(.blockTapped(blocks[1]))
        await store.send(\.binding.comment, "Add a magic link.")
        await store.send(.blockTapped(blocks[1]))
        XCTAssertEqual(store.state.picked, blocks[1])
        XCTAssertEqual(store.state.comment, "Add a magic link.")

        await store.send(\.binding.comment, "  ")
        await store.send(.blockTapped(blocks[1]))
        XCTAssertNil(store.state.picked)
    }
}
