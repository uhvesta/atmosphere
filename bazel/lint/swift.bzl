"""Lint aspects for Swift sources: swift-format and SwiftLint.

Modeled on rules_rust's rustfmt aspect. They check every target in this
repository that returns `SwiftInfo` (swift_library, swift_binary, swift_test,
...), on the Swift sources that the target itself compiles, skipping
generated files. Tag a target `no-lint` to skip it.

The Swift rules don't list `SwiftInfo` in `provides`, so `required_providers`
would match none of them; the aspects check for the provider instead.

The tools come from toolchains (see //bazel/toolchains/swift:tools.bzl), so
each action gets a binary for its execution platform. Each tool reads its
own native config file through a label_flag, like
`@rules_rust//rust/settings:rustfmt.toml`:

    --//bazel/lint:swift_format_config=//some:.swift-format
    --//bazel/lint:swiftlint_config=//some:.swiftlint.yml

The aspects follow the rules_lint conventions of the other linters in
linters.bzl: they honor `--@aspect_rules_lint//lint:fail_on_violation` and
fill the rules_lint_human and rules_lint_machine (SARIF) output groups, from
one action per target.
"""

load(
    "@aspect_rules_lint//lint/private:lint_aspect.bzl",
    "LintOptionsInfo",
    "output_files",
)
load("@rules_swift//swift:providers.bzl", "SwiftInfo")
load("//bazel/toolchains/swift:tools.bzl", "SWIFTLINT_TOOLCHAIN", "SWIFT_FORMAT_TOOLCHAIN")

def _swift_sources(target, ctx):
    """The Swift files in this repository that `target` itself compiles."""
    if SwiftInfo not in target or target.label.repo_name or "no-lint" in ctx.rule.attr.tags:
        return []
    srcs = []
    for module in target[SwiftInfo].direct_modules:
        context = module.compilation_context
        if context:
            srcs.extend([f for f in context.direct_sources if f.is_source])
    return srcs

def _lint(target, ctx, mnemonic, toolchain_type, args, hint):
    """Runs the toolchain's tool once on the target's Swift sources.

    //bazel/lint/runner writes both rules_lint reports from that one run: the
    tool's output for people, and SARIF parsed from it for tools. Like
    rules_rust's process_wrapper, it also handles what a plain action can't:
    with an exit code file it records the status and succeeds; without one
    (fail_on_violation), a finding fails the action with the hint.
    """
    srcs = _swift_sources(target, ctx)
    if not srcs:
        return []
    toolchain = ctx.toolchains[toolchain_type]
    if not toolchain:
        fail(("No {tool} toolchain runs on this execution platform, so {label} can't be linted. " +
              "Build on a host that @swift_toolchains supports, or tag the target `no-lint`.").format(
            tool = mnemonic,
            label = target.label,
        ))

    outputs, info = output_files(mnemonic, target, ctx)
    tool = toolchain.tool
    report = outputs.human
    cmd = ctx.actions.args()
    cmd.add("--tool-name", mnemonic)
    cmd.add("--human", report.out)
    cmd.add("--machine", outputs.machine.out)
    if report.exit_code:
        cmd.add("--exit-code", report.exit_code)
    cmd.add("--hint", hint.format(label = "//{}:{}".format(target.label.package, target.label.name)))
    cmd.add("--")
    cmd.add(tool.executable)
    cmd.add_all(args)
    cmd.add(ctx.file._config)
    cmd.add_all(srcs)

    action_outputs = [report.out, outputs.machine.out]
    if report.exit_code:
        action_outputs.append(report.exit_code)
    ctx.actions.run(
        executable = ctx.executable._runner,
        arguments = [cmd],
        inputs = srcs + [ctx.file._config],
        outputs = action_outputs,
        tools = [tool],
        mnemonic = mnemonic,
        progress_message = "Linting %{label} with " + mnemonic,
        toolchain = toolchain_type,
    )

    # rules_lint gives each report its own exit code file; both come from the
    # same run.
    if outputs.machine.exit_code:
        ctx.actions.symlink(output = outputs.machine.exit_code, target_file = report.exit_code)
    return [info]

def _swift_format_impl(target, ctx):
    return _lint(
        target,
        ctx,
        mnemonic = "SwiftFormat",
        toolchain_type = SWIFT_FORMAT_TOOLCHAIN,
        # Sandboxed inputs are symlinks, which swift-format otherwise skips
        # without a word.
        args = ["lint", "--strict", "--follow-symlinks", "--configuration"],
        hint = "{label} is not formatted the way swift-format wants. Run `bazel run //:format` to fix it in place.",
    )

def _swiftlint_impl(target, ctx):
    return _lint(
        target,
        ctx,
        mnemonic = "SwiftLint",
        toolchain_type = SWIFTLINT_TOOLCHAIN,
        # SourceKit comes from Xcode, which the build never reads, so rules
        # that need it are skipped.
        args = ["lint", "--strict", "--quiet", "--no-cache", "--disable-sourcekit", "--config"],
        hint = "SwiftLint found problems in {label}. Run `bazel run //:format` for the layout ones and fix the rest " +
               "by hand. Silence a rule for one line with `// swiftlint:disable:next <rule>` only when the code is " +
               "right and the rule is wrong.",
    )

def _swift_lint_aspect(implementation, doc, config, toolchain_type):
    return aspect(
        implementation = implementation,
        doc = doc,
        attrs = {
            "_config": attr.label(default = config, allow_single_file = True),
            "_options": attr.label(
                default = Label("@aspect_rules_lint//lint:options"),
                providers = [LintOptionsInfo],
            ),
            "_runner": attr.label(
                default = Label("//bazel/lint/runner"),
                executable = True,
                cfg = "exec",
            ),
        },
        # Optional, so targets that aren't Swift don't need the toolchain.
        toolchains = [config_common.toolchain_type(toolchain_type, mandatory = False)],
    )

swift_format = _swift_lint_aspect(
    _swift_format_impl,
    doc = "Fails when a Swift target's sources differ from what `swift-format` would write.",
    config = Label(":swift_format_config"),
    toolchain_type = SWIFT_FORMAT_TOOLCHAIN,
)

swiftlint = _swift_lint_aspect(
    _swiftlint_impl,
    doc = "Fails when `swiftlint lint --strict` reports a violation in a Swift target's sources.",
    config = Label(":swiftlint_config"),
    toolchain_type = SWIFTLINT_TOOLCHAIN,
)
