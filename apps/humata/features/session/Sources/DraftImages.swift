import Foundation
import HumataDaemonClient
import ImageIO
import UniformTypeIdentifiers

/// An image put in the box for the next message: a chip with a spinner
/// while it's read and made small enough to send, then its thumbnail.
package struct DraftImage: Identifiable, Hashable, Sendable {
    package var id: String
    /// What it's called on its chip, such as the file's name.
    package var name: String
    /// Nil while it's being prepared.
    package var image: ImageAttachment?
    /// A small copy for the chip.
    package var thumbnail: Data?

    package init(id: String, name: String, image: ImageAttachment? = nil, thumbnail: Data? = nil) {
        self.id = id
        self.name = name
        self.image = image
        self.thumbnail = thumbnail
    }

    package var isPreparing: Bool { image == nil }
}

/// An image made ready to send, with its thumbnail for the chip.
package struct PreparedImage: Equatable, Sendable {
    package var image: ImageAttachment
    package var thumbnail: Data?

    package init(image: ImageAttachment, thumbnail: Data?) {
        self.image = image
        self.thumbnail = thumbnail
    }
}

/// Why an image couldn't go in the box.
package enum ImagePrepError: Error, Equatable, CustomStringConvertible {
    /// The file couldn't be read.
    case unreadable(name: String)
    /// It isn't an image the system can open.
    case notAnImage(name: String)

    package var description: String {
        switch self {
        case .unreadable(let name):
            "Couldn’t read \(name). Check that the file is still there and you can open it, then add it again."
        case .notAnImage(let name):
            "\(name) isn’t an image Humata can open. Save it as PNG or JPEG and add that instead."
        }
    }
}

/// Makes images ready to send: kept as they are when the agents take them
/// and they're small enough, otherwise made smaller and saved as JPEG.
/// Slow on big images, so it's run off the main thread.
package enum ImagePrep {
    /// Images larger than this on their longest side are made smaller; the
    /// models see no more detail than about this.
    static let longestSide = 2_048
    /// Images up to this size are sent as they are, if the kind is taken.
    static let sendAsIs = 4 * 1024 * 1024
    static let thumbnailSide = 160

    /// The image `data`, named `name`, ready to send, and its thumbnail.
    package static func prepare(_ data: Data, name: String) throws(ImagePrepError) -> PreparedImage {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
            CGImageSourceGetCount(source) > 0,
            let type = CGImageSourceGetType(source) as String?
        else { throw .notAnImage(name: name) }
        let thumbnail = scaled(source, side: thumbnailSide).flatMap { encode($0, as: .png) }
        let taken: [String: String] = [
            UTType.png.identifier: "image/png", UTType.jpeg.identifier: "image/jpeg",
            UTType.gif.identifier: "image/gif", UTType.webP.identifier: "image/webp",
        ]
        if let mediaType = taken[type], data.count <= sendAsIs, longest(source) <= longestSide {
            return PreparedImage(image: ImageAttachment(mediaType: mediaType, data: data), thumbnail: thumbnail)
        }
        guard let smaller = scaled(source, side: longestSide), let jpeg = encode(smaller, as: .jpeg) else {
            throw .notAnImage(name: name)
        }
        return PreparedImage(image: ImageAttachment(mediaType: "image/jpeg", data: jpeg), thumbnail: thumbnail)
    }

    /// The image file at `url`, ready to send.
    package static func prepare(_ url: URL) throws(ImagePrepError) -> PreparedImage {
        let name = url.lastPathComponent
        let accessing = url.startAccessingSecurityScopedResource()
        defer { if accessing { url.stopAccessingSecurityScopedResource() } }
        guard let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { throw .unreadable(name: name) }
        return try prepare(data, name: name)
    }

    /// A small copy of `data` for a sent message's bubble.
    package static func thumbnail(_ data: Data, side: Int = 320) -> Data? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        return scaled(source, side: side).flatMap { encode($0, as: .png) }
    }

    private static func longest(_ source: CGImageSource) -> Int {
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        let width = properties?[kCGImagePropertyPixelWidth] as? Int ?? .max
        let height = properties?[kCGImagePropertyPixelHeight] as? Int ?? .max
        return max(width, height)
    }

    private static func scaled(_ source: CGImageSource, side: Int) -> CGImage? {
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: side,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }

    private static func encode(_ image: CGImage, as type: UTType) -> Data? {
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data, type.identifier as CFString, 1, nil) else {
            return nil
        }
        let options: [CFString: Any] = [kCGImageDestinationLossyCompressionQuality: 0.85]
        CGImageDestinationAddImage(destination, image, options as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return data as Data
    }
}
