import Foundation
import LifrAssistantClient
import XCTest

@testable import LifrStoreLive

final class GuidanceFileTests: XCTestCase {
    private var folder: URL!

    override func setUpWithError() throws {
        folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: folder)
    }

    private var file: GuidanceFile { GuidanceFile(url: folder.appending(path: "Guidance.json")) }

    func testNothingSavedLoadsLifrsDefault() throws {
        XCTAssertEqual(try file.load(), Guidance())
    }

    func testSavedGuidanceLoadsAgainAndIsReadableOnlyByLifr() throws {
        var guidance = Guidance()
        try guidance.save("Be brief.", id: UUID(), at: Date(timeIntervalSince1970: 1_793_700_000))
        try file.save(guidance)

        XCTAssertEqual(try file.load(), guidance)
        let attributes = try FileManager.default.attributesOfItem(atPath: file.url.path)
        XCTAssertEqual(attributes[.posixPermissions] as? Int, 0o600)
        let folderAttributes = try FileManager.default.attributesOfItem(atPath: folder.path)
        XCTAssertEqual(folderAttributes[.posixPermissions] as? Int, 0o700)
    }

    func testDamagedFileFailsInsteadOfLosingTheHistory() throws {
        try file.save(Guidance())
        try Data("{".utf8).write(to: file.url)
        XCTAssertThrowsError(try file.load()) { error in
            guard case .storageFailed = error as? GuidanceError else { return XCTFail("\(error)") }
        }
    }
}
