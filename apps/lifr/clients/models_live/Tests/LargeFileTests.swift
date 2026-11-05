import Darwin
import Foundation
import LifrModelsClient
import XCTest

@testable import LifrModelsLive

/// A model's files are gigabytes, more than iOS lets Lifr hold at once:
/// checking one mustn't keep what it has read.
final class LargeFileTests: XCTestCase {
    private let file = FileManager.default.temporaryDirectory.appending(
        path: "large-\(UUID()).litertlm", directoryHint: .notDirectory)

    override func tearDown() async throws {
        try? FileManager.default.removeItem(at: file)
    }

    func testCheckingAGigabyteFileHoldsLittleOfIt() throws {
        // Sparse: a gigabyte to read, without taking the disk.
        XCTAssertTrue(FileManager.default.createFile(atPath: file.path(percentEncoded: false), contents: nil))
        let handle = try FileHandle(forWritingTo: file)
        try handle.truncate(atOffset: 1 << 30)
        try handle.close()
        let before = try footprint()

        let matches = try ModelLibrary.matches(file, .sha256(String(repeating: "0", count: 64)))

        XCTAssertFalse(matches)
        XCTAssertLessThan(try footprint() - before, 128 << 20)
    }

    /// The memory iOS counts against Lifr's limit.
    private func footprint() throws -> Int64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        guard result == KERN_SUCCESS else { throw XCTSkip("task_info failed: \(result)") }
        return Int64(info.phys_footprint)
    }
}
