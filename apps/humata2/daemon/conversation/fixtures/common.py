"""Shared pieces of the real-CLI conversation layout fixtures (ROAM X1).

Each layout test runs an installed CLI twice against a loopback model server,
inside a throwaway HOME, then writes a JSON report of what the CLI stored:
files, record shapes, SQLite tables and every place the project folder or the
conversation id is spelled out. No personal state, credentials or upstream
model calls are involved. Python standard library only.
"""
import contextlib
import http.server
import json
import os
from pathlib import Path
import re
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import urllib.parse
import uuid

MAX_SCAN = 64 * 1024 * 1024
MAX_SAMPLE = 4 * 1024 * 1024
MAX_PATHS_PER_TYPE = 400
SQLITE_MAGIC = b"SQLite format 3\x00"


class LayoutFailure(Exception):
    pass


def binary_or_skip(label, names):
    """Return the installed CLI path, or end the test cleanly when unset."""
    for name in names:
        value = os.environ.get(name)
        if value:
            if not Path(value).is_file():
                raise SystemExit("%s is set to %r, which is not a file. Point it at the installed %s executable." % (name, value, label))
            return value
    print("SKIP: %s not tested. Set %s to the installed %s executable (see apps/humata2/terminal/provider/README.md)." % (label, " or ".join(names), label))
    raise SystemExit(0)


# ---------------------------------------------------------------- model server

class _Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def _send(self, content_type, data):
        try:
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        if self.path.split("?", 1)[0] == "/v1/models":
            self._send("application/json", json.dumps({"object": "list", "data": [{"id": "fixture-layout", "object": "model", "owned_by": "local-fixture"}]}).encode())
        else:
            self.send_error(404)

    def do_POST(self):
        route = self.path.split("?", 1)[0]
        length = int(self.headers.get("Content-Length", "0"))
        if not 0 < length <= 8 * 1024 * 1024:
            self.send_error(413)
            return
        body = json.loads(self.rfile.read(length))
        if route == "/v1/messages/count_tokens":
            self._send("application/json", json.dumps({"input_tokens": 10}).encode())
            return
        if route not in ("/v1/messages", "/v1/responses", "/v1/chat/completions"):
            self.send_error(404)
            return
        with self.server.lock:
            self.server.routes.append(route)
            index = len(self.server.routes)
        answer = "LAYOUT_ASSISTANT_%d" % index
        if route == "/v1/messages":
            self._anthropic(body, answer, index)
        elif route == "/v1/responses":
            self._responses(answer, index)
        else:
            self._completions(body, answer, index)

    def _anthropic(self, body, answer, index):
        message = {"id": "msg_fixture_%d" % index, "type": "message", "role": "assistant", "model": body.get("model", "fake"), "content": [{"type": "text", "text": answer}], "stop_reason": "end_turn", "stop_sequence": None, "usage": {"input_tokens": 10, "output_tokens": 1}}
        if not body.get("stream"):
            self._send("application/json", json.dumps(message).encode())
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
        self._send("text/event-stream", "".join("event: %s\ndata: %s\n\n" % (f["type"], json.dumps(f)) for f in frames).encode())

    def _responses(self, answer, index):
        response_id = "resp_fixture_%d" % index
        item = {"id": "msg_fixture_%d" % index, "type": "message", "role": "assistant", "status": "completed", "phase": "final_answer", "content": [{"type": "output_text", "text": answer, "annotations": []}]}
        events = [
            {"type": "response.created", "response": {"id": response_id, "object": "response", "status": "in_progress", "output": []}},
            {"type": "response.output_item.done", "output_index": 0, "item": item},
            {"type": "response.completed", "response": {"id": response_id, "object": "response", "status": "completed", "output": [item], "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}},
        ]
        self._send("text/event-stream", "".join("event: %s\ndata: %s\n\n" % (e["type"], json.dumps(e)) for e in events).encode())

    def _completions(self, body, answer, index):
        identifier = "chatcmpl_fixture_%d" % index
        usage = {"prompt_tokens": 20, "completion_tokens": 4, "total_tokens": 24}
        if body.get("stream"):
            chunks = [
                {"id": identifier, "object": "chat.completion.chunk", "created": 1, "model": "fixture-layout", "choices": [{"index": 0, "delta": {"role": "assistant", "content": answer}, "finish_reason": None}]},
                {"id": identifier, "object": "chat.completion.chunk", "created": 1, "model": "fixture-layout", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": usage},
            ]
            self._send("text/event-stream", ("".join("data: %s\n\n" % json.dumps(c) for c in chunks) + "data: [DONE]\n\n").encode())
        else:
            self._send("application/json", json.dumps({"id": identifier, "object": "chat.completion", "created": 1, "model": "fixture-layout", "choices": [{"index": 0, "message": {"role": "assistant", "content": answer}, "finish_reason": "stop"}], "usage": usage}).encode())


class ModelServer(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(("127.0.0.1", 0), _Handler)
        self.lock = threading.Lock()
        self.routes = []


@contextlib.contextmanager
def model_server():
    server = ModelServer()
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        yield server
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=3)


# ------------------------------------------------------------------- workspace

class Workspace:
    def __init__(self, root, raw_root):
        self.root = root
        self.raw_root = raw_root
        self.home = root / "home"
        self.project = root / "work dir" / "proj.one"
        self.tmp = root / "tmp"
        for folder in (self.home, self.project, self.tmp):
            folder.mkdir(mode=0o700, parents=True)

    def env(self, extra):
        home = self.home
        base = {
            "PATH": "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin",
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_STATE_HOME": str(home / ".local" / "state"),
            "XDG_DATA_HOME": str(home / ".local" / "share"),
            "XDG_CACHE_HOME": str(home / ".cache"),
            "TMPDIR": str(self.tmp),
            "TERM": "dumb", "NO_COLOR": "1", "CI": "1",
        }
        base.update(extra)
        return base


@contextlib.contextmanager
def workspace(cli):
    parent = os.environ.get("TEST_TMPDIR") or "/tmp"
    with tempfile.TemporaryDirectory(prefix="h2-%s-layout-" % cli, dir=parent) as directory:
        yield Workspace(Path(os.path.realpath(directory)), Path(directory))


def outputs_dir(cli):
    declared = os.environ.get("TEST_UNDECLARED_OUTPUTS_DIR")
    path = Path(declared) if declared else Path(tempfile.gettempdir()) / ("h2-layout-out-" + cli)
    path.mkdir(parents=True, exist_ok=True)
    return path


# ------------------------------------------------------------------- CLI runs

def run_cli(stage, argv, ws, env, timeout=90):
    process = subprocess.Popen(argv, cwd=ws.project, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        _, stderr = process.communicate(timeout=8)
        raise LayoutFailure("%s timed out after %ds. Last stderr: %s" % (stage, timeout, stderr[-1500:]))
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
    if process.returncode != 0:
        raise LayoutFailure("%s exited %d. Last stderr: %s" % (stage, process.returncode, stderr[-1500:]))
    return stdout


def json_lines(text, stage):
    try:
        return [json.loads(line) for line in text.splitlines() if line.strip()]
    except json.JSONDecodeError as error:
        raise LayoutFailure("%s did not print JSON lines, so its conversation id cannot be read: %s" % (stage, error))


def new_prompts():
    nonce = uuid.uuid4().hex[:10]
    return ("LAYOUT_PROMPT_ONE_" + nonce, "LAYOUT_PROMPT_TWO_" + nonce)


# ---------------------------------------------------------------------- report

def _dashed(path):
    return {"dashed": path.replace("/", "-").replace(".", "-"), "dashed_all": re.sub(r"[^A-Za-z0-9]", "-", path)}


def needles(ws, conversation_id):
    result = {"project": str(ws.project), "conversation_id": conversation_id}
    result.update({"project_" + k: v for k, v in _dashed(str(ws.project)).items()})
    return result


# What a published sample must not hold: the person's home folder (also as the
# CLI dashes it), any /Users/<name> path, git remote URLs and branch names.
_USER_PATH = re.compile(r"/Users/(?!USER(?![A-Za-z0-9]))[^/\s\"'\\:;,<>]+")
_USER_DASHED = re.compile(r"-Users-(?!USER-)[A-Za-z0-9]+(?=-)")
_REMOTE_URL = re.compile(
    r"git@[A-Za-z0-9.\-]+:[A-Za-z0-9._/\-]+"
    r"|(?:https?|ssh|git)://[^\s\"'\\<>]*(?:github\.com|gitlab\.com|bitbucket\.org)[^\s\"'\\<>]*"
)
_BRANCH_FIELDS = (
    re.compile(r'"gitBranch"\s*:\s*"([^"\\]+)"'),
    re.compile(r'"git_branch"\s*:\s*"([^"\\]+)"'),
    re.compile(r"Current branch: (\S+)"),
)
_KEEP_BRANCHES = {"", "main", "master", "HEAD", "develop", "trunk"}
_EXAMPLE_REMOTE = "https://example.com/org/repo.git"


def git_remotes():
    """The fetch/push URLs of the repository this test runs in, best effort."""
    try:
        done = subprocess.run(["git", "remote", "-v"], capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return []
    urls = set()
    for line in done.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 2:
            urls.add(parts[1])
    return sorted(urls)


def branches_in(contents):
    """Git branch names written into the CLI's files, except the trunk names."""
    found = set()
    for full, _, _ in contents.values():
        try:
            if full.stat().st_size > MAX_SAMPLE:
                continue
            text = full.read_bytes().decode("utf-8", "replace")
        except OSError:
            continue
        for pattern in _BRANCH_FIELDS:
            found.update(pattern.findall(text))
    return sorted(name for name in found if name not in _KEEP_BRANCHES)


def _rewrites(ws, branches=(), remotes=()):
    """(old, new) text pairs, longest first, for testdata sanitising."""
    pairs = set()
    actuals = [(ws.project, "/WORK", "-WORK"), (ws.home, "/HOME", "-HOME"), (ws.root, "/ROOT", "-ROOT")]
    real_home = os.path.expanduser("~")
    if real_home.startswith("/") and len(real_home) > 1:
        actuals.append((Path(real_home), "/Users/USER", "-Users-USER"))
    for actual, new, dashed_new in actuals:
        variants = {str(actual), str(actual).replace(str(ws.root), str(ws.raw_root), 1)}
        for base in variants:
            pairs.add((base, new))
            for form in _dashed(base).values():
                pairs.add((form, dashed_new))
    for branch in branches:
        if branch not in _KEEP_BRANCHES:
            pairs.add((branch, "example-branch"))
    for remote in remotes:
        pairs.add((remote, _EXAMPLE_REMOTE))
    return sorted(pairs, key=lambda p: -len(p[0]))


def sanitize_text(text, pairs):
    for old, new in pairs:
        text = text.replace(old, new)
    text = _USER_PATH.sub("/Users/USER", text)
    text = _USER_DASHED.sub("-Users-USER", text)
    return _REMOTE_URL.sub(_EXAMPLE_REMOTE, text)


def sanitize_bytes(data, pairs):
    try:
        return sanitize_text(data.decode("utf-8"), pairs).encode("utf-8")
    except UnicodeDecodeError:
        for old, new in pairs:
            data = data.replace(old.encode(), new.encode())
        return data


def _shown(value):
    if isinstance(value, (bytes, bytearray)):
        return {"blob_len": len(value), "head_hex": bytes(value[:16]).hex()}
    if isinstance(value, str) and len(value) > 400:
        return value[:400] + "...(%d chars)" % len(value)
    return value


def _type_name(value):
    return type(value).__name__


def _walk_paths(value, prefix, out, hits, needle_map, depth=0):
    if depth > 8:
        return
    if isinstance(value, dict):
        for key, child in value.items():
            path = prefix + "." + key if prefix else key
            out.setdefault(path, set()).add(_type_name(child))
            _walk_paths(child, path, out, hits, needle_map, depth + 1)
    elif isinstance(value, list):
        path = prefix + "[]"
        for child in value:
            out.setdefault(path, set()).add(_type_name(child))
            _walk_paths(child, path, out, hits, needle_map, depth + 1)
    elif isinstance(value, str):
        for label, text in needle_map.items():
            if text in value:
                hits.setdefault(prefix, set()).add(label)


def _record_label(record):
    if not isinstance(record, dict):
        return "<" + _type_name(record) + ">"
    kind = record.get("type")
    label = kind if isinstance(kind, str) else "<none>"
    payload = record.get("payload")
    if isinstance(payload, dict) and isinstance(payload.get("type"), str):
        label += "/" + payload["type"]
    return label


def _structure(records, needle_map):
    by_type = {}
    hits = {}
    for record in records:
        label = _record_label(record)
        entry = by_type.setdefault(label, {"count": 0, "paths": {}})
        entry["count"] += 1
        if len(entry["paths"]) < MAX_PATHS_PER_TYPE:
            _walk_paths(record, "", entry["paths"], hits, needle_map)
        else:
            _walk_paths(record, "", {}, hits, needle_map)
    shape = {label: {"count": e["count"], "key_paths": {p: sorted(t) for p, t in sorted(e["paths"].items())}} for label, e in sorted(by_type.items())}
    return shape, {path: sorted(labels) for path, labels in sorted(hits.items())}


def _light_structure(text, suffix):
    keys = []
    for line in text.splitlines():
        match = re.match(r"\s*\[+([^\]]+)\]+\s*$", line) if suffix == ".toml" else None
        if match:
            keys.append("[" + match.group(1) + "]")
            continue
        match = re.match(r'^\s*("?[A-Za-z0-9_.\-/ ]+"?)\s*[=:]', line)
        if match and not line.lstrip().startswith("#"):
            keys.append(match.group(1).strip())
    return sorted(set(keys))[:500]


def _sqlite_report(path, scratch, needle_map):
    copy_dir = Path(tempfile.mkdtemp(prefix="db-", dir=scratch))
    copy = copy_dir / path.name
    shutil.copyfile(path, copy)
    for suffix in ("-wal", "-shm"):
        sibling = Path(str(path) + suffix)
        if sibling.exists():
            shutil.copyfile(sibling, str(copy) + suffix)
    opened_as = "mode=ro"
    uri = "file:" + urllib.parse.quote(str(copy), safe="/") + "?mode=ro"
    try:
        connection = sqlite3.connect(uri, uri=True)
        connection.execute("select count(*) from sqlite_master").fetchone()
    except sqlite3.Error:
        # A WAL database cannot always be opened read-only without its -shm
        # file. The scratch copy may then be opened normally; the CLI's own
        # file is never touched.
        opened_as = "scratch copy, read-write (mode=ro failed)"
        connection = sqlite3.connect(str(copy))
    report = {"opened_as": opened_as, "objects": []}
    try:
        for kind, name, sql in connection.execute("select type, name, sql from sqlite_master order by type, name").fetchall():
            entry = {"type": kind, "name": name, "create": sql}
            if kind == "table":
                quoted = '"' + name.replace('"', '""') + '"'
                try:
                    entry["rows"] = connection.execute("select count(*) from " + quoted).fetchone()[0]
                    entry.update(_scan_table(connection, quoted, needle_map))
                except sqlite3.Error as error:
                    entry["error"] = str(error)
            report["objects"].append(entry)
    finally:
        connection.close()
        shutil.rmtree(copy_dir, ignore_errors=True)
    return report


def _scan_table(connection, quoted, needle_map):
    cursor = connection.execute("select * from " + quoted)
    columns = [d[0] for d in cursor.description]
    counts = {}
    id_rows = []
    project_rows = []
    scanned = 0
    encoded = {label: text.encode() for label, text in needle_map.items()}
    while True:
        batch = cursor.fetchmany(500)
        if not batch or scanned > 200000:
            break
        for row in batch:
            scanned += 1
            matched = set()
            for value in row:
                raw = value if isinstance(value, (bytes, bytearray)) else value.encode() if isinstance(value, str) else None
                if raw is None:
                    continue
                for label, needle in encoded.items():
                    if needle in raw:
                        matched.add(label)
            for label in matched:
                counts[label] = counts.get(label, 0) + 1
            shown = {c: _shown(v) for c, v in zip(columns, row)}
            if "conversation_id" in matched and len(id_rows) < 50:
                id_rows.append(shown)
            elif matched and len(project_rows) < 5:
                project_rows.append(shown)
    result = {"columns": columns}
    if counts:
        result["needle_row_counts"] = counts
    if id_rows:
        result["rows_with_conversation_id"] = id_rows
    if project_rows:
        result["sample_rows_with_project_path"] = project_rows
    return result


def build_report(cli, version, ws, conversation_id, prompts):
    needle_map = needles(ws, conversation_id)
    encoded = {label: text.encode() for label, text in needle_map.items()}
    home = ws.home
    files = []
    directories = []
    scratch = Path(tempfile.mkdtemp(prefix="scan-", dir=ws.root))
    contents = {}
    for current, dirnames, filenames in os.walk(home):
        dirnames.sort()
        for name in dirnames:
            full = Path(current) / name
            rel = str(full.relative_to(home))
            entry = {"path": rel}
            hits = [label for label, text in needle_map.items() if text in rel]
            if hits:
                entry["path_hits"] = hits
            directories.append(entry)
        for name in sorted(filenames):
            full = Path(current) / name
            rel = str(full.relative_to(home))
            entry = {"path": rel}
            if full.is_symlink():
                entry["symlink_to"] = os.readlink(full)
                files.append(entry)
                continue
            size = full.stat().st_size
            entry["size"] = size
            hits = [label for label, text in needle_map.items() if text in rel]
            if hits:
                entry["path_hits"] = hits
            if size > MAX_SCAN:
                entry["skipped"] = "larger than scan limit"
                files.append(entry)
                continue
            data = full.read_bytes()
            raw_hits = {label: data.count(needle) for label, needle in encoded.items() if needle in data}
            if raw_hits:
                entry["content_hits"] = raw_hits
            is_db = data.startswith(SQLITE_MAGIC)
            suffix = full.suffix.lower()
            if is_db:
                entry["kind"] = "sqlite"
                try:
                    entry["sqlite"] = _sqlite_report(full, scratch, needle_map)
                except (sqlite3.Error, OSError) as error:
                    entry["sqlite_error"] = str(error)
            elif suffix == ".jsonl":
                records = []
                bad = 0
                for line in data.decode("utf-8", "replace").splitlines():
                    if not line.strip():
                        continue
                    try:
                        records.append(json.loads(line))
                    except json.JSONDecodeError:
                        bad += 1
                entry["kind"] = "jsonl"
                entry["lines"] = len(records)
                if bad:
                    entry["unparsable_lines"] = bad
                entry["record_types"], entry["key_path_hits"] = _structure(records, needle_map)
            elif suffix == ".json":
                entry["kind"] = "json"
                try:
                    document = json.loads(data.decode("utf-8", "replace"))
                    entry["record_types"], entry["key_path_hits"] = _structure([document], needle_map)
                except json.JSONDecodeError:
                    entry["unparsable"] = True
            elif suffix in (".yaml", ".yml", ".toml"):
                entry["kind"] = suffix[1:]
                entry["top_keys"] = _light_structure(data.decode("utf-8", "replace"), suffix)
            if not entry.get("key_path_hits"):
                entry.pop("key_path_hits", None)
            files.append(entry)
            if not is_db:
                contents[rel] = (full, hits, raw_hits)
    shutil.rmtree(scratch, ignore_errors=True)

    mentions = [f for f in files if f.get("path_hits") or f.get("content_hits") or any(o.get("needle_row_counts") for o in f.get("sqlite", {}).get("objects", []))]
    report = {
        "cli": cli,
        "version": version,
        "conversation_id": conversation_id,
        "prompts": list(prompts),
        "project_path": str(ws.project),
        "home_path": str(home),
        "needles": needle_map,
        "directories": directories,
        "files": files,
        "files_mentioning_project_or_id": [{"path": f["path"], "path_hits": f.get("path_hits", []), "content_hits": f.get("content_hits", {}), "json_key_paths": f.get("key_path_hits", {})} for f in mentions],
    }
    return report, contents


def copy_sample(cli, ws, conversation_id, contents, output):
    """Copy the conversation's own files, rewritten to /HOME and /WORK."""
    pairs = _rewrites(ws, branches_in(contents), git_remotes())
    target = output / "sample" / cli
    if target.exists():
        shutil.rmtree(target)
    copied = []
    for rel, (full, path_hits, content_hits) in sorted(contents.items()):
        own = "conversation_id" in path_hits or "conversation_id" in content_hits
        projecty = any(k.startswith("project") for k in list(path_hits) + list(content_hits))
        if not (own or projecty) or full.stat().st_size > MAX_SAMPLE:
            continue
        destination = target / sanitize_text(rel, pairs)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(sanitize_bytes(full.read_bytes(), pairs))
        copied.append(sanitize_text(rel, pairs))
    return copied


def check_robust(report, contents, ws, conversation_id, prompts):
    """Only facts every layout must satisfy; the report is the real output."""
    owned = [f["path"] for f in report["files"] if conversation_id in f["path"]] + [d["path"] for d in report["directories"] if conversation_id in d["path"]]
    if not owned:
        raise LayoutFailure("No file or folder under the temporary HOME has the conversation id %s in its name. The layout report still lists everything the CLI wrote; read it to learn where this CLI keeps conversations." % conversation_id)
    first, second = prompts
    ordered = False
    for rel, (full, _, content_hits) in contents.items():
        if conversation_id not in rel and "conversation_id" not in content_hits:
            continue
        text = full.read_bytes().decode("utf-8", "replace")
        a = text.find(first)
        if a >= 0 and text.find(second, a + len(first)) >= 0:
            ordered = True
            break
    if not ordered:
        raise LayoutFailure("No file that names conversation %s contains both prompts in order. Check the report for where this CLI stores user messages; it may keep them outside plain files." % conversation_id)
    parent_mentioned = any(f.get("path_hits") or any(k.startswith("project") for k in f.get("content_hits", {})) for f in report["files"])
    if not parent_mentioned:
        raise LayoutFailure("Nothing under the temporary HOME records the project folder path, so a carry would have nothing to rewrite. Check whether the CLI keeps the path somewhere this fixture does not look.")


def finish(cli, version, ws, conversation_id, prompts):
    output = outputs_dir(cli)
    report, contents = build_report(cli, version, ws, conversation_id, prompts)
    report["sample_files"] = copy_sample(cli, ws, conversation_id, contents, output)
    # The report names the temporary root; keep it readable but stable.
    pairs = _rewrites(ws, branches_in(contents), git_remotes())
    text = sanitize_text(json.dumps(report, indent=2, sort_keys=False), pairs)
    (output / (cli + "_layout.json")).write_text(text + "\n")
    check_robust(report, contents, ws, conversation_id, prompts)
    print(json.dumps({"cli": cli, "version": version, "conversation_id": conversation_id, "files": len(report["files"]), "sample_files": report["sample_files"], "report": str(output / (cli + "_layout.json"))}))


def main(run):
    try:
        run()
    except (LayoutFailure, subprocess.TimeoutExpired, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
