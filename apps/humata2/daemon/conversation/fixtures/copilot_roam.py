"""Installed Copilot CLI: fork by hand, carry to another HOME, resume (ROAM X2).

A three-turn conversation in folder A, a fork of its session-state folder
before turn 2, and a carry of the original to folder B under another
COPILOT_HOME. Copilot resumes by exact id (--resume=<id>). Offline, loopback
Chat Completions only.
"""
import contextlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common
import roam_common as rc

UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
SESSIONS = "SELECT id, cwd FROM sessions WHERE id = ?"


def environment(ws, server):
    copilot = ws.home / ".copilot"
    cache = ws.home / ".cache" / "copilot"
    for folder in (copilot, cache):
        folder.mkdir(mode=0o700, parents=True, exist_ok=True)
    env = ws.env({
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "COPILOT_HOME": str(copilot), "COPILOT_CACHE_HOME": str(cache),
        "COPILOT_OFFLINE": "true", "COPILOT_TELEMETRY_DISABLED": "true",
        "COPILOT_OTEL_ENABLED": "false", "COPILOT_DISABLE_LOGIN_SHELL_ENV": "true",
        "COPILOT_PLUGIN_DIR_ONLY": "true", "COPILOT_DISABLE_DESKTOP_NOTIFICATIONS": "1",
        "COPILOT_SYSTEM_PROXY": "0", "HTTP_PROXY": "http://127.0.0.1:9",
        "HTTPS_PROXY": "http://127.0.0.1:9", "NO_PROXY": "127.0.0.1,localhost",
        "COPILOT_PROVIDER_BASE_URL": "http://127.0.0.1:%d/v1" % server.server_port,
        "COPILOT_PROVIDER_TYPE": "openai", "COPILOT_PROVIDER_WIRE_API": "completions",
        "COPILOT_PROVIDER_MODEL_ID": "gpt-4.1", "COPILOT_PROVIDER_WIRE_MODEL": "fixture-layout",
        "COPILOT_MODEL": "gpt-4.1", "COPILOT_PROVIDER_MAX_PROMPT_TOKENS": "128000",
        "COPILOT_PROVIDER_MAX_OUTPUT_TOKENS": "1024",
    })
    return env, copilot


def base_argv(binary):
    return [binary, "--no-auto-update", "--disable-client-telemetry", "--disable-builtin-mcps", "--no-custom-instructions", "--no-bash-env", "--no-ask-user", "--sync-session=off", "--output-format=json", "--stream=off", "--log-level=error"]


def session_folders(copilot):
    root = copilot / "session-state"
    return sorted(p for p in root.glob("*") if p.is_dir() and UUID.match(p.name)) if root.is_dir() else []


def record_of(line):
    try:
        value = json.loads(line)
    except json.JSONDecodeError:
        return {}
    return value if isinstance(value, dict) else {}


def text_files(folder):
    for p in sorted(folder.rglob("*")):
        if p.is_file() and not p.is_symlink() and not p.name.endswith(".lock"):
            yield p


def copy_with(src, dst, pairs):
    """Copy a session folder to `dst`, every text file with `pairs` applied, binary files as they are."""
    for p in text_files(src):
        target = dst / p.relative_to(src)
        target.parent.mkdir(parents=True, exist_ok=True)
        data = p.read_bytes()
        try:
            target.write_text(rc.rewrite_text(data.decode("utf-8"), pairs))
        except UnicodeDecodeError:
            target.write_bytes(data)


def run():
    binary = common.binary_or_skip("Copilot", ["TERMINAL_COPILOT_BIN", "COPILOT_BIN"])
    prompts = rc.roam_prompts()
    report = rc.Report("copilot")
    with contextlib.ExitStack() as stack:
        server = stack.enter_context(rc.recording_server())
        ws_a = rc.new_workspace(stack, "copilot")
        ws_b = rc.new_workspace(stack, "copilot")
        try:
            body(binary, prompts, report, server, ws_a, ws_b)
        finally:
            report.finish([("A", ws_a), ("B", ws_b)])


def body(binary, prompts, report, server, ws_a, ws_b):
    env_a, copilot_a = environment(ws_a, server)
    env_b, copilot_b = environment(ws_b, server)
    argv = base_argv(binary)
    report.data["version"] = common.run_cli("copilot --version", [binary, "--no-auto-update", "--version"], ws_a, env_a, timeout=30).strip().splitlines()[0]

    common.run_cli("Copilot turn 1", argv + ["--prompt", prompts[0]], ws_a, env_a)
    folders = session_folders(copilot_a)
    if len(folders) != 1:
        raise common.LayoutFailure("Copilot's first turn did not leave exactly one session folder (got %r)." % [f.name for f in folders])
    conv_id = folders[0].name
    for number in (1, 2):
        common.run_cli("Copilot turn %d" % (number + 1), argv + ["--resume=" + conv_id, "--prompt", prompts[number]], ws_a, env_a)
    folder = copilot_a / "session-state" / conv_id
    events_path = folder / "events.jsonl"
    original_hash = rc.sha256(events_path)
    lines = rc.jsonl_lines(events_path)
    records = [record_of(l) for l in lines]
    users = [i for i, r in enumerate(records) if r.get("type") == "user.message"]
    report.expect(len(users) == 3, "events.jsonl holds %d user.message events, expected 3." % len(users))
    report.observe("session_folder_files", rc.tree_listing(folder))
    report.observe("event_types", sorted({r.get("type", "<none>") for r in records}))
    report.observe("sessions_row", rc.sqlite_rows(copilot_a / "session-store.db", SESSIONS, (conv_id,)))
    report.fact("resume by id works (--resume=<id>)", "yes", {"turns 2 and 3 resumed the same session folder": True})

    # --- fork by hand before turn 2
    fork_id = str(uuid.uuid4())
    cut = users[1]
    while cut > 0 and records[cut - 1].get("type") in ("session.shutdown", "session.resume"):
        cut -= 1
    kept = [l.decode("utf-8").replace(conv_id, fork_id) for l in lines[:cut]]
    fork_folder = copilot_a / "session-state" / fork_id
    copy_with(folder, fork_folder, [(conv_id, fork_id)])
    (fork_folder / "events.jsonl").write_text("".join(kept))
    report.expect(not any(prompts[1] in l or prompts[2] in l for l in kept), "The hand fork still holds turn 2 or 3.")
    row_before = rc.sqlite_rows(copilot_a / "session-store.db", SESSIONS, (fork_id,))
    mark = server.mark()
    out, why = rc.try_run(report, "Copilot fork resume", argv + ["--resume=" + fork_id, "--prompt", prompts[3]], ws_a, env_a)
    seen = rc.history_seen(server.bodies_since(mark), prompts)
    ok = out is not None and seen == [True, False, False, True]
    report.fact("--resume takes a hand-copied session folder with a new id and no sessions-table row", "yes" if ok else "no", {"history_seen_p1_p2_p3_p4": seen, "error": why, "sessions_row_before": row_before})
    report.expect(ok, "Copilot did not resume the hand fork with only turn 1 as history (history seen: %r, error: %s)." % (seen, why))
    report.observe("sessions_row_of_fork_after_resume", rc.sqlite_rows(copilot_a / "session-store.db", SESSIONS, (fork_id,)))
    report.expect(rc.sha256(events_path) == original_hash, "Forking and resuming the fork changed the parent's events.jsonl.")

    # --- carry the original to folder B under another COPILOT_HOME
    pairs = rc.path_pairs(ws_a.project, ws_b.project, extra=[(copilot_a, copilot_b), (ws_a.home, ws_b.home)])
    copy_with(folder, copilot_b / "session-state" / conv_id, pairs)
    mark = server.mark()
    out, why = rc.try_run(report, "Copilot carry resume", argv + ["--resume=" + conv_id, "--prompt", prompts[3]], ws_b, env_b)
    bodies = server.bodies_since(mark)
    seen = rc.history_seen(bodies, prompts)
    ok = out is not None and seen == [True, True, True, True]
    report.fact("a carried session folder resumes in folder B under another COPILOT_HOME with all three turns", "yes" if ok else "no", {"history_seen_p1_p2_p3_p4": seen, "error": why})
    report.expect(ok, "Copilot did not resume the carried session in folder B with all three turns (history seen: %r, error: %s)." % (seen, why))
    needles = [str(ws_a.project), str(ws_a.home), str(ws_a.root)]
    naming = rc.files_naming(ws_b.home, needles)
    leaked = [n for n in needles if any(n in b for b in bodies)]
    report.fact("nothing under the new HOME or in the requests sent from B names folder A or A's HOME", "yes" if not any(naming.values()) and not leaked else "no", {"files_naming_a": {k: v[:5] for k, v in naming.items() if v}, "requests_naming_a": leaked})
    report.expect(not any(naming.values()) and not leaked, "After the carry something under folder B's HOME still names folder A (see the report's evidence).")
    report.observe("a_files_naming_a_project_in_original_folder", sorted(p.name for p in text_files(folder) if str(ws_a.project).encode() in p.read_bytes()))


if __name__ == "__main__":
    common.main(run)
