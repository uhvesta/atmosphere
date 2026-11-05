"""Who signs Lifr, from the person's own user.bazelrc.

The team and bundle ID prefix aren't checked in. Set them in a gitignored
`user.bazelrc` at the repository root, which .bazelrc imports:

    common --repo_env=LIFR_TEAM_ID=ABCDE12345
    common --repo_env=LIFR_BUNDLE_PREFIX=com.yourname
    # Optional: the iCloud container that syncs the records.
    common --repo_env=LIFR_ICLOUD_CONTAINER=iCloud.com.yourname.lifr

`@lifr_identity//:defs.bzl` holds the values, for the bundle IDs, the App
Group, the iCloud container, the provisioning profile and the generated
Swift constants. Without
them the apps build ad hoc as `com.example.lifr`, and signed configs stop
with a message saying what to set.
"""

_TEAM = "LIFR_TEAM_ID"
_PREFIX = "LIFR_BUNDLE_PREFIX"
_DEFAULT_PREFIX = "com.example"
_ICLOUD = "LIFR_ICLOUD_CONTAINER"

def _valid_team(team):
    return len(team) == 10 and team.isalnum() and team.upper() == team

def _valid_prefix(prefix):
    parts = prefix.split(".")
    for part in parts:
        if not part or not part.replace("-", "").isalnum():
            return False
    return len(parts) >= 2

def _valid_container(container):
    return container.startswith("iCloud.") and _valid_prefix(container[len("iCloud."):])

def _lifr_identity_impl(repository_ctx):
    team = repository_ctx.getenv(_TEAM, "") or ""
    prefix = repository_ctx.getenv(_PREFIX, "") or _DEFAULT_PREFIX
    if team and not _valid_team(team):
        fail("{} is \"{}\", which isn't a team ID. Use the 10 capital letters and digits shown under Membership at developer.apple.com/account, in user.bazelrc: common --repo_env={}=ABCDE12345".format(_TEAM, team, _TEAM))
    icloud = repository_ctx.getenv(_ICLOUD, "") or ""
    if icloud and not _valid_container(icloud):
        fail("{} is \"{}\", which isn't an iCloud container ID. Use the one you made under Identifiers at developer.apple.com/account, starting with iCloud., in user.bazelrc: common --repo_env={}=iCloud.com.yourname.lifr".format(_ICLOUD, icloud, _ICLOUD))
    if icloud and not team:
        fail("{} is set without {}. iCloud needs a signed build: set your team too, or remove {} to keep the records on each device.".format(_ICLOUD, _TEAM, _ICLOUD))
    if not _valid_prefix(prefix):
        fail("{} is \"{}\", which isn't a reverse-DNS prefix. Use something like com.yourname in user.bazelrc: common --repo_env={}=com.yourname".format(_PREFIX, prefix, _PREFIX))
    bundle_id = prefix + ".lifr"
    repository_ctx.file("BUILD.bazel", "exports_files([\"defs.bzl\"])\n")
    repository_ctx.file("defs.bzl", "\n".join([
        '"""Generated from {} and {}. See //apps/lifr:identity.bzl."""'.format(_TEAM, _PREFIX),
        "",
        "# Empty when user.bazelrc doesn't set one: the apps then build ad hoc only.",
        "TEAM_ID = {}".format(repr(team)),
        "BUNDLE_ID = {}".format(repr(bundle_id)),
        "CLI_BUNDLE_ID = {}".format(repr(bundle_id + ".cli")),
        "FLOW_HOST_BUNDLE_ID = {}".format(repr(bundle_id + ".flowhost")),
        "# Team-prefixed, so `lifr` reaches the container without a profile.",
        "APP_GROUP = {}".format(repr((team or "UNSIGNED") + "." + bundle_id)),
        "# Empty unless user.bazelrc sets one: the records then stay on each device.",
        "ICLOUD_CONTAINER = {}".format(repr(icloud)),
        "",
    ]))

lifr_identity = repository_rule(
    implementation = _lifr_identity_impl,
    environ = [_TEAM, _PREFIX, _ICLOUD],
    doc = "Writes @lifr_identity//:defs.bzl from LIFR_TEAM_ID, LIFR_BUNDLE_PREFIX and LIFR_ICLOUD_CONTAINER.",
)
