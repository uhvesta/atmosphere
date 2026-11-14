"""Entitlements from the signing identity in user.bazelrc (see identity.bzl)."""

load("@bazel_skylib//rules:common_settings.bzl", "BuildSettingInfo")
load("@lifr_identity//:defs.bzl", "APP_GROUP", "ICLOUD_CONTAINER", "IOS_APP_GROUP", "TEAM_ID")

_PLIST = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
{}</dict>
</plist>
"""

_APP_GROUP = """\t<key>com.apple.security.application-groups</key>
\t<array>
\t\t<string>{}</string>
\t</array>
"""

_ICLOUD = """\t<key>com.apple.developer.icloud-container-identifiers</key>
\t<array>
\t\t<string>{}</string>
\t</array>
\t<key>com.apple.developer.icloud-services</key>
\t<array>
\t\t<string>CloudKit</string>
\t</array>
"""

# TestFlight and App Store builds must use CloudKit's Production
# environment; Xcode adds this when it exports one.
_PRODUCTION = """\t<key>com.apple.developer.icloud-container-environment</key>
\t<string>Production</string>
"""

_KEYCHAIN = """\t<key>keychain-access-groups</key>
\t<array>
{}\t</array>
"""

# macOS only runs sandboxed extensions, such as the widgets.
_SANDBOX = """\t<key>com.apple.security.app-sandbox</key>
\t<true/>
"""

# The hardened runtime blocks the microphone without it.
_MICROPHONE = """\t<key>com.apple.security.device.audio-input</key>
\t<true/>
"""

def _lifr_entitlements_impl(ctx):
    if ctx.attr._signed[BuildSettingInfo].value and not TEAM_ID:
        fail("Signed builds need your Apple team. Add `common --repo_env=LIFR_TEAM_ID=<your team ID>` " +
             "and `common --repo_env=LIFR_BUNDLE_PREFIX=<e.g. com.yourname>` to user.bazelrc at the " +
             "repository root (it's gitignored). The team ID is under Membership at " +
             "developer.apple.com/account. Changing the prefix gives the apps new bundle IDs, so " +
             "an installed Lifr keeps its data under the old one.")
    output = ctx.actions.declare_file(ctx.label.name + ".entitlements")
    keys = _APP_GROUP.format(APP_GROUP) if ctx.attr.app_group else ""
    if IOS_APP_GROUP and ctx.attr.ios_app_group:
        keys += _APP_GROUP.format(IOS_APP_GROUP)
    if ICLOUD_CONTAINER and ctx.attr.icloud:
        keys += _ICLOUD.format(ICLOUD_CONTAINER)
        if ctx.attr.production:
            keys += _PRODUCTION
    groups = [group for group in ctx.attr.keychain_groups if group]
    if groups:
        keys += _KEYCHAIN.format("".join(["\t\t<string>{}</string>\n".format(group) for group in groups]))
    if ctx.attr.microphone:
        keys += _MICROPHONE
    if ctx.attr.sandbox:
        keys += _SANDBOX
    ctx.actions.write(output, _PLIST.format(keys))
    return [DefaultInfo(files = depset([output]))]

lifr_entitlements = rule(
    implementation = _lifr_entitlements_impl,
    attrs = {
        "app_group": attr.bool(
            doc = "Whether to grant the Mac App Group whose container holds the local API's socket.",
        ),
        "icloud": attr.bool(
            default = True,
            doc = "Whether to grant iCloud when user.bazelrc sets a container. An unsigned build can't " +
                  "have it: macOS won't launch an ad hoc signed app that claims it.",
        ),
        "ios_app_group": attr.bool(
            doc = "Whether to grant the iPhone App Group the widgets read Today from, when user.bazelrc " +
                  "sets one.",
        ),
        "keychain_groups": attr.string_list(
            doc = "The keychain access groups it may use, the first being where its own items go. Empty " +
                  "ones, as unsigned builds have, are left out.",
        ),
        "microphone": attr.bool(
            doc = "Whether to let the Mac app, under the hardened runtime, record from the microphone.",
        ),
        "production": attr.bool(
            doc = "Whether iCloud uses CloudKit's Production environment, as TestFlight and App Store " +
                  "builds must. Off, it uses Development.",
        ),
        "sandbox": attr.bool(
            doc = "Whether to sandbox it, which macOS requires of the widgets' extension.",
        ),
        "_signed": attr.label(default = "//apps/lifr:signed"),
    },
    doc = "Writes `<name>.entitlements`, with iCloud when user.bazelrc sets a container, and stops a " +
          "signed build that has no team set.",
)
