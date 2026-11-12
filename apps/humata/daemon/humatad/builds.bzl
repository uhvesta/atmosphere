"""humatad_builds: one binary built for several platforms, each named by them.

Each build is named `<prefix>-<os>-<arch>`, with the names `uname -sm` gives
lowercased (`darwin-arm64`, `linux-x86_64`, `linux-aarch64`), so humatad finds
the build for a host by asking it `uname -sm`.
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

def _humatad_builds_impl(ctx):
    outputs = []
    for name, binaries in ctx.split_attr.binary.items():
        binary = binaries[DefaultInfo].files_to_run.executable
        output = ctx.actions.declare_file("{}/{}-{}".format(ctx.label.name, ctx.attr.prefix, name))

        # A copy, not a symlink, so the app bundle gets the file itself. A
        # Mac build is signed ad hoc, as the linker signs only the Mac's own
        # architecture, and an app holding unsigned code can't be signed.
        command = 'cp "$1" "$2" && chmod 755 "$2"'
        if name.startswith("darwin-"):
            command += ' && /usr/bin/codesign --force --sign - "$2"'
        ctx.actions.run_shell(
            inputs = [binary],
            outputs = [output],
            command = command,
            arguments = [binary.path, output.path],
            mnemonic = "CopyHumatadBuild",
        )
        outputs.append(output)
    return [DefaultInfo(files = depset(outputs))]

humatad_builds = rule(
    implementation = _humatad_builds_impl,
    attrs = {
        "binary": attr.label(cfg = _split, mandatory = True),
        # Each platform to build for, with its name as `uname -sm` gives it.
        "platforms": attr.label_keyed_string_dict(mandatory = True),
        "prefix": attr.string(mandatory = True),
    },
)
