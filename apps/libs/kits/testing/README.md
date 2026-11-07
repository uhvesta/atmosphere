# TestingKit

Given/then checks as a plain test binary. swift-testing isn't in the static
Linux SDK, and rules_swift runs macOS tests through Xcode's xctest, so the
kits that build with the hermetic toolchains test themselves with this
instead: each test binary lists its cases and calls `runSuites`, which prints
PASS or FAIL for each and exits non-zero when one fails.

## Use it

```starlark
swift_binary(
    name = "notes_tests",
    srcs = glob(["Tests/*.swift"]),
    deps = [":notes", "//apps/libs/kits/testing"],
)

sh_test(
    name = "notes_test",
    srcs = ["//apps/libs/kits/testing:run_tests.sh"],
    args = ["$(rootpath :notes_tests)"],
    data = [":notes_tests"],
)
```

```swift
// Tests/main.swift
import TestingKit

await runSuites([("Notes", noteTests())])

// Tests/NoteTests.swift
func noteTests() -> [TestCase] {
    [
        TestCase("a renamed note keeps its body") {
            var note = Note(title: "Draft", body: "Hi")
            note.rename("Final")
            try expectEqual(note.body, "Hi")
        },
        TestCase("an empty title is refused") {
            try expectError(NoteError.emptyTitle) { try Note(title: "", body: "") }
        },
        .async("the store saves a note") {
            let store = NoteStore()
            try await store.save(Note(title: "A", body: ""))
            try expect(await store.count == 1)
        },
    ]
}
```

Run it with `bazel test //pkg:notes_test`.

## What it gives you

- `TestCase(name) { … }` for a check, and `TestCase.async` for one that
  awaits.
- `expect(_:)`, `expectEqual(_:_:)` and `expectError(_:_:)`. A failure names
  the file and line.
- `optional.unwrap()` gives you the value, or fails the check when it's nil,
  so tests never force unwrap.
