import Foundation
import ImageIO
import LifrAssistantClient
import UniformTypeIdentifiers
import XCTest

@testable import LifrStoreLive

final class AttachmentFilesTests: XCTestCase {
    private var root: URL!
    private let conversation = UUID()

    override func setUpWithError() throws {
        root = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: root)
    }

    private var files: AttachmentFiles { AttachmentFiles(root: root) }

    func testAPhotoIsKeptSmallerAndWithoutWhereItWasTaken() throws {
        let photo = try Self.photo(width: 3000, height: 1500, latitude: 52.37)
        let attachment = try files.save(photo, kind: .image, seconds: nil, conversation: conversation)

        let url = try XCTUnwrap(files.file(attachment, conversation: conversation))
        let source = try XCTUnwrap(CGImageSourceCreateWithURL(url as CFURL, nil))
        let properties = try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
        XCTAssertEqual(properties[kCGImagePropertyPixelWidth] as? Int, 1024)
        XCTAssertEqual(properties[kCGImagePropertyPixelHeight] as? Int, 512)
        XCTAssertNil(properties[kCGImagePropertyGPSDictionary])
        XCTAssertEqual(CGImageSourceGetType(source) as String?, UTType.jpeg.identifier)
    }

    func testFilesAreReadableOnlyByLifr() throws {
        let attachment = try files.save(
            Self.wav, kind: .audio, seconds: 3, conversation: conversation)

        XCTAssertEqual(attachment.seconds, 3)
        let url = try XCTUnwrap(files.file(attachment, conversation: conversation))
        XCTAssertEqual(try Data(contentsOf: url), Self.wav)
        let file = try FileManager.default.attributesOfItem(atPath: url.path)
        XCTAssertEqual(file[.posixPermissions] as? Int, 0o600)
        for folder in [root, url.deletingLastPathComponent()] {
            let attributes = try FileManager.default.attributesOfItem(atPath: try XCTUnwrap(folder).path)
            XCTAssertEqual(attributes[.posixPermissions] as? Int, 0o700)
        }
    }

    func testWhatIsntAPhotoOrARecordingIsRefused() {
        XCTAssertThrowsError(
            try files.save(Data("not a photo".utf8), kind: .image, seconds: nil, conversation: conversation)
        ) { error in
            XCTAssertEqual(error as? AttachmentError, .unreadablePhoto)
        }
        XCTAssertThrowsError(
            try files.save(Data("not a recording".utf8), kind: .audio, seconds: 2, conversation: conversation)
        ) { error in
            guard case .recordingFailed = error as? AttachmentError else { return XCTFail("\(error)") }
        }
    }

    func testRemovingAConversationsAttachmentsLeavesOthers() throws {
        let other = UUID()
        let mine = try files.save(Self.wav, kind: .audio, seconds: 1, conversation: conversation)
        let theirs = try files.save(Self.wav, kind: .audio, seconds: 1, conversation: other)

        try files.removeAll(conversation)
        try files.removeAll(conversation)

        XCTAssertNil(files.file(mine, conversation: conversation))
        XCTAssertNotNil(files.file(theirs, conversation: other))
    }

    func testAForkKeepsItsOwnCopyOfEachAttachmentHere() throws {
        let fork = UUID()
        let recording = try files.save(Self.wav, kind: .audio, seconds: 1, conversation: conversation)
        let synced = ChatAttachment(id: UUID(), kind: .image)

        try files.copy(recording, from: conversation, to: fork)
        try files.copy(synced, from: conversation, to: fork)
        try files.removeAll(conversation)

        let url = try XCTUnwrap(files.file(recording, conversation: fork))
        XCTAssertEqual(try Data(contentsOf: url), Self.wav)
        XCTAssertEqual(try FileManager.default.attributesOfItem(atPath: url.path)[.posixPermissions] as? Int, 0o600)
        XCTAssertNil(files.file(synced, conversation: fork))
    }

    func testAnAttachmentFromAnotherDeviceIsntHere() {
        let synced = ChatAttachment(id: UUID(), kind: .image)
        XCTAssertNil(files.file(synced, conversation: conversation))
    }

    /// The header of an empty 16 kHz mono WAV file.
    private static let wav: Data = {
        var data = Data("RIFF".utf8)
        data.append(contentsOf: [36, 0, 0, 0])
        data.append(Data("WAVEfmt ".utf8))
        data.append(contentsOf: [16, 0, 0, 0, 1, 0, 1, 0, 0x80, 0x3E, 0, 0, 0, 0x7D, 0, 0, 2, 0, 16, 0])
        data.append(Data("data".utf8))
        data.append(contentsOf: [0, 0, 0, 0])
        return data
    }()

    /// A gray JPEG, tagged with where it was taken.
    private static func photo(width: Int, height: Int, latitude: Double) throws -> Data {
        let context = try XCTUnwrap(
            CGContext(
                data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(gray: 0.5, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let image = try XCTUnwrap(context.makeImage())
        let output = NSMutableData()
        let destination = try XCTUnwrap(
            CGImageDestinationCreateWithData(output as CFMutableData, UTType.jpeg.identifier as CFString, 1, nil))
        let gps: [CFString: Any] = [kCGImagePropertyGPSLatitude: latitude, kCGImagePropertyGPSLatitudeRef: "N"]
        CGImageDestinationAddImage(destination, image, [kCGImagePropertyGPSDictionary: gps] as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        let source = try XCTUnwrap(CGImageSourceCreateWithData(output as CFData, nil))
        let saved = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        XCTAssertNotNil(saved?[kCGImagePropertyGPSDictionary], "the test photo should say where it was taken")
        return output as Data
    }
}
