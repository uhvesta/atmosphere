import InferenceKitLiteRT
import LifrModelsClient
import XCTest

/// Gemma 4 E2B writing the same answer with each of its profile's speedups,
/// on this Mac's GPU and CPU, taking the likeliest token each time. Prints
/// how fast each wrote and the memory it took, the figures the catalog's
/// profile comes from.
@MainActor
final class SpeedOnRealModelTests: XCTestCase {
    func testEachSpeculationWritesTheAnswer() async throws {
        let model = try RealModel.installedModel()
        let prompt = "List the planets of the solar system in order, with one sentence about each."
        for backend in [ModelBackend.gpu, .cpu] {
            for speculation in [ModelProfile.Speculation.off, .adaptive, .on] {
                var profile = model.spec.profile
                profile.speculation = speculation
                profile.sampling = .greedy
                let before = ContextWindowOnRealModelTests.footprint()
                let engine = try await LiteRTLMEngine.load(model, on: backend, profile: profile)
                let chat = try await engine.chat(instructions: "Answer plainly.", constrained: false)
                // The first turn compiles the GPU's programs; the second is timed.
                _ = try await chat.send(
                    "Say hi.", attachments: [], format: nil, thinkingTokens: nil, thought: nil
                ) { _ in }
                let clock = ContinuousClock()
                let start = clock.now
                let answer = try await chat.send(
                    prompt, attachments: [], format: nil, thinkingTokens: nil, thought: nil
                ) { _ in }
                let seconds = (clock.now - start) / .seconds(1)
                XCTAssertFalse(answer.isEmpty, "No answer on the \(backend) with speculation \(speculation)")
                let rate = Double(answer.count) / max(seconds, 0.001)
                let megabytes = (ContextWindowOnRealModelTests.footprint() - before) / 1_000_000
                print("\(backend) \(speculation): \(Int(rate)) characters/s, \(megabytes) MB")
            }
        }
    }
}
