import Foundation
import InferenceKit
import InferenceKitLive
import InferenceServerKit
import JSONKit

/// Humata's model server on its own, as the app runs it: for the
/// end-to-end tests, with a scripted model, and by hand.
///
///     humata-model-server [--port N] [--script steps.json] [--apple]
///
/// `--script` serves "scripted", which answers each request with the next
/// step of a JSON array: `{"answer": "text"}`, or `{"call": "write_file",
/// "arguments": {…}}`. `--apple` serves Apple's on-device model as "apple".
/// The port it listens on is printed on the first line.
@main
enum ModelServerMain {
    static func main() async {
        do {
            let options = try Options(CommandLine.arguments.dropFirst())
            var models: [ServedModel] = []
            if let script = options.script { models.append(ServedModel(id: "scripted", runtime: try scripted(script))) }
            if options.apple { models.append(ServedModel(id: "apple", runtime: AppleRuntime())) }
            let served = models
            let server = ModelServer { served }
            let port = try await server.start(port: options.port)
            print(port)
            fflush(stdout)
            while true { try await Task.sleep(for: .seconds(3600)) }
        } catch {
            FileHandle.standardError.write(Data("humata-model-server: \(error)\n".utf8))
            exit(1)
        }
    }

    /// A model that follows the script in `file`.
    static func scripted(_ file: URL) throws -> ScriptedRuntime {
        let steps = try Wire.decoder.decode([Step].self, from: Data(contentsOf: file))
        return ScriptedRuntime(
            steps: steps.map { step in
                if let name = step.call {
                    ScriptedRuntime.call(name, step.arguments ?? [:], then: ScriptedRuntime.answer(""))
                } else {
                    ScriptedRuntime.answer(.string(step.answer ?? ""))
                }
            })
    }

    struct Step: Decodable {
        var answer: String?
        var call: String?
        var arguments: JSONValue?
    }
}

struct Options {
    var port: UInt16 = 0
    var script: URL?
    var apple = false

    init(_ arguments: ArraySlice<String>) throws {
        var rest = arguments
        while let argument = rest.popFirst() {
            switch argument {
            case "--port":
                guard let value = rest.popFirst().flatMap(UInt16.init) else { throw OptionsError.port }
                port = value
            case "--script":
                guard let path = rest.popFirst() else { throw OptionsError.script }
                script = URL(filePath: path)
            case "--apple":
                apple = true
            default:
                throw OptionsError.unknown(argument)
            }
        }
        if script == nil, !apple { throw OptionsError.noModels }
    }
}

enum OptionsError: Error, CustomStringConvertible {
    case port
    case script
    case noModels
    case unknown(String)

    var description: String {
        switch self {
        case .port: "--port needs a number from 0 to 65535; 0 picks a free port."
        case .script: "--script needs the path of a JSON array of steps."
        case .noModels: "Give it a model to serve: --script steps.json, --apple, or both."
        case .unknown(let argument): "\(argument) isn't an option. Use --port, --script or --apple."
        }
    }
}
