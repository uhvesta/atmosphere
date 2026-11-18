"""Shared pieces of the real-CLI fork, carry and resume fixtures (ROAM X2).

Each test runs an installed CLI against a loopback model server inside
throwaway HOMEs: a conversation of three turns in folder A, a fork of it made
by hand the way the conversation adapter makes one, and a carry of it to
folder B under another HOME. The loopback server keeps every request it gets,
so "the CLI resumed with these turns" is read from what the CLI sent as
history, not from its own printed output. The report answers each fact the
adapter marks "unproven (X2)" with yes, no or not testable. No personal
state, credentials or upstream model calls are involved. Standard library
only.
"""
import contextlib
import hashlib
import http.server
import io
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import tempfile
import threading
import types
import uuid

import common

MAIN_ROUTES = ("/v1/messages", "/v1/responses", "/v1/chat/completions")
ANSWERS = ("yes", "no", "not testable")


# ---------------------------------------------------------------- model server

class _Recording(common._Handler):
    """The X1 loopback handler, keeping each POST body before it answers."""

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if 0 < length <= 8 * 1024 * 1024 else b""
        with self.server.lock:
            self.server.requests.append((self.path.split("?", 1)[0], raw.decode("utf-8", "replace")))
        self.rfile = io.BytesIO(raw)
        super().do_POST()


class RecordingServer(common.ModelServer):
    def __init__(self):
        http.server.ThreadingHTTPServer.__init__(self, ("127.0.0.1", 0), _Recording)
        self.lock = threading.Lock()
        self.routes = []
        self.requests = []

    def mark(self):
        with self.lock:
            return len(self.requests)

    def bodies_since(self, mark):
        """The bodies of model calls made after `mark`, in order."""
        with self.lock:
            return [body for route, body in self.requests[mark:] if route in MAIN_ROUTES]


@contextlib.contextmanager
def recording_server():
    server = RecordingServer()
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        yield server
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=3)


def roam_prompts():
    nonce = uuid.uuid4().hex[:10]
    return tuple("ROAM_PROMPT_%d_%s" % (n, nonce) for n in (1, 2, 3, 4))


def history_seen(bodies, prompts):
    """Which of `prompts` any of the model calls carried, as a list of booleans."""
    return [any(p in body for body in bodies) for p in prompts]


# ------------------------------------------------------------------- the report

class Report:
    """Facts (yes / no / not testable, with evidence), observations and failures.

    `expect` records a failed assertion and goes on, so the report always holds
    everything that could be learned; `finish` raises when anything failed.
    """

    def __init__(self, cli):
        self.cli = cli
        self.data = {"cli": cli, "version": None, "facts": {}, "observations": {}, "failures": []}

    def fact(self, key, answer, evidence):
        if answer not in ANSWERS:
            raise common.LayoutFailure("Internal error: fact %r has answer %r; use one of %s." % (key, answer, ", ".join(ANSWERS)))
        self.data["facts"][key] = {"answer": answer, "evidence": evidence}

    def observe(self, key, value):
        self.data["observations"][key] = value

    def expect(self, ok, message):
        if not ok:
            self.data["failures"].append(message)
        return ok

    def finish(self, workspaces, name=None):
        """Write <name>.json (default <cli>_roam; paths scrubbed) and raise if an expectation failed."""
        output = common.outputs_dir(self.cli)
        pairs = _pairs(workspaces)
        name = name or self.cli + "_roam"
        text = common.sanitize_text(json.dumps(self.data, indent=2), pairs)
        (output / (name + ".json")).write_text(text + "\n")
        print(json.dumps({"cli": self.cli, "version": self.data["version"], "failures": len(self.data["failures"]), "report": str(output / (name + ".json"))}))
        if self.data["failures"]:
            raise common.LayoutFailure("%d expectation(s) failed. First: %s. The report lists every one, and the facts learned so far." % (len(self.data["failures"]), self.data["failures"][0]))


def _pairs(workspaces):
    pairs = set()
    for label, ws in workspaces:
        for actual, new in ((ws.project, "/%s/work" % label), (ws.home, "/%s/home" % label), (ws.root, "/%s/root" % label)):
            for base in {str(actual), str(actual).replace(str(ws.root), str(ws.raw_root), 1)}:
                pairs.add((base, new))
                pairs.add((re.sub(r"[^A-Za-z0-9]", "-", base), "-%s-%s" % (label, new.rsplit("/", 1)[1])))
    real_home = os.path.expanduser("~")
    if real_home.startswith("/") and len(real_home) > 1:
        pairs.add((real_home, "/Users/USER"))
    return sorted(pairs, key=lambda p: -len(p[0]))


# -------------------------------------------------------------------- helpers

def new_workspace(stack, cli):
    return stack.enter_context(common.workspace(cli))


def try_run(report, stage, argv, ws, env, cwd=None, timeout=90):
    """Run a CLI; return (stdout, None), or (None, why) when it fails. Never raises for a CLI failure."""
    target = types.SimpleNamespace(project=cwd or ws.project)
    try:
        return common.run_cli(stage, argv, target, env, timeout=timeout), None
    except common.LayoutFailure as error:
        report.observe("failed_stage_" + stage.replace(" ", "_"), str(error)[-600:])
        return None, str(error)


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def dashed(path):
    return re.sub(r"[^A-Za-z0-9]", "-", str(path))


def json_escaped(text):
    return json.dumps(text)[1:-1]


def rewrite_text(text, pairs):
    for old, new in pairs:
        if old and old != new:
            text = text.replace(old, new)
    return text


def path_pairs(folder_a, folder_b, extra=()):
    """(old, new) pairs, plain and JSON-escaped, for the folder then each extra pair."""
    out = []
    for old, new in [(str(folder_a), str(folder_b))] + [(str(a), str(b)) for a, b in extra]:
        out.append((old, new))
        if json_escaped(old) != old:
            out.append((json_escaped(old), json_escaped(new)))
    return out


def files_naming(root, needles):
    """{needle: [relative paths under root whose name or bytes hold it]}; links are not followed."""
    found = {n: [] for n in needles}
    root = Path(root)
    for current, dirs, names in os.walk(root, followlinks=False):
        for name in names:
            full = Path(current) / name
            if full.is_symlink() or not full.is_file():
                continue
            rel = str(full.relative_to(root))
            try:
                data = full.read_bytes()
            except OSError:
                continue
            for needle in needles:
                if needle in rel or needle.encode() in data:
                    found[needle].append(rel)
    return {n: sorted(v) for n, v in found.items()}


def jsonl_lines(path):
    """Complete lines of a JSONL file as bytes, each with its newline."""
    data = Path(path).read_bytes()
    lines = data.splitlines(keepends=True)
    return [line for line in lines if line.endswith(b"\n")]


def sqlite_rows(db, sql, args=()):
    """Rows of a read on a copy of `db` with its -wal and -shm; None when the file is missing."""
    db = Path(db)
    if not db.is_file():
        return None
    scratch = Path(tempfile.mkdtemp(prefix="roam-db-"))
    try:
        copy = scratch / db.name
        shutil.copyfile(db, copy)
        for suffix in ("-wal", "-shm"):
            side = Path(str(db) + suffix)
            if side.exists():
                shutil.copyfile(side, str(copy) + suffix)
        connection = sqlite3.connect(str(copy))
        try:
            return [list(row) for row in connection.execute(sql, args).fetchall()]
        finally:
            connection.close()
    except sqlite3.Error as error:
        return "sqlite error: %s" % error
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def replace_in_strings(value, old, new):
    """Every string in a JSON value with `old` replaced by `new`."""
    if isinstance(value, str):
        return value.replace(old, new)
    if isinstance(value, list):
        return [replace_in_strings(v, old, new) for v in value]
    if isinstance(value, dict):
        return {k: replace_in_strings(v, old, new) for k, v in value.items()}
    return value


def tree_listing(root):
    root = Path(root)
    return sorted(str(p.relative_to(root)) + ("/" if p.is_dir() else "") for p in root.rglob("*"))
