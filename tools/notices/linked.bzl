"""humata2_linked_crates: the external Rust crates the shipped binaries link.

An aspect walks the normal dependencies of the binaries (not proc macros or
build scripts, which don't ship) and keeps every Rust library that comes from
a crate_universe repository, whichever hub made it. The rule writes one
sorted `name version` line per crate, the version as the spoke repository
spells it (a `+` in a semver build tag becomes `-`). The notices test compares that list
with THIRD_PARTY_NOTICES.txt.
"""

load("@rules_rust//rust:defs.bzl", "rust_common")

_SPOKE_MARKER = "rules_rust++crate+crates"

LinkedCratesInfo = provider(
    doc = "The `name version` lines of the crates a target links.",
    fields = {"crates": "depset[str]"},
)

def _is_version(text):
    parts = text.split(".")
    if len(parts) < 3:
        return False
    for part in parts[:2]:
        if not part.isdigit():
            return False
    return parts[2][:1].isdigit()

def _name_and_version(repo_name):
    # "rules_rust++crate+crates_gpui_fast__foo-bar-1.2.3-rc.1": the spoke repo
    # name ends in <crate>-<version>, and a crate name may hold hyphens.
    spoke = repo_name.split("__", 1)[1]
    parts = spoke.split("-")
    for i in range(1, len(parts)):
        if _is_version(parts[i]):
            return "{} {}".format("-".join(parts[:i]), "-".join(parts[i:]))
    fail("Cannot read the crate name and version from the repository {}.".format(repo_name))

def _targets(value):
    # An attribute is a target, a list of them, or a dict (a split transition
    # keys it by platform; a label-keyed dict holds them as keys). Starlark
    # has no recursion, so unwrap two levels.
    flat = []
    for item in (value.values() + value.keys() if type(value) == "dict" else [value]):
        for inner in (item if type(item) == "list" else [item]):
            if type(inner) == "Target":
                flat.append(inner)
    return flat

def _aspect_impl(target, ctx):
    if "build_script" in ctx.rule.kind:
        # A build script and its build dependencies run at build time only.
        return [LinkedCratesInfo(crates = depset())]
    own = []
    if rust_common.crate_info in target:
        info = target[rust_common.crate_info]
        if info.type == "proc-macro":
            return [LinkedCratesInfo(crates = depset())]
        repo = target.label.repo_name
        if repo.startswith(_SPOKE_MARKER) and "__" in repo:
            own.append(_name_and_version(repo))
    children = []
    for attr_name in dir(ctx.rule.attr):
        if attr_name not in _FOLLOWED:
            continue
        for dep in _targets(getattr(ctx.rule.attr, attr_name)):
            if LinkedCratesInfo in dep:
                children.append(dep[LinkedCratesInfo].crates)
    return [LinkedCratesInfo(crates = depset(own, transitive = children))]

# `deps` carries linked Rust code. `holder`, `humata`, `humatad2` and `stack`
# are the binaries of //apps/humata2/daemon/humatad:tools_mac.
_FOLLOWED = ["deps", "holder", "humata", "humatad2", "stack"]

_linked_aspect = aspect(
    implementation = _aspect_impl,
    attr_aspects = _FOLLOWED,
)

def _impl(ctx):
    found = {}
    for binary in ctx.attr.binaries:
        for line in binary[LinkedCratesInfo].crates.to_list():
            found[line] = True
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, "\n".join(sorted(found)) + "\n")
    return [DefaultInfo(files = depset([out]), runfiles = ctx.runfiles(files = [out]))]

humata2_linked_crates = rule(
    implementation = _impl,
    doc = "One sorted `name version` line per external crate the binaries link.",
    attrs = {
        "binaries": attr.label_list(
            aspects = [_linked_aspect],
            mandatory = True,
            doc = "The shipped binaries, or the bundle rule that holds them.",
        ),
    },
)
