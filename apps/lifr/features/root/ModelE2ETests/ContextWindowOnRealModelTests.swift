import Darwin
import InferenceKitLiteRT
import LifrAPI
import LifrModelsClient
import LifrModelsLive
import XCTest

/// Gemma 4 E2B at the context windows Settings offers, on this Mac's GPU:
/// each loads, and at the largest it reads a prompt longer than the
/// default window. Prints the memory each window took, the figures the
/// catalog's estimate comes from.
@MainActor
final class ContextWindowOnRealModelTests: XCTestCase {
    func testTheLargestWindowReadsALongPrompt() async throws {
        var model = try RealModel.installedModel()
        let windows = ModelCatalog.gemma4E2B.contextChoices
        let largest = try XCTUnwrap(windows.max(), "Gemma 4 E2B offers no windows")
        for window in windows.sorted() {
            model.chosenWindow = window
            let before = Self.footprint()
            let engine = try await LiteRTLMEngine.load(model, on: .gpu, greedy: true)
            let chat = try await engine.chat(instructions: "Answer in one word.", constrained: false)
            // About 1.3 tokens a word: past the default window at the
            // largest, a short prompt at the others.
            let filler = window == largest ? String(repeating: "The sky is blue today. ", count: 2_400) : ""
            let answer = try await chat.send(
                filler + "What colour is the sky?", attachments: [], format: nil, thinkingTokens: nil, thought: nil,
                partial: { _ in })
            XCTAssertFalse(answer.isEmpty, "No answer at \(window) tokens")
            print("Window \(window): \((Self.footprint() - before) / 1_000_000) MB")
        }
    }

    /// The memory this process takes, as the system counts it.
    static func footprint() -> Int64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? Int64(info.phys_footprint) : 0
    }
}
