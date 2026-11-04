"""Install Bazel-provided developer tools into the ignored .tools directory."""

import argparse
import filecmp
import os
from pathlib import Path
import shutil
import sys
import tempfile


def runfile(path: str) -> Path:
    runfiles_dir = os.environ.get("RUNFILES_DIR")
    if not runfiles_dir:
        runfiles_dir = next(
            (parent for parent in Path(__file__).parents if parent.name.endswith(".runfiles")),
            None,
        )
    if runfiles_dir is None:
        raise ValueError("Bazel runfiles directory is missing")
    file = (Path(runfiles_dir) / path).resolve(strict=True)
    if not file.is_file():
        raise ValueError(f"Bazel runfile is not a file: {path}")
    return file


def require_executable(path: Path) -> None:
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"Expected executable is missing: {path}")


def link(source: Path, destination: Path) -> None:
    if destination.is_symlink():
        if destination.resolve() == source:
            return
        destination.unlink()
    elif destination.exists():
        raise ValueError(f"Refusing to replace a non-symlink: {destination}")
    destination.symlink_to(source)


def copy_executable(source: Path, destination: Path) -> None:
    if destination.is_file() and not destination.is_symlink():
        if filecmp.cmp(source, destination, shallow=False):
            return

    with tempfile.NamedTemporaryFile(dir=destination.parent, prefix=".devtool.", delete=False) as temp:
        temporary_path = Path(temp.name)
    try:
        shutil.copyfile(source, temporary_path)
        temporary_path.chmod(0o755)
        temporary_path.replace(destination)
    finally:
        temporary_path.unlink(missing_ok=True)


def main() -> None:
    workspace = os.environ.get("BUILD_WORKSPACE_DIRECTORY")
    if not workspace:
        raise ValueError("Run this installer with bazel run //tools/ide:install_devtools")

    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--go", required=True)
    parser.add_argument("--cargo", required=True)
    parser.add_argument("--gopls", required=True)
    parser.add_argument("--starpls", required=True)
    parser.add_argument("--rust-analyzer-toolchain", nargs="+", required=True)
    args = parser.parse_args()

    go = runfile(args.go)
    cargo = runfile(args.cargo)
    gopls = runfile(args.gopls)
    starpls = runfile(args.starpls)
    analyzer_paths = [
        path for path in args.rust_analyzer_toolchain if path.endswith("/bin/rust-analyzer")
    ]
    if len(analyzer_paths) != 1:
        raise ValueError("Expected one rust-analyzer binary in Bazel runfiles")
    analyzer = runfile(analyzer_paths[0])

    go_root = go.parent.parent
    rust_root = cargo.parent.parent
    analyzer_root = analyzer.parent.parent

    for executable in (
        go_root / "bin/go",
        rust_root / "bin/cargo",
        rust_root / "bin/rustc",
        analyzer_root / "bin/rust-analyzer",
        gopls,
        starpls,
    ):
        require_executable(executable)

    tools_dir = Path(workspace) / ".tools"
    bin_dir = tools_dir / "bin"
    sdk_dir = tools_dir / "sdk"
    bin_dir.mkdir(parents=True, exist_ok=True)
    sdk_dir.mkdir(parents=True, exist_ok=True)

    link(go_root, sdk_dir / "go")
    link(rust_root, sdk_dir / "rust")
    link(analyzer_root, sdk_dir / "rust-analyzer")
    link(analyzer_root / "bin/rust-analyzer", bin_dir / "rust-analyzer")
    copy_executable(gopls, bin_dir / "gopls")
    copy_executable(starpls, bin_dir / "starpls")
    print(f"Installed Go, Rust, gopls, rust-analyzer, and starpls under {tools_dir}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        sys.exit(str(error))
