"""Toolchains for the Swift source tools: swift-format and SwiftLint.

Each tool has its own toolchain type, so Bazel picks a binary that runs on
the execution platform, the same way it picks swiftc:

- swift-format ships in the swift.org release, so @swift_toolchains registers
  one toolchain per release and build host, next to the compilers.
- SwiftLint builds from source (the `swiftlint` Bazel module) with those
  compilers, so one toolchain covers every host that can build Swift.

Rules and aspects read the tool with
`ctx.toolchains[SWIFT_FORMAT_TOOLCHAIN].tool`, a FilesToRunProvider, and its
runfiles with `.runfiles`.
"""

SWIFT_FORMAT_TOOLCHAIN = Label("//bazel/toolchains/swift:swift_format_toolchain_type")
SWIFTLINT_TOOLCHAIN = Label("//bazel/toolchains/swift:swiftlint_toolchain_type")

def _swift_tool_toolchain_impl(ctx):
    info = ctx.attr.tool[DefaultInfo]
    return [platform_common.ToolchainInfo(
        runfiles = info.default_runfiles,
        tool = info.files_to_run,
    )]

swift_tool_toolchain = rule(
    implementation = _swift_tool_toolchain_impl,
    doc = "A toolchain() implementation that carries one executable tool.",
    attrs = {
        "tool": attr.label(
            mandatory = True,
            executable = True,
            cfg = "exec",
            doc = "The tool, built for or downloaded for the execution platform.",
        ),
    },
)

def _tool_binary(toolchain_type):
    def _impl(ctx):
        toolchain = ctx.toolchains[toolchain_type]
        out = ctx.actions.declare_file(ctx.label.name)
        ctx.actions.symlink(output = out, target_file = toolchain.tool.executable, is_executable = True)
        return [DefaultInfo(
            executable = out,
            runfiles = ctx.runfiles(files = [toolchain.tool.executable]).merge(toolchain.runfiles),
        )]

    return rule(
        implementation = _impl,
        executable = True,
        toolchains = [toolchain_type],
    )

# `bazel run`-able swift-format for the host, from the selected release. The
# toolchain's binary is built for the execution platform; `bazel run` uses
# it on the host, which is the execution platform unless remote execution
# says otherwise.
swift_format_binary = _tool_binary(SWIFT_FORMAT_TOOLCHAIN)
