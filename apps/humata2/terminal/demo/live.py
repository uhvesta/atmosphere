"""Publish a Bazel-built milestone to one private, persistent local demo host."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
from urllib.error import URLError
from urllib.request import urlopen


def private_directory(path):
    try:
        path.mkdir(mode=0o700, parents=True)
    except FileExistsError:
        pass
    if not path.is_dir() or path.stat().st_mode & 0o077:
        raise RuntimeError(f"Demo state must be a private directory: {path}. Choose a new directory; existing permissions were preserved.")


def publish(bundle, assets, promote):
    """Keep older versioned assets so existing pages survive a milestone update."""
    private_directory(assets)
    if not (bundle / "terminal_web_bg.wasm").is_file():
        raise RuntimeError("The built browser bundle is incomplete. Rebuild the Bazel live target.")
    digest = hashlib.sha256()
    for source in sorted(bundle.rglob("*")):
        if source.is_file():
            digest.update(source.relative_to(bundle).as_posix().encode() + b"\0")
            digest.update(source.read_bytes())
    release = "build-" + digest.hexdigest()[:20]
    destination = assets / release
    if not destination.exists():
        temporary = Path(tempfile.mkdtemp(prefix=".publishing-", dir=assets))
        try:
            shutil.copytree(bundle, temporary, dirs_exist_ok=True)
            temporary.rename(destination)
        finally:
            if temporary.exists():
                shutil.rmtree(temporary)
    index = (bundle / "index.html").read_text()
    reference = "import('./terminal_web.js')"
    if reference not in index:
        raise RuntimeError("The built viewer entry point changed. Update the demo publisher before refreshing this milestone.")
    index = index.replace(reference, f"import('./{release}/terminal_web.js')")
    if promote:
        temporary_index = assets / ".index.next"
        temporary_index.write_text(index)
        temporary_index.replace(assets / "index.html")
    return release


def healthy(port):
    try:
        with urlopen(f"http://127.0.0.1:{port}/health", timeout=1) as response:
            return response.read(32) == b"ok"
    except (OSError, URLError):
        return False


def run():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", type=Path, required=True)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--state", type=Path, default=Path.home() / ".humata-terminal-demo")
    parser.add_argument("--port", type=int, default=8741)
    parser.add_argument("--profiles", type=Path)
    parser.add_argument("--start", action="append", default=[])
    parser.add_argument("--stage", action="store_true", help="Prepare candidate assets without changing the stable viewer entry point.")
    parser.add_argument("--quiet", action="store_true", help="Write the private URL file without logging its capability.")
    parser.add_argument("--require-running", action="store_true", help="Refresh assets only; refuse to start a host from a test environment.")
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535:
        raise RuntimeError("Choose a demo port between 1024 and 65535.")
    os.umask(0o077)
    state = args.state.expanduser().absolute()
    private_directory(state)
    with (state / "publisher.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        release = publish(args.bundle, state / "assets", promote=not args.stage)
        pid_path = state / "server.json"
        token_path = state / "browser.token"
        reused = False
        saved = {}
        current_host_build = hashlib.sha256(args.host.read_bytes()).hexdigest()[:20]
        if pid_path.exists():
            saved = json.loads(pid_path.read_text())
            try:
                os.kill(saved["pid"], 0)
                reused = saved["port"] == args.port and token_path.exists() and healthy(args.port)
            except ProcessLookupError:
                pass
            if not reused:
                try:
                    os.kill(saved["pid"], 0)
                except ProcessLookupError:
                    pass
                else:
                    raise RuntimeError("The recorded demo host is still running but its health check or port does not match. Inspect its private server.log before restarting it.")
        if not reused:
            if args.require_running:
                raise RuntimeError("The demo host is not running. Start it with bazel run //apps/humata2/terminal/demo:live in your normal shell before verifying a milestone.")
            if healthy(args.port):
                raise RuntimeError(f"Port {args.port} already has another server. Choose another port; no process was stopped.")
            # Data dependencies include the holder alongside the host runfiles.
            runfiles = Path(os.environ.get("RUNFILES_DIR", str(args.host) + ".runfiles"))
            holder = runfiles / "_main/apps/humata2/daemon/holder/humata2_holder"
            if not holder.is_file():
                raise RuntimeError("The holder executable is missing from demo runfiles. Rebuild the Bazel live target.")
            command = [str(args.host), "--state", str(state), "--holder", str(holder),
                       "--static-dir", str(state / "assets"), "--listen", f"127.0.0.1:{args.port}"]
            if args.profiles:
                command += ["--profiles", str(args.profiles.expanduser().absolute())]
            for profile in args.start or ([] if args.profiles else ["shell"]):
                command += ["--start", profile]
            with (state / "server.log").open("ab") as log:
                child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
            next_pid = state / "server.json.next"
            next_pid.write_text(json.dumps({"pid": child.pid, "port": args.port, "host_build": current_host_build}))
            next_pid.replace(pid_path)
            for _ in range(200):
                if child.poll() is not None:
                    pid_path.unlink(missing_ok=True)
                    raise RuntimeError(f"The demo host exited. Inspect {state / 'server.log'}; existing terminal holders were preserved.")
                if token_path.exists() and healthy(args.port):
                    break
                time.sleep(0.1)
            else:
                raise RuntimeError(f"The demo host has not become healthy. Inspect {state / 'server.log'} before retrying.")
        token = token_path.read_text().strip()
        entry = f"/{release}/index.html" if args.stage else "/"
        viewer_url = f"http://127.0.0.1:{args.port}{entry}#token={token}"
        url_name = "candidate.url" if args.stage else "viewer.url"
        next_url = state / (url_name + ".next")
        next_url.write_text(viewer_url + "\n")
        next_url.replace(state / url_name)
        print(f"Published {release}; {'kept the running host and its terminals' if reused else 'started the persistent terminal host'}.")
        if not args.quiet:
            print(viewer_url)
        print(f"Private state and server log: {state}")
        if reused:
            print("Candidate assets were staged; the stable page is unchanged." if args.stage else "Browser assets were refreshed; the existing host binary and launch profiles keep running.")
            if saved.get("host_build") != current_host_build:
                print("The running host build differs or was not recorded. Gracefully restart with the same private state when deploying backend changes; the running host was preserved.")


if __name__ == "__main__":
    try:
        run()
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        print(f"Could not publish the terminal demo: {error}", file=sys.stderr)
        sys.exit(1)
