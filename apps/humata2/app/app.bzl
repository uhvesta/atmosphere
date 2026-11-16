"""humata2_app: Humata 2's Mac app bundle, from the Rust desktop binary.

rules_apple's macos_application wants a Swift or Objective-C main, and the
app is a Rust executable, so this rule lays the bundle out itself:

    Humata2.app/Contents/Info.plist
    Humata2.app/Contents/PkgInfo
    Humata2.app/Contents/MacOS/humata2
    Humata2.app/Contents/Helpers/{humatad2,humata2_holder,humata}
    Humata2.app/Contents/Resources/

The app finds its tools in Contents/Helpers (desktop/src/live.rs). The
bundle is signed ad hoc so it runs from the build; the release daemon only
accepts a Developer ID app with the hardened runtime (D135), so
tools/release/release.sh signs it again and names the copy "Humata 2.app".
"""

_SCRIPT = r"""
set -eu
app="$1"; exe="$2"; tools="$3"; platform="$4"; plist="$5"; bundle_id="$6"
contents="$app/Contents"
mkdir -p "$contents/MacOS" "$contents/Helpers" "$contents/Resources"
cp "$exe" "$contents/MacOS/humata2"
chmod 755 "$contents/MacOS/humata2"
for name in humatad2 humata2_holder humata; do
  [ -f "$tools/$platform/$name" ] || {
    echo "humata2_app: $tools has no $platform/$name; the tools bundle must be built for $platform." >&2
    exit 1
  }
  cp "$tools/$platform/$name" "$contents/Helpers/$name"
  chmod 755 "$contents/Helpers/$name"
done
sed "s/@BUNDLE_ID@/$bundle_id/g" "$plist" > "$contents/Info.plist"
printf 'APPL????' > "$contents/PkgInfo"
/usr/bin/codesign --force --sign - "$contents/MacOS/humata2"
/usr/bin/codesign --force --sign - "$app"
"""

def _humata2_app_impl(ctx):
    app = ctx.actions.declare_directory("Humata2.app")
    executable = ctx.attr.executable[DefaultInfo].files_to_run.executable
    tools = ctx.file.tools
    args = ctx.actions.args()
    args.add_all([app.path, executable, tools.path, ctx.attr.platform, ctx.file.infoplist, ctx.attr.bundle_id])
    ctx.actions.run_shell(
        inputs = [executable, tools, ctx.file.infoplist],
        outputs = [app],
        command = _SCRIPT,
        arguments = [args],
        mnemonic = "Humata2App",
        progress_message = "Bundling Humata 2.app",
    )
    return [DefaultInfo(files = depset([app]))]

humata2_app = rule(
    implementation = _humata2_app_impl,
    attrs = {
        # The reverse-DNS bundle ID; the release daemon accepts only this one.
        "bundle_id": attr.string(mandatory = True),
        # The desktop binary, which becomes Contents/MacOS/humata2.
        "executable": attr.label(mandatory = True, executable = True, cfg = "target"),
        # Info.plist with @BUNDLE_ID@ where the bundle ID goes.
        "infoplist": attr.label(mandatory = True, allow_single_file = [".plist"]),
        # The tools folder's subfolder for this Mac, as `uname -sm` names it.
        "platform": attr.string(mandatory = True),
        # A tools_bundle folder (daemon/humatad/tools.bzl).
        "tools": attr.label(mandatory = True, allow_single_file = True),
    },
)
