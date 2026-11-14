"""`lifr_strings`: the app's String Catalog, synced with the code's strings."""

load("@apple_support//lib:apple_support.bzl", "apple_support")

visibility("private")

_StringsInfo = provider(
    doc = "The .stringsdata directories of Lifr's Swift libraries and the kits it uses.",
    fields = {"directories": "depset of directories of .stringsdata files"},
)

def _strings_aspect_impl(target, ctx):
    own = []

    # Lifr's own libraries and the shared kits and features it shows, whose
    # words are looked up in the app's catalog.
    is_lifr = ctx.label.workspace_name == "" and ctx.label.package.startswith(("apps/lifr/", "apps/libs/"))
    if is_lifr and OutputGroupInfo in target and hasattr(target[OutputGroupInfo], "swift_localized_strings"):
        own = [target[OutputGroupInfo].swift_localized_strings]
    deps = [dep[_StringsInfo].directories for dep in getattr(ctx.rule.attr, "deps", []) if _StringsInfo in dep]
    return [_StringsInfo(directories = depset(transitive = own + deps))]

_strings_aspect = aspect(
    implementation = _strings_aspect_impl,
    attr_aspects = ["deps"],
)

def _ios_impl(_settings, _attr):
    return {
        "//command_line_option:apple_platform_type": "ios",
        "//command_line_option:cpu": "ios_sim_arm64",
        "//command_line_option:ios_minimum_os": "18.0",
        "//command_line_option:platforms": ["//bazel/xcode:ios_sim_arm64"],
    }

# The iPhone's own words, such as the Live Activity's, are only in code
# built for iOS.
_ios = transition(
    implementation = _ios_impl,
    inputs = [],
    outputs = [
        "//command_line_option:apple_platform_type",
        "//command_line_option:cpu",
        "//command_line_option:ios_minimum_os",
        "//command_line_option:platforms",
    ],
)

def _lifr_strings_impl(ctx):
    output = ctx.actions.declare_file(ctx.label.name + "/" + ctx.file.catalog.basename)
    directories = depset(transitive = [
        dep[_StringsInfo].directories
        for dep in ctx.attr.deps + ctx.attr.ios_deps
    ])
    args = ctx.actions.args()
    args.add(output)
    args.add(ctx.file.catalog)
    args.add_all(directories, expand_directories = False)
    apple_support.run_shell(
        actions = ctx.actions,
        xcode_config = ctx.attr._xcode_config[apple_common.XcodeVersionConfig],
        apple_fragment = ctx.fragments.apple,
        arguments = [args],
        command = """set -eu
out="$1"
cp "$2" "$out"
chmod u+w "$out"
shift 2
xcrun xcstringstool sync "$out" --stringsdata $(find "$@" -name '*.stringsdata' | sort)
""",
        inputs = depset([ctx.file.catalog], transitive = [directories]),
        outputs = [output],
        mnemonic = "SyncXCStrings",
        progress_message = "Syncing %{output} with the code's strings",
    )
    return [DefaultInfo(files = depset([output]))]

lifr_strings = rule(
    implementation = _lifr_strings_impl,
    doc = """The checked-in String Catalog with every string the libraries in
`deps` show added, and the ones they no longer show marked stale.

The libraries must be built with the `swift.emit_localized_strings`
feature, which `--config=xcode` turns on.""",
    attrs = {
        "catalog": attr.label(
            allow_single_file = [".xcstrings"],
            doc = "The checked-in catalog.",
            mandatory = True,
        ),
        "deps": attr.label_list(
            aspects = [_strings_aspect],
            doc = "The app's libraries.",
        ),
        "ios_deps": attr.label_list(
            aspects = [_strings_aspect],
            cfg = _ios,
            doc = "The libraries whose words differ on iOS, built for it too.",
        ),
    } | apple_support.action_required_attrs(),
    fragments = ["apple"],
)
