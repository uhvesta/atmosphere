"""Real Copilot /fork, private PTY, exact-child-ID SIGKILL recovery.

`copilot help commands` and the official CLI reference document /fork, not a
--fork startup flag. Parent fork-info metadata may change; parent conversation
messages must remain unchanged. No manual session copying or ID rewriting.
https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference
"""
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import threading
import time
import uuid

from copilot_resume import Completions, FixtureFailure, ids, records, stored, stop_owned


class Terminal:
    def __init__(self, binary, args, project, env):
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(project)
            os.execve(binary, [binary, *args], env)
        self.pid, self.fd = pid, fd
        self.project = str(project.resolve())
        self.trust_answered = False
        self.output = bytearray()
        self.queries = bytearray()
        self.closed = False
        self.status = None
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))

    def write(self, text):
        os.write(self.fd, text.encode())

    def pump(self, seconds=0.05):
        if self.closed:
            return
        if not select.select([self.fd], [], [], seconds)[0]:
            return
        try:
            chunk = os.read(self.fd, 65536)
        except OSError as error:
            if error.errno != errno.EIO:
                raise
            chunk = b""
        if not chunk:
            self.closed = True
            return
        self.output.extend(chunk)
        if len(self.output) > 1024 * 1024:
            del self.output[:-1024 * 1024]
        if not self.trust_answered:
            plain = re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", self.output).decode(errors="replace")
            if "Confirm folder trust" in plain and self.project in plain and "❯ 1. Yes" in plain:
                # Approve only our exact empty owned project, for this session.
                # Never choose remember-folder or grant tools/URLs globally.
                self.write("\r")
                self.trust_answered = True
        self.queries.extend(chunk)
        # Answer terminal capability queries, not model or application messages.
        responses = {
            b"\x1b[6n": b"\x1b[1;1R", b"\x1b[c": b"\x1b[?1;2c",
            b"\x1b[>c": b"\x1b[>0;1;0c", b"\x1b[?u": b"\x1b[?0u",
            b"\x1b]10;?\x1b\\": b"\x1b]10;rgb:eeee/eeee/eeee\x1b\\",
            b"\x1b]11;?\x1b\\": b"\x1b]11;rgb:2222/2222/2222\x1b\\",
        }
        for query, answer in responses.items():
            if query in self.queries:
                self.queries = self.queries.replace(query, b"")
                os.write(self.fd, answer)
        if len(self.queries) > 256:
            del self.queries[:-256]

    def kill(self):
        if self.status is None:
            try:
                os.killpg(self.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            deadline = time.monotonic() + 8
            while self.status is None and time.monotonic() < deadline:
                pid, status = os.waitpid(self.pid, os.WNOHANG)
                if pid:
                    self.status = status
                    break
                self.pump()
            if self.status is None:
                raise FixtureFailure("cleanup", "the owned interactive CLI did not exit after SIGKILL")
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None
        return self.status


def conversation(entries):
    return [(event["type"], event.get("data", {}).get("content")) for event in entries if event.get("type") in {"user.message", "assistant.message"}]


def environment(root, port):
    home, state, cache, project = [root / name for name in ["home", "copilot", "cache", "project"]]
    for folder in [home, state, cache, project, root / "tmp"]:
        folder.mkdir(mode=0o700)
    env = {
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(home),
        "COPILOT_HOME": str(state), "COPILOT_CACHE_HOME": str(cache),
        "TMPDIR": str(root / "tmp"), "XDG_CONFIG_HOME": str(home / "config"),
        "XDG_STATE_HOME": str(home / "state"), "XDG_DATA_HOME": str(home / "data"),
        "COPILOT_OFFLINE": "true", "COPILOT_TELEMETRY_DISABLED": "true",
        "COPILOT_OTEL_ENABLED": "false", "COPILOT_DISABLE_LOGIN_SHELL_ENV": "true",
        "COPILOT_PLUGIN_DIR_ONLY": "true", "COPILOT_DISABLE_DESKTOP_NOTIFICATIONS": "1",
        "COPILOT_SYSTEM_PROXY": "0", "HTTP_PROXY": "http://127.0.0.1:9",
        "HTTPS_PROXY": "http://127.0.0.1:9", "NO_PROXY": "127.0.0.1,localhost",
        "COPILOT_PROVIDER_BASE_URL": f"http://127.0.0.1:{port}/v1",
        "COPILOT_PROVIDER_TYPE": "openai", "COPILOT_PROVIDER_WIRE_API": "completions",
        "COPILOT_PROVIDER_MODEL_ID": "gpt-4.1", "COPILOT_PROVIDER_WIRE_MODEL": "fixture-resume",
        "COPILOT_MODEL": "gpt-4.1", "COPILOT_PROVIDER_MAX_PROMPT_TOKENS": "128000",
        "COPILOT_PROVIDER_MAX_OUTPUT_TOKENS": "1024", "TERM": "xterm-256color", "NO_COLOR": "1", "CI": "1",
    }
    return state, project, env


def wait_state(terminal, home, predicate, stage, seconds=35):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        terminal.pump()
        try:
            state = stored(home)
        except FixtureFailure:
            # A concurrently appended final JSONL line may be incomplete. A
            # persistent parse failure still fails this bounded operation.
            continue
        if predicate(state):
            return state
        if terminal.closed:
            raise FixtureFailure(stage, "the real interactive CLI exited before completing the operation")
    raise FixtureFailure(stage, "the real interactive CLI did not persist the expected session state; inspect the owned terminal trace")


def main():
    binary = os.environ.get("TERMINAL_COPILOT_BIN")
    if not binary or not Path(binary).is_file():
        raise FixtureFailure("setup", "set TERMINAL_COPILOT_BIN to the installed real Copilot executable")
    output = Path(os.environ.get("TEST_UNDECLARED_OUTPUTS_DIR", "/tmp/copilot-fork-proof"))
    output.mkdir(parents=True, exist_ok=True)
    nonce = uuid.uuid4().hex[:12]
    parent_prompt, branch_prompt, verify_prompt = [name + "_" + nonce for name in ["PARENT_USER_FIXTURE", "CHILD_ONLY_USER_FIXTURE", "VERIFY_CHILD_USER_FIXTURE"]]
    server = Completions((parent_prompt, "UNUSED_PENDING_" + nonce, branch_prompt))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    children, terminal = [], None
    try:
        with tempfile.TemporaryDirectory(prefix="h2-copilot-fork-", dir="/tmp") as directory:
            root = Path(directory)
            state_home, project, env = environment(root, server.server_port)
            common = ["--no-auto-update", "--disable-client-telemetry", "--disable-builtin-mcps", "--no-custom-instructions", "--no-bash-env", "--no-ask-user", "--sync-session=off", "--log-level=error", "--no-color", "--no-mouse"]
            version = subprocess.run([binary, "--no-auto-update", "--version"], cwd=project, env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20, check=True).stdout.splitlines()[0]

            def prompt(stage, text, identity=None):
                args = [binary, *common, "--output-format=json", "--stream=off"] + ([] if identity is None else ["--resume=" + identity]) + ["--prompt", text]
                process = subprocess.Popen(args, cwd=project, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
                children.append(process)
                stdout, stderr = process.communicate(timeout=40)
                (output / (stage + ".jsonl")).write_text(stdout)
                (output / (stage + ".stderr.txt")).write_text(stderr)
                if process.returncode:
                    raise FixtureFailure(stage, "the real CLI failed: " + stderr[-1800:])
                return records(stdout, stage)

            seed_events = prompt("parent-seed", parent_prompt)
            parent_ids = ids(seed_events)
            saved = stored(state_home)
            if len(parent_ids) != 1 or set(saved) != parent_ids:
                raise FixtureFailure("seed", "the real CLI did not create one structured parent UUID")
            parent = parent_ids.pop()
            parent_messages = conversation(saved[parent])
            parent_log_before = (state_home / "session-state" / parent / "events.jsonl").read_bytes()
            if parent_prompt not in json.dumps(parent_messages) or "SAVED_ASSISTANT_FIXTURE" not in json.dumps(parent_messages):
                raise FixtureFailure("seed", "the parent turn was not committed")
            before_fork_calls = len(server.requests)
            # --interactive feeds this real slash command through Copilot's
            # interactive submission path; no SDK fork call or filesystem edit.
            terminal = Terminal(binary, [*common, "--resume=" + parent, "--interactive", "/fork branch-" + nonce], project, env)
            saved = wait_state(terminal, state_home, lambda state: parent in state and len(state) == 2 and all(conversation(events) == parent_messages for identity, events in state.items() if identity != parent), "native /fork")
            child = next(identity for identity in saved if identity != parent)
            if ids(saved[child]) != {child} or len(server.requests) != before_fork_calls:
                raise FixtureFailure("native /fork", "the fork did not produce a new native UUID without an inference call")
            if conversation(saved[parent]) != parent_messages or conversation(saved[child]) != parent_messages:
                raise FixtureFailure("native /fork", "the real fork did not preserve parent conversation history independently")
            # The fork itself is a real native TUI operation. Terminate that
            # process, then use documented prompt mode to commit a child-only
            # turn without guessing the interactive editor's submit semantics.
            status = terminal.kill()
            (output / "copilot-fork-terminal.txt").write_bytes(terminal.output)
            if not os.WIFSIGNALED(status) or os.WTERMSIG(status) != signal.SIGKILL:
                raise FixtureFailure("fork SIGKILL", "the owned interactive CLI was not killed by SIGKILL")
            terminal = None
            prompt("child-only", branch_prompt, child)
            saved = stored(state_home)
            if branch_prompt not in json.dumps(conversation(saved[child])) or "RESUMED_ASSISTANT_FIXTURE" not in json.dumps(conversation(saved[child])):
                raise FixtureFailure("child-only turn", "the branch-only completed turn did not persist in the native fork")
            pending_prompt = "PENDING_CHILD_USER_FIXTURE_" + nonce
            server.markers = (parent_prompt, pending_prompt, verify_prompt)
            args = [binary, *common, "--output-format=json", "--stream=off", "--resume=" + child, "--prompt", pending_prompt]
            pending = subprocess.Popen(args, cwd=project, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
            children.append(pending)
            if not server.pending.wait(35):
                raise FixtureFailure("child pending turn", "the forked child did not reach its held local inference request")
            pending_stdout, pending_stderr = stop_owned(pending)
            (output / "child-interrupted.jsonl").write_text(pending_stdout)
            (output / "child-interrupted.stderr.txt").write_text(pending_stderr)
            if pending.returncode != -signal.SIGKILL:
                raise FixtureFailure("child SIGKILL", "the forked child CLI was not killed by SIGKILL")
            server.release.set()
            server.markers = (parent_prompt, "UNUSED_PENDING_" + nonce, verify_prompt)
            prompt("child-resumed", verify_prompt, child)
            saved = stored(state_home)
            parent_log_after = (state_home / "session-state" / parent / "events.jsonl").read_bytes()
            if set(saved) != {parent, child} or ids(saved[child]) != {child} or ids(saved[parent]) != {parent}:
                raise FixtureFailure("resume child", "resume selected or created an unexpected session UUID")
            if conversation(saved[parent]) != parent_messages or branch_prompt in json.dumps(saved[parent]) or verify_prompt in json.dumps(saved[parent]):
                raise FixtureFailure("parent isolation", "fork-only conversation leaked into the parent")
            last = json.dumps(server.requests[-1]["body"])
            expected = [parent_prompt, "SAVED_ASSISTANT_FIXTURE", branch_prompt, "RESUMED_ASSISTANT_FIXTURE", verify_prompt]
            if not all(value in last for value in expected):
                raise FixtureFailure("resume child", "the resumed child did not send inherited and branch-only committed history to the local model")
            if server.failures:
                raise FixtureFailure("provider", "; ".join(server.failures))
            proof = {"proof": "real-copilot-native-fork-sigkill-child-resume", "version": version, "operation": "/fork", "parent_session_id": parent, "child_session_id": child, "different_child_uuid": parent != child, "child_id_preserved_after_sigkill": True, "fork_cli_sigkill": True, "branch_pending_turn_sigkill": True, "inherited_parent_history_retained": True, "branch_only_history_retained": True, "parent_conversation_unchanged": True, "parent_event_log_bytes_unchanged": parent_log_after == parent_log_before, "parent_event_log_note": "Copilot appends native fork/resume metadata; conversation messages remain unchanged", "parent_log_sha256_before": hashlib.sha256(parent_log_before).hexdigest(), "parent_log_sha256_after": hashlib.sha256(parent_log_after).hexdigest(), "model_requests": len(server.requests), "offline": True, "provider": "deterministic loopback fixture", "personal_state_used": False}
            (output / "copilot-fork-proof.json").write_text(json.dumps(proof, indent=2))
            print(json.dumps(proof))
    finally:
        server.release.set()
        if terminal is not None:
            terminal.kill()
            (output / "copilot-fork-terminal.txt").write_bytes(terminal.output)
        for child in reversed(children):
            stop_owned(child)
        server.shutdown()
        server.server_close()
        thread.join(timeout=3)


if __name__ == "__main__":
    try:
        main()
    except (FixtureFailure, subprocess.TimeoutExpired, subprocess.CalledProcessError) as error:
        print(str(error), file=__import__("sys").stderr)
        raise SystemExit(1)
