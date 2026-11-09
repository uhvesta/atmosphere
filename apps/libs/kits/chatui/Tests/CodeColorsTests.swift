import TestingKit

@testable import ChatUI

func codeColorsTests() -> [TestCase] {
    [
        TestCase("given Swift, then its keywords, strings, numbers and comments") {
            let runs = try CodeSyntax.of("swift").unwrap().runs("let name = \"Soil\" // 2 bags\nreturn 42")
            try expectEqual(
                runs.compactMap { run in run.token.map { "\($0):\(run.text)" } },
                ["keyword:let", "string:\"Soil\"", "comment:// 2 bags", "keyword:return", "number:42"])
        },
        TestCase("given runs, then together they're the code") {
            let code = "def plant(seeds):\n    # water them\n    return f'{seeds} planted' /* not a comment */"
            let runs = try CodeSyntax.of("python").unwrap().runs(code)
            try expectEqual(runs.map(\.text).joined(), code)
        },
        TestCase("given an escaped quote, then the string goes on past it") {
            let runs = try CodeSyntax.of("js").unwrap().runs(#"say("a \"b\" c")"#)
            try expectEqual(runs.filter { $0.token == .string }.map(\.text), [#""a \"b\" c""#])
        },
        TestCase("given a quote not closed, then the string ends with its line") {
            let runs = try CodeSyntax.of("c").unwrap().runs("x = 'a\ny = 1")
            try expectEqual(runs.filter { $0.token == .string }.map(\.text), ["'a"])
            try expectEqual(runs.filter { $0.token == .number }.map(\.text), ["1"])
        },
        TestCase("given SQL, then keywords in any case") {
            let runs = try CodeSyntax.of("SQL").unwrap().runs("SELECT name from plants -- all")
            try expectEqual(runs.filter { $0.token != nil }.map(\.text), ["SELECT", "from", "-- all"])
        },
        TestCase("given a digit inside a word, then it's not a number") {
            let runs = try CodeSyntax.of("go").unwrap().runs("var bed2 = x1")
            try expectEqual(runs.filter { $0.token == .number }.map(\.text), [])
        },
        TestCase("given no language or plain text, then nothing to color") {
            try expect(CodeSyntax.of(nil) == nil)
            try expect(CodeSyntax.of("text") == nil)
        },
    ]
}
