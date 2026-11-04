import Foundation
import LifrTesting

@testable import LifrBlob

private struct Note: Codable, Equatable {
    var title: String
    var lines: [String]
}

private let long = Note(title: "Week", lines: Array(repeating: "Water the plants and pay rent", count: 200))

/// The error `body` throws, or a failure when it doesn't throw one.
private func blobError(_ body: () throws -> Void) throws -> BlobError {
    do {
        try body()
    } catch let error as BlobError {
        return error
    }
    try expect(false, "expected a BlobError")
    fatalError("unreachable")
}

func blobTests() -> [TestCase] {
    [
        TestCase("given a value, then it reads back from the current format") {
            let data = try Blob.encode(long)
            try expectEqual(try Blob.decode(Note.self, from: data), long)
            try expectEqual(try Blob.format(of: data), .current)
        },
        TestCase("given repetitive JSON, then the blob is much smaller than the JSON") {
            let json = try JSONEncoder().encode(long)
            try expect(try Blob.encode(long).count * 10 < json.count)
        },
        TestCase("given a value too small to compress, then it's kept plain and still reads back") {
            let tiny = Note(title: "A", lines: [])
            let data = try Blob.encode(tiny)
            try expectEqual(try Blob.format(of: data).codec, .none)
            try expectEqual(try Blob.decode(Note.self, from: data), tiny)
        },
        TestCase("given plain JSON written before blobs had a header, then it reads as legacy") {
            let object = try JSONEncoder().encode(long)
            try expectEqual(try Blob.format(of: object), .legacy)
            try expectEqual(try Blob.decode(Note.self, from: object), long)
            let array = try JSONEncoder().encode([long])
            try expectEqual(try Blob.decode([Note].self, from: array), [long])
        },
        TestCase("given an uncompressed blob, then it reads the same as a compressed one") {
            let data = try Blob.encode(long, as: BlobFormat(encoding: .json, codec: .none))
            try expectEqual(try Blob.format(of: data).codec, .none)
            try expectEqual(try Blob.decode(Note.self, from: data), long)
        },
        TestCase("given a blob from a newer version, then reading it says which part is unknown") {
            let encoding = try blobError { _ = try Blob.decode(Note.self, from: Data([76, 66, 2, 0, 0, 123])) }
            guard case .unknownEncoding(2) = encoding else { return try expect(false, "\(encoding)") }
            let codec = try blobError { _ = try Blob.decode(Note.self, from: Data([76, 66, 1, 9, 0, 123])) }
            guard case .unknownCodec(9) = codec else { return try expect(false, "\(codec)") }
            let dictionary = try blobError { _ = try Blob.decode(Note.self, from: Data([76, 66, 1, 1, 0x81, 0x01])) }
            guard case .unknownDictionary(129) = dictionary else { return try expect(false, "\(dictionary)") }
        },
        TestCase("given damaged bytes, then reading fails instead of returning a value") {
            var data = try Blob.encode(long)
            data.replaceSubrange(8..<16, with: Data(repeating: 0xFF, count: 8))
            let damaged = try blobError { _ = try Blob.decode(Note.self, from: data) }
            guard case .decompressFailed = damaged else { return try expect(false, "\(damaged)") }
            for bytes in [Data(), Data([76]), Data([76, 66, 1]), Data([1, 2, 3, 4]), Data([76, 66, 1, 0, 0x80])] {
                let error = try blobError { _ = try Blob.decode(Note.self, from: bytes) }
                guard case .unreadable = error else { return try expect(false, "\(error)") }
            }
        },
        TestCase("given a blob of another type, then reading it fails to decode") {
            let data = try Blob.encode(long)
            let error = try blobError { _ = try Blob.decode([Int].self, from: data) }
            guard case .decodeFailed = error else { return try expect(false, "\(error)") }
        },
        TestCase("given varints, then each one reads back as written") {
            for value: UInt64 in [0, 1, 127, 128, 300, 1 << 35, .max] {
                var data = Data()
                Blob.appendVarint(value, to: &data)
                var index = 0
                try expectEqual(Blob.readVarint([UInt8](data), at: &index), value)
                try expectEqual(index, data.count)
            }
        },
    ]
}
