// swift-tools-version: 6.0

// Third-party Swift packages for Lifr. Bazel builds them through
// rules_swift_package_manager (see MODULE.bazel); this package has no targets
// of its own. After changing a dependency, run
// `bazel run @swift_package//:resolve` to update Package.resolved.

import PackageDescription

let package = Package(
    name: "lifr-deps",
    platforms: [.iOS(.v18), .macOS(.v15)],
    dependencies: [
        .package(url: "https://github.com/pointfreeco/swift-composable-architecture", exact: "1.26.2"),
        .package(url: "https://github.com/pointfreeco/swift-snapshot-testing", exact: "1.19.7"),
        .package(url: "https://github.com/ml-explore/mlx-swift-lm", exact: "3.32.3"),
        .package(url: "https://github.com/huggingface/swift-transformers", exact: "1.4.0"),
    ]
)
