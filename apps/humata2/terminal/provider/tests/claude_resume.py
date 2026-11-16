"""Installed Claude Code recovery, isolated state and synthetic local Messages API.

Official protocol: https://platform.claude.com/docs/en/build-with-claude/streaming
Routing: https://code.claude.com/docs/en/env-vars
No real credentials, upstream model calls, user repositories or personal state.
"""
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import uuid

from codex_resume import stop_owned

SAVED_USER = "CLAUDE_SAVED_USER_FIXTURE"
SAVED_ASSISTANT = "CLAUDE_SAVED_ASSISTANT_FIXTURE"
PENDING_USER = "CLAUDE_PENDING_USER_FIXTURE"
VERIFY_USER = "CLAUDE_VERIFY_USER_FIXTURE"


class FixtureFailure(Exception):
    def __init__(self, stage, reason):
        self.stage = stage
        self.reason = reason
        super().__init__(f"Claude Code recovery fixture failed during {stage}: {reason}")


def events(output, stage):
    try:
        return [json.loads(line) for line in output.splitlines() if line.strip()]
    except json.JSONDecodeError as error:
        raise FixtureFailure(stage, "CLI did not emit its documented JSON event stream") from error


class Messages(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(("127.0.0.1", 0), Handler)
        self.lock = threading.Lock()
        self.requests = []
        self.total_requests = 0
        self.pending = threading.Event()
        self.release = threading.Event()


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def send_json(self, value):
        data = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        route = self.path.split("?", 1)[0]
        if route not in ["/v1/messages", "/v1/messages/count_tokens"]:
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        if not 0 < length <= 2 * 1024 * 1024:
            self.send_error(413)
            return
        body = json.loads(self.rfile.read(length))
        if route.endswith("/count_tokens"):
            self.send_json({"input_tokens": 10})
            return
        last_user = next((json.dumps(message.get("content", "")) for message in reversed(body.get("messages", [])) if message.get("role") == "user"), "")
        stage = next((marker for marker in [VERIFY_USER, PENDING_USER, SAVED_USER] if marker in last_user), None)
        with self.server.lock:
            self.server.total_requests += 1
            if stage:
                self.server.requests.append((stage, body))
            index = len(self.server.requests)
        if stage == PENDING_USER:
            self.server.pending.set()
            self.server.release.wait(50)
        answer = SAVED_ASSISTANT if stage == SAVED_USER else "CLAUDE_RESTORED_ASSISTANT_FIXTURE"
        message = {"id": f"msg_fixture_{index}", "type": "message", "role": "assistant", "model": body.get("model", "fake"), "content": [{"type": "text", "text": answer}], "stop_reason": "end_turn", "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 1}}
        try:
            if not body.get("stream"):
                self.send_json(message)
                return
            start = dict(message, content=[], stop_reason=None, usage={"input_tokens": 10, "output_tokens": 0})
            frames = [
                {"type": "message_start", "message": start},
                {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
                {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": answer}},
                {"type": "content_block_stop", "index": 0},
                {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 1}},
                {"type": "message_stop"},
            ]
            data = "".join(f"event: {frame['type']}\ndata: {json.dumps(frame)}\n\n" for frame in frames).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


def session(records, expected, stage):
    ids = {record["session_id"] for record in records if record.get("session_id")}
    if ids != {expected}:
        raise FixtureFailure(stage, "structured session identifiers did not match the exact seeded UUID")
    return expected


def completed(records, stage):
    results = [record for record in records if record.get("type") == "result"]
    if len(results) != 1 or results[0].get("is_error") or results[0].get("subtype") != "success":
        raise FixtureFailure(stage, "the actual CLI did not emit a successful final result")


def run():
    binary = os.environ.get("TERMINAL_CLAUDE_BIN")
    if not binary or not Path(binary).is_file():
        raise FixtureFailure("setup", "set TERMINAL_CLAUDE_BIN to the installed Claude Code executable")
    server = Messages()
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    owned = []
    try:
        with tempfile.TemporaryDirectory(prefix="h2-claude-resume-", dir=os.environ.get("TEST_TMPDIR", "/tmp")) as directory:
            root = Path(directory)
            home, config, project = [root / part for part in ["home", "claude", "project"]]
            for folder in [home, config, project]:
                folder.mkdir(mode=0o700)
            env = {"PATH": "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(home), "CLAUDE_CONFIG_DIR": str(config), "XDG_CONFIG_HOME": str(home / "config"), "XDG_STATE_HOME": str(home / "state"), "XDG_DATA_HOME": str(home / "data"), "TERM": "dumb", "NO_COLOR": "1", "CI": "1", "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{server.server_port}", "ANTHROPIC_API_KEY": "sk-ant-local-fixture-not-a-real-credential", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "DISABLE_AUTOUPDATER": "1"}
            version = subprocess.run([binary, "--version"], env=env, cwd=project, capture_output=True, text=True, timeout=10, check=True).stdout.strip()
            common = ["-p", "--verbose", "--output-format", "stream-json", "--model", "fake", "--setting-sources", ""]

            def launch(args):
                process = subprocess.Popen([binary, *common, *args], cwd=project, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
                owned.append(process)
                return process

            identity = str(uuid.uuid4())
            first = launch(["--session-id", identity, SAVED_USER])
            stdout, stderr = first.communicate(timeout=50)
            if first.returncode != 0:
                raise FixtureFailure("Claude first turn", "isolated Messages fixture failed: " + stderr[-1500:])
            records = events(stdout, "Claude first turn")
            session(records, identity, "Claude first turn")
            completed(records, "Claude first turn")
            active = launch(["--resume", identity, PENDING_USER])
            if not server.pending.wait(35):
                raise FixtureFailure("Claude interruption", "the resumed CLI did not reach its held loopback model request")
            if active.poll() is not None:
                raise FixtureFailure("Claude interruption", "the CLI exited before its intended SIGKILL")
            stdout, _ = stop_owned(active)
            if active.returncode != -signal.SIGKILL:
                raise FixtureFailure("Claude interruption", "the fixture process was not terminated by SIGKILL")
            session(events(stdout, "Claude interruption"), identity, "Claude interruption")
            server.release.set()
            restored = launch(["--resume", identity, VERIFY_USER])
            stdout, stderr = restored.communicate(timeout=50)
            if restored.returncode != 0:
                raise FixtureFailure("Claude recovery", "the exact saved conversation did not resume: " + stderr[-1500:])
            records = events(stdout, "Claude recovery")
            session(records, identity, "Claude recovery")
            completed(records, "Claude recovery")
            with server.lock:
                requests = list(server.requests)
            resumed = [body for stage, body in requests if stage == VERIFY_USER]
            if len(resumed) != 1:
                raise FixtureFailure("Claude recovery", "expected exactly one explicit recovery verification turn")
            messages = json.dumps(resumed[0].get("messages", []))
            if SAVED_USER not in messages or SAVED_ASSISTANT not in messages or VERIFY_USER not in messages:
                raise FixtureFailure("Claude recovery", "the request omitted the prior saved user/assistant context")
            with server.lock:
                before_missing = server.total_requests
            missing = launch(["--resume", str(uuid.uuid4()), "CLAUDE_MISSING_RESUME_FIXTURE"])
            missing.communicate(timeout=20)
            with server.lock:
                after_missing = server.total_requests
            if missing.returncode == 0 or after_missing != before_missing:
                raise FixtureFailure("Claude missing conversation", "resume silently created or queried a different conversation")
            proof = {"provider": "installed Claude Code", "version": version, "model": "deterministic loopback Anthropic Messages fixture", "structuredSessionId": True, "sameConversationAfterSigkill": True, "retainedUserAndAssistantContext": True, "missingSessionRejectedBeforeModel": True, "realCredentialsUsed": False, "personalStateUsed": False, "upstreamModelCalls": False, "primaryRequestStages": [stage for stage, _ in requests]}
            output = Path(os.environ.get("TEST_UNDECLARED_OUTPUTS_DIR", root))
            (output / "claude-resume-proof.json").write_text(json.dumps(proof, indent=2) + "\n")
            print(json.dumps(proof))
    finally:
        server.release.set()
        for process in owned:
            stop_owned(process)
        server.shutdown()
        server.server_close()
        worker.join(timeout=3)


if __name__ == "__main__":
    try:
        run()
    except (FixtureFailure, subprocess.TimeoutExpired, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error
