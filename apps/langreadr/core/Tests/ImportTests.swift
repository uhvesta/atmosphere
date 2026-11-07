import Foundation
import LangReadrCore
import TestingKit

private func texts(_ draft: Draft) -> [String] { draft.chapters.flatMap(\.paragraphs).map(\.text) }

func importTests() -> [TestCase] {
    [
        TestCase("given paragraphs parted by blank lines, then each one's lines are joined") {
            let draft = try Importer.text("The cat sat\non the mat.\n\nIt slept.")
            try expectEqual(texts(draft), ["The cat sat on the mat.", "It slept."])
        },
        TestCase("given no blank lines, then each line is a paragraph") {
            let draft = try Importer.text("我是学生。\n我住在北京。")
            try expectEqual(texts(draft), ["我是学生。", "我住在北京。"])
        },
        TestCase("given lines of a script written without spaces, then they're joined without one") {
            let draft = try Importer.text("我是\n学生。\n\n好。")
            try expectEqual(texts(draft).first, "我是学生。")
        },
        TestCase("given headings, then each starts a chapter, and the first names the book") {
            let draft = try Importer.text("# Uno\nHola.\n# Dos\nAdiós.")
            try expectEqual(draft.chapters.map(\.title), ["Uno", "Dos"])
            try expectEqual(draft.title, "Uno")
        },
        TestCase("given no title or heading, then the first paragraph names the book, cut short") {
            let long = String(repeating: "palabra ", count: 20)
            let draft = try Importer.text(long)
            try expect(draft.title.count == 60 && draft.title.hasSuffix("…"))
        },
        TestCase("given only spaces, then the import fails as empty") {
            try expectError(ImportError.empty(.text)) { try Importer.text("  \n\n ") }
        },
        TestCase("given a page, then its headings and paragraphs are read and its scripts and menus aren't") {
            let html = """
                <html><head><title>Noticias &amp; más</title><style>p{}</style></head><body>
                <nav><a>Inicio</a></nav><h1>El tiempo</h1><p>Hoy hace <b>sol</b>.</p>
                <script>var x = "<p>no</p>";</script><p>Mañana&nbsp;llueve&#33;</p><footer>©</footer>
                </body></html>
                """
            let draft = try Importer.html(html, source: .web(URL(fileURLWithPath: "/a")))
            try expectEqual(draft.title, "Noticias & más")
            try expectEqual(draft.chapters.map(\.title), ["El tiempo"])
            try expectEqual(texts(draft), ["Hoy hace sol.", "Mañana llueve!"])
        },
        TestCase("given a page that marks its article, then only the article is read") {
            let html = "<div><p>Menu</p></div><article><p>Story.</p></article><p>Related</p>"
            let draft = try Importer.html(html, source: .text)
            try expectEqual(texts(draft), ["Story."])
        },
        TestCase("given a page of only scripts, then the import fails as empty") {
            let url = URL(fileURLWithPath: "/app")
            try expectError(ImportError.empty(.web(url))) {
                try Importer.html("<script>app()</script>", source: .web(url))
            }
        },
        TestCase("given a UTF-16 text file, then it's read and named by its file") {
            var data = Data([0xFF, 0xFE])
            data.append(try "Bonjour.".data(using: .utf16LittleEndian).unwrap())
            let draft = try Importer.file(data, name: "lecture.txt")
            try expectEqual(texts(draft), ["Bonjour."])
            try expectEqual(draft.title, "lecture")
            try expectEqual(draft.source, .file(name: "lecture.txt"))
        },
        TestCase("given an HTML file, then it's read as a page") {
            let draft = try Importer.file(Data("<p>Ciao.</p>".utf8), name: "a.html")
            try expectEqual(texts(draft), ["Ciao."])
        },
        TestCase("given a kind of file it can't open, then the import fails saying so") {
            try expectError(ImportError.unsupportedFile(name: "a.pdf")) { try Importer.file(Data(), name: "a.pdf") }
        },
        TestCase("given bytes that aren't text, then the import fails saying so") {
            try expectError(ImportError.notText(name: "a.txt")) {
                try Importer.file(Data([0xC3, 0x28, 0xFF]), name: "a.txt")
            }
        },
    ]
}
