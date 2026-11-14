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

    func testALongBlockIsQuotedCutAndAShortOneWhole() throws {
        let long = (1...40).map { "Step \($0) of the long plan." }
        let source = ["# Login", ""] + long + ["", "Remember me for a month."]
        let blocks = MarkdownBlock.blocks(source)
        let plan = try XCTUnwrap(blocks.first { $0.text.contains("Step 40 of the long plan.") })
        let short = try XCTUnwrap(blocks.first { $0.text == "Remember me for a month." })
        var state = SpecCommentsFeature.State(title: "Login")
        state.drafts = [
            SpecCommentsFeature.Draft(id: UUID(), block: plan, text: "Too many steps."),
            SpecCommentsFeature.Draft(id: UUID(), block: short, text: "A week is enough."),
        ]

        let message = state.message
        XCTAssertLessThan(message.count, plan.text.count)
        XCTAssertTrue(message.contains("Step 1 of the long plan."))
        XCTAssertFalse(message.contains("Step 40 of the long plan."))
        XCTAssertTrue(message.contains("Remember me for a month."))
        XCTAssertTrue(message.contains("Too many steps."))
        XCTAssertTrue(message.contains("A week is enough."))
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
