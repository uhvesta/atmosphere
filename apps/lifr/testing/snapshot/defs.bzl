"""`lifr_snapshot_test`: snapshot tests on the pinned iOS simulator."""

load("@rules_apple//apple:ios.bzl", "ios_unit_test")
load("@rules_apple//apple:resources.bzl", "apple_resource_group")
load("@rules_swift//swift:swift_library.bzl", "swift_library")

visibility("//apps/lifr/...")

# The libraries only build for iOS. The test transitions to the simulator
# itself, so it only needs Xcode.
_IOS = ["//bazel/xcode", "@platforms//os:ios"]

def lifr_snapshot_test(name, srcs, deps = [], **kwargs):
    """Snapshot tests for one package, run on iPhone 17 with iOS 26.2.

    References live in `SnapshotTests/__Snapshots__/<test file>/` and are
    copied into the test bundle. Verify with `bazel test --config=xcode`, and
    re-record with `bazel test --config=record_snapshots`, which writes into
    the source tree and fails so the run isn't mistaken for a pass.

    Args:
      name: the test's name.
      srcs: Swift sources of XCTestCase subclasses.
      deps: what the tests use. LifrSnapshotTesting is added.
      **kwargs: passed to ios_unit_test.
    """
    apple_resource_group(
        name = name + "_references",
        structured_resources = native.glob(
            ["SnapshotTests/__Snapshots__/**"],
            allow_empty = True,
        ),
        strip_structured_resources_prefixes = ["SnapshotTests"],
        testonly = True,
        target_compatible_with = _IOS,
    )
    swift_library(
        name = name + "_lib",
        package_name = "lifr",
        testonly = True,
        srcs = srcs,
        data = [":" + name + "_references"],
        module_name = name.title().replace("_", ""),
        target_compatible_with = _IOS,
        deps = deps + ["//apps/lifr/testing/snapshot"],
    )
    ios_unit_test(
        name = name,
        # Recording resolves this file's runfiles symlink to find the source
        # tree.
        data = ["BUILD.bazel"],
        env = {
            "LIFR_SNAPSHOTS": "SnapshotTests/__Snapshots__",
            "LIFR_SNAPSHOT_PACKAGE": native.package_name(),
            "LIFR_SNAPSHOT_TARGET": "//{}:{}".format(native.package_name(), name),
        },
        minimum_os_version = "18.0",
        runner = "//apps/lifr/testing/snapshot:iphone_17",
        target_compatible_with = ["//bazel/xcode"],
        deps = [":" + name + "_lib"],
        **kwargs
    )
