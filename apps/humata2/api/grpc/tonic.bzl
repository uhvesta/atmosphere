"""Generates tonic's gRPC client and server code for a domain proto_library.

Runs protoc-gen-tonic on a domain proto_library's direct sources and wraps the
generated `humata.v1.tonic.rs` beside the domain's prost message crate so both
read as one `humata::v1` module.
"""

load("@protobuf//bazel/common:proto_common.bzl", "proto_common")
load("@protobuf//bazel/common:proto_info.bzl", "ProtoInfo")
load("@rules_rust//rust:defs.bzl", "rust_library")

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

def _grpc_domain_lib_rs_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".rs")
    private_uses = "".join([
        "        use {}::humata::v1::*;\n".format(crate)
        for crate in ctx.attr.private_use_crates
    ])
    reexports = "".join([
        "        pub use {}::humata::v1::*;\n".format(crate)
        for crate in ctx.attr.reexport_crates
    ])
    tonic_include = '        include!(env!("HUMATA2_TONIC_RS"));\n' if ctx.attr.include_tonic else ""
    content = (
        "// Generated wrapper for humata.v1 gRPC domain crate.\n" +
        "#![allow(unused_imports, ambiguous_glob_reexports)]\n\n" +
        "pub mod humata {\n" +
        "    pub mod v1 {\n" +
        private_uses +
        reexports +
        tonic_include +
        "    }\n" +
        "}\n\n" +
        "pub use prost_types::{Duration, Timestamp};\n"
    )
    ctx.actions.write(output = out, content = content)
    return [DefaultInfo(files = depset([out]))]

_grpc_domain_lib_rs = rule(
    implementation = _grpc_domain_lib_rs_impl,
    attrs = {
        "include_tonic": attr.bool(default = True),
        "private_use_crates": attr.string_list(default = []),
        "reexport_crates": attr.string_list(default = []),
    },
)

def rust_grpc_domain(
        name,
        proto,
        proto_rust,
        proto_crate_name,
        crate_name,
        dep_proto_rusts = [],
        dep_proto_crates = [],
        visibility = None):
    """Defines a per-domain tonic gRPC rust_library for a humata.v1 domain.

    Args:
        name: Name of the generated rust_library target.
        proto: The domain proto_library target.
        proto_rust: The domain prost rust_library target.
        proto_crate_name: Rust crate name of `proto_rust` to re-export.
        crate_name: Rust crate name for the generated gRPC library.
        dep_proto_rusts: Additional domain prost rust_library targets needed by tonic services.
        dep_proto_crates: Crate names of `dep_proto_rusts` imported privately into `humata::v1`.
        visibility: Bazel visibility list for the generated rust_library.
    """
    gen_name = name + "_tonic_gen"
    lib_rs_name = name + "_lib_rs"

    rust_tonic_services(
        name = gen_name,
        package = "humata.v1",
        plugin = ":protoc-gen-tonic",
        proto = proto,
    )

    _grpc_domain_lib_rs(
        name = lib_rs_name,
        include_tonic = True,
        private_use_crates = dep_proto_crates,
        reexport_crates = [proto_crate_name],
    )

    rust_library(
        name = name,
        srcs = [":" + lib_rs_name],
        compile_data = [":" + gen_name],
        crate_name = crate_name,
        crate_root = ":" + lib_rs_name,
        edition = "2021",
        rustc_env = {"HUMATA2_TONIC_RS": "$(execpath :{})".format(gen_name)},
        tags = ["noclippy"],
        visibility = visibility,
        deps = [proto_rust] + dep_proto_rusts + [
            "@crates//:prost",
            "@crates//:prost-types",
            "@crates//:tonic",
            "@crates//:tonic-prost",
        ],
    )

def rust_grpc_umbrella(
        name,
        crate_name,
        deps,
        reexport_crates,
        visibility = None):
    """Defines a thin re-export umbrella rust_library combining all domain gRPC crates.

    Args:
        name: Name of the generated umbrella rust_library target.
        crate_name: Rust crate name for the umbrella library.
        deps: Domain gRPC rust_library targets to combine.
        reexport_crates: Domain gRPC crate names whose `humata::v1::*` items are re-exported.
        visibility: Bazel visibility list for the umbrella rust_library.
    """
    lib_rs_name = name + "_lib_rs"

    _grpc_domain_lib_rs(
        name = lib_rs_name,
        include_tonic = False,
        reexport_crates = reexport_crates,
    )

    rust_library(
        name = name,
        srcs = [":" + lib_rs_name],
        crate_name = crate_name,
        crate_root = ":" + lib_rs_name,
        edition = "2021",
        tags = ["noclippy"],
        visibility = visibility,
        deps = deps + [
            "@crates//:prost-types",
        ],
    )
