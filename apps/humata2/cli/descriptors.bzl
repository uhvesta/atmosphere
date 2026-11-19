"""Writes one FileDescriptorSet for a proto_library and everything it imports.

The CLI prints any reply as JSON by walking its descriptor (CLI.md §7), so
it embeds the descriptors of every message it can receive. protoc reads the
descriptor sets Bazel already made for each proto_library and writes them
out again as one set, imports included.
"""

load("@protobuf//bazel/common:proto_common.bzl", "proto_common")
load("@protobuf//bazel/common:proto_info.bzl", "ProtoInfo")

def _import_path(source):
    return proto_common.get_import_path(source)

def _proto_descriptor_set_impl(ctx):
    targets = list(ctx.attr.protos)
    if ctx.attr.proto != None:
        targets.append(ctx.attr.proto)
    if not targets:
        fail("proto_descriptor_set requires `proto` or `protos`")

    infos = [t[ProtoInfo] for t in targets]
    transitive_sets = depset(transitive = [info.transitive_descriptor_sets for info in infos])
    direct_sources = depset(transitive = [depset(info.direct_sources) for info in infos])
    out = ctx.actions.declare_file(ctx.label.name + ".binpb")

    args = ctx.actions.args()
    args.add_joined(
        transitive_sets,
        join_with = ctx.configuration.host_path_separator,
        format_joined = "--descriptor_set_in=%s",
    )
    args.add(out, format = "--descriptor_set_out=%s")
    args.add("--include_imports")
    args.add_all(direct_sources, map_each = _import_path)

    ctx.actions.run(
        executable = ctx.executable._protoc,
        arguments = [args],
        inputs = transitive_sets,
        outputs = [out],
        mnemonic = "ProtoDescriptorSet",
        progress_message = "Writing the descriptor set of %{label}",
    )
    return [DefaultInfo(files = depset([out]))]

proto_descriptor_set = rule(
    implementation = _proto_descriptor_set_impl,
    doc = "Writes `<name>.binpb`: the FileDescriptorSet of `proto`/`protos`'s sources and their imports.",
    attrs = {
        "proto": attr.label(
            doc = "A single proto_library whose sources, with their imports, are written.",
            providers = [ProtoInfo],
        ),
        "protos": attr.label_list(
            doc = "Multiple proto_library targets whose sources, with their imports, are written.",
            providers = [ProtoInfo],
        ),
        "_protoc": attr.label(
            default = Label("@protobuf//:protoc"),
            cfg = "exec",
            executable = True,
        ),
    },
)
