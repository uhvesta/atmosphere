"""Bazel rules for tests that take Humata 2 snapshots (humata2_snapshot)."""

load("@rules_rust//rust:defs.bzl", "rust_test")

def _snapshot_globs(srcs):
    """The `__snapshots__/<stem>/**` directory beside each test source."""
    globs = []
    for src in srcs:
        directory, _, file = src.rpartition("/")
        prefix = directory + "/" if directory else ""
        globs.append("%s__snapshots__/%s/**" % (prefix, file.removesuffix(".rs")))
    return globs

def humata2_snapshot_test(name, srcs, data = [], env = {}, tags = [], shard_count = 0, **kwargs):
    """A rust_test that checks its snapshots, and `<name>_record`, which records them.

    The snapshots beside each source (`__snapshots__/<file stem>/`) are the
    test's data, so `bazel test` reads them from the runfiles; a failed
    check's output goes to the test's undeclared outputs
    (bazel-testlogs/<package>/<name>/test.outputs/). `bazel run :<name>_record`
    runs the same test with HUMATA2_SNAPSHOT=record in the source tree, so
    it writes the snapshots there; read `git diff` before committing them.

    Args:
      name: the test's name.
      srcs: the test's sources; snapshots are kept beside each.
      data: more runfiles.
      env: more environment variables, for both targets.
      tags: tags for both; the record target is also `manual`.
      shard_count: optional Bazel shard_count for the check target only (never `_record`).
      **kwargs: passed to both rust_test targets.
    """
    snapshots = native.glob(
        _snapshot_globs(srcs),
        exclude = ["**/.failed/**"],
        allow_empty = True,
    )
    check_kwargs = dict(kwargs)
    if shard_count > 0:
        check_kwargs["shard_count"] = shard_count
    rust_test(
        name = name,
        srcs = srcs,
        data = data + snapshots,
        env = env,
        tags = tags,
        **check_kwargs
    )
    record_env = dict(env)
    record_env["HUMATA2_SNAPSHOT"] = "record"
    rust_test(
        name = name + "_record",
        srcs = srcs,
        data = data + snapshots,
        env = record_env,
        tags = tags + ["manual"],
        **kwargs
    )
