import Foundation
import LifrAPI
import LifrTesting

func wireTests() -> [TestCase] {
    [
        TestCase("given a line that isn't JSON, then parseError with no id") {
            let fixture = try Fixture()
            let response = fixture.call(line: "list_avenues", grant: fixture.agent)
            try expectEqual(response.code, .parseError)
            try expectEqual(response.id, nil)
        },
        TestCase("given JSON that isn't a request, then invalidRequest keeps the id") {
            let fixture = try Fixture()
            let response = fixture.call(line: #"{"id": 7, "method": "list_avenues"}"#, grant: fixture.agent)
            try expectEqual(response.code, .invalidRequest)
            try expectEqual(response.id, 7)
        },
        TestCase("given a valid line, then the response answers the same id") {
            let fixture = try Fixture()
            let response = fixture.call(line: #"{"jsonrpc": "2.0", "id": 3, "method": "list_avenues"}"#,
                                        grant: fixture.agent)
            try expectEqual(response.id, 3)
            try expect(response.error == nil, "list_avenues succeeds")
        },
        TestCase("given a message with newlines in a string, then it's still one line") {
            let line = try Wire.line(["body": "first\nsecond"] as JSONValue)
            try expectEqual(line.filter { $0 == UInt8(ascii: "\n") }.count, 1)
            try expectEqual(line.last, UInt8(ascii: "\n"))
        },
        TestCase("given whole and fractional numbers, then they round-trip as written") {
            let value: JSONValue = ["count": 3, "ratio": .number(0.5), "nested": [true, .null, "x"]]
            let data = try Wire.encoder.encode(value)
            try expectEqual(String(decoding: data, as: UTF8.self),
                            #"{"count":3,"nested":[true,null,"x"],"ratio":0.5}"#)
            try expectEqual(try Wire.decoder.decode(JSONValue.self, from: data), value)
        },
    ]
}
