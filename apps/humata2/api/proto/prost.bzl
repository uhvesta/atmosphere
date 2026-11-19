"""Generates same-package prost message crates for split humata.v1 proto domains.

Every humata/v1/*.proto file shares `package humata.v1;` (wire-contractual for
gRPC paths and error details). This rule runs protoc-gen-prost on only a
proto_library's `direct_sources` with `--prost_opt=extern_path=.google.protobuf=::prost_types`
and wraps the generated `humata.v1.rs` in a `rust_library` that re-exports its
direct domain proto dependencies so all same-package type references resolve.
"""

load("@protobuf//bazel/common:proto_common.bzl", "proto_common")
load("@protobuf//bazel/common:proto_info.bzl", "ProtoInfo")
load("@rules_rust//rust:defs.bzl", "rust_library")

def _import_path(source):
    return proto_common.get_import_path(source)

def _rust_prost_messages_impl(ctx):
    info = ctx.attr.proto[ProtoInfo]
    out = ctx.actions.declare_file("{}/{}.rs".format(ctx.label.name, ctx.attr.package))

    args = ctx.actions.args()
    args.add_joined(
        info.transitive_descriptor_sets,
        join_with = ctx.configuration.host_path_separator,
        format_joined = "--descriptor_set_in=%s",
    )
    args.add(ctx.executable.plugin, format = "--plugin=protoc-gen-prost=%s")
    args.add(out.dirname, format = "--prost_out=%s")
    args.add("--prost_opt=flat_output_dir")
    for proto_target, crate_name in ctx.attr.dep_proto_crates.items():
        dep_info = proto_target[ProtoInfo]
        for src in dep_info.direct_sources:
            args.add("--prost_opt=dep={}={}".format(_import_path(src), crate_name))
    args.add_all(info.direct_sources, map_each = _import_path)

    ctx.actions.run(
        executable = ctx.executable._protoc,
        arguments = [args],
        inputs = info.transitive_descriptor_sets,
        tools = [ctx.executable.plugin],
        outputs = [out],
        mnemonic = "ProstDomainGen",
        progress_message = "Generating prost messages for %{label}",
    )
    return [DefaultInfo(files = depset([out]))]

rust_prost_messages = rule(
    implementation = _rust_prost_messages_impl,
    doc = "Writes `<name>/<package>.rs`: prost message types for `proto`'s direct sources.",
    attrs = {
        "dep_proto_crates": attr.label_keyed_string_dict(
            doc = "Mapping from imported domain proto_library targets to their Rust crate names.",
            default = {},
            providers = [ProtoInfo],
        ),
        "package": attr.string(
            doc = "The proto package whose messages are generated.",
            mandatory = True,
        ),
        "plugin": attr.label(
            doc = "protoc-gen-prost, built from the workspace's pinned crate.",
            cfg = "exec",
            executable = True,
            mandatory = True,
        ),
        "proto": attr.label(
            doc = "The proto_library whose direct sources are generated.",
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

def _prost_domain_lib_rs_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".rs")
    vis = "" if ctx.attr.include_prost else "pub "
    reexports = "".join([
        "        {}use {}::humata::v1::*;\n".format(vis, crate)
        for crate in ctx.attr.reexport_crates
    ])
    prost_include = '        include!(env!("HUMATA2_PROST_RS"));\n' if ctx.attr.include_prost else ""
    compat_mods = (
        "\npub mod timestamp_proto {\n" +
        "    pub mod google {\n" +
        "        pub mod protobuf {\n" +
        "            pub use prost_types::Timestamp;\n" +
        "        }\n" +
        "    }\n" +
        "}\n\n" +
        "pub mod duration_proto {\n" +
        "    pub mod google {\n" +
        "        pub mod protobuf {\n" +
        "            pub use prost_types::Duration;\n" +
        "        }\n" +
        "    }\n" +
        "}\n"
    ) if ctx.attr.include_compat_mods else ""
    content = (
        "// Generated wrapper for same-package prost domain crate.\n" +
        "#![allow(unused_imports, ambiguous_glob_reexports)]\n\n" +
        "pub mod humata {\n" +
        "    pub mod v1 {\n" +
        reexports +
        prost_include +
        "    }\n" +
        "}\n\n" +
        "pub use prost_types::{Duration, Timestamp};\n" +
        compat_mods
    )
    ctx.actions.write(output = out, content = content)
    return [DefaultInfo(files = depset([out]))]

_prost_domain_lib_rs = rule(
    implementation = _prost_domain_lib_rs_impl,
    attrs = {
        "include_compat_mods": attr.bool(default = False),
        "include_prost": attr.bool(default = True),
        "reexport_crates": attr.string_list(default = []),
    },
)

def rust_prost_domain(
        name,
        proto,
        crate_name,
        deps = [],
        dep_proto_crates = {},
        reexport_crates = [],
        visibility = None):
    """Defines a same-package prost rust_library for a humata.v1 domain proto_library.

    Args:
        name: Name of the generated rust_library target.
        proto: The domain proto_library target.
        crate_name: Rust crate name for the generated library.
        deps: Direct domain rust_library targets to depend on.
        dep_proto_crates: Mapping from imported domain proto_library targets to their Rust crate names.
        reexport_crates: Crate names whose `humata::v1::*` items are re-exported.
        visibility: Bazel visibility list for the generated rust_library.
    """
    gen_name = name + "_gen"
    lib_rs_name = name + "_lib_rs"

    rust_prost_messages(
        name = gen_name,
        dep_proto_crates = dep_proto_crates,
        package = "humata.v1",
        plugin = ":protoc-gen-prost-domain",
        proto = proto,
    )

    _prost_domain_lib_rs(
        name = lib_rs_name,
        reexport_crates = reexport_crates,
    )

    rust_library(
        name = name,
        srcs = [":" + lib_rs_name],
        compile_data = [":" + gen_name],
        crate_name = crate_name,
        crate_root = ":" + lib_rs_name,
        edition = "2021",
        rustc_env = {"HUMATA2_PROST_RS": "$(execpath :{})".format(gen_name)},
        tags = ["noclippy"],
        visibility = visibility,
        deps = deps + [
            "@crates//:prost",
            "@crates//:prost-types",
        ],
    )

def rust_prost_umbrella(
        name,
        crate_name,
        deps,
        reexport_crates,
        visibility = None):
    """Defines a thin re-export umbrella rust_library combining all domain prost crates.

    Args:
        name: Name of the generated umbrella rust_library target.
        crate_name: Rust crate name for the umbrella library.
        deps: Domain prost rust_library targets to combine.
        reexport_crates: Domain prost crate names whose `humata::v1::*` items are re-exported.
        visibility: Bazel visibility list for the umbrella rust_library.
    """
    lib_rs_name = name + "_lib_rs"

    _prost_domain_lib_rs(
        name = lib_rs_name,
        include_compat_mods = True,
        include_prost = False,
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
