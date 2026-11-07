"""FlowKit's Bazel macros: snapshot tests on a pinned iOS simulator or the Mac, and the app hosted flows run in."""

load("@rules_apple//apple:ios.bzl", "ios_application", "ios_unit_test")
load("@rules_apple//apple:macos.bzl", "macos_unit_test")
load("@rules_apple//apple:resources.bzl", "apple_resource_group")
load("@rules_swift//swift:swift_library.bzl", "swift_library")

visibility("//apps/...")

# The libraries only build for the test's platform. The test transitions to
# it itself, so it only needs Xcode.
_IOS = ["//bazel/xcode", "@platforms//os:ios"]
_MACOS = ["//bazel/xcode", "@platforms//os:macos"]

def flow_snapshot_test(
        name,
        srcs,
        deps = [],
        data = [],
        host = None,
        mac = False,
        package_name = None,
        runner = "//apps/libs/kits/flow:iphone_17",
        **kwargs):
    """Snapshot tests for one package, run on a pinned simulator or the Mac.

    References live in `SnapshotTests/__Snapshots__/<test file>/` and are
    copied into the test bundle. Verify with `bazel test --config=xcode`, and
    re-record with `bazel test --config=record_snapshots`, which writes into
    the source tree and fails so the run isn't mistaken for a pass.

    Args:
      name: the test's name.
      srcs: Swift sources of XCTestCase subclasses.
      deps: what the tests use. FlowKit is added.
      data: files the tests read at run time, from TEST_SRCDIR.
      host: an app from `flow_host_app` to run inside, for tests that need a
        window scene, such as flows through presented sheets with
        `FlowRecorder`. iOS only.
      mac: run on this Mac instead. It needs the terminal's Accessibility
        and Screen Recording access, which `bazel test` doesn't have, so
        verify and record with //apps/libs/kits/flow:run_mac.sh. Its images
        follow the Mac's macOS version, so record them again after updating
        macOS.
      package_name: the Swift package the tests are in, to use the `package`
        declarations of modules in it.
      runner: the simulator iOS tests run on. Keep one per app, so references
        don't drift between machines.
      **kwargs: passed to ios_unit_test or macos_unit_test.
    """
    if mac and host:
        fail("host is for iOS: Mac snapshot tests open their own windows.")
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
        package_name = package_name,
        testonly = True,
        srcs = srcs,
        data = [":" + name + "_references"],
        module_name = name.title().replace("_", ""),
        target_compatible_with = platform,
        deps = deps + ["//apps/libs/kits/flow"],
    )
    common = dict(
        name = name,
        # Recording resolves this file's runfiles symlink to find the source
        # tree.
        data = ["BUILD.bazel"] + data,
        env = {
            "FLOWKIT_PACKAGE": native.package_name(),
            "FLOWKIT_SNAPSHOTS": "SnapshotTests/__Snapshots__",
            "FLOWKIT_TARGET": "//{}:{}".format(native.package_name(), name),
        },
        target_compatible_with = ["//bazel/xcode"],
        deps = [":" + name + "_lib"],
    )
    if mac:
        macos_unit_test(minimum_os_version = "15.0", **(common | kwargs))
        return
    ios_unit_test(
        minimum_os_version = "18.0",
        runner = runner,
        test_host = host,
        **(common | kwargs)
    )

def flow_host_app(name, bundle_id, **kwargs):
    """An empty iPhone app for hosted flow snapshot tests to run in.

    Args:
      name: the app's name.
      bundle_id: its bundle ID. It's only installed on simulators, so it
        needs no signing.
      **kwargs: passed to ios_application.
    """
    ios_application(
        name = name,
        testonly = True,
        bundle_id = bundle_id,
        families = ["iphone"],
        infoplists = ["//apps/libs/kits/flow:Host/Info.plist"],
        minimum_os_version = "18.0",
        target_compatible_with = ["//bazel/xcode"],
        deps = ["//apps/libs/kits/flow:host_lib"],
        **kwargs
    )
