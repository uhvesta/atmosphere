"""Register Rust toolchains from the Cargo workspace metadata."""

load("@rules_rust//rust/private:repositories.bzl", "rust_register_toolchains")
load("@toml.bzl", "toml")
load("//tools/rules/platform:host.bzl", "host_binary_key")

_TOOLCHAIN_TRIPLES = {
    "darwin_amd64": "x86_64-apple-darwin",
    "darwin_arm64": "aarch64-apple-darwin",
    "linux_amd64": "x86_64-unknown-linux-gnu",
    "linux_arm64": "aarch64-unknown-linux-gnu",
}

# Targets we cross-compile to (see //tools/platforms). The musl toolchains
# need the @llvm libc constraint, or they would also match glibc platforms.
# The glibc toolchains leave libc unconstrained so they also serve
# @platforms//host on Linux (exec-configuration builds, for example proc
# macros). Their order in MODULE.bazel's register_toolchains makes musl win
# for musl platforms.
_CROSS_TARGETS = {
    "aarch64-apple-darwin": ["@platforms//os:macos", "@platforms//cpu:aarch64"],
    "aarch64-unknown-linux-gnu": ["@platforms//os:linux", "@platforms//cpu:aarch64"],
    "aarch64-unknown-linux-musl": ["@platforms//os:linux", "@platforms//cpu:aarch64", "@llvm//constraints/libc:musl"],
    "x86_64-unknown-linux-gnu": ["@platforms//os:linux", "@platforms//cpu:x86_64"],
    "x86_64-unknown-linux-musl": ["@platforms//os:linux", "@platforms//cpu:x86_64", "@llvm//constraints/libc:musl"],
}

def _rust_metadata_impl(repository_ctx):
    repository_ctx.file("BUILD.bazel", 'exports_files(["edition.bzl"], visibility = ["@//tools/format:__pkg__"])\n')
    repository_ctx.file("edition.bzl", 'RUST_EDITION = "{}"\n'.format(repository_ctx.attr.edition))

_rust_metadata = repository_rule(
    implementation = _rust_metadata_impl,
    attrs = {"edition": attr.string(mandatory = True)},
)

def _rust_from_cargo_impl(module_ctx):
    cargo = toml.decode(module_ctx.read(Label("//:Cargo.toml")))
    workspace_package = cargo["workspace"]["package"]
    triple = _TOOLCHAIN_TRIPLES[host_binary_key(module_ctx.os)]
    toolchain_triples = {triple: "rust_host"}

    rust_register_toolchains(
        hub_name = "rust_toolchains",
        edition = workspace_package["edition"],
        versions = [workspace_package["rust-version"]],
        toolchain_triples = toolchain_triples,
        extra_target_triples = _CROSS_TARGETS,
        rustfmt_toolchain_triples = toolchain_triples,
    )
    _rust_metadata(name = "rust_metadata", edition = workspace_package["edition"])
    return module_ctx.extension_metadata(reproducible = True)

rust_from_cargo = module_extension(
    implementation = _rust_from_cargo_impl,
    os_dependent = True,
    arch_dependent = True,
)
