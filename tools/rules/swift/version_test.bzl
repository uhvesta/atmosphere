"""swift_version_test: build a Swift binary with one release for one platform.

The binary is built through a transition that sets --platforms and
--@swift_toolchains//:version, so one `bazel test` covers every release and
platform. When the platform is the one running the test, the binary also runs,
and its output must contain `expect`.
"""

def _transition_impl(_settings, attr):
    return {
        "//command_line_option:platforms": str(attr.platform),
        "@swift_toolchains//:version": attr.version,
    }

_version_transition = transition(
    implementation = _transition_impl,
    inputs = [],
    outputs = [
        "//command_line_option:platforms",
        "@swift_toolchains//:version",
    ],
)

def _swift_version_test_impl(ctx):
    binary = ctx.attr.binary[0][DefaultInfo].files_to_run.executable
    if ctx.attr.run:
        body = """\
out=$("$bin" 2>&1)
printf '%s\\n' "$out"
case "$out" in
  *{expect}*) ;;
  *)
    echo "expected output containing {expect}" >&2
    exit 1
    ;;
esac
""".format(expect = repr(ctx.attr.expect).replace('"', "'"))
    else:
        body = 'test -s "$bin" && echo "built $bin"\n'
    script = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(
        script,
        "#!/bin/sh\nset -eu\nbin={}\n".format(binary.short_path) + body,
        is_executable = True,
    )
    return [DefaultInfo(
        executable = script,
        runfiles = ctx.runfiles(files = [binary]),
    )]

swift_version_test = rule(
    implementation = _swift_version_test_impl,
    test = True,
    doc = "Builds `binary` with Swift `version` for `platform`, and runs it if `run`.",
    attrs = {
        "binary": attr.label(cfg = _version_transition, executable = True, mandatory = True),
        "expect": attr.string(default = "hello from Swift"),
        "platform": attr.label(mandatory = True),
        "run": attr.bool(doc = "Run the binary; only when `platform` is the host's."),
        "version": attr.string(mandatory = True),
    },
)
