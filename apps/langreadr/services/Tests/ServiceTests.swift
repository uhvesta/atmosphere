import Foundation
import LangReadrCore
import LangReadrServices
import TestingKit

func serviceTests() -> [TestCase] {
    [
        TestCase("given Japanese text, then it's told as Japanese") {
            try expectEqual(ReaderServices.live.detectLanguage("今日はとても良い天気ですね。散歩に行きましょう。")?.base, "ja")
        },
        TestCase("given Chinese without spaces, then its words are found, each where it is") {
            let words = ReaderServices.live.words("我是学生", Language("zh-Hans"))
            try expect(words.map(\.text).contains("学生"))
            let student = try words.first { $0.text == "学生" }.unwrap()
            try expectEqual(student.start, 2)
            try expectEqual(student.end, 4)
        },
        TestCase("given Cyrillic, then the transform writes it in Latin letters; Latin text needs none") {
            try expectEqual(ReaderServices.live.transliterate("Привет"), "Privet")
            try expectEqual(ReaderServices.live.transliterate("Hola"), nil)
        },
        TestCase("given a link without its scheme, then it's read as https") {
            try expectEqual(
                try ReaderServices.link(" example.com/a "), try URL(string: "https://example.com/a").unwrap())
        },
        TestCase("given words that aren't a link, then they're refused saying so") {
            try expectError(PageError.notALink("hello there")) { try ReaderServices.link("hello there") }
        },
    ]
}
