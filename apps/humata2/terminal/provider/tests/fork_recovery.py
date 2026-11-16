"""Actual provider fork -> committed branch -> SIGKILL -> exact child resume.

Uses only new private state and deterministic loopback model protocols. No
session files are copied/read by this harness; provider CLIs own persistence.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import uuid

from codex_resume import isolated, stop_owned

PARENT_USER = "PARENT_COMMITTED_USER_FIXTURE"
PARENT_ANSWER = "PARENT_COMMITTED_ASSISTANT_FIXTURE"
FORK_USER = "FORK_ONLY_COMMITTED_USER_FIXTURE"
FORK_ANSWER = "FORK_ONLY_COMMITTED_ASSISTANT_FIXTURE"
PENDING = "FORK_PENDING_USER_FIXTURE"
CHILD_VERIFY = "FORK_CHILD_VERIFY_FIXTURE"
PARENT_VERIFY = "PARENT_ISOLATION_VERIFY_FIXTURE"


class Failure(Exception):
    def __init__(self, stage, reason):
        self.stage, self.reason = stage, reason
        super().__init__(f"Provider fork fixture failed during {stage}: {reason}")


def require(value, stage, reason):
    if not value:
        raise Failure(stage, reason)


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, provider):
        super().__init__(("127.0.0.1", 0), Handler)
        self.provider = provider
        self.lock = threading.Lock()
        self.requests = []
        self.pending = threading.Event()
        self.release = threading.Event()


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def send_body(self, value, content_type):
        data = value.encode() if isinstance(value, str) else json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        path = self.path.split("?", 1)[0]
        if path not in ["/v1/responses", "/v1/messages", "/v1/messages/count_tokens"]:
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        if not 0 < length <= 2 * 1024 * 1024:
            self.send_error(413)
            return
        body = json.loads(self.rfile.read(length))
        if path.endswith("count_tokens"):
            self.send_body({"input_tokens": 10}, "application/json")
            return
        history = body.get("input" if self.server.provider == "codex" else "messages", [])
        last_user = next((json.dumps(item.get("content", [])) for item in reversed(history) if item.get("role") == "user"), "")
        stage = next((item for item in [PARENT_VERIFY, CHILD_VERIFY, PENDING, FORK_USER, PARENT_USER] if item in last_user), None)
        with self.server.lock:
            if stage:
                self.server.requests.append((stage, history))
            index = len(self.server.requests)
        if stage == PENDING:
            self.server.pending.set()
            self.server.release.wait(45)
        answer = PARENT_ANSWER if stage == PARENT_USER else FORK_ANSWER if stage == FORK_USER else "VERIFIED_LOCAL_FIXTURE"
        try:
            if self.server.provider == "codex":
                item = {"id": f"msg_fork_{index}", "type": "message", "role": "assistant", "status": "completed", "phase": "final_answer", "content": [{"type": "output_text", "text": answer, "annotations": []}]}
                response = {"id": f"resp_fork_{index}", "object": "response", "status": "completed", "output": [item], "usage": {"input_tokens": 10, "output_tokens": 1, "total_tokens": 11}}
                frames = [{"type": "response.created", "response": dict(response, status="in_progress", output=[])}, {"type": "response.output_item.done", "output_index": 0, "item": item}, {"type": "response.completed", "response": response}]
            else:
                message = {"id": f"msg_fork_{index}", "type": "message", "role": "assistant", "model": body.get("model", "fake"), "content": [{"type": "text", "text": answer}], "stop_reason": "end_turn", "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 1}}
                if not body.get("stream"):
                    self.send_body(message, "application/json")
                    return
                frames = [{"type": "message_start", "message": dict(message, content=[], stop_reason=None)}, {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}, {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": answer}}, {"type": "content_block_stop", "index": 0}, {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 1}}, {"type": "message_stop"}]
            self.send_body("".join(f"event: {frame['type']}\ndata: {json.dumps(frame)}\n\n" for frame in frames), "text/event-stream")
        except (BrokenPipeError, ConnectionResetError):
            pass


def records(output, stage):
    try:
        return [json.loads(line) for line in output.splitlines() if line.strip()]
    except json.JSONDecodeError as error:
        raise Failure(stage, "the real CLI did not emit its JSON event protocol") from error


def completed_identity(provider, entries, stage):
    if provider == "codex":
        ids = [entry["thread_id"] for entry in entries if entry.get("type") == "thread.started"]
        require(len(ids) == 1 and any(entry.get("type") == "turn.completed" for entry in entries), stage, "missing structured thread identity or completed turn")
        identity = ids[0]
    else:
        result = [entry for entry in entries if entry.get("type") == "result"]
        require(len(result) == 1 and result[0].get("subtype") == "success" and not result[0].get("is_error"), stage, "the real CLI did not complete its turn")
        identity = result[0].get("session_id")
    uuid.UUID(identity)
    return identity


def run(provider):
    key = "TERMINAL_CODEX_BIN" if provider == "codex" else "TERMINAL_CLAUDE_BIN"
    binary = os.environ.get(key)
    require(binary and Path(binary).is_file(), "setup", f"set {key} to the real installed CLI executable")
    server = Server(provider)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    owned = []
    try:
        with tempfile.TemporaryDirectory(prefix=f"h2-{provider}-fork-", dir=os.environ.get("TEST_TMPDIR", "/tmp")) as folder:
            root = Path(folder)
            home, _, project, env = isolated(root)
            if provider == "codex":
                configuration = f'model_providers.fixture={{name="fixture",base_url="http://127.0.0.1:{server.server_port}/v1",wire_api="responses",requires_openai_auth=false,supports_websockets=false,request_max_retries=0,stream_max_retries=0}}'
                common = ["-c", 'model_provider="fixture"', "-c", 'model="fixture-fork"', "-c", configuration, "-c", "features.plugins=false"]
            else:
                config = root / "claude"
                config.mkdir(mode=0o700)
                env.update({"CLAUDE_CONFIG_DIR": str(config), "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{server.server_port}", "ANTHROPIC_API_KEY": "sk-ant-local-fixture-not-a-real-credential", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "DISABLE_AUTOUPDATER": "1"})
                common = ["-p", "--verbose", "--output-format", "stream-json", "--model", "fake", "--setting-sources", ""]
            version = subprocess.run([binary, "--version"], env=env, cwd=project, capture_output=True, text=True, timeout=10, check=True).stdout.strip()

            def launch(args):
                process = subprocess.Popen([binary, *common, *args], cwd=project, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
                owned.append(process)
                return process

            def finish(args, stage):
                process = launch(args)
                out, err = process.communicate(timeout=40)
                require(process.returncode == 0, stage, "actual CLI failed: " + err[-1500:])
                return completed_identity(provider, records(out, stage), stage)

            def resume(identity, prompt):
                return ["exec", "resume", "--json", "--skip-git-repo-check", identity, prompt] if provider == "codex" else ["--resume", identity, prompt]

            parent = finish(["exec", "--json", "--skip-git-repo-check", PARENT_USER] if provider == "codex" else ["--session-id", str(uuid.uuid4()), PARENT_USER], "parent seed")
            fork_args = ["exec", "fork", "--json", "--skip-git-repo-check", parent, FORK_USER] if provider == "codex" else ["--resume", parent, "--fork-session", FORK_USER]
            child = finish(fork_args, "real provider fork")
            require(child != parent, "real provider fork", "fork did not create a distinct structured conversation ID")
            active = launch(resume(child, PENDING))
            require(server.pending.wait(30), "fork interruption", "the child did not reach its held local model request")
            require(active.poll() is None, "fork interruption", "the child exited before its intended SIGKILL")
            stop_owned(active)
            require(active.returncode == -signal.SIGKILL, "fork interruption", "the fixture did not kill its owned process")
            server.release.set()
            require(finish(resume(child, CHILD_VERIFY), "resume exact fork after SIGKILL") == child, "child identity", "resume changed the forked conversation ID")
            require(finish(resume(parent, PARENT_VERIFY), "verify parent isolation") == parent, "parent identity", "verification changed the parent's conversation ID")
            with server.lock:
                requests = list(server.requests)

            def context(stage):
                matches = [history for marker, history in requests if marker == stage]
                require(len(matches) == 1, stage, "expected one explicit fixture turn")
                return json.dumps(matches[0])

            fork_context = context(FORK_USER)
            require(all(item in fork_context for item in [PARENT_USER, PARENT_ANSWER, FORK_USER]), "fork inheritance", "the real fork omitted committed parent history")
            child_context = context(CHILD_VERIFY)
            require(all(item in child_context for item in [PARENT_USER, PARENT_ANSWER, FORK_USER, FORK_ANSWER, CHILD_VERIFY]), "fork persistence", "the resumed fork omitted committed parent or child history")
            parent_context = context(PARENT_VERIFY)
            require(all(item in parent_context for item in [PARENT_USER, PARENT_ANSWER]), "parent persistence", "parent lost its own committed history")
            require(all(item not in parent_context for item in [FORK_USER, FORK_ANSWER, PENDING, CHILD_VERIFY]), "parent isolation", "child-only work contaminated the parent conversation")
            proof = {"provider": provider, "version": version, "forkOperation": "codex exec fork" if provider == "codex" else "claude --resume --fork-session", "actualProviderFork": True, "distinctStructuredChildId": True, "committedParentContextInherited": True, "forkOnlyTurnSurvivedSigkill": True, "sameChildIdAfterResume": True, "parentExcludedChildHistoryBeforeVerificationTurn": True, "interruptedUserPresentAfterResume": PENDING in child_context, "model": "deterministic loopback protocol fixture", "realCredentialsUsed": False, "personalStateUsed": False, "upstreamModelCalls": False}
            output = Path(os.environ.get("TEST_UNDECLARED_OUTPUTS_DIR", root))
            (output / f"{provider}-fork-proof.json").write_text(json.dumps(proof, indent=2) + "\n")
            print(json.dumps(proof))
    finally:
        server.release.set()
        for process in owned:
            stop_owned(process)
        server.shutdown()
        server.server_close()
        worker.join(timeout=3)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--provider", choices=["codex", "claude"], required=True)
    try:
        run(parser.parse_args().provider)
    except (Failure, subprocess.TimeoutExpired, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error
