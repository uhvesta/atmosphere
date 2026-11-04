import Foundation
import LifrAPI
import LifrCLI
import LifrCore
import XCTest

/// The app as the CLI sees it: the API's golden fixture behind a grant.
final class FixtureConnection: Connection {
    let fixture: Fixture
    var grant: Grant?
    var confirm: (Confirmation) -> Bool = { _ in true }
    /// The params of each request, in order.
    private(set) var requests: [JSONValue] = []
    /// Files `lifr` can read, by path.
    var files: [String: String] = [:]

    init(grant: (Fixture) -> Grant? = { $0.agent }) throws {
        fixture = try Fixture()
        self.grant = grant(fixture)
    }

    func call(_ method: APIMethod, _ params: JSONValue) throws(APIError) -> JSONValue {
        requests.append(params)
        let response = fixture.call(method.rawValue, params, grant: grant, confirm: confirm).response
        if let error = response.error { throw error.data }
        return response.result ?? .null
    }
}

/// What a run of `lifr` printed.
struct Run: Equatable {
    var status: Int32
    var output: String
    var error: String
}

extension FixtureConnection {
    func run(_ arguments: String...) -> Run { run(arguments) }

    func run(_ arguments: [String]) -> Run {
        var output = ""
        var error = ""
        let cli = CLI(
            connect: { self }, standardOutput: { output += $0 }, standardError: { error += $0 },
            timeZone: TimeZone(identifier: "UTC")!,
            readFile: { path in
                guard let text = self.files[path] else { throw CocoaError(.fileReadNoSuchFile) }
                return text
            })
        let status = cli.run(arguments)
        return Run(status: status, output: output, error: error)
    }
}

/// A golden file from //apps/lifr/api, copied into the test bundle.
func golden(_ name: String) throws -> JSONValue {
    let bundle = Bundle.allBundles.first { $0.bundleURL.pathExtension == "xctest" }!
    let url = try XCTUnwrap(bundle.url(forResource: name, withExtension: "json"), "\(name).json isn't in the bundle")
    return try Wire.decoder.decode(JSONValue.self, from: Data(contentsOf: url))
}

func json(_ text: String) throws -> JSONValue {
    try Wire.decoder.decode(JSONValue.self, from: Data(text.utf8))
}

extension JSONValue {
    subscript(key: String) -> JSONValue? {
        if case .object(let fields) = self { fields[key] } else { nil }
    }

    var array: [JSONValue] {
        if case .array(let values) = self { values } else { [] }
    }
}
