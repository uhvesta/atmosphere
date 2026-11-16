"""Real Copilot CLI, isolated state, offline loopback Chat Completions fixture.

Primary references: `copilot help sessions`, `copilot help providers-env`, and
https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-llm-models
https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference
No personal configuration/authentication, repository prompts, or upstream models.
This tests provider CLI durability and context transport, not model reasoning.
"""
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import time
import uuid


class FixtureFailure(Exception):
    def __init__(self, stage, reason):
        self.stage = stage
        self.reason = reason
        super().__init__(f"Copilot resume fixture failed during {stage}: {reason}")


class Completions(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, markers):
        super().__init__(("127.0.0.1", 0), Handler)
        self.markers = markers
        self.requests = []
        self.failures = []
        self.lock = threading.Lock()
        self.pending = threading.Event()
        self.release = threading.Event()


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.path == "/v1/models":
            body = json.dumps({"object": "list", "data": [{"id": "fixture-resume", "object": "model", "owned_by": "local-fixture"}]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_error(404)

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            self.server.failures.append("unexpected local provider route: " + self.path)
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        if not 0 < length <= 4 * 1024 * 1024:
            self.server.failures.append("invalid local request length")
            self.send_error(413)
            return
        body = json.loads(self.rfile.read(length))
        encoded = json.dumps(body)
        seed, interrupted, resumed = self.server.markers
        stage = "resumed" if resumed in encoded else "interrupted" if interrupted in encoded else "seed" if seed in encoded else "unknown"
        with self.server.lock:
            if len(self.server.requests) >= 32:
                self.server.failures.append("the CLI exceeded 32 bounded fixture requests")
                self.send_error(429)
                return
            self.server.requests.append({"stage": stage, "body": body})
        if stage == "unknown":
            self.server.failures.append("a provider request did not contain a fixture conversation marker")
        if stage == "interrupted":
            self.server.pending.set()
            self.server.release.wait(45)
        answer = "SAVED_ASSISTANT_FIXTURE" if stage == "seed" else "RESUMED_ASSISTANT_FIXTURE"
        identifier = "chatcmpl_fixture_" + stage
        usage = {"prompt_tokens": 20, "completion_tokens": 4, "total_tokens": 24}
        if body.get("stream"):
            chunks = [
                {"id": identifier, "object": "chat.completion.chunk", "created": 1, "model": "fixture-resume", "choices": [{"index": 0, "delta": {"role": "assistant", "content": answer}, "finish_reason": None}]},
                {"id": identifier, "object": "chat.completion.chunk", "created": 1, "model": "fixture-resume", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": usage},
            ]
            data = ("".join("data: " + json.dumps(chunk) + "\n\n" for chunk in chunks) + "data: [DONE]\n\n").encode()
            content_type = "text/event-stream"
        else:
            data = json.dumps({"id": identifier, "object": "chat.completion", "created": 1, "model": "fixture-resume", "choices": [{"index": 0, "message": {"role": "assistant", "content": answer}, "finish_reason": "stop"}], "usage": usage}).encode()
            content_type = "application/json"
        try:
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


def records(text, stage):
    try:
        return [json.loads(line) for line in text.splitlines() if line.strip()]
    except json.JSONDecodeError as error:
        raise FixtureFailure(stage, "the real CLI did not emit JSONL; inspect its isolated output artifact") from error


def ids(records):
    found = set()

    def visit(value):
        if isinstance(value, dict):
            if value.get("type") == "session.start":
                data = value.get("data", value)
                identity = data.get("sessionId")
                if isinstance(identity, str):
                    uuid.UUID(identity)
                    found.add(identity)
            for nested in value.values():
                visit(nested)
        elif isinstance(value, list):
            for nested in value:
                visit(nested)

    visit(records)
    return found


def stored(home):
    root = home / "session-state"
    result = {}
    if root.is_dir():
        for directory in root.iterdir():
            if not directory.is_dir():
                continue
            try:
                uuid.UUID(directory.name)
            except ValueError:
                continue
            log = directory / "events.jsonl"
            if log.is_file():
                if log.stat().st_size > 16 * 1024 * 1024:
                    raise FixtureFailure("metadata", "the isolated session event log exceeded 16 MiB")
                entries = records(log.read_text(), "session metadata")
                result[directory.name] = entries
    return result


def stop_owned(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
    return process.communicate(timeout=8)


def main():
    binary = os.environ.get("TERMINAL_COPILOT_BIN")
    if not binary or not Path(binary).is_file():
        raise FixtureFailure("setup", "set TERMINAL_COPILOT_BIN to the installed real Copilot executable")
    output = Path(os.environ.get("TEST_UNDECLARED_OUTPUTS_DIR", "/tmp/copilot-resume-proof"))
    output.mkdir(parents=True, exist_ok=True)
    nonce = uuid.uuid4().hex[:12]
    markers = tuple(label + "_" + nonce for label in ["SAVED_USER_FIXTURE", "INTERRUPTED_USER_FIXTURE", "RESUMED_USER_FIXTURE"])
    server = Completions(markers)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    children = []
    try:
        with tempfile.TemporaryDirectory(prefix="h2-copilot-", dir="/tmp") as directory:
            root = Path(directory)
            home, copilot, cache, project = [root / name for name in ["home", "copilot", "cache", "project"]]
            for folder in [home, copilot, cache, project, root / "tmp"]:
                folder.mkdir(mode=0o700)
            # Build, rather than filter, the environment. No GitHub keys,
            # proxies, shell init hooks, OTel settings, or personal state survive.
            env = {
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(home),
                "COPILOT_HOME": str(copilot), "COPILOT_CACHE_HOME": str(cache),
                "TMPDIR": str(root / "tmp"), "XDG_CONFIG_HOME": str(home / "config"),
                "XDG_STATE_HOME": str(home / "state"), "XDG_DATA_HOME": str(home / "data"),
                "COPILOT_OFFLINE": "true", "COPILOT_TELEMETRY_DISABLED": "true",
                "COPILOT_OTEL_ENABLED": "false", "COPILOT_DISABLE_LOGIN_SHELL_ENV": "true",
                "COPILOT_PLUGIN_DIR_ONLY": "true", "COPILOT_DISABLE_DESKTOP_NOTIFICATIONS": "1",
                "COPILOT_SYSTEM_PROXY": "0", "HTTP_PROXY": "http://127.0.0.1:9",
                "HTTPS_PROXY": "http://127.0.0.1:9", "NO_PROXY": "127.0.0.1,localhost",
                "COPILOT_PROVIDER_BASE_URL": f"http://127.0.0.1:{server.server_port}/v1",
                "COPILOT_PROVIDER_TYPE": "openai", "COPILOT_PROVIDER_WIRE_API": "completions",
                "COPILOT_PROVIDER_MODEL_ID": "gpt-4.1", "COPILOT_PROVIDER_WIRE_MODEL": "fixture-resume",
                "COPILOT_MODEL": "gpt-4.1", "COPILOT_PROVIDER_MAX_PROMPT_TOKENS": "128000",
                "COPILOT_PROVIDER_MAX_OUTPUT_TOKENS": "1024", "TERM": "dumb", "NO_COLOR": "1", "CI": "1",
            }
            common = [binary, "--no-auto-update", "--disable-client-telemetry", "--disable-builtin-mcps", "--no-custom-instructions", "--no-bash-env", "--no-ask-user", "--sync-session=off", "--output-format=json", "--stream=off", "--log-level=error"]
            version = subprocess.run([binary, "--no-auto-update", "--version"], cwd=project, env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20, check=True).stdout.strip()

            def start(stage, prompt, identity=None):
                args = common + ([] if identity is None else ["--resume=" + identity]) + ["--prompt", prompt]
                process = subprocess.Popen(args, cwd=project, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
                children.append(process)
                return process

            def finish(process, stage):
                stdout, stderr = process.communicate(timeout=45)
                (output / (stage + ".jsonl")).write_text(stdout)
                (output / (stage + ".stderr.txt")).write_text(stderr)
                if process.returncode != 0:
                    raise FixtureFailure(stage, f"the isolated real CLI exited {process.returncode}: {stderr[-1800:]}")
                return records(stdout, stage)

            seed = start("seed", markers[0])
            seed_events = finish(seed, "seed")
            saved = stored(copilot)
            candidates = ids(seed_events) | set(saved)
            if len(candidates) != 1:
                raise FixtureFailure("seed", "expected exactly one structured or persisted full session UUID")
            identity = candidates.pop()
            if identity not in saved or ids(saved[identity]) != {identity}:
                raise FixtureFailure("seed", "saved event metadata did not bind the CLI's exact session UUID")
            if "SAVED_ASSISTANT_FIXTURE" not in json.dumps(saved[identity]):
                raise FixtureFailure("seed", "the completed assistant turn was not persisted")

            pending = start("interrupted", markers[1], identity)
            if not server.pending.wait(35):
                stdout, stderr = stop_owned(pending)
                (output / "interrupted.jsonl").write_text(stdout)
                (output / "interrupted.stderr.txt").write_text(stderr)
                raise FixtureFailure("interrupted", "the exact resumed session never reached its pending local inference request: " + stderr[-1800:])
            stdout, stderr = stop_owned(pending)
            (output / "interrupted.jsonl").write_text(stdout)
            (output / "interrupted.stderr.txt").write_text(stderr)
            if pending.returncode != -signal.SIGKILL:
                raise FixtureFailure("interrupted", "the owned CLI process was not terminated by SIGKILL")
            server.release.set()

            resumed = start("resumed", markers[2], identity)
            resumed_events = finish(resumed, "resumed")
            after = stored(copilot)
            if set(after) != {identity} or ids(after[identity]) != {identity} or ids(resumed_events) - {identity}:
                raise FixtureFailure("resumed", "recovery created or selected another session UUID")
            with server.lock:
                requests = [entry["body"] for entry in server.requests if entry["stage"] == "resumed"]
            required = [markers[0], "SAVED_ASSISTANT_FIXTURE", markers[1], markers[2]]
            summaries = [{"stage": entry["stage"], "message_roles": [message.get("role") for message in entry["body"].get("messages", [])], "contains": {label: value in json.dumps(entry["body"]) for label, value in zip(["saved_user", "saved_assistant", "interrupted_user", "resumed_user"], required)}} for entry in server.requests]
            (output / "provider-request-presence.json").write_text(json.dumps(summaries, indent=2))
            committed = [markers[0], "SAVED_ASSISTANT_FIXTURE", markers[2]]
            if not requests or not all(value in json.dumps(requests[-1]) for value in committed):
                raise FixtureFailure("resumed", "the real CLI did not send committed user/assistant history and the new verification prompt to the local model")
            interrupted_retained = markers[1] in json.dumps(requests[-1])
            if "RESUMED_ASSISTANT_FIXTURE" not in json.dumps(after[identity]):
                raise FixtureFailure("resumed", "the recovered turn did not complete and persist")
            # --resume must fail closed for a nonexistent exact UUID. In
            # contrast, --session-id is allowed to create one and is never used.
            before_missing = len(server.requests)
            missing = start("missing-session", "MISSING_SESSION_MUST_NOT_START_" + nonce, str(uuid.uuid4()))
            missing_stdout, missing_stderr = missing.communicate(timeout=30)
            (output / "missing-session.jsonl").write_text(missing_stdout)
            (output / "missing-session.stderr.txt").write_text(missing_stderr)
            if missing.returncode == 0 or len(server.requests) != before_missing or set(stored(copilot)) != {identity}:
                raise FixtureFailure("missing session", "--resume did not reject the missing UUID without creating a session or sending inference")
            if server.failures:
                raise FixtureFailure("provider", "; ".join(server.failures))
            proof = {"proof": "real-copilot-sigkill-exact-session-resume", "version": version, "session_id": identity, "kill_signal": "SIGKILL", "session_id_unchanged": True, "prior_user_and_assistant_context_retained": True, "interrupted_user_context_retained": interrupted_retained, "interrupted_turn_policy_observed": "retained" if interrupted_retained else "uncommitted user turn rolled back by Copilot; fixture did not replay it", "missing_uuid_rejected_without_new_session_or_model_call": True, "resumed_turn_persisted": True, "provider": "deterministic loopback OpenAI-compatible fixture", "offline": True, "personal_state_used": False, "model_requests": len(server.requests)}
            (output / "copilot-resume-proof.json").write_text(json.dumps(proof, indent=2))
            print(json.dumps(proof))
    finally:
        server.release.set()
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
