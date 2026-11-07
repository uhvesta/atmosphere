import Foundation
import LangReadrCore
import TestingKit

private func emptyShelf() throws -> Shelf {
    let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    return Shelf(folder: folder)
}

func shelfTests() -> [TestCase] {
    [
        TestCase("given saved books, then they read back newest first") {
            let shelf = try emptyShelf()
            var older = sampleBook()
            older.importedAt = Date(timeIntervalSince1970: 10)
            var newer = sampleBook()
            newer.id = UUID()
            newer.importedAt = Date(timeIntervalSince1970: 20)
            try shelf.save(older)
            try shelf.save(newer)
            let read = shelf.books()
            try expectEqual(read.books, [newer, older])
            try expect(read.unreadable.isEmpty)
        },
        TestCase("given a damaged file, then the other books still read and it's reported") {
            let shelf = try emptyShelf()
            try shelf.save(sampleBook())
            try Data([1, 2, 3]).write(to: shelf.folder.appendingPathComponent("bad.book"))
            let read = shelf.books()
            try expectEqual(read.books.count, 1)
            try expectEqual(read.unreadable.count, 1)
        },
        TestCase("given a deleted book, then it's gone, and deleting it again is fine") {
            let shelf = try emptyShelf()
            let book = sampleBook()
            try shelf.save(book)
            try shelf.delete(book)
            try shelf.delete(book)
            try expect(shelf.books().books.isEmpty)
        },
    ]
}
