import Foundation

// Given/then tests as a plain binary. swift-testing isn't in the static Linux
// SDK, and rules_swift runs macOS tests through Xcode's xctest, so each test
// binary lists its cases and calls `runSuites`, which exits non-zero when a
// check fails.

/// One given/then check.
public struct TestCase {
    let name: String
    let body: () throws -> Void

    public init(_ name: String, _ body: @escaping () throws -> Void) {
        self.name = name
        self.body = body
    }
}

struct Failure: Error, CustomStringConvertible {
    let message: String
    let file: StaticString
    let line: UInt

    var description: String { "\(file):\(line): \(message)" }
}

public func expect(
    _ condition: Bool, _ message: @autoclosure () -> String = "expectation failed",
    file: StaticString = #filePath, line: UInt = #line
) throws {
    if !condition { throw Failure(message: message(), file: file, line: line) }
}

public func expectEqual<T: Equatable>(
    _ actual: T, _ expected: T,
    file: StaticString = #filePath, line: UInt = #line
) throws {
    if actual != expected {
        throw Failure(message: "got \(actual), expected \(expected)", file: file, line: line)
    }
}

extension Optional {
    /// The value, or a failed check when there's none.
    public func unwrap(
        _ message: @autoclosure () -> String = "expected a value", file: StaticString = #filePath, line: UInt = #line
    ) throws -> Wrapped {
        guard let self else { throw Failure(message: message(), file: file, line: line) }
        return self
    }
}

/// Checks that `body` throws `expected`.
public func expectError<E: Error & Equatable, T>(
    _ expected: E, file: StaticString = #filePath, line: UInt = #line,
    _ body: () throws -> T
) throws {
    let value: T
    do {
        value = try body()
    } catch let error as E {
        return try expectEqual(error, expected, file: file, line: line)
    } catch {
        throw Failure(message: "got \(error), expected \(expected)", file: file, line: line)
    }
    throw Failure(message: "got \(value), expected error \(expected)", file: file, line: line)
}

/// Runs every case, prints PASS or FAIL for each, and exits.
public func runSuites(_ suites: [(String, [TestCase])]) -> Never {
    var failures = 0
    for (suite, tests) in suites {
        for test in tests {
            do {
                try test.body()
                print("PASS \(suite): \(test.name)")
            } catch {
                failures += 1
                print("FAIL \(suite): \(test.name)\n  \(error)")
            }
        }
    }
    print(failures == 0 ? "All tests passed." : "\(failures) test(s) failed.")
    exit(failures == 0 ? 0 : 1)
}
