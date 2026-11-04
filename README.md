# monty-oxide

monty-oxide is an example of a polyglot, monorepo-style codebase that builds its own opinionated development tools from first principles. Planned, not done: an example CI scheduler and setup, an example remote build execution (RBE) implementation, a Linux sandbox tool, its own linters, and possibly its own version control.

## Bootstrap

- Install [direnv](https://direnv.net/docs/installation.html) and hook it into your shell.
- Run `direnv allow`. This runs `tools/bootstrap/bazelisk.sh`, which installs the pinned Bazelisk as `bazel` in `.tools/bin` and puts it on `PATH`.

Everything else (toolchains, linters, and formatting tools) is fetched hermetically by Bazel.

Without direnv, run `sh tools/bootstrap/bazelisk.sh` and add `.tools/bin` to `PATH`.

## Contributing

- `bazel build //...` and `bazel test //...` build and test everything.
- `bazel run //:format` formats the code.
- `bazel build //:all` lints the code.

For stacked PRs or trees of PRs, `bazel run //tools/bootstrap/revup` installs [revup](https://github.com/Skydio/revup).

`AGENTS.md` holds the code guidelines. Toolchain details are in [TOOLCHAINS.md](TOOLCHAINS.md).
