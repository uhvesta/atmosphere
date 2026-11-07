import Foundation
import LifrAPI
import LifrCore
import TestingKit

extension RPCResponse {
    /// The result as `type`, or a failure naming the error.
    func value<T: Decodable>(_ type: T.Type, file: StaticString = #filePath, line: UInt = #line) throws -> T {
        if let error {
            try expect(false, "got \(error.data.code): \(error.message)", file: file, line: line)
        }
        return try (result ?? .null).decode(type)
    }

    var code: APIErrorCode? { error?.data.code }
}
