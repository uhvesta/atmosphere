import Foundation
import LangReadrCore
import TestingKit

func sampleBook() -> Book {
    Book(
        title: "Notes", language: Language("zh-Hans"), source: .text, importedAt: Date(timeIntervalSince1970: 0),
        chapters: [Chapter(paragraphs: [Paragraph(text: "我是学生。"), Paragraph(text: "I live here.")])])
}

func bookTests() -> [TestCase] {
    [
        TestCase("given a language with a script, then its base drops the script") {
            try expectEqual(Language("zh-Hans").base, "zh")
        },
        TestCase("given Han characters, then each counts as a word") {
            let book = sampleBook()
            try expectEqual(book.wordCount, 4 + 3)
            try expectEqual(book.minutes, 1)
        },
        TestCase("given a new language to translate into, then translations go and transliterations stay") {
            var book = sampleBook()
            let first = try book.paragraphs.first.unwrap().id
            book.translate(into: Language("en"))
            book.annotate(first, translation: "I am a student.", transliteration: "wǒ shì xuéshēng.")
            book.translate(into: Language("de"))
            try expectEqual(book.annotations[first], Annotation(transliteration: "wǒ shì xuéshēng."))
        },
        TestCase("given the same language again, then translations stay") {
            var book = sampleBook()
            let first = try book.paragraphs.first.unwrap().id
            book.translate(into: Language("en"))
            book.annotate(first, translation: "I am a student.")
            book.translate(into: Language("en"))
            try expectEqual(book.annotations[first]?.translation, "I am a student.")
        },
        TestCase("given a passage picked backwards, then it reads forwards, trimmed") {
            let paragraph = Paragraph(text: "Hola, buenos días.")
            let passage = try Passage(in: paragraph, from: 17, to: 5).unwrap()
            try expectEqual(passage.text, "buenos días")
            try expect(passage.contains(6) && !passage.contains(17))
        },
        TestCase("given a passage outside its paragraph or of only spaces, then there's none") {
            let paragraph = Paragraph(text: "a  b")
            try expect(Passage(in: paragraph, from: 0, to: 9) == nil)
            try expect(Passage(in: paragraph, from: 1, to: 3) == nil)
        },
        TestCase("given a chat about a passage already asked about, then it's the same chat") {
            var book = sampleBook()
            let paragraph = try book.paragraphs.first.unwrap()
            let passage = try Passage(in: paragraph, from: 0, to: 2).unwrap()
            let first = book.chat(about: passage, at: Date())
            let again = book.chat(about: passage, at: Date())
            try expectEqual(first.id, again.id)
            try expectEqual(book.chats(in: paragraph.id).count, 1)
        },
        TestCase("given a book with notes and chats, then it reads back as it was saved") {
            var book = sampleBook()
            let paragraph = try book.paragraphs.first.unwrap()
            book.words[WordKey(paragraph: paragraph.id, word: "学生")] = WordNote(
                word: "学生", transliteration: "xuéshēng", meaning: "student", inSentence: "I am one")
            var chat = book.chat(about: try Passage(in: paragraph, from: 2, to: 4).unwrap(), at: Date())
            chat.conversation.messages.append(ReaderMessage(role: .person, text: "why?", at: Date()))
            book.save(chat)
            let data = try JSONEncoder().encode(book)
            try expectEqual(try JSONDecoder().decode(Book.self, from: data), book)
        },
        TestCase("given Latin text with accents, then it needs no transliteration, and Cyrillic does") {
            try expect(Script.isLatin("Mañana, ça va? 123"))
            try expect(!Script.isLatin("Привет"))
        },
        TestCase("given Japanese, then the model writes its readings; Russian uses the transform") {
            try expectEqual(Script.transliteration(for: Language("ja")), .model)
            try expectEqual(Script.transliteration(for: Language("ru")), .transform)
        },
    ]
}
