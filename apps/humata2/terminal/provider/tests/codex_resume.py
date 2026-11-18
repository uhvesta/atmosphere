"""Real installed Codex, isolated state, deterministic loopback Responses fixture.

No personal config/auth/session files, repository prompts or upstream model calls.
This proves CLI persistence/recovery, not real model reasoning or provider uptime.
"""
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import uuid


class FixtureFailure(Exception):
    def __init__(self, stage, reason):
        self.stage = stage
        self.reason = reason
        super().__init__(f"Codex resume fixture failed during {stage}: {reason}")


class Responses(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, hold_second=True):
        super().__init__(("127.0.0.1", 0), Handler)
        self.requests = []
        self.lock = threading.Lock()
        self.pending = threading.Event()
        self.release = threading.Event()
        self.hold_second = hold_second
        self.request_counter_path = None


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        if self.path != "/v1/responses":
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        if not 0 < length <= 2 * 1024 * 1024:
            self.send_error(413)
            return
        body = json.loads(self.rfile.read(length))
        with self.server.lock:
            index = len(self.server.requests)
            self.server.requests.append(body)
            if self.server.request_counter_path is not None:
                counter = self.server.request_counter_path
                temporary = counter.with_suffix(".tmp")
                temporary.write_text(json.dumps({"requests": len(self.server.requests)}))
                os.replace(temporary, counter)
        if index == 1 and self.server.hold_second:
            self.server.pending.set()
            self.server.release.wait(45)
        answer = "SAVED_ASSISTANT_FIXTURE" if index == 0 else "RESUMED_ASSISTANT_FIXTURE"
        response_id = f"resp_fixture_{index}"
        item = {"id": f"msg_fixture_{index}", "type": "message", "role": "assistant", "status": "completed", "phase": "final_answer", "content": [{"type": "output_text", "text": answer, "annotations": []}]}
        events = [
            {"type": "response.created", "response": {"id": response_id, "object": "response", "status": "in_progress", "output": []}},
            {"type": "response.output_item.done", "output_index": 0, "item": item},
            {"type": "response.completed", "response": {"id": response_id, "object": "response", "status": "completed", "output": [item], "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}},
        ]
        data = "".join("event: " + event["type"] + "\ndata: " + json.dumps(event) + "\n\n" for event in events).encode()
        try:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


def events(output, stage):
    try:
        return [json.loads(line) for line in output.splitlines() if line.strip()]
    except json.JSONDecodeError as error:
        raise FixtureFailure(stage, "CLI did not emit its documented JSON event stream") from error


def thread_id(records, stage):
    values = [event["thread_id"] for event in records if event.get("type") == "thread.started"]
    if len(values) != 1:
        raise FixtureFailure(stage, "expected one structured thread.started identifier")
    uuid.UUID(values[0])
    return values[0]


def stop_owned(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
    return process.communicate(timeout=8)


def end_by_codex_home(codex_home):
    """End every process this fixture's isolated CODEX_HOME started.

    Codex starts an app-server daemon of its own, in a session of its own
    and reparented to launchd, which killing the owned process groups never
    reaches. Each carries the fixture's CODEX_HOME, a fresh temporary folder,
    in its environment, so `ps -E` finds them, and nothing else, by it.
    """
    marker = f"CODEX_HOME={codex_home}"
    def found():
        listed = subprocess.run(["ps", "-A", "-E", "-ww", "-o", "pid=,command="], capture_output=True, text=True, check=False).stdout
        pids = []
        for line in listed.splitlines():
            fields = line.split(None, 1)
            if len(fields) == 2 and fields[0].isdigit() and int(fields[0]) != os.getpid() and any(word == marker for word in fields[1].split()):
                pids.append(int(fields[0]))
        return pids
    for sig, wait in [(signal.SIGTERM, 3.0), (signal.SIGKILL, 3.0)]:
        pids = found()
        if not pids:
            return
        for pid in pids:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + wait
        while found() and time.monotonic() < deadline:
            time.sleep(0.1)


def isolated(root):
    home = root / "home"
    codex_home = root / "codex"
    project = root / "project"
    for folder in [home, codex_home, project]:
        folder.mkdir(mode=0o700)
    # Construct the environment; never inherit credentials, proxy/provider
    # settings, shell startup hooks or the user's HOME/CODEX_HOME.
    env = {"PATH": "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin", "HOME": str(home), "CODEX_HOME": str(codex_home), "XDG_CONFIG_HOME": str(home / "config"), "XDG_STATE_HOME": str(home / "state"), "XDG_DATA_HOME": str(home / "data"), "TERM": "dumb", "NO_COLOR": "1", "CI": "1"}
    return home, codex_home, project, env


def prepare_host():
    """Keep isolated state/server alive for a separate real Host test driver.

    One stdout JSON line is the entire protocol. The owner must stop its Host
    children/holders before sending a stop line or closing stdin. Dropping a
    Host by itself deliberately preserves its terminal processes.
    """
    binary = os.environ.get("TERMINAL_CODEX_BIN")
    if not binary or not Path(binary).is_file():
        raise FixtureFailure("setup", "set TERMINAL_CODEX_BIN to the installed Codex executable")
    server = Responses(hold_second=False)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    seed = None
    try:
        with tempfile.TemporaryDirectory(prefix="h2-codex-host-", dir=os.environ.get("TEST_TMPDIR", "/tmp")) as directory:
            root = Path(directory)
            _, codex_home, project, env = isolated(root)
            try:
                server.request_counter_path = root / "model-requests.json"
                server.request_counter_path.write_text(json.dumps({"requests": 0}))
                (codex_home / "config.toml").write_text(
                    'model_provider = "fixture"\nmodel = "fixture-resume"\n'
                    '[features]\nplugins = false\n'
                    '[model_providers.fixture]\nname = "fixture"\n'
                    f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                    'wire_api = "responses"\nrequires_openai_auth = false\n'
                    'supports_websockets = false\nrequest_max_retries = 0\nstream_max_retries = 0\n'
                )
                # The helper's stdin is a live control pipe. Never let Codex exec
                # inherit it: exec reads additional non-TTY input until EOF.
                seed = subprocess.Popen([binary, "exec", "--json", "--skip-git-repo-check", "SAVED_USER_FIXTURE"], cwd=project, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
                try:
                    stdout, stderr = seed.communicate(timeout=40)
                except subprocess.TimeoutExpired as error:
                    _, stderr = stop_owned(seed)
                    with server.lock:
                        request_count = len(server.requests)
                    raise FixtureFailure("host seed", f"CLI seed timed out with {request_count} local Responses requests; bounded stderr: " + stderr[-2500:]) from error
                if seed.returncode != 0:
                    raise FixtureFailure("host seed", "isolated CLI seed failed: " + stderr[-1500:])
                records = events(stdout, "host seed")
                identity = thread_id(records, "host seed")
                if not any(event.get("type") == "turn.completed" for event in records):
                    raise FixtureFailure("host seed", "the seed turn did not complete")
                receipts = root / "owned-pids.jsonl"
                wrapper = root / "codex-wrapper"
                interactive_env = dict(env, TERM="xterm-256color", COLORTERM="truecolor")
                interactive_env.pop("NO_COLOR", None)
                wrapper.write_text(
                    "#!/usr/bin/python3\nimport json,os,sys\n"
                    f"binary={binary!r}\nenv={interactive_env!r}\nidentity={identity!r}\nreceipts={str(receipts)!r}\n"
                    "args=sys.argv[1:]\n"
                    "if not args or args[0]!='resume' or args[-1]!=identity:\n"
                    " raise SystemExit('The isolated fixture wrapper requires resume and its exact saved session ID')\n"
                    "fd=os.open(receipts,os.O_WRONLY|os.O_CREAT|os.O_APPEND,0o600)\n"
                    "try:\n"
                    " os.write(fd,(json.dumps({'pid':os.getpid(),'requested_session_id':identity})+'\\n').encode())\n"
                    " os.fsync(fd)\n"
                    "finally:\n os.close(fd)\n"
                    "os.execve(binary,[binary,*args],env)\n"
                )
                wrapper.chmod(0o700)
                metadata = {"version": 1, "session_id": identity, "binary": binary, "cwd": str(project), "resume_args": [], "wrapper": str(wrapper), "pid_receipts": str(receipts), "model_requests": str(server.request_counter_path), "expected_history": "SAVED_ASSISTANT_FIXTURE"}
                print(json.dumps(metadata), flush=True)
                # Any explicit stop line or EOF ends this owned fixture. No stdout
                # model content, transcript or environment is exposed to the owner.
                sys.stdin.readline(4096)
            finally:
                end_by_codex_home(codex_home)
    finally:
        server.release.set()
        if seed is not None:
            stop_owned(seed)
        server.shutdown()
        server.server_close()
        worker.join(timeout=3)


def run():
    binary = os.environ.get("TERMINAL_CODEX_BIN")
    if not binary or not Path(binary).is_file():
        raise FixtureFailure("setup", "set TERMINAL_CODEX_BIN to the installed Codex executable")
    server = Responses()
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    owned = []
    try:
        with tempfile.TemporaryDirectory(prefix="h2-codex-resume-", dir=os.environ.get("TEST_TMPDIR", "/tmp")) as root:
            root = Path(root)
            _, codex_home, project, env = isolated(root)
            try:
                provider = f'model_providers.fixture={{name="fixture",base_url="http://127.0.0.1:{server.server_port}/v1",wire_api="responses",requires_openai_auth=false,supports_websockets=false,request_max_retries=0,stream_max_retries=0}}'
                common = ["-c", 'model_provider="fixture"', "-c", 'model="fixture-resume"', "-c", provider, "-c", "features.plugins=false"]

                def launch(args):
                    process = subprocess.Popen([binary, *common, *args], cwd=project, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
                    owned.append(process)
                    return process

                first = launch(["exec", "--json", "--skip-git-repo-check", "SAVED_USER_FIXTURE"])
                stdout, stderr = first.communicate(timeout=40)
                if first.returncode != 0:
                    raise FixtureFailure("first turn", "CLI failed against the isolated loopback fixture: " + stderr[-1500:])
                records = events(stdout, "first turn")
                identity = thread_id(records, "first turn")
                if not any(event.get("type") == "turn.completed" for event in records):
                    raise FixtureFailure("first turn", "the deterministic completion did not finish")
                active = launch(["exec", "resume", "--json", "--skip-git-repo-check", identity, "INTERRUPTED_USER_FIXTURE"])
                if not server.pending.wait(30):
                    raise FixtureFailure("interruption", "the resumed CLI did not reach its held local request")
                if active.poll() is not None:
                    raise FixtureFailure("interruption", "the process exited before the intended SIGKILL")
                interrupted_stdout, _ = stop_owned(active)
                if active.returncode != -signal.SIGKILL:
                    raise FixtureFailure("interruption", "the owned CLI was not terminated by SIGKILL")
                if thread_id(events(interrupted_stdout, "interruption"), "interruption") != identity:
                    raise FixtureFailure("interruption", "resume selected a different conversation")
                server.release.set()
                restored = launch(["exec", "resume", "--json", "--skip-git-repo-check", identity, "VERIFY_RESUME_FIXTURE"])
                stdout, stderr = restored.communicate(timeout=40)
                if restored.returncode != 0:
                    raise FixtureFailure("recovery", "CLI resume failed: " + stderr[-1500:])
                records = events(stdout, "recovery")
                if thread_id(records, "recovery") != identity or not any(event.get("type") == "turn.completed" for event in records):
                    raise FixtureFailure("recovery", "the exact saved conversation did not complete after SIGKILL")
                with server.lock:
                    requests = list(server.requests)
                if len(requests) != 3:
                    raise FixtureFailure("recovery", "unexpected model request count; no retry should be necessary")
                resumed_input = json.dumps(requests[2].get("input", []))
                if "SAVED_USER_FIXTURE" not in resumed_input or "SAVED_ASSISTANT_FIXTURE" not in resumed_input or "VERIFY_RESUME_FIXTURE" not in resumed_input:
                    raise FixtureFailure("recovery", "the provider request omitted the saved prior user/assistant context")
                missing = launch(["exec", "resume", "--json", "--skip-git-repo-check", str(uuid.uuid4()), "MISSING_RESUME_FIXTURE"])
                missing_stdout, _ = missing.communicate(timeout=20)
                with server.lock:
                    after_missing = len(server.requests)
                if missing.returncode == 0 or after_missing != len(requests) or any(event.get("type") == "thread.started" for event in events(missing_stdout, "missing conversation")):
                    raise FixtureFailure("missing conversation", "resume silently created or queried a different conversation")
                proof = {"provider": "installed Codex CLI", "model": "deterministic loopback Responses fixture", "structuredSessionId": True, "sameConversationAfterSigkill": True, "retainedUserAndAssistantContext": True, "requests": len(requests), "missingSessionRejectedBeforeModel": True, "personalStateUsed": False, "upstreamModelCalls": False}
                output = Path(os.environ.get("TEST_UNDECLARED_OUTPUTS_DIR", root))
                (output / "codex-resume-proof.json").write_text(json.dumps(proof, indent=2) + "\n")
                print(json.dumps(proof))
            finally:
                for process in owned:
                    stop_owned(process)
                end_by_codex_home(codex_home)
    finally:
        server.release.set()
        for process in owned:
            stop_owned(process)
        server.shutdown()
        server.server_close()
        worker.join(timeout=3)


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--prepare-host"]:
            prepare_host()
        elif not sys.argv[1:]:
            run()
        else:
            raise FixtureFailure("arguments", "use no arguments for the smoke or --prepare-host for the owned Host fixture")
    except (FixtureFailure, subprocess.TimeoutExpired) as error:
        raise SystemExit(str(error)) from error
