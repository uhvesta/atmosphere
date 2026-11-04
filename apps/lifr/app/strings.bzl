"""`lifr_strings`: the app's String Catalog, synced with the code's strings."""

load("@apple_support//lib:apple_support.bzl", "apple_support")

visibility("private")

_StringsInfo = provider(
    doc = "The .stringsdata directories of Lifr's Swift libraries.",
    fields = {"directories": "depset of directories of .stringsdata files"},
)

def _strings_aspect_impl(target, ctx):
    own = []
    is_lifr = ctx.label.workspace_name == "" and ctx.label.package.startswith("apps/lifr/")
    if is_lifr and OutputGroupInfo in target and hasattr(target[OutputGroupInfo], "swift_localized_strings"):
        own = [target[OutputGroupInfo].swift_localized_strings]
    deps = [dep[_StringsInfo].directories for dep in getattr(ctx.rule.attr, "deps", []) if _StringsInfo in dep]
    return [_StringsInfo(directories = depset(transitive = own + deps))]

_strings_aspect = aspect(
    implementation = _strings_aspect_impl,
    attr_aspects = ["deps"],
)

def _lifr_strings_impl(ctx):
    output = ctx.actions.declare_file(ctx.label.name + "/" + ctx.file.catalog.basename)
    directories = depset(transitive = [dep[_StringsInfo].directories for dep in ctx.attr.deps])
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
    } | apple_support.action_required_attrs(),
    fragments = ["apple"],
)
