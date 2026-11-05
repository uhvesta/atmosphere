"""`lifr_snapshot_test`: snapshot tests on the pinned iOS simulator, or the Mac."""

load("@rules_apple//apple:ios.bzl", "ios_unit_test")
load("@rules_apple//apple:macos.bzl", "macos_unit_test")
load("@rules_apple//apple:resources.bzl", "apple_resource_group")
load("@rules_swift//swift:swift_library.bzl", "swift_library")

visibility("//apps/lifr/...")

# The libraries only build for the test's platform. The test transitions to
# it itself, so it only needs Xcode.
_IOS = ["//bazel/xcode", "@platforms//os:ios"]
_MACOS = ["//bazel/xcode", "@platforms//os:macos"]

def lifr_snapshot_test(name, srcs, deps = [], data = [], hosted = False, mac = False, **kwargs):
    """Snapshot tests for one package, run on iPhone 17 with iOS 26.2, or the Mac.

    References live in `SnapshotTests/__Snapshots__/<test file>/` and are
    copied into the test bundle. Verify with `bazel test --config=xcode`, and
    re-record with `bazel test --config=record_snapshots`, which writes into
    the source tree and fails so the run isn't mistaken for a pass.

    Args:
      name: the test's name.
      srcs: Swift sources of XCTestCase subclasses.
      deps: what the tests use. LifrSnapshotTesting is added.
      data: files the tests read at run time, from TEST_SRCDIR.
      hosted: run inside an app, for tests that need a window scene, such as
        flows through presented sheets with `FlowRecorder`. iOS only.
      mac: run on this Mac instead. It needs the terminal's Accessibility
        and Screen Recording access, which `bazel test` doesn't have, so
        verify and record with `run_mac.sh` in this package. Its images
        follow the Mac's macOS version, so record them again after updating
        macOS.
      **kwargs: passed to ios_unit_test or macos_unit_test.
    """
    if mac and hosted:
        fail("hosted is for iOS: Mac snapshot tests open their own windows.")
    platform = _MACOS if mac else _IOS
    apple_resource_group(
        name = name + "_references",
        structured_resources = native.glob(
            ["SnapshotTests/__Snapshots__/**"],
            allow_empty = True,
        ),
        strip_structured_resources_prefixes = ["SnapshotTests"],
        testonly = True,
        target_compatible_with = platform,
    )
    swift_library(
        name = name + "_lib",
        package_name = "lifr",
        testonly = True,
        srcs = srcs,
        data = [":" + name + "_references"],
        module_name = name.title().replace("_", ""),
        target_compatible_with = platform,
        deps = deps + ["//apps/lifr/testing/snapshot"],
    )
    common = dict(
        name = name,
        # Recording resolves this file's runfiles symlink to find the source
        # tree.
        data = ["BUILD.bazel"] + data,
        env = {
            "LIFR_SNAPSHOTS": "SnapshotTests/__Snapshots__",
            "LIFR_SNAPSHOT_PACKAGE": native.package_name(),
            "LIFR_SNAPSHOT_TARGET": "//{}:{}".format(native.package_name(), name),
        },
        target_compatible_with = ["//bazel/xcode"],
        deps = [":" + name + "_lib"],
    )
    if mac:
        macos_unit_test(minimum_os_version = "15.0", **(common | kwargs))
        return
    ios_unit_test(
        minimum_os_version = "18.0",
        runner = "//apps/lifr/testing/snapshot:iphone_17",
        test_host = "//apps/lifr/testing/snapshot:host" if hosted else None,
        **(common | kwargs)
    )
