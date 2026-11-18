"""Installed Copilot CLI: the facts the first fork/carry fixture left open (ROAM round 3).

Scenarios, each in throwaway HOMEs against a loopback model that can answer
with tool calls:
  tools    a view, a bash and a subagent turn; fork by hand; carry to folder B
  compact  /compact; fork before and after it; carry with its checkpoint file
  inter    one interactive process, two prompts; /rewind; resume
  git      workspace.yaml for a multi-line name, YAML quoting of branches,
           a detached head, and whether the CLI reads a recorded branch back
Writes copilot_roam2.json (facts: yes / no / not testable, with evidence) and
scrubbed captures under testdata/ (placeholders {folder}, {id}, {copilot}) to
$TEST_UNDECLARED_OUTPUTS_DIR. Offline, loopback only, no credentials.
"""
import contextlib
import http.server
import json
import os
from pathlib import Path
import re
import select
import shutil
import signal
import subprocess
import sys
import threading
import time
import types
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common
import copilot_roam as cr
import roam_common as rc

SUMMARY_MARK = "COMPACT_SUMMARY_MARK"
NOTE_MARK = "NOTE_FILE_MARK"
TOOL_DONE = "TOOL_DONE_MARK"
# What a headless run is allowed to do: the tools in the project folder only.
ALLOW = ["--allow-all-tools"]
ANSI = re.compile(rb"\x1b\[[0-9;?]*[a-zA-Z]")


# ------------------------------------------------------------ scripted model

class _Scripted(rc._Recording):
    """Answers a tool call for a marked prompt, a summary for a compaction, else text."""

    def _completions(self, body, answer, index):
        reply = self.server.reply(body)
        if reply is None:
            return super()._completions(body, answer, index)
        usage = {"prompt_tokens": 20, "completion_tokens": 4, "total_tokens": 24}
        choice = {"index": 0, "message": reply, "finish_reason": "tool_calls" if reply.get("tool_calls") else "stop"}
        self._send("application/json", json.dumps({"id": "c%d" % index, "object": "chat.completion", "created": 1, "model": "fixture-layout", "choices": [choice], "usage": usage}).encode())


class _Server(rc.RecordingServer):
    def __init__(self):
        http.server.ThreadingHTTPServer.__init__(self, ("127.0.0.1", 0), _Scripted)
        self.lock = threading.Lock()
        self.routes = []
        self.requests = []
        self.project = "/nowhere"

    def reply(self, body):
        messages = body.get("messages", [])
        if not messages:
            return None
        last = messages[-1]
        text = json.dumps(last)
        if last.get("role") == "tool":
            return {"role": "assistant", "content": TOOL_DONE}
        if "Please create a detailed summary" in text:
            return {"role": "assistant", "content": SUMMARY_MARK}
        names = {t.get("function", {}).get("name") for t in body.get("tools", [])}
        if "SCENARIO_VIEW" in text and "view" in names:
            return _call("view", {"path": self.project + "/note.txt"})
        if "SCENARIO_BASH" in text and "bash" in names:
            return _call("bash", {"command": "pwd", "description": "print the folder"})
        if "SCENARIO_SUB" in text and "task" in names:
            return _call("task", {"name": "helper", "description": "a helper job", "prompt": "SUBAGENT_WORK_MARK", "agent_type": "explore"})
        return None


def _call(name, arguments):
    return {"role": "assistant", "content": None, "tool_calls": [{"id": "call_" + name, "type": "function", "function": {"name": name, "arguments": json.dumps(arguments)}}]}


@contextlib.contextmanager
def scripted_server():
    server = _Server()
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        yield server
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=3)


# --------------------------------------------------------------- the machine

WORKSPACES = []


def workspace(stack):
    ws = rc.new_workspace(stack, "copilot")
    WORKSPACES.append(("W%d" % len(WORKSPACES), ws))
    return ws


class Box:
    """A workspace, its CLI environment and the CLI to run in it."""

    def __init__(self, binary, server, ws, git=False):
        self.binary, self.server, self.ws = binary, server, ws
        self.env, self.copilot = cr.environment(ws, server)
        self.env.update({"GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1", "GIT_AUTHOR_NAME": "t", "GIT_COMMITTER_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.invalid", "GIT_COMMITTER_EMAIL": "t@example.invalid"})
        self.env["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin"
        (ws.project / "note.txt").write_text(NOTE_MARK + "\n")
        if git:
            self.git("init", "-q", "-b", "main")
            self.git("add", ".")
            self.git("commit", "-qm", "first")

    def git(self, *args):
        done = subprocess.run(["/usr/bin/git", *args], cwd=self.ws.project, env=self.env, capture_output=True, text=True)
        return done.returncode == 0

    def argv(self, extra=()):
        return cr.base_argv(self.binary) + list(extra)

    def headless(self, report, stage, prompt, resume=None, extra=ALLOW):
        self.server.project = str(self.ws.project)
        mark = self.server.mark()
        argv = self.argv(extra) + (["--resume=" + resume] if resume else []) + ["--prompt", prompt]
        out, why = rc.try_run(report, stage, argv, self.ws, self.env)
        return out, why, self.server.bodies_since(mark)

    def sessions(self):
        return cr.session_folders(self.copilot)

    def events(self, cid):
        return [cr.record_of(l) for l in rc.jsonl_lines(self.copilot / "session-state" / cid / "events.jsonl")]


def new_session(box, report, stage, prompt):
    before = {p.name for p in box.sessions()}
    out, why, bodies = box.headless(report, stage, prompt)
    fresh = [p for p in box.sessions() if p.name not in before]
    if out is None or len(fresh) != 1:
        raise common.LayoutFailure("%s did not leave one new session folder (%s)." % (stage, why))
    return fresh[0].name, bodies


def lines_before_turn(box, cid, turn):
    """The transcript's complete lines before turn `turn` (from 1), the way the adapter cuts one."""
    lines = rc.jsonl_lines(box.copilot / "session-state" / cid / "events.jsonl")
    records = [cr.record_of(l) for l in lines]
    users = [i for i, r in enumerate(records) if r.get("type") == "user.message" and not isinstance(r.get("data", {}).get("source"), str)]
    cut = users[turn - 1]
    while cut > 0 and records[cut - 1].get("type") in ("session.shutdown", "session.resume"):
        cut -= 1
    return lines[:cut]


def hand_fork(box, cid, turn, with_files=False):
    fork_id = str(uuid.uuid4())
    kept = [l.decode("utf-8").replace(cid, fork_id) for l in lines_before_turn(box, cid, turn)]
    folder = box.copilot / "session-state" / fork_id
    if with_files:
        cr.copy_with(box.copilot / "session-state" / cid, folder, [(cid, fork_id)])
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "events.jsonl").write_text("".join(kept))
    return fork_id


def carry(src, dst, cid):
    """Copy session `cid` of `src` to `dst`, every text file rewritten, binary files as they are."""
    pairs = rc.path_pairs(src.ws.project, dst.ws.project, extra=[(src.copilot, dst.copilot), (src.ws.home, dst.ws.home)])
    cr.copy_with(src.copilot / "session-state" / cid, dst.copilot / "session-state" / cid, pairs)


def leaked(src, dst, bodies):
    needles = [str(src.ws.project), str(src.ws.home), str(src.ws.root)]
    naming = rc.files_naming(dst.ws.home, needles)
    return {"files_naming_a": {k: v[:5] for k, v in naming.items() if v}, "requests_naming_a": [n for n in needles if any(n in b for b in bodies)]}


def seen(bodies, marks):
    return [any(m in b for b in bodies) for m in marks]


def types_of(records):
    return sorted({r.get("type", "<none>") for r in records})


def data_keys(records):
    keys = {}
    for r in records:
        keys.setdefault(r.get("type", "<none>"), set()).update(r.get("data", {}).keys())
    return {k: sorted(v) for k, v in sorted(keys.items())}


# ---------------------------------------------------------------- captures

class Captures:
    """Scrubbed copies of session files, with {folder}, {id} and {copilot} in place of the real ones."""

    def __init__(self):
        self.root = common.outputs_dir("copilot") / "testdata"
        if self.root.exists():
            shutil.rmtree(self.root)
        self.root.mkdir(parents=True)

    def save(self, box, cid, label, files):
        pairs = [(str(box.copilot), "{copilot}"), (str(box.ws.project), "{folder}"), (rc.json_escaped(str(box.ws.project)), "{folder}"), (cid, "{id}")]
        pairs += [(str(box.ws.root), "/ROOT"), (str(box.ws.home), "/HOME")]
        real_home = os.path.expanduser("~")
        if len(real_home) > 1:
            pairs.append((real_home, "/Users/USER"))
        pairs = sorted(pairs, key=lambda p: -len(p[0]))
        source = box.copilot / "session-state" / cid
        out = self.root / label
        out.mkdir(parents=True, exist_ok=True)
        for rel, name in files:
            data = (source / rel).read_bytes()
            (out / name).write_bytes(common.sanitize_bytes(data, pairs))


# ------------------------------------------------------------- the scenarios

def scenario_tools(binary, server, report, caps, stack):
    a = Box(binary, server, workspace(stack))
    b = Box(binary, server, workspace(stack))
    prompts = rc.roam_prompts()
    marks = [prompts[0] + " SCENARIO_VIEW", prompts[1] + " SCENARIO_BASH", prompts[2] + " SCENARIO_SUB", prompts[3]]
    cid, bodies = new_session(a, report, "tools turn 1", marks[0])
    for number in (1, 2):
        out, why, _ = a.headless(report, "tools turn %d" % (number + 1), marks[number], resume=cid)
        report.expect(out is not None, "tools turn %d failed: %s" % (number + 1, why))
    records = a.events(cid)
    users = [i for i, r in enumerate(records) if r.get("type") == "user.message"]
    plain = [i for i in users if not isinstance(records[i]["data"].get("source"), str)]
    sub = [i for i in users if isinstance(records[i]["data"].get("source"), str)]
    ends = [r for r in records if r.get("type") == "assistant.turn_end"]
    top_ends = [r for r in ends if "parentToolCallId" not in r["data"]]
    report.observe("tools_event_types", types_of(records))
    report.observe("tools_data_keys", data_keys(records))
    report.observe("tools_top_level_extra_fields", sorted({k for r in records for k in r if k not in ("type", "data", "id", "timestamp", "parentId")}))
    report.fact("a tool call writes tool.execution_start/complete and a second assistant.turn_start/turn_end inside the same prompt's turn", "yes" if len(top_ends) > len(plain) else "no", {"user_messages_typed": len(plain), "turn_end_records_of_the_main_agent": len(top_ends)})
    report.fact("a subagent's own prompt is a user.message with a `source` field and parentAgentTaskId, and is not a turn the person typed", "yes" if len(sub) == 1 and len(plain) == 3 else "no", {"typed": len(plain), "agent_issued": len(sub), "source_values": [records[i]["data"].get("source") for i in sub]})
    report.fact("the subagent's records carry a top-level agentId and a parentToolCallId on its turn start, message and end", "yes" if any("agentId" in r for r in records) and any("parentToolCallId" in r.get("data", {}) for r in records) else "no", {"types_with_agentId": sorted({r["type"] for r in records if "agentId" in r})})
    caps.save(a, cid, "tools", [("events.jsonl", "events.jsonl"), ("workspace.yaml", "workspace.yaml")])

    fork_id = hand_fork(a, cid, 3)
    out, why, fork_bodies = a.headless(report, "tools fork resume", marks[3], resume=fork_id)
    s = seen(fork_bodies, [prompts[0], prompts[1], prompts[2], prompts[3], NOTE_MARK, str(a.ws.project)])
    ok = out is not None and s[:5] == [True, True, False, True, True]
    report.fact("a hand fork of a conversation with tool calls (view, bash), cut before turn 3, resumes with the tool results as history", "yes" if ok else "no", {"seen_p1_p2_p3_p4_noteMark_folderA": s, "error": why})
    report.expect(ok, "The fork of the tool conversation did not resume with turns 1 and 2 and the view result (%r, %s)." % (s, why))

    carry(a, b, cid)
    out, why, carried = b.headless(report, "tools carry resume", marks[3], resume=cid)
    s = seen(carried, [prompts[0], prompts[1], prompts[2], prompts[3], NOTE_MARK, SUMMARY_MARK])
    ok = out is not None and s[:5] == [True, True, True, True, True]
    leak = leaked(a, b, carried)
    report.fact("a carried conversation with tool calls and a subagent resumes in folder B with every turn and the view result", "yes" if ok else "no", {"seen": s, "error": why})
    report.fact("after that carry nothing under the new HOME and no request to the model names folder A or A's HOME", "yes" if not any(leak.values()) else "no", leak)
    report.expect(ok, "The carried tool conversation did not resume with all turns (%r, %s)." % (s, why))
    # A turn that is cut right after its first assistant.turn_end (the tool turn) is still running.
    cut_at = next(i for i, r in enumerate(records) if r.get("type") == "assistant.turn_end" and users[0] < i)
    report.observe("tools_first_turn_end_is_followed_by", records[cut_at + 1].get("type"))


def scenario_compact(binary, server, report, caps, stack):
    a = Box(binary, server, workspace(stack))
    b = Box(binary, server, workspace(stack))
    prompts = rc.roam_prompts()
    cid, _ = new_session(a, report, "compact turn 1", prompts[0])
    for stage, prompt in (("compact turn 2", prompts[1]), ("compact the history", "/compact"), ("compact turn 3", prompts[2])):
        out, why, _ = a.headless(report, stage, prompt, resume=cid)
        report.expect(out is not None, "%s failed: %s" % (stage, why))
    folder = a.copilot / "session-state" / cid
    records = a.events(cid)
    report.observe("compact_event_types", types_of(records))
    report.observe("compact_data_keys", {k: v for k, v in data_keys(records).items() if "compaction" in k or "usage_checkpoint" in k})
    report.observe("compact_folder_files", rc.tree_listing(folder))
    users = [r for r in records if r.get("type") == "user.message"]
    report.fact("/compact writes no user.message, so it is not a turn", "yes" if len(users) == 3 else "no", {"user_messages": len(users)})
    report.fact("/compact adds session.compaction_start, session.compaction_complete and session.usage_checkpoint, a checkpoints/NNN-*.md file and a session.db SQLite file", "yes" if {"session.compaction_start", "session.compaction_complete"} <= set(types_of(records)) and any(p.name.endswith(".md") and p.name != "index.md" for p in (folder / "checkpoints").glob("*")) and (folder / "session.db").is_file() else "no", {"files": rc.tree_listing(folder)})
    db = rc.sqlite_rows(folder / "session.db", "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
    rows = {}
    if isinstance(db, list):
        for (name,) in db:
            count = rc.sqlite_rows(folder / "session.db", 'SELECT count(*) FROM "%s"' % name)
            rows[name] = count[0][0] if isinstance(count, list) else count
    report.observe("compact_session_db_rows_per_table", rows)
    data = (folder / "session.db").read_bytes()
    report.fact("session.db holds no path, id or prompt of the conversation", "yes" if not any(n.encode() in data for n in (str(a.ws.project), str(a.ws.home), cid, prompts[0])) else "no", {"tables_with_rows": {k: v for k, v in rows.items() if v}})
    caps.save(a, cid, "compact", [("events.jsonl", "events.jsonl"), ("workspace.yaml", "workspace.yaml"), (next(str(p.relative_to(folder)) for p in (folder / "checkpoints").glob("0*.md")), "checkpoint.md"), ("checkpoints/index.md", "index.md")])

    after = hand_fork(a, cid, 3)
    out, why, fork_bodies = a.headless(report, "compact fork after resume", prompts[3], resume=after)
    s = seen(fork_bodies, [prompts[0], prompts[1], prompts[2], prompts[3], SUMMARY_MARK])
    ok = out is not None and s[3] and s[4] and not s[2]
    report.fact("a hand fork cut after /compact, with only events.jsonl (no checkpoint file), resumes and its request carries the summary", "yes" if ok else "no", {"seen_p1_p2_p3_p4_summary": s, "error": why})
    report.expect(ok, "The fork after /compact did not resume with the summary (%r, %s)." % (s, why))
    before = hand_fork(a, cid, 2)
    out, why, fork_bodies = a.headless(report, "compact fork before", prompts[3], resume=before)
    s = seen(fork_bodies, [prompts[0], prompts[1], prompts[3], SUMMARY_MARK])
    ok = out is not None and s == [True, False, True, False]
    report.fact("a hand fork cut before the compacted turns resumes with turn 1 only and no summary", "yes" if ok else "no", {"seen_p1_p2_p4_summary": s, "error": why})

    carry(a, b, cid)
    out, why, carried = b.headless(report, "compact carry resume", prompts[3], resume=cid)
    s = seen(carried, [prompts[2], prompts[3], SUMMARY_MARK])
    ok = out is not None and s[1] and s[2]
    report.fact("a carried compacted conversation, with its checkpoint file and session.db copied as they are, resumes in folder B with the summary", "yes" if ok else "no", {"seen_p3_p4_summary": s, "error": why})
    report.expect(ok, "The carried compacted conversation did not resume (%r, %s)." % (s, why))
    report.fact("after the compaction carry nothing under the new HOME or in B's requests names folder A", "yes" if not any(leaked(a, b, carried).values()) else "no", leaked(a, b, carried))
    solo = Box(binary, server, workspace(stack))
    cr.copy_with(folder, solo.copilot / "session-state" / cid, rc.path_pairs(a.ws.project, solo.ws.project, extra=[(a.copilot, solo.copilot), (a.ws.home, solo.ws.home)]))
    (solo.copilot / "session-state" / cid / "session.db").unlink()
    out, why, bare = solo.headless(report, "compact carry without session.db", prompts[3], resume=cid)
    report.fact("a carried compacted conversation resumes without session.db too", "yes" if out is not None and seen(bare, [SUMMARY_MARK])[0] else "no", {"error": why})


class Tty:
    def __init__(self, argv, env, cwd):
        import fcntl, pty, struct, termios
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(cwd)
            os.execve(argv[0], argv, env)
        self.pid, self.fd, self.buf = pid, fd, b""
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 160, 0, 0))

    def pump(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([self.fd], [], [], 0.2)
            if ready:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    return
                if not data:
                    return
                self.buf += data

    def send(self, text):
        os.write(self.fd, text.encode())

    def screen(self):
        return ANSI.sub(b"", self.buf).decode("utf-8", "replace")

    def close(self):
        """End the CLI: ask, drain what it still prints, then stop it. Never waits on a full terminal."""
        try:
            os.kill(self.pid, signal.SIGTERM)
        except OSError:
            pass
        self.pump(3)
        for _ in range(2):
            try:
                done, _ = os.waitpid(self.pid, os.WNOHANG)
            except OSError:
                break
            if done:
                break
            try:
                os.kill(self.pid, signal.SIGKILL)
            except OSError:
                break
            time.sleep(1)
        try:
            os.close(self.fd)
        except OSError:
            pass


def typed(box):
    folders = box.sessions()
    out = []
    for folder in folders:
        path = folder / "events.jsonl"
        if path.is_file():
            out += [cr.record_of(l).get("data", {}).get("content") for l in rc.jsonl_lines(path) if b'"type":"user.message"' in l]
    return out


def scenario_inter(binary, server, report, caps, stack):
    a = Box(binary, server, workspace(stack))
    b = Box(binary, server, workspace(stack))
    prompts = rc.roam_prompts()
    argv = [binary] + [x for x in a.argv(ALLOW)[1:] if not x.startswith("--output-format") and x not in ("--stream=off", "--no-ask-user")]
    tty = Tty(argv, a.env, str(a.ws.project))
    try:
        tty.pump(40)
        # The first keys typed while the screen is still starting are lost or cut, so a
        # throwaway prompt goes first; the two prompts the facts are about must arrive whole.
        for prompt in ("WARMUP_PROMPT", prompts[0], prompts[1]):
            for _ in range(4):
                tty.send(prompt)
                tty.pump(2)
                tty.send("\r")
                tty.pump(12)
                taken = typed(a)
                if (prompt in taken) if prompt != "WARMUP_PROMPT" else bool(taken):
                    break
            else:
                report.expect(False, "The interactive CLI never took the prompt %s (the screen's tail: %s)." % (prompt, tty.screen()[-300:]))
        tty.pump(5)
        cid = a.sessions()[0].name
        records = a.events(cid)
        users = [r for r in records if r.get("type") == "user.message"]
        later = [i + 1 for i, r in enumerate(users) if r["data"].get("content") == prompts[1]]
        second = later[0] if later else 3
        report.observe("inter_event_types", types_of(records))
        report.observe("inter_shapes", [r["type"] for r in records])
        shutdowns_before_exit = sum(1 for r in records if r.get("type") == "session.shutdown")
        report.fact("one interactive process with several prompts writes one user.message each and no session.shutdown or session.resume between them", "yes" if len(users) >= 3 and shutdowns_before_exit == 0 and "session.resume" not in types_of(records) else "no", {"user_messages": len(users), "shutdowns_so_far": shutdowns_before_exit})
        report.observe("inter_folder_while_running", rc.tree_listing(a.copilot / "session-state" / cid))
        fork_id = hand_fork(a, cid, second)
        out, why, fork_bodies = a.headless(report, "inter fork resume", prompts[3], resume=fork_id)
        s = seen(fork_bodies, prompts)
        ok = out is not None and s[0] and not s[1] and s[3]
        report.fact("a hand fork between two prompts of one interactive process, with no shutdown before the cut, resumes with the first prompt only", "yes" if ok else "no", {"seen_p1_p2_p3_p4": s, "error": why})
        report.expect(ok, "The fork of the interactive conversation did not resume with only its first prompt (%r, %s)." % (s, why))
        # Rewind to the second prompt (conversation only).
        tty.buf = b""
        tty.send("/rewind")
        tty.pump(2)
        tty.send("\r")
        tty.pump(4)
        menu = tty.screen()
        report.observe("inter_rewind_menu_has_both_prompts", [p in menu for p in prompts[:2]])
        target = 0 if prompts[1] in menu.split(prompts[0])[0] or menu.find(prompts[1]) < menu.find(prompts[0]) else 1
        for _ in range(target):
            tty.send("\x1b[B")
            tty.pump(1)
        tty.send("\r")
        tty.pump(3)
        tty.send("\r")
        tty.pump(5)
        report.observe("inter_rewind_screen_tail", tty.screen()[-400:])
        rewound = a.events(cid)
        left = [r["data"].get("content") for r in rewound if r.get("type") == "user.message"]
        report.observe("inter_after_rewind_types", [r["type"] for r in rewound][-6:])
        report.observe("inter_after_rewind_user_messages", left)
        report.fact("/rewind (conversation only) rewrites events.jsonl in place, dropping the rewound prompt and everything after it, and appends a session.info record", "yes" if any(r.get("type") == "session.info" for r in rewound) and prompts[1] not in left and prompts[0] in left else "no", {"user_messages_left": len(left), "last_types": [r["type"] for r in rewound][-3:]})
        report.observe("inter_rewind_info", [r["data"] for r in rewound if r.get("type") == "session.info"])
        # The rewound prompt is put back in the input box; clear it before leaving.
        tty.send("\x15")
        tty.pump(1)
        tty.send("/exit")
        tty.pump(2)
        tty.send("\r")
        tty.pump(8)
    finally:
        tty.close()
    after = a.events(cid)
    report.observe("inter_final_types", [r["type"] for r in after])
    left = [r["data"].get("content") for r in after if r.get("type") == "user.message"]
    out, why, rewind_bodies = a.headless(report, "inter resume after rewind", prompts[3], resume=cid)
    s = seen(rewind_bodies, prompts)
    report.fact("a conversation resumes after a rewind with only the turns kept", "yes" if out is not None and s[3] and s[0] and not s[1] else "no", {"seen_p1_p2_p3_p4": s, "user_messages_kept": len(left), "error": why})
    caps.save(a, cid, "interactive", [("events.jsonl", "events.jsonl"), ("workspace.yaml", "workspace.yaml")])
    carry(a, b, cid)
    out, why, carried = b.headless(report, "inter carry resume", prompts[3], resume=cid)
    report.fact("a carried interactive conversation resumes in folder B", "yes" if out is not None and seen(carried, [prompts[3]])[0] else "no", {"error": why, **leaked(a, b, carried)})


BRANCHES = ["feat/x", "yes", "true", "null", "123", "1e3", "007", "+1", "@at#x", "a#b", "{x}", "x'y", "2026-01-01", "Null"]


def scenario_git(binary, server, report, caps, stack):
    a = Box(binary, server, workspace(stack), git=True)
    b = Box(binary, server, workspace(stack), git=True)
    prompts = rc.roam_prompts()
    # A name of several lines.
    cid, _ = new_session(a, report, "git multi-line prompt", prompts[0] + ": the first line\nsecond line # not a comment")
    text = (a.copilot / "session-state" / cid / "workspace.yaml").read_text()
    report.observe("git_workspace_yaml_multiline_name", text)
    report.fact("workspace.yaml holds a multi-line name as a block scalar (`name: |-` then indented lines)", "yes" if re.search(r"^name: \|-?\n(  .*\n?)+", text, re.M) else "no", {"name_lines": [l for l in text.splitlines() if l.startswith("name:") or l.startswith("  ")]})
    caps.save(a, cid, "block_name", [("workspace.yaml", "workspace.yaml"), ("events.jsonl", "events.jsonl")])
    # How the CLI quotes a branch.
    quoted = {}
    for branch in BRANCHES:
        if not a.git("checkout", "-q", "-B", branch):
            continue
        sid, _ = new_session(a, report, "git branch " + branch, "hi")
        line = [l for l in (a.copilot / "session-state" / sid / "workspace.yaml").read_text().splitlines() if l.startswith("branch:")]
        start = a.events(sid)[0]["data"]["context"]
        quoted[branch] = {"yaml": line[0] if line else None, "start_context_branch": start.get("branch")}
    report.observe("git_branch_quoting", quoted)
    report.fact("the CLI writes a branch plain in workspace.yaml unless YAML would read it as another type or an indicator (true, null, numbers, a leading @ { # etc.), then single-quoted, and context.branch in events.jsonl is always the bare name", "yes" if all(v["start_context_branch"] == k for k, v in quoted.items()) and quoted.get("true", {}).get("yaml") == "branch: 'true'" and quoted.get("feat/x", {}).get("yaml") == "branch: feat/x" else "no", {"rows": quoted})
    # Detached head.
    a.git("checkout", "-q", "main")
    a.git("checkout", "-q", "--detach")
    did, _ = new_session(a, report, "git detached", prompts[1])
    start = a.events(did)[0]["data"]["context"]
    detached_yaml = [l for l in (a.copilot / "session-state" / did / "workspace.yaml").read_text().splitlines() if l.startswith("branch:")]
    report.observe("git_detached_start_context_keys", sorted(start.keys()))
    report.fact("on a detached head the CLI records the branch as detached@<first 7 of the commit> in both files and the full commit as context.headCommit", "yes" if re.fullmatch(r"detached@[0-9a-f]{7}", start.get("branch", "")) and start.get("headCommit", "").startswith(start["branch"][9:]) and detached_yaml == ["branch: " + start["branch"]] else "no", {"context_branch": start.get("branch"), "yaml": detached_yaml, "has_headCommit": "headCommit" in start})
    caps.save(a, did, "detached", [("workspace.yaml", "workspace.yaml"), ("events.jsonl", "events.jsonl")])
    # Read-back: carry a session whose files name another branch and commit; B is on its own branch.
    a.git("checkout", "-q", "-B", "branch-in-a")
    rid, _ = new_session(a, report, "git readback session", prompts[2])
    b.git("checkout", "-q", "-B", "branch-in-b")
    carry(a, b, rid)
    marker_branch, marker_commit = "SENTINEL_BRANCH_MARK", "0123456789abcdef0123456789abcdef01234567"
    folder_b = b.copilot / "session-state" / rid
    events = folder_b / "events.jsonl"
    edited = []
    for line in events.read_text().splitlines(True):
        r = json.loads(line)
        if r.get("type") == "session.start":
            r["data"]["context"]["branch"] = marker_branch
            r["data"]["context"]["headCommit"] = marker_commit
            line = json.dumps(r, separators=(",", ":")) + "\n"
        edited.append(line)
    events.write_text("".join(edited))
    wy = folder_b / "workspace.yaml"
    wy.write_text(re.sub(r"^branch: .*$", "branch: " + marker_branch, wy.read_text(), flags=re.M))
    out, why, bodies = b.headless(report, "git readback resume", prompts[3], resume=rid)
    after_yaml = [l for l in wy.read_text().splitlines() if l.startswith("branch:")]
    resumed = [r for r in b.events(rid) if r.get("type") == "session.resume"]
    resume_branch = resumed[-1]["data"].get("context", {}).get("branch") if resumed else None
    named_in_request = any(marker_branch in body or marker_commit in body for body in bodies)
    real_in_request = any("branch-in-b" in body for body in bodies)
    report.fact("the CLI does not read the recorded branch or commit back: it re-reads git on resume (session.resume's context and workspace.yaml's branch: follow the folder's own branch)", "yes" if out is not None and resume_branch == "branch-in-b" and after_yaml == ["branch: branch-in-b"] and not named_in_request else "no", {"resume_context_branch": resume_branch, "yaml_branch_after": after_yaml, "recorded_branch_in_request": named_in_request, "folder_branch_in_request": real_in_request, "error": why})
    report.observe("git_readback_start_branch_after", b.events(rid)[0]["data"]["context"].get("branch"))


# ---------------------------------------------------------------------- main

SCENARIOS = {"tools": scenario_tools, "compact": scenario_compact, "inter": scenario_inter, "git": scenario_git}


def run():
    binary = common.binary_or_skip("Copilot", ["TERMINAL_COPILOT_BIN", "COPILOT_BIN"])
    wanted = [n for n in os.environ.get("ROAM2_ONLY", "").split(",") if n] or list(SCENARIOS)
    report = rc.Report("copilot")
    probe = types.SimpleNamespace(project=Path("/tmp"))
    report.data["version"] = common.run_cli("copilot --version", [binary, "--no-auto-update", "--version"], probe, {"PATH": "/usr/bin:/bin", "HOME": "/tmp"}, timeout=30).strip().splitlines()[0]
    caps = Captures()
    with contextlib.ExitStack() as stack:
        server = stack.enter_context(scripted_server())
        try:
            for name in wanted:
                try:
                    SCENARIOS[name](binary, server, report, caps, stack)
                except common.LayoutFailure as error:
                    report.expect(False, "scenario %s: %s" % (name, error))
        finally:
            report.finish(WORKSPACES, name="copilot_roam2")


if __name__ == "__main__":
    common.main(run)
