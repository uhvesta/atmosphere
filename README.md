# monty-oxide

- **Bootstrap:** `sh tools/scripts/bootstrap/bazelisk.sh` installs pinned Bazelisk as `bazel` on macOS or Linux (x86-64 or ARM64).
- **Direnv (optional):** Install [direnv](https://direnv.net/docs/installation.html), hook it into your shell, then run `direnv allow` to bootstrap Bazelisk and put `.tools/bin` on `PATH` automatically. Without direnv, add `.tools/bin` to `PATH` yourself.
- **Editor tools (optional):** `bazel run //tools:install_devtools` installs Go, Rust, `gopls`, `rust-analyzer`, and `starpls` under ignored `.tools`. Point your editor at them manually.
- **Stacked PRs (optional):** `bazel run //tools/scripts/bootstrap/revup` installs [revup](https://github.com/Skydio/revup) as `.tools/bin/revup`, with its own pinned Python, git, and GitHub CLI. The GitHub token comes from `GH_TOKEN` or your existing `gh` login and is never written to disk. It also sets this clone's commit template to `.gitmessage` (What, Why, Approach), which revup turns into the PR description.
- **Pinned versions:** Go comes from `go.mod`, Rust from `Cargo.toml`, Bazel from `.bazelversion`, and the bootstrap tools (Bazelisk, revup, and revup's Python, git, and GitHub CLI) from `tools/scripts/bootstrap/versions.toml`. After editing that file, `bazel run //tools/scripts/bootstrap/lock` pins each download's URL and SHA-256 in `versions.lock`.
- **Build and test:** `bazel build //...` and `bazel test //...`.
- **Generate Go BUILD files:** `bazel run //:gazelle`.
- **Version checks:** `bazel run check-deps` reports all drift;
  `bazel run update-deps` applies it. Scope either with `-- --bazel`,
  `--bzlmod`, `--golang`, `--gomod`, `--rust`, or `--crates`; `--all`
  is explicit.
- **Format:** `bazel run //:format -- path/to/files`; check with `bazel run //:format.check -- path/to/files`.
