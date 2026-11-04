"""Aspect linters for source targets in this repository."""

load("@aspect_rules_lint//lint:buildifier.bzl", "lint_buildifier_aspect")
load("@aspect_rules_lint//lint:shellcheck.bzl", "lint_shellcheck_aspect")
load("@aspect_rules_lint//lint:vale.bzl", "lint_vale_aspect")
load("@aspect_rules_lint_rust//:clippy.bzl", "lint_clippy_aspect")

buildifier = lint_buildifier_aspect(
    binary = Label("@buildifier//:buildifier"),
    args = ["--mode=check"],
)

clippy = lint_clippy_aspect(
    config = Label("//:.clippy.toml"),
)

shellcheck = lint_shellcheck_aspect(
    binary = Label("@aspect_rules_lint//lint:shellcheck_bin"),
    config = Label("//:.shellcheckrc"),
)

vale = lint_vale_aspect(
    binary = Label("//bazel/lint:vale"),
    config = Label("//:.vale.ini"),
)
