"""humata2_notices: THIRD_PARTY_NOTICES.txt for Humata 2.app, from the crate lock.

The repository reads `cargo-bazel-lock.json` and the lock of the
@crates_gpui_fast hub, follows the normal
dependencies of the crates the app ships to find every Rust crate that ends
up in it, and writes one text file: per crate its name, version, license
expression and URL, then the text of every LICENSE, LICENCE, COPYING and
NOTICE file at the top of its source. A crate without such a file gets the
standard text for each of its license IDs (licenses/). A hand-kept section
for what isn't a crate (extras.txt, plus the license files named by
`extra_license_files`) follows.

What counts as shipped: the `deps` of the crates in `roots`, and of the
workspace crates they depend on (read from their Cargo.toml, because the lock
doesn't list path dependencies), for macOS on Apple silicon. Dev
dependencies, proc macros and build scripts don't reach the app, and the
lock keeps them apart, so they are left out.

The repository holds:

    THIRD_PARTY_NOTICES.txt   the notices
    CRATES.txt                one "name version" line per crate in it
"""

_TARGET = "aarch64-apple-darwin"
_SPOKE = "@@rules_rust++crate+{hub}__{name}-{version}//:BUILD.bazel"
_LICENSE_PREFIXES = ("LICENSE", "LICENCE", "COPYING", "NOTICE")
_RULE = "=" * 80
_THIN = "-" * 80

def _lock_deps(entry, conditions):
    # `deps` is a list, or {"common": [...], "selects": {condition: [...]}}.
    deps = (entry.get("common_attrs") or {}).get("deps") or []
    if type(deps) == "list":
        return [dep["id"] for dep in deps]
    ids = [dep["id"] for dep in deps.get("common", [])]
    for condition, selected in deps.get("selects", {}).items():
        if _TARGET in conditions.get(condition, []):
            ids += [dep["id"] for dep in selected]
    return ids

def _manifest_deps(text, workspace_names):
    # The workspace crates a Cargo.toml depends on, outside dev and build
    # dependencies.
    found = []
    section = ""
    for raw in text.split("\n"):
        line = raw.strip()
        if line.startswith("["):
            section = line
            if line.startswith("[dependencies.") and line.endswith("]"):
                name = line[len("[dependencies."):-1].strip()
                if name in workspace_names:
                    found.append(name)
            continue
        if section == "[dependencies]" or (section.startswith("[target.") and section.endswith(".dependencies]")):
            end = len(line)
            for mark in ("=", "."):
                at = line.find(mark)
                if at >= 0 and at < end:
                    end = at
            name = line[:end].strip()
            if name in workspace_names:
                found.append(name)
    return found

def _license_files(directory):
    files = []
    for child in directory.readdir():
        upper = child.basename.upper()
        if upper.startswith(_LICENSE_PREFIXES) and not child.is_dir:
            files.append(child)
    return sorted(files, key = lambda f: f.basename)

def _section(title, body_parts):
    lines = [_RULE, title, _THIN]
    for heading, text in body_parts:
        lines.append(heading)
        lines.append("")
        lines.append(text.strip("\n"))
        lines.append("")
    return "\n".join(lines)

def _walk(rctx, lock_label, roots, hub):
    """The shipped crates of one hub: lock entries, each with its spoke repo."""
    lock = json.decode(rctx.read(lock_label))
    crates = lock["crates"]
    conditions = lock["conditions"]
    members = lock["workspace_members"]
    workspace_ids = {id.split(" ")[0]: id for id in members}

    pending = list(roots) if roots else list(members.keys())
    seen = {}
    for _ in range(100000):
        if not pending:
            break
        id = pending.pop()
        if id in seen:
            continue
        entry = crates.get(id)
        if entry == None:
            fail("{} is in humata2_notices roots or a Cargo.toml but not in {}. Regenerate the lock with `CARGO_BAZEL_REPIN=1 bazel mod tidy`.".format(id, lock_label))
        seen[id] = True
        pending += _lock_deps(entry, conditions)
        if id in members:
            manifest = rctx.read(Label("@@//{}:Cargo.toml".format(members[id])))
            pending += [workspace_ids[name] for name in _manifest_deps(manifest, workspace_ids)]
    return [(crates[id], hub) for id in seen if crates[id].get("repository")]

def _notices_impl(rctx):
    fallbacks = {}
    for label in rctx.attr.fallback_licenses:
        name = label.name.rsplit("/", 1)[-1]
        fallbacks[name.removesuffix(".txt")] = rctx.read(label)

    # Every crate the shipped binaries link, from both hubs. A crate at the
    # same version in both gets one section, read from @crates.
    found = {}
    for entry, hub in (
        _walk(rctx, rctx.attr.gpui_fast_lock, rctx.attr.gpui_fast_roots, "crates_gpui_fast") +
        _walk(rctx, rctx.attr.lock, rctx.attr.roots, "crates")
    ):
        found["{} {}".format(entry["name"], entry["version"])] = (entry, hub)

    listing = sorted(found.keys())
    shipped = listing
    sections = []
    for key in listing:
        entry, hub = found[key]
        name = entry["name"]
        version = entry["version"]
        source = rctx.path(Label(_SPOKE.format(hub = hub, name = name, version = version.replace("+", "-")))).dirname
        parts = [(f.basename, rctx.read(f)) for f in _license_files(source)]
        if not parts:
            for spdx in entry.get("license_ids", []):
                text = fallbacks.get(spdx)
                if text == None:
                    text = "The crate ships no license file. The standard text of {0} is at https://spdx.org/licenses/{0}.html".format(spdx)
                parts.append(("{} (standard text; the crate ships no license file)".format(spdx), text))
        title = "\n".join([
            "Crate: {} {}".format(name, version),
            "License: {}".format(entry.get("license") or "not stated in the lock"),
            "URL: {}".format(entry.get("package_url") or entry.get("repository", {}).get("Http", {}).get("url", "")),
        ])
        sections.append(_section(title, parts))

    header = "\n".join([
        "Humata 2 THIRD-PARTY NOTICES",
        "",
        "Humata 2 includes the software below. Each part is under its own license,",
        "whose text follows its name. This file is generated from Humata 2's crate",
        "lock and the Ghostty source it is built with; edit neither by hand.",
        "",
        "{} Rust crates, then the parts that are not crates.".format(len(shipped)),
        "",
    ])
    extras = rctx.read(rctx.attr.extras).strip("\n") + "\n"
    extra_parts = [(label.name, rctx.read(label)) for label in rctx.attr.extra_license_files]
    extra_section = "\n".join([_RULE, "Parts that are not crates", _THIN, extras, ""]) + "\n".join(
        ["{}\n\n{}\n".format(heading, text.strip("\n")) for heading, text in extra_parts],
    )

    rctx.file("THIRD_PARTY_NOTICES.txt", header + "\n" + "\n".join(sections) + "\n" + extra_section + "\n")
    rctx.file("CRATES.txt", "\n".join(listing) + "\n")
    rctx.file("BUILD.bazel", 'exports_files(["THIRD_PARTY_NOTICES.txt", "CRATES.txt"], visibility = ["//visibility:public"])\n')

humata2_notices = repository_rule(
    implementation = _notices_impl,
    doc = "THIRD_PARTY_NOTICES.txt and CRATES.txt for Humata 2.app (see the file docs).",
    attrs = {
        "extra_license_files": attr.label_list(
            default = [Label("@libghostty_vt//:LICENSE")],
            doc = "License files of the parts that are not crates, appended after extras.",
        ),
        "extras": attr.label(
            default = Label("//tools/notices:extras.txt"),
            allow_single_file = True,
            doc = "The hand-kept text for the parts that are not crates.",
        ),
        "fallback_licenses": attr.label_list(
            default = [
                Label("//tools/notices:licenses/0BSD.txt"),
                Label("//tools/notices:licenses/Apache-2.0.txt"),
                Label("//tools/notices:licenses/BSD-2-Clause.txt"),
                Label("//tools/notices:licenses/BSD-3-Clause.txt"),
                Label("//tools/notices:licenses/BSL-1.0.txt"),
                Label("//tools/notices:licenses/CC0-1.0.txt"),
                Label("//tools/notices:licenses/MIT.txt"),
                Label("//tools/notices:licenses/MPL-2.0.txt"),
                Label("//tools/notices:licenses/Unicode-3.0.txt"),
                Label("//tools/notices:licenses/Unlicense.txt"),
                Label("//tools/notices:licenses/Zlib.txt"),
            ],
            doc = "Standard license texts, named <SPDX ID>.txt, for crates with no license file.",
        ),
        "gpui_fast_lock": attr.label(
            default = Label("//apps/humata2/engine/gpui-fast-crates:cargo-bazel-lock.json"),
            allow_single_file = True,
            doc = "The lock of the @crates_gpui_fast hub, which GPUI Fast targets (terminal, markdown, diff) link.",
        ),
        "gpui_fast_roots": attr.string_list(
            default = [],
            doc = "Lock IDs to start from in the GPUI Fast lock. Empty means its workspace member, whose Cargo.toml lists only what GPUI Fast targets name.",
        ),
        "lock": attr.label(
            default = Label("//:cargo-bazel-lock.json"),
            allow_single_file = True,
            doc = "The crate_universe lock; the repository is rebuilt when it changes.",
        ),
        "roots": attr.string_list(
            default = [
                "humata2-desktop 0.0.0",
                "humata2-holder 0.0.0",
                "humata2-cli 0.0.0",
                "humatad2 0.0.0",
            ],
            doc = "Lock IDs of the binaries the app ships: the desktop app, the holder, the humata CLI and humatad2.",
        ),
    },
)
