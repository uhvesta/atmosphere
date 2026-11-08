import CaptureKit
import Foundation
import TestingKit

func fileTests() -> [TestCase] {
    let folder = FileManager.default.temporaryDirectory.appending(path: "CaptureTests-\(UUID().uuidString)")
    func write(_ data: Data, as name: String) throws -> URL {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let url = folder.appending(path: name)
        try data.write(to: url)
        return url
    }
    return [
        TestCase("given a Markdown file, then its text is read, trimmed") {
            let url = try write(Data("\n# Plan\n\nWater the fern.\n".utf8), as: "Plan.md")
            try expectEqual(
                try PickedFile.read(url, limit: 100),
                .text(try FileText(name: "Plan.md", text: "# Plan\n\nWater the fern.", limit: 100)))
        },
        TestCase("given text past the limit, then the start is kept and it's marked cut") {
            let url = try write(Data("abcdefghij".utf8), as: "Long.txt")
            let text = try PickedFile.read(url, limit: 4).text.unwrap("not text")
            try expectEqual(text.text, "abcd")
            try expect(text.isCut)
        },
        TestCase("given UTF-16 with its byte order mark, then it's read") {
            let url = try write(Data([0xFF, 0xFE]) + ("héllo".data(using: .utf16LittleEndian) ?? Data()), as: "U.txt")
            let text = try PickedFile.read(url, limit: 100).text.unwrap("not text")
            try expectEqual(text.text, "héllo")
        },
        TestCase("given bytes that aren't text, then it says so") {
            let url = try write(Data([0xC3, 0x28, 0xFF]), as: "Bin.txt")
            try expectError(FileError.notText(name: "Bin.txt")) { try PickedFile.read(url, limit: 100) }
        },
        TestCase("given only whitespace, then it's empty") {
            let url = try write(Data("  \n ".utf8), as: "Blank.txt")
            try expectError(FileError.empty(name: "Blank.txt")) { try PickedFile.read(url, limit: 100) }
        },
        TestCase("given a photo, then its bytes are kept as a photo") {
            let url = try write(Data([1, 2, 3]), as: "Fern.jpg")
            try expectEqual(try PickedFile.read(url, limit: 100), .photo(Data([1, 2, 3])))
        },
        TestCase("given a model that doesn't read photos, then photos aren't offered") {
            try expect(!PickedFile.types(photos: false).contains(.image))
            try expect(PickedFile.types(photos: true).contains(.image))
        },
    ]
}

extension PickedFile {
    var text: FileText? {
        guard case .text(let text) = self else { return nil }
        return text
    }
}

await runSuites([("Files", fileTests())])
