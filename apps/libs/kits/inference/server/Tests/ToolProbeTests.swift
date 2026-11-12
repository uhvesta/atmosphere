import Foundation
import InferenceKit
import InferenceServerKit
import JSONKit
import XCTest

final class ToolProbeTests: XCTestCase {
    private var server: ModelServer?

    override func tearDown() {
        server?.stop()
        super.tearDown()
    }

    private func serve(_ models: [ServedModel]) async throws -> URL {
        let server = ModelServer { models }
        self.server = server
        let port = try await server.start()
        return try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/v1"))
    }

    private static let done = ScriptedRuntime.answer("Done")

    private func model(_ steps: [ScriptedRuntime.Step]) -> ServedModel {
        ServedModel(id: "m", runtime: ScriptedRuntime(steps: steps))
    }

    func testAModelThatDoesEachJobWithTheRightToolIsGood() async throws {
        let base = try await serve([
            model([
                ScriptedRuntime.call("read_file", ["path": "README.md"], then: Self.done),
                ScriptedRuntime.call("write_file", ["path": "notes.txt", "contents": "Hello"], then: Self.done),
                ScriptedRuntime.call("run_command", ["command": "make test"], then: Self.done),
            ])
        ])
        let verdict = await ToolProbe.run(base: base, model: "m")
        XCTAssertEqual(verdict, .good)
    }

    func testAModelThatAnswersInWordsIsPoor() async throws {
        let base = try await serve([model([ScriptedRuntime.answer("README.md says hello.")])])
        let verdict = await ToolProbe.run(base: base, model: "m")
        XCTAssertEqual(verdict, .poor)
    }

    func testAModelThatPicksTheWrongToolOrArgumentsIsPoor() async throws {
        let wrongTool = try await serve([
            model([ScriptedRuntime.call("run_command", ["command": "cat README.md"], then: Self.done)])
        ])
        let first = await ToolProbe.run(base: wrongTool, model: "m")
        XCTAssertEqual(first, .poor)
        server?.stop()

        let wrongArguments = try await serve([
            model([
                ScriptedRuntime.call("read_file", ["path": "README.md"], then: Self.done),
                ScriptedRuntime.call("write_file", ["path": "notes.txt"], then: Self.done),
            ])
        ])
        let second = await ToolProbe.run(base: wrongArguments, model: "m")
        XCTAssertEqual(second, .poor)
    }

    func testAModelTheServerDoesNotHaveIsNotJudged() async throws {
        let base = try await serve([model([])])
        let verdict = await ToolProbe.run(base: base, model: "other")
        XCTAssertNil(verdict)
    }

    func testTheListSaysWhatSetsAModelApart() async throws {
        let base = try await serve([
            ServedModel(id: "a", runtime: ScriptedRuntime(steps: []), detail: "Calls tools well"),
            ServedModel(id: "b", runtime: ScriptedRuntime(steps: [])),
        ])
        let (data, _) = try await URLSession.shared.data(from: base.appending(path: "models"))
        guard case .object(let listed) = try Wire.decoder.decode(JSONValue.self, from: data),
            case .array(let models)? = listed["data"]
        else { return XCTFail("no models listed") }
        let details = models.map { model -> JSONValue? in
            guard case .object(let fields) = model else { return nil }
            return fields["detail"]
        }
        XCTAssertEqual(details, [.string("Calls tools well"), nil])
    }
}
