"""Package the Bazel-built GPUI Wasm To-do application and its generated JS imports."""

def _bundle_impl(ctx):
    out = ctx.actions.declare_directory(ctx.label.name)
    wasm = ctx.file.wasm
    ctx.actions.run_shell(
        inputs = [wasm, ctx.file.index] + ctx.files.notice_files,
        tools = [ctx.executable._bindgen],
        outputs = [out],
        arguments = [
            ctx.executable._bindgen.path,
            wasm.path,
            out.path,
            ctx.file.index.path,
        ] + [file.path for file in ctx.files.notice_files],
        command = 'set -eu; destination="$3"; "$1" "$2" --target web --out-dir "$destination" --out-name web_todo --remove-name-section; cp "$4" "$destination/index.html"; shift 4; for license in "$@"; do cp "$license" "$destination/"; done',
        mnemonic = "WebTodoWasmBundle",
        progress_message = "Packaging To-do GPUI browser application",
    )
    return [DefaultInfo(files = depset([out]), runfiles = ctx.runfiles(files = [out]))]

wasm_bundle = rule(
    implementation = _bundle_impl,
    attrs = {
        "index": attr.label(allow_single_file = [".html"], mandatory = True),
        "notice_files": attr.label_list(allow_files = [".txt"]),
        "wasm": attr.label(allow_single_file = True, mandatory = True),
        "_bindgen": attr.label(
            default = "@terminal_wasm_bindgen//:wasm-bindgen",
            executable = True,
            cfg = "exec",
            allow_single_file = True,
        ),
    },
)
