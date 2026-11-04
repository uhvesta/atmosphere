import LifrSnapshotTesting
import SnapshotTesting
import SwiftUI
import XCTest

private struct Card: View {
    var title: String
    var tint: Color

    var body: some View {
        HStack {
            Image(systemName: "figure.run")
                .accessibilityHidden(true)
            Text(title)
                .font(.headline)
            Spacer()
            Button("Start") {}
        }
        .padding()
        .background(tint.opacity(0.2), in: RoundedRectangle(cornerRadius: 12))
        .padding()
    }
}

final class SampleSnapshotTests: XCTestCase {
    private let size = CGSize(width: 320, height: 96)

    func testImage() {
        assertSnapshot(
            of: Card(title: "Training plan", tint: .blue),
            as: .image(layout: .fixed(width: size.width, height: size.height)),
            named: "card"
        )
    }

    func testAccessibilityTree() {
        assertSnapshot(of: Card(title: "Training plan", tint: .blue), as: .accessibilityTree(size: size), named: "card")
    }

    func testVisualChangeFails() throws {
        let store = try SnapshotStore.current()
        try XCTSkipIf(store.isRecording, "Recording would overwrite testImage's reference with the changed card.")
        let failure = store.verify(
            of: Card(title: "Training plan", tint: .orange),
            as: .image(layout: .fixed(width: size.width, height: size.height)),
            named: "card",
            testName: "testImage()"
        )
        XCTAssertNotNil(failure, "A different tint must not match the reference.")
    }

    func testLFSPointerFails() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let directory = root.appendingPathComponent("SampleSnapshotTests")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try Data("version https://git-lfs.github.com/spec/v1\noid sha256:0\nsize 1\n".utf8)
            .write(to: directory.appendingPathComponent("testImage.card.png"))

        let store = SnapshotStore(root: root, isRecording: false, target: "//sample")
        let failure = store.verify(
            of: Card(title: "Training plan", tint: .blue),
            as: .image(layout: .fixed(width: size.width, height: size.height)),
            named: "card",
            testName: "testImage()"
        )
        XCTAssertNotNil(failure, "A pointer file must fail before any image comparison.")
    }

    func testMissingReferenceFails() {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = SnapshotStore(root: root, isRecording: false, target: "//sample")
        XCTAssertNotNil(
            store.verify(of: Card(title: "New", tint: .blue), as: .accessibilityTree(size: size), named: "missing")
        )
    }
}
