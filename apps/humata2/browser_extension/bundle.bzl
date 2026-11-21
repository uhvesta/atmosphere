"""Packages the read-only Humata 2 browser extension into self-contained chromium/ and firefox/ directories."""

_SCRIPT = r"""
set -eu
out="$1"
chromium_manifest="$2"
firefox_manifest="$3"
background_js="$4"
content_js="$5"

mkdir -p "$out/chromium/shared" "$out/firefox/shared"
cp "$chromium_manifest" "$out/chromium/manifest.json"
cp "$background_js" "$out/chromium/shared/background.js"
cp "$content_js" "$out/chromium/shared/content.js"

cp "$firefox_manifest" "$out/firefox/manifest.json"
cp "$background_js" "$out/firefox/shared/background.js"
cp "$content_js" "$out/firefox/shared/content.js"
"""

def _browser_extension_bundle_impl(ctx):
    out = ctx.actions.declare_directory(ctx.label.name)
    args = ctx.actions.args()
    args.add_all([
        out.path,
        ctx.file.chromium_manifest,
        ctx.file.firefox_manifest,
        ctx.file.background_js,
        ctx.file.content_js,
    ])
    ctx.actions.run_shell(
        inputs = [
            ctx.file.chromium_manifest,
            ctx.file.firefox_manifest,
            ctx.file.background_js,
            ctx.file.content_js,
        ],
        outputs = [out],
        command = _SCRIPT,
        arguments = [args],
        mnemonic = "BrowserExtensionBundle",
        progress_message = "Packaging Humata 2 read-only browser extension",
    )
    return [DefaultInfo(files = depset([out]))]

browser_extension_bundle = rule(
    implementation = _browser_extension_bundle_impl,
    attrs = {
        "background_js": attr.label(mandatory = True, allow_single_file = [".js"]),
        "chromium_manifest": attr.label(mandatory = True, allow_single_file = [".json"]),
        "content_js": attr.label(mandatory = True, allow_single_file = [".js"]),
        "firefox_manifest": attr.label(mandatory = True, allow_single_file = [".json"]),
    },
)
