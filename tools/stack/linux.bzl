"""linux_builds: one target's static Linux builds, cross-built on any host.

The output is one folder with `<platform>/<file name>` in it, platforms named
as `uname -sm` names them, lowercased (`linux-aarch64`, `linux-x86_64`), as
the tools bundle names them (apps/humata2/daemon/humatad/tools.bzl). The
//bazel/platforms linux_* platforms link musl statically, so each build runs
on any Linux.
"""

def _split_impl(_settings, attr):
    return {
        name: {"//command_line_option:platforms": str(platform)}
        for platform, name in attr.platforms.items()
    }

_split = transition(
    implementation = _split_impl,
    inputs = [],
    outputs = ["//command_line_option:platforms"],
)

def _linux_builds_impl(ctx):
    out = ctx.actions.declare_directory(ctx.label.name)
    args = ctx.actions.args()
    args.add(out.path)
    inputs = []
    for platform in sorted(ctx.attr.platforms.values()):
        build = ctx.split_attr.binary[platform][DefaultInfo].files_to_run.executable
        inputs.append(build)
        args.add_all([platform, ctx.attr.file_name, build])
    ctx.actions.run_shell(
        inputs = inputs,
        outputs = [out],
        command = """
set -eu
out="$1"; shift
while [ "$#" -gt 0 ]; do
  mkdir -p "$out/$1"
  cp "$3" "$out/$1/$2"
  chmod 755 "$out/$1/$2"
  shift 3
done
""",
        arguments = [args],
        mnemonic = "LinuxBuilds",
        progress_message = "Collecting the Linux builds of %s" % ctx.attr.file_name,
    )
    return [DefaultInfo(files = depset([out]))]

linux_builds = rule(
    implementation = _linux_builds_impl,
    attrs = {
        "binary": attr.label(cfg = _split, mandatory = True),
        "file_name": attr.string(mandatory = True),
        # Each platform to build for, with its name as `uname -sm` gives it.
        "platforms": attr.label_keyed_string_dict(mandatory = True),
    },
)
