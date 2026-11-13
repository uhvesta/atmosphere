import BlobKit
import Foundation

/// Why the shelf couldn't keep or read a book.
public enum ShelfError: Error, CustomStringConvertible {
    case write(book: String, underlying: any Error)
    case read(file: String, underlying: BlobError)
    /// The file couldn't be read off the device at all.
    case load(file: String, underlying: any Error)
    case delete(book: String, underlying: any Error)

    public var description: String {
        switch self {
        case .write(let book, let error):
            "\(book) couldn't be saved: \(error.localizedDescription). Free some space on the device, then make "
                + "any change to save it again."
        case .read(let file, let error):
            "The book in \(file) couldn't be opened: \(error). It stays on the device, so a newer version of the "
                + "app may open it; delete it to import it again."
        case .load(let file, let error):
            "The book in \(file) couldn't be read: \(error.localizedDescription). It stays on the device; open "
                + "the app again to try again."
        case .delete(let book, let error):
            "\(book) couldn't be deleted: \(error.localizedDescription). Try again."
        }
    }
}

/// The books on the device, one file each, in a folder of their own.
public struct Shelf: Sendable {
    public let folder: URL

    public init(folder: URL) { self.folder = folder }

    /// The shelf in the app's Application Support folder.
    public static var standard: Shelf {
        let support =
            FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        return Shelf(folder: support.appendingPathComponent("Books", isDirectory: true))
    }

    public func save(_ book: Book) throws(ShelfError) {
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try Blob.encode(book).write(to: file(for: book.id), options: Self.writing)
        } catch {
            throw .write(book: book.title, underlying: error)
        }
    }

    /// Every book it can read, the most recently imported first, and the
    /// files it couldn't.
    public func books() -> (books: [Book], unreadable: [ShelfError]) {
        let files =
            (try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil))?
            .filter { $0.pathExtension == "book" } ?? []
        var books: [Book] = []
        var unreadable: [ShelfError] = []
        for file in files {
            let data: Data
            do {
                data = try Data(contentsOf: file)
            } catch {
                unreadable.append(.load(file: file.lastPathComponent, underlying: error))
                continue
            }
            do {
                books.append(try Blob.decode(Book.self, from: data))
            } catch {
                unreadable.append(.read(file: file.lastPathComponent, underlying: error))
            }
        }
        return (books.sorted { $0.importedAt > $1.importedAt }, unreadable)
    }

    public func delete(_ book: Book) throws(ShelfError) {
        let url = file(for: book.id)
        guard FileManager.default.fileExists(atPath: url.path) else { return }
        do {
            try FileManager.default.removeItem(at: url)
        } catch {
            throw .delete(book: book.title, underlying: error)
        }
    }

    /// Encrypted while the device is locked, where the system can.
    #if canImport(Darwin)
        private static let writing: Data.WritingOptions = [.atomic, .completeFileProtection]
    #else
        private static let writing: Data.WritingOptions = [.atomic]
    #endif

    private func file(for id: UUID) -> URL { folder.appendingPathComponent("\(id.uuidString).book") }
}
