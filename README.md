# atmosphere

atmosphere is an example of a polyglot, monorepo-style codebase that builds its own opinionated development tools from first principles. Planned, not done: an example CI scheduler and setup, an example remote build execution (RBE) implementation, a Linux sandbox tool, its own linters, and possibly its own version control.

## Bootstrap

- Install [direnv](https://direnv.net/docs/installation.html) and hook it into your shell.
- Run `direnv allow`. This runs `tools/bootstrap/bazelisk.sh`, which installs the pinned Bazelisk as `bazel` in `.tools/bin` and puts it on `PATH`.

Everything else (toolchains, linters, and formatting tools) is fetched hermetically by Bazel.

Without direnv, run `sh tools/bootstrap/bazelisk.sh`, then add `.tools/bin` to `PATH` or run `.tools/bin/bazel` directly.

## Running Humata

`./humata.sh` installs Bazelisk if needed, builds the Humata Mac app, installs it as `~/Applications/Humata.app` and opens it. It needs Xcode. Pass `--no-open` to install without opening.

## Contributing

- `bazel build //...` and `bazel test //...` build and test everything.
- `bazel run //:format` formats the code.

For stacked PRs or trees of PRs, `bazel run //tools/bootstrap/revup` installs [revup](https://github.com/Skydio/revup).

## Planned: vendored Rust dependencies

Today Rust crates come from crates.io through `crate_universe`, and a crate we need to change gets a small patch through its `crate.annotation(patches = …)` in `MODULE.bazel`. To revisit: vendor our Rust dependencies and build them with Bazel, set up automatically. Each crate's upstream source would be checked in with its license and a record of its version and checksum, and our changes kept as named patch files on top. One command would update a crate (respecting the 3-day cooldown), reapply the patches, stop on a conflict, and run the tests that use it. A check would fail if the checked-in source isn't exactly upstream plus our patches.

`AGENTS.md` holds the code guidelines. Toolchain details are in [TOOLCHAINS.md](TOOLCHAINS.md).
