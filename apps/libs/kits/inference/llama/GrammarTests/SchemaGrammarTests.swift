import XCTest

@testable import InferenceKitGrammar

final class SchemaGrammarTests: XCTestCase {
    private func grammar(_ schema: String, file: StaticString = #filePath, line: UInt = #line) throws -> GBNF {
        try GBNF(SchemaGrammar.gbnf(for: schema))
    }

    func testAnObjectTakesItsPropertiesInOrderAndNothingElse() throws {
        let task = try grammar(
            #"""
            {"type":"object","properties":{"title":{"type":"string","description":"What to do"},
            "done":{"type":"boolean"}},"required":["title","done"],"additionalProperties":false}
            """#)
        XCTAssertTrue(task.accepts(#"{"title":"Buy milk","done":false}"#))
        XCTAssertTrue(task.accepts(" {\n  \"title\": \"Buy \\\"oat\\\" milk\",\n  \"done\": true\n}\n"))
        XCTAssertFalse(task.accepts(#"{"done":false,"title":"Buy milk"}"#))
        XCTAssertFalse(task.accepts(#"{"title":"Buy milk"}"#))
        XCTAssertFalse(task.accepts(#"{"title":"Buy milk","done":false,"extra":1}"#))
        XCTAssertFalse(task.accepts(#"{"title":3,"done":false}"#))
        XCTAssertFalse(task.accepts(#"{"title":"Buy milk","done":false"#))
    }

    func testAnOptionalPropertyCanBeLeftOutWithItsComma() throws {
        let schema = try grammar(
            #"""
            {"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"},"c":{"type":"string"}},
            "required":["b"]}
            """#)
        for accepted in [#"{"b":""}"#, #"{"a":"","b":""}"#, #"{"b":"","c":""}"#, #"{"a":"","b":"","c":""}"#] {
            XCTAssertTrue(schema.accepts(accepted), accepted)
        }
        for rejected in ["{}", #"{"a":""}"#, #"{,"b":""}"#, #"{"b":"",}"#, #"{"a":"",,"b":""}"#, #"{"a":"""b":""}"#] {
            XCTAssertFalse(schema.accepts(rejected), rejected)
        }
    }

    func testAnObjectWhosePropertiesAreAllOptionalCanBeEmpty() throws {
        let schema = try grammar(#"{"type":"object","properties":{"a":{"type":"boolean"},"b":{"type":"null"}}}"#)
        for accepted in ["{}", "{ }", #"{"a":true}"#, #"{"b":null}"#, #"{"a":false,"b":null}"#] {
            XCTAssertTrue(schema.accepts(accepted), accepted)
        }
        XCTAssertFalse(schema.accepts(#"{"b":null,"a":true}"#))
        XCTAssertFalse(schema.accepts(#"{,}"#))
    }

    func testAnEnumAndAConstTakeOnlyTheirValues() throws {
        let choice = try grammar(#"{"type":"string","enum":["low","high","say \"hi\""]}"#)
        XCTAssertTrue(choice.accepts(#""low""#))
        XCTAssertTrue(choice.accepts(#""say \"hi\"""#))
        XCTAssertFalse(choice.accepts(#""medium""#))
        let constant = try grammar(#"{"const":"add_task"}"#)
        XCTAssertTrue(constant.accepts(#""add_task""#))
        XCTAssertFalse(constant.accepts(#""add""#))
    }

    func testAnyOfTakesEachChoice() throws {
        let step = try grammar(
            #"""
            {"anyOf":[
            {"type":"object","properties":{"tool":{"const":"search"},"arguments":{"type":"object","properties":
            {"query":{"type":"string"}},"required":["query"],"additionalProperties":false}},
            "required":["tool","arguments"],"additionalProperties":false},
            {"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],
            "additionalProperties":false}]}
            """#)
        XCTAssertTrue(step.accepts(#"{"tool":"search","arguments":{"query":"rent"}}"#))
        XCTAssertTrue(step.accepts(#"{"answer":"Done."}"#))
        XCTAssertFalse(step.accepts(#"{"tool":"delete","arguments":{"query":"rent"}}"#))
        XCTAssertFalse(step.accepts(#"{"tool":"search","answer":"Done."}"#))
    }

    func testAnArrayTakesAnyNumberOfItsItems() throws {
        let list = try grammar(#"{"type":"array","items":{"type":"string","enum":["a","b"]}}"#)
        for accepted in ["[]", #"["a"]"#, #"[ "a", "b", "a" ]"#] {
            XCTAssertTrue(list.accepts(accepted), accepted)
        }
        for rejected in [#"["c"]"#, #"["a",]"#, #"[,"a"]"#, #""a""#] {
            XCTAssertFalse(list.accepts(rejected), rejected)
        }
    }

    func testNumbersIntegersAndUnionsOfTypes() throws {
        let number = try grammar(#"{"type":"number"}"#)
        for accepted in ["0", "-12", "3.25", "1e9", "-0.5E-3"] { XCTAssertTrue(number.accepts(accepted), accepted) }
        for rejected in ["01", "1.", ".5", "+1", "NaN"] { XCTAssertFalse(number.accepts(rejected), rejected) }
        let integer = try grammar(#"{"type":"integer"}"#)
        XCTAssertTrue(integer.accepts("42"))
        XCTAssertFalse(integer.accepts("4.2"))
        let maybe = try grammar(#"{"type":["string","null"]}"#)
        XCTAssertTrue(maybe.accepts("null"))
        XCTAssertTrue(maybe.accepts(#""x""#))
        XCTAssertFalse(maybe.accepts("false"))
    }

    func testAStringTakesEscapesButNotRawControlCharacters() throws {
        let string = try grammar(#"{"type":"string"}"#)
        XCTAssertTrue(string.accepts(#""line\nnext é \\ \/""#))
        XCTAssertTrue(string.accepts("\"café 日本\""))
        XCTAssertFalse(string.accepts("\"line\nnext\""))
        XCTAssertFalse(string.accepts(#""\q""#))
    }

    func testASchemaWithoutConstraintsTakesAnyJSON() throws {
        let any = try grammar("{}")
        for accepted in ["null", "[1,{\"a\":[true]}]", #"{"x":"y"}"#] { XCTAssertTrue(any.accepts(accepted), accepted) }
        XCTAssertFalse(any.accepts("{x}"))
    }

    func testPropertiesWithTheSameNameInDifferentObjectsKeepTheirOwnSchemas() throws {
        let schema = try grammar(
            #"""
            {"type":"object","properties":{"a":{"type":"object","properties":{"name":{"type":"boolean"}},
            "required":["name"]},"name":{"type":"string"}},"required":["a","name"]}
            """#)
        XCTAssertTrue(schema.accepts(#"{"a":{"name":true},"name":"x"}"#))
        XCTAssertFalse(schema.accepts(#"{"a":{"name":"x"},"name":true}"#))
    }

    func testAPropertyNamedLikeARuleStillWorks() throws {
        let schema = try grammar(
            #"""
            {"type":"object","properties":{"root":{"type":"string"},"ws":{"type":"boolean"},
            "日付":{"type":"null"}},"required":["root","ws","日付"]}
            """#)
        XCTAssertTrue(schema.accepts(#"{"root":"x","ws":true,"日付":null}"#))
    }

    func testASchemaItCannotHoldAModelToSaysWhichKeyword() {
        XCTAssertThrowsError(try SchemaGrammar.gbnf(for: #"{"type":"string","pattern":"^a+$"}"#)) {
            XCTAssertEqual($0 as? SchemaGrammarError, .unsupported(keyword: "pattern"))
        }
        XCTAssertThrowsError(try SchemaGrammar.gbnf(for: #"{"type":"thing"}"#)) {
            XCTAssertEqual($0 as? SchemaGrammarError, .invalid(keyword: "type"))
        }
        XCTAssertThrowsError(try SchemaGrammar.gbnf(for: #"{"type":"string""#)) {
            guard case .notJSON? = $0 as? SchemaGrammarError else { return XCTFail("\($0)") }
        }
        XCTAssertThrowsError(try SchemaGrammar.gbnf(for: String(repeating: "[", count: 500))) {
            guard case .notJSON? = $0 as? SchemaGrammarError else { return XCTFail("\($0)") }
        }
    }
}
