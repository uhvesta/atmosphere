import Foundation
import SnapshotTesting
import XCTest

/// Where one test target's reference snapshots live, and whether this run
/// records them.
///
/// Tests read references from the test bundle, where `lifr_snapshot_test`
/// copies `SnapshotTests/__Snapshots__`. With `--config=record_snapshots`
/// they write new references to the same directory in the source tree.
package struct SnapshotStore: Sendable {
  /// The directory that holds one `__Snapshots__/<test file>` folder per
  /// test file.
  package var root: URL
  package var isRecording: Bool
  /// The Bazel label to put in "record with" hints.
  package var target: String

  package init(root: URL, isRecording: Bool, target: String) {
    self.root = root
    self.isRecording = isRecording
    self.target = target
  }

  /// The store `lifr_snapshot_test` set up for this test process.
  package static func current() throws(SnapshotSetupError) -> SnapshotStore {
    let environment = ProcessInfo.processInfo.environment
    guard let directory = environment["LIFR_SNAPSHOTS"], let target = environment["LIFR_SNAPSHOT_TARGET"]
    else { throw .notInSnapshotTest }
    if let outputs = environment["TEST_UNDECLARED_OUTPUTS_DIR"] {
      // swift-snapshot-testing writes failure diffs here, so they end up
      // in bazel-testlogs.
      setenv("SNAPSHOT_ARTIFACTS", outputs, 1)
    }
    if environment["LIFR_RECORD_SNAPSHOTS"] == "1" {
      return SnapshotStore(root: try sourceDirectory(directory, environment: environment), isRecording: true, target: target)
    }
    guard let bundle = Bundle.allBundles.first(where: { $0.bundleURL.pathExtension == "xctest" }),
      let resources = bundle.resourceURL
    else { throw .noTestBundle }
    return SnapshotStore(
      root: resources.appendingPathComponent("__Snapshots__", isDirectory: true),
      isRecording: false,
      target: target
    )
  }

  /// Finds `directory` in the source tree: the test's runfiles hold a
  /// symlink to the package's BUILD.bazel, and resolving it leads back to the
  /// workspace.
  private static func sourceDirectory(
    _ directory: String,
    environment: [String: String]
  ) throws(SnapshotSetupError) -> URL {
    guard let runfiles = environment["TEST_SRCDIR"], let package = environment["LIFR_SNAPSHOT_PACKAGE"] else {
      throw .noSourceTree
    }
    let marker = URL(fileURLWithPath: runfiles)
      .appendingPathComponent("_main/\(package)/BUILD.bazel")
      .resolvingSymlinksInPath()
    guard marker.path.hasSuffix("/\(package)/BUILD.bazel"), !marker.path.contains("/execroot/"),
      !marker.path.contains("/runfiles/")
    else { throw .noSourceTree }
    return marker.deletingLastPathComponent().appendingPathComponent(directory, isDirectory: true)
  }

  /// Compares `value` with its reference and returns why it differs, or nil
  /// when it matches. Recording always returns a message, so a recording run
  /// fails and can't be mistaken for a passing one.
  package func verify<Value, Format>(
    of value: @autoclosure () throws -> Value,
    as snapshotting: Snapshotting<Value, Format>,
    named name: String,
    file: StaticString = #filePath,
    testName: String = #function,
    line: UInt = #line
  ) -> String? {
    let fileName = URL(fileURLWithPath: "\(file)").deletingPathExtension().lastPathComponent
    let directory = root.appendingPathComponent(fileName, isDirectory: true)
    if !isRecording, let pointer = lfsPointers(in: directory).first {
      return """
        \(pointer.lastPathComponent) is a Git LFS pointer, not the snapshot it stands for. \
        Install Git LFS and fetch the images with `git lfs install && git lfs pull`, then run the test again.
        """
    }
    let failure = SnapshotTesting.verifySnapshot(
      of: try value(),
      as: snapshotting,
      named: name,
      record: isRecording ? .all : .never,
      snapshotDirectory: directory.path,
      file: file,
      testName: testName,
      line: line
    )
    guard let failure else { return nil }
    if isRecording {
      return "Recorded \(fileName)/\(name). Review the change with `git diff`, then run the test again without --config=record_snapshots."
    }
    return """
      \(failure)

      If the new rendering is right, record it with \
      `bazel test --config=record_snapshots \(target)` and review the diff.
      """
  }

  /// Files in `directory` that Git LFS left as pointers because the images
  /// weren't fetched.
  private func lfsPointers(in directory: URL) -> [URL] {
    let files = (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
    let header = Data("version https://git-lfs.github.com/spec/".utf8)
    return files.filter { url in
      guard let handle = try? FileHandle(forReadingFrom: url) else { return false }
      defer { try? handle.close() }
      return (try? handle.read(upToCount: header.count)) == header
    }
  }
}

/// Why a test can't find its snapshots.
package enum SnapshotSetupError: Error, CustomStringConvertible {
  case notInSnapshotTest
  case noTestBundle
  case noSourceTree

  package var description: String {
    switch self {
    case .notInSnapshotTest:
      "LIFR_SNAPSHOTS isn't set. Snapshot tests must be built with lifr_snapshot_test from //apps/lifr/testing/snapshot:defs.bzl."
    case .noTestBundle:
      "The test bundle has no resources, so the reference snapshots are missing. Build the test with lifr_snapshot_test."
    case .noSourceTree:
      "Recording couldn't find the source tree from the test's runfiles. Record with `bazel test --config=record_snapshots <target>` from the workspace, not from Xcode."
    }
  }
}

/// Asserts that `value` matches its reference snapshot.
package func assertSnapshot<Value, Format>(
  of value: @autoclosure () throws -> Value,
  as snapshotting: Snapshotting<Value, Format>,
  named name: String,
  file: StaticString = #filePath,
  testName: String = #function,
  line: UInt = #line
) {
  let store: SnapshotStore
  do {
    store = try SnapshotStore.current()
  } catch {
    XCTFail(error.description, file: file, line: line)
    return
  }
  if let failure = store.verify(of: try value(), as: snapshotting, named: name, file: file, testName: testName, line: line) {
    XCTFail(failure, file: file, line: line)
  }
}
