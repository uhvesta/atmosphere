"""tools_bundle: humatad2, humata2_holder, humata, stack and humata2_fusefs for every platform (TS-2).

The bundle is one folder, the one `humatad2 serve --tools` reads:

    manifest.json
    <platform>/humatad2
    <platform>/humata2_holder
    <platform>/humata
    <platform>/stack
    <platform>/humata2_fusefs

Platforms are named as `uname -sm` names them, lowercased (`darwin-arm64`,
`linux-x86_64`, `linux-aarch64`), so the daemon finds a machine's files by
asking it. The manifest gives each file's path, relative to the folder, and
its SHA-256, which the daemon checks before sending anything.
"""

def _split_impl(_settings, attr):
    return {
        name: {"//command_line_option:platforms": str(platform)}
        for platform, name in attr.platforms.items()
    }

_split = transition(
    implementation = _split_impl,
    inputs = [],
    outputs = ["//command_line_option:platforms"],
)

# The file names the daemon looks for, by attribute.
_FILES = {
    "fusefs": "humata2_fusefs",
    "holder": "humata2_holder",
    "humata": "humata",
    "humatad2": "humatad2",
    "stack": "stack",
}

# Copies each build into the folder (a Mac build signed ad hoc, as the
# linker signs only the Mac's own architecture and an app holding unsigned
# code can't be signed), then writes the manifest. Arguments come in
# threes: platform, file name, build.
_SCRIPT = r"""
set -eu
out="$1"
shift
mkdir -p "$out"
sum() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    sha256sum "$1" | cut -d' ' -f1
  fi
}
manifest='{"platforms": {'
last=''
while [ "$#" -gt 0 ]; do
  platform="$1"; name="$2"; build="$3"
  shift 3
  mkdir -p "$out/$platform"
  cp "$build" "$out/$platform/$name"
  chmod 755 "$out/$platform/$name"
  case "$platform" in
    darwin-*) /usr/bin/codesign --force --sign - "$out/$platform/$name" ;;
  esac
  entry="\"$name\": {\"path\": \"$platform/$name\", \"sha256\": \"$(sum "$out/$platform/$name")\"}"
  if [ "$platform" = "$last" ]; then
    manifest="$manifest, $entry"
  else
    [ -z "$last" ] || manifest="$manifest}, "
    manifest="$manifest\"$platform\": {$entry"
    last="$platform"
  fi
done
[ -z "$last" ] || manifest="$manifest}"
printf '%s}}\n' "$manifest" > "$out/manifest.json"
"""

def _tools_bundle_impl(ctx):
    out = ctx.actions.declare_directory(ctx.label.name)
    args = ctx.actions.args()
    args.add(out.path)
    inputs = []
    for platform in sorted(ctx.attr.platforms.values()):
        for attribute, name in sorted(_FILES.items()):
            build = getattr(ctx.split_attr, attribute)[platform][DefaultInfo].files_to_run.executable
            inputs.append(build)
            args.add_all([platform, name, build])
    ctx.actions.run_shell(
        inputs = inputs,
        outputs = [out],
        command = _SCRIPT,
        arguments = [args],
        mnemonic = "Humata2ToolsBundle",
        progress_message = "Bundling Humata 2's tools for %s" % ", ".join(sorted(ctx.attr.platforms.values())),
    )
    return [DefaultInfo(files = depset([out]))]

tools_bundle = rule(
    implementation = _tools_bundle_impl,
    attrs = {
        # The mount's own process (//apps/humata2/daemon/fusefs_adapter), apart
        # from humatad2 so an exec swap of the daemon never tears the mount down.
        "fusefs": attr.label(cfg = _split, mandatory = True),
        "holder": attr.label(cfg = _split, mandatory = True),
        "humata": attr.label(cfg = _split, mandatory = True),
        "humatad2": attr.label(cfg = _split, mandatory = True),
        # Each platform to build for, with its name as `uname -sm` gives it.
        "platforms": attr.label_keyed_string_dict(mandatory = True),
        "stack": attr.label(cfg = _split, mandatory = True),
    },
)
