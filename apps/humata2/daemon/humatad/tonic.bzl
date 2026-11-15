"""Generates tonic's gRPC client and server code for a proto_library.

rust_prost_library makes the message types with the workspace's prost and
no tonic, because Humata 1 shares its toolchain. This rule runs
protoc-gen-tonic on the same protos and writes one file per package, which
a rust_library includes beside the message types.
"""

load("@protobuf//bazel/common:proto_common.bzl", "proto_common")
load("@protobuf//bazel/common:proto_info.bzl", "ProtoInfo")

def _import_path(source):
    return proto_common.get_import_path(source)

def _rust_tonic_services_impl(ctx):
    info = ctx.attr.proto[ProtoInfo]
    out = ctx.actions.declare_file("{}/{}.tonic.rs".format(ctx.label.name, ctx.attr.package))

    args = ctx.actions.args()
    args.add_joined(
        info.transitive_descriptor_sets,
        join_with = ctx.configuration.host_path_separator,
        format_joined = "--descriptor_set_in=%s",
    )
    args.add(ctx.executable.plugin, format = "--plugin=protoc-gen-tonic=%s")
    args.add(out.dirname, format = "--tonic_out=%s")
    args.add("--tonic_opt=no_include,flat_output_dir")
    args.add_all(info.direct_sources, map_each = _import_path)

    ctx.actions.run(
        executable = ctx.executable._protoc,
        arguments = [args],
        inputs = info.transitive_descriptor_sets,
        tools = [ctx.executable.plugin],
        outputs = [out],
        mnemonic = "TonicGen",
        progress_message = "Generating tonic services for %{label}",
    )
    return [DefaultInfo(files = depset([out]))]

rust_tonic_services = rule(
    implementation = _rust_tonic_services_impl,
    doc = "Writes `<name>/<package>.tonic.rs`: tonic's clients and servers for the services in `proto`.",
    attrs = {
        "package": attr.string(
            doc = "The proto package whose services are generated.",
            mandatory = True,
        ),
        "plugin": attr.label(
            doc = "protoc-gen-tonic, built from the workspace's pinned crate.",
            cfg = "exec",
            executable = True,
            mandatory = True,
        ),
        "proto": attr.label(
            doc = "The proto_library whose own sources are generated.",
            mandatory = True,
            providers = [ProtoInfo],
        ),
        "_protoc": attr.label(
            default = Label("@protobuf//:protoc"),
            cfg = "exec",
            executable = True,
        ),
    },
)
