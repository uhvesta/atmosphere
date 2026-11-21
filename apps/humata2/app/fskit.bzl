"""humata2_fskit_appex: builds the macOS FSKit `.appex` bundle (`HumataFS.appex`).

Compiles the Swift `UnaryFileSystemExtension` against the host macOS SDK's
`FSKit.framework` and `ExtensionFoundation.framework`, stamps its `Info.plist`
and entitlements with the bundle ID and App Group, and signs it ad hoc so it
ships inside `Humata2.app/Contents/Extensions/HumataFS.appex`.
"""

_SCRIPT = r"""
set -eu
appex="$1"; plist="$2"; entitlements="$3"; bundle_id="$4"; app_group="$5"
shift 5

contents="$appex/Contents"
mkdir -p "$contents/MacOS"

sdk=$(/usr/bin/xcrun --sdk macosx --show-sdk-path)
/usr/bin/xcrun --sdk macosx swiftc \
  -target arm64-apple-macos26.0 \
  -sdk "$sdk" \
  -parse-as-library \
  -O \
  -framework FSKit \
  -framework ExtensionFoundation \
  -framework Foundation \
  -o "$contents/MacOS/HumataFS" \
  "$@"

chmod 755 "$contents/MacOS/HumataFS"
sed -e "s/@EXTENSION_BUNDLE_ID@/$bundle_id/g" \
    -e "s/@APP_GROUP@/$app_group/g" \
    "$plist" > "$contents/Info.plist"
printf 'XPC!????' > "$contents/PkgInfo"

ent_tmp=$(mktemp)
trap 'rm -f "$ent_tmp"' EXIT
sed "s/@APP_GROUP@/$app_group/g" "$entitlements" > "$ent_tmp"

/usr/bin/codesign --force --sign - --entitlements "$ent_tmp" "$contents/MacOS/HumataFS"
/usr/bin/codesign --force --sign - --entitlements "$ent_tmp" "$appex"
"""

def _humata2_fskit_appex_impl(ctx):
    appex = ctx.actions.declare_directory("HumataFS.appex")
    args = ctx.actions.args()
    args.add_all([
        appex.path,
        ctx.file.infoplist.path,
        ctx.file.entitlements.path,
        ctx.attr.bundle_id,
        ctx.attr.app_group,
    ])
    args.add_all(ctx.files.srcs)
    ctx.actions.run_shell(
        inputs = [ctx.file.infoplist, ctx.file.entitlements] + ctx.files.srcs,
        outputs = [appex],
        command = _SCRIPT,
        arguments = [args],
        mnemonic = "Humata2FsKitAppex",
        progress_message = "Building HumataFS.appex",
        # Uses host Xcode SDK (FSKit.framework is not in the hermetic LLVM SDK).
        execution_requirements = {"no-sandbox": "1"},
    )
    return [DefaultInfo(files = depset([appex]))]

humata2_fskit_appex = rule(
    implementation = _humata2_fskit_appex_impl,
    attrs = {
        "app_group": attr.string(mandatory = True),
        "bundle_id": attr.string(mandatory = True),
        "entitlements": attr.label(mandatory = True, allow_single_file = [".entitlements"]),
        "infoplist": attr.label(mandatory = True, allow_single_file = [".plist"]),
        "srcs": attr.label_list(mandatory = True, allow_files = [".swift"]),
    },
)
