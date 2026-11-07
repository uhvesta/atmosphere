"""`lifr_snapshot_test`: FlowKit's snapshot tests, hosted in Lifr's flow host app."""

load("//apps/libs/kits/flow:defs.bzl", "flow_snapshot_test")

visibility("//apps/lifr/...")

def lifr_snapshot_test(name, hosted = False, **kwargs):
    """A `flow_snapshot_test` from //apps/libs/kits/flow:defs.bzl, in Lifr's Swift package.

    Args:
      name: the test's name.
      hosted: run inside Lifr's flow host app, for tests that need a window
        scene, such as flows through presented sheets. iOS only.
      **kwargs: passed to flow_snapshot_test.
    """
    flow_snapshot_test(
        name = name,
        host = "//apps/lifr/testing/snapshot:host" if hosted else None,
        package_name = "lifr",
        **kwargs
    )
