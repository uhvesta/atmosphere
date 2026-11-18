"""Installed CLI resumes a conversation forked by Humata's adapter.

One prompt asks the CLI to remember a word, in folder A under a throwaway
HOME. The conversation crate's fork (fork_helper) writes a copy under a new id
for folder B. The CLI then resumes the fork with its own resume command and is
asked for the word. The loopback model server answers with the word only when
the request the CLI sent holds it, so a named word means the CLI loaded the
forked history. The test fails when the CLI refuses the fork. Loopback model
API only; the real ~/.claude, ~/.codex and ~/.copilot are never touched.
Usage: fork_resume.py <claude|codex|copilot>
"""
import contextlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import claude_roam
import codex_roam
import common
import copilot_roam
import roam_common as rc

CLIS = {
    "claude": ("Claude Code", ["TERMINAL_CLAUDE_BIN", "CLAUDE_BIN"]),
    "codex": ("Codex", ["TERMINAL_CODEX_BIN", "CODEX_BIN"]),
    "copilot": ("Copilot", ["TERMINAL_COPILOT_BIN", "COPILOT_BIN"]),
}
MARKER = "THE_WORD_IS_"


class _Remembering(rc._Recording):
    """The recording handler, answering MARKER<word> when the request holds the word."""

    word = None

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if 0 < length <= 8 * 1024 * 1024 else b""
        self.rfile = io.BytesIO(raw)
        self._raw = raw.decode("utf-8", "replace")
        super().do_POST()
        # super() read the body from the BytesIO above; keep the recorded copy.

    def _answer(self, answer):
        word = self.server.word
        return MARKER + word if word and word in self._raw else answer

    def _anthropic(self, body, answer, index):
        super()._anthropic(body, self._answer(answer), index)

    def _responses(self, answer, index):
        super()._responses(self._answer(answer), index)

    def _completions(self, body, answer, index):
        super()._completions(body, self._answer(answer), index)


class _Server(rc.RecordingServer):
    def __init__(self, word):
        import http.server
        import threading
        http.server.ThreadingHTTPServer.__init__(self, ("127.0.0.1", 0), _Remembering)
        self.lock = threading.Lock()
        self.routes = []
        self.requests = []
        self.word = word


def helper_binary():
    root = Path(os.environ["TEST_SRCDIR"]) / os.environ["TEST_WORKSPACE"]
    return str(root / "apps/humata2/daemon/conversation/fork_helper")


def fork(cli, ws_a, ws_b, cli_home, conv_id, new_id):
    argv = [helper_binary(), cli, str(ws_a.home), str(cli_home), str(ws_a.project), conv_id, str(ws_b.project), new_id]
    done = subprocess.run(argv, capture_output=True, text=True, timeout=60)
    if done.returncode != 0:
        raise common.LayoutFailure("Humata's adapter could not fork the %s conversation. Its message: %s" % (cli, done.stderr.strip()[-1500:]))
    return json.loads(done.stdout)


def first_turn(cli, binary, prompt, ws, env, cli_home, flags):
    """Run the one prompt; return the conversation id."""
    if cli == "claude":
        out = common.run_cli("Claude turn 1", [binary] + claude_roam.FLAGS + [prompt], ws, env)
        ids = claude_roam.session_ids(out)
        if len(ids) != 1:
            raise common.LayoutFailure("Claude's first turn did not report exactly one session_id (got %r)." % sorted(ids))
        return next(iter(ids))
    if cli == "codex":
        out = common.run_cli("Codex turn 1", [binary] + flags + ["exec", "--json", "--skip-git-repo-check", prompt], ws, env)
        conv = codex_roam.thread_of(out)
        if conv is None:
            raise common.LayoutFailure("Codex's first turn did not print exactly one thread.started event.")
        return conv
    common.run_cli("Copilot turn 1", copilot_roam.base_argv(binary) + ["--prompt", prompt], ws, env)
    folders = copilot_roam.session_folders(cli_home)
    if len(folders) != 1:
        raise common.LayoutFailure("Copilot's first turn did not leave exactly one session folder (got %r)." % [f.name for f in folders])
    return folders[0].name


def resume_argv(cli, binary, flags, new_id, prompt):
    if cli == "claude":
        return [binary] + claude_roam.FLAGS + ["--resume", new_id, prompt]
    if cli == "codex":
        return [binary] + flags + ["exec", "resume", "--json", "--skip-git-repo-check", new_id, prompt]
    return copilot_roam.base_argv(binary) + ["--session-id", new_id, "--prompt", prompt]


def run():
    if len(sys.argv) != 2 or sys.argv[1] not in CLIS:
        raise SystemExit("Usage: fork_resume.py <claude|codex|copilot>")
    cli = sys.argv[1]
    label, names = CLIS[cli]
    binary = common.binary_or_skip(label, names)
    word = "WORD" + uuid.uuid4().hex[:10].upper()
    remember = "Please remember this word for later: %s. Reply with OK." % word
    ask = "What word did I ask you to remember? Reply with %s followed by the word." % MARKER
    report = {"cli": cli, "version": None, "word": word}
    with contextlib.ExitStack() as stack:
        server = _Server(word)
        import threading
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        stack.callback(lambda: (server.shutdown(), server.server_close(), worker.join(timeout=3)))
        ws_a = rc.new_workspace(stack, cli)
        ws_b = rc.new_workspace(stack, cli)
        flags = []
        if cli == "claude":
            env, cli_home = claude_roam.environment(ws_a, server)
            version_argv = [binary, "--version"]
        elif cli == "codex":
            env, cli_home, flags = codex_roam.environment(ws_a, server)
            version_argv = [binary, "--version"]
        else:
            env, cli_home = copilot_roam.environment(ws_a, server)
            version_argv = [binary, "--no-auto-update", "--version"]
        try:
            report["version"] = common.run_cli(cli + " --version", version_argv, ws_a, env, timeout=30).strip().splitlines()[0]
            conv_id = first_turn(cli, binary, remember, ws_a, env, cli_home, flags)
            report["parent_id"] = conv_id
            report["parent_history_sent_word"] = any(word in b for b in server.bodies_since(0))
            new_id = str(uuid.uuid4())
            report["fork"] = fork(cli, ws_a, ws_b, cli_home, conv_id, new_id)
            report["fork_id"] = new_id
            mark = server.mark()
            # The fork lives in folder B but under the same HOME and CLI home.
            out, why = rc.try_run(None_report(report), "resume fork", resume_argv(cli, binary, flags, new_id, ask), ws_a, env, cwd=ws_b.project)
            report["cli_accepted_fork"] = why is None
            if why is not None:
                report["refusal"] = why[-1500:]
            else:
                bodies = server.bodies_since(mark)
                report["request_held_word"] = any(word in b for b in bodies)
                report["reply_names_word"] = (MARKER + word) in out
                report["reply_tail"] = out[-600:]
        finally:
            output = common.outputs_dir(cli)
            pairs = rc._pairs([("A", ws_a), ("B", ws_b)])
            text = common.sanitize_text(json.dumps(report, indent=2), pairs)
            (output / (cli + "_fork_resume.json")).write_text(text + "\n")
            print(text)
    if not report.get("cli_accepted_fork"):
        raise common.LayoutFailure("%s did not accept the forked conversation %s. See %s_fork_resume.json for its refusal; Humata's fork for this format needs fixing." % (label, report.get("fork_id"), cli))


class None_report:
    """try_run wants an object with observe(); keep its notes in the report."""

    def __init__(self, data):
        self.data = data

    def observe(self, key, value):
        self.data.setdefault("observations", {})[key] = value


if __name__ == "__main__":
    common.main(run)
