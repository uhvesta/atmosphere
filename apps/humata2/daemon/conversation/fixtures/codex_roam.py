"""Installed Codex: fork by hand, carry to another HOME, rewrite in place (ROAM X2).

A three-turn conversation in folder A, a fork of its rollout before turn 2
made the way the conversation adapter makes one, a carry of the original to
folder B under another CODEX_HOME, and an in-place rewrite of the original to
a folder C of another length, with the thread row and the thread_history
projection checked before and after. Loopback Responses API only.
"""
import contextlib
import json
import os
from pathlib import Path
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common
import roam_common as rc

NEW_ROW = "SELECT id, cwd, rollout_path FROM threads WHERE id = ?"
TURNS = "SELECT turn_id, rollout_byte_offset, rollout_end_byte_offset FROM thread_turns WHERE thread_id = ? ORDER BY rollout_ordinal"
PROJECTION = "SELECT next_rollout_byte_offset, next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id = ?"


def environment(ws, server):
    home = ws.home / ".codex"
    home.mkdir(mode=0o700, exist_ok=True)
    env = ws.env({"CODEX_HOME": str(home)})
    provider = 'model_providers.fixture={name="fixture",base_url="http://127.0.0.1:%d/v1",wire_api="responses",requires_openai_auth=false,supports_websockets=false,request_max_retries=0,stream_max_retries=0}' % server.server_port
    flags = ["-c", 'model_provider="fixture"', "-c", 'model="fixture-layout"', "-c", provider, "-c", "features.plugins=false"]
    return env, home, flags


def thread_of(text):
    values = [r["thread_id"] for r in common.json_lines(text, "output") if r.get("type") == "thread.started"]
    return values[0] if len(values) == 1 else None


def record_of(line):
    try:
        value = json.loads(line)
    except json.JSONDecodeError:
        return {}
    return value if isinstance(value, dict) else {}


def rollout_of(codex_home, conv_id):
    found = sorted((codex_home / "sessions").glob("*/*/*/rollout-*-%s.jsonl" % conv_id))
    return found[0] if len(found) == 1 else None


def turn_ids(lines):
    """The turn ids of the rollout's UserMessage item_completed lines, in order."""
    ids = []
    for line in lines:
        payload = record_of(line).get("payload", {})
        if payload.get("type") == "item_completed" and payload.get("item", {}).get("type") == "UserMessage" and payload.get("turn_id") not in ids:
            ids.append(payload["turn_id"])
    return ids


def fork_lines(lines, turn, old_id, new_id):
    """The lines before the start of `turn`, every string equal to the old id set to the new one."""
    cut = next((i for i, line in enumerate(lines) if record_of(line).get("payload", {}).get("turn_id") == turn), None)
    if cut is None:
        raise common.LayoutFailure("The rollout has no line for turn %s, so it cannot be forked before it." % turn)
    out = []
    for line in lines[:cut]:
        value = rc.replace_in_strings(json.loads(line), old_id, new_id)
        out.append(json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode("utf-8") + b"\n")
    return out


def line_starts(data):
    starts = {0}
    for i, byte in enumerate(data):
        if byte == 10:
            starts.add(i + 1)
    return starts


def history_state(codex_home, conv_id, rollout):
    """thread_history's turn offsets and projection, judged against the rollout's real lines."""
    turns = rc.sqlite_rows(codex_home / "thread_history_1.sqlite", TURNS, (conv_id,))
    projection = rc.sqlite_rows(codex_home / "thread_history_1.sqlite", PROJECTION, (conv_id,))
    data = rollout.read_bytes() if rollout and rollout.is_file() else b""
    starts = line_starts(data)
    out = {"rollout_size": len(data), "turn_rows": turns, "projection": projection}
    if isinstance(turns, list):
        bad = []
        for turn_id, begin, end in turns:
            if begin is not None and begin not in starts:
                bad.append([turn_id, "begin", begin])
            if end is not None and end not in starts and end != len(data):
                bad.append([turn_id, "end", end])
        out["offsets_off_a_line_start"] = bad
    if isinstance(projection, list) and projection:
        out["projection_matches_size"] = projection[0][0] == len(data)
    return out


def run():
    binary = common.binary_or_skip("Codex", ["TERMINAL_CODEX_BIN", "CODEX_BIN"])
    prompts = rc.roam_prompts()
    report = rc.Report("codex")
    with contextlib.ExitStack() as stack:
        server = stack.enter_context(rc.recording_server())
        ws_a = rc.new_workspace(stack, "codex")
        ws_b = rc.new_workspace(stack, "codex")
        try:
            body(binary, prompts, report, server, ws_a, ws_b)
        finally:
            report.finish([("A", ws_a), ("B", ws_b)])


def body(binary, prompts, report, server, ws_a, ws_b):
    env_a, home_a, flags = environment(ws_a, server)
    env_b, home_b, _ = environment(ws_b, server)
    report.data["version"] = common.run_cli("codex --version", [binary, "--version"], ws_a, env_a, timeout=20).strip()
    exec_flags = ["--json", "--skip-git-repo-check"]

    def resume(ws, env, home_flags, conv, prompt, stage, cwd=None):
        return rc.try_run(report, stage, [binary] + home_flags + ["exec", "resume"] + exec_flags + [conv, prompt], ws, env, cwd=cwd)

    # --- three turns in folder A
    first = common.run_cli("Codex turn 1", [binary] + flags + ["exec"] + exec_flags + [prompts[0]], ws_a, env_a)
    conv_id = thread_of(first)
    if conv_id is None:
        raise common.LayoutFailure("Codex's first turn did not print exactly one thread.started event.")
    for number in (1, 2):
        out = common.run_cli("Codex turn %d" % (number + 1), [binary] + flags + ["exec", "resume"] + exec_flags + [conv_id, prompts[number]], ws_a, env_a)
        if thread_of(out) != conv_id:
            raise common.LayoutFailure("Resuming %s reported another thread, so turn %d did not continue the conversation." % (conv_id, number + 1))
    rollout = rollout_of(home_a, conv_id)
    if rollout is None:
        raise common.LayoutFailure("Codex did not leave exactly one sessions/<Y>/<M>/<D>/rollout-*-%s.jsonl." % conv_id)
    original = rollout.read_bytes()
    original_hash = rc.sha256(rollout)
    lines = rc.jsonl_lines(rollout)
    turns = turn_ids(lines)
    report.expect(len(turns) == 3, "The rollout has %d turns, expected 3." % len(turns))
    report.observe("thread_row_after_three_turns", rc.sqlite_rows(home_a / "state_5.sqlite", NEW_ROW, (conv_id,)))
    report.observe("archived_sessions_present", (home_a / "archived_sessions").exists())
    report.observe("shell_snapshots_files", sorted(str(p.relative_to(home_a)) for p in (home_a / "shell_snapshots").rglob("*") if p.is_file()) if (home_a / "shell_snapshots").is_dir() else [])
    report.fact("archived_sessions/ or shell_snapshots/ hold any of a conversation's own files", "no" if not (home_a / "archived_sessions").exists() and not report.data["observations"]["shell_snapshots_files"] else "yes", {"archived_sessions": (home_a / "archived_sessions").exists(), "shell_snapshots_files": report.data["observations"]["shell_snapshots_files"]})
    for key in ("tool-call, reasoning, compaction, rollback and interrupted-turn records and how they mark the current path", "a prompt added to a running turn (one turn_id, two UserMessages)"):
        report.fact(key, "not testable", "The loopback model answers plain text and exec runs one prompt per turn; no tool call, rollback or second prompt is possible.")

    # --- what the CLI's own `exec fork` writes
    mark_files = {p for p in (home_a / "sessions").rglob("rollout-*.jsonl")}
    out, why = rc.try_run(report, "Codex native fork", [binary] + flags + ["exec", "fork"] + exec_flags + [conv_id, prompts[3]], ws_a, env_a)
    new_files = sorted({p for p in (home_a / "sessions").rglob("rollout-*.jsonl")} - mark_files)
    if out is not None and len(new_files) == 1:
        native_lines = rc.jsonl_lines(new_files[0])
        meta = next((record_of(l).get("payload", {}) for l in native_lines if record_of(l).get("type") == "session_meta"), {})
        report.fact("exec fork writes a parent link into session_meta", "yes" if any("fork" in k or "parent" in k for k in meta) else "no", {"session_meta_keys": sorted(meta), "forked_from_in_any_line": any(conv_id in l.decode() and record_of(l).get("type") == "session_meta" and meta.get("id") != conv_id for l in native_lines), "new_file_name_time_differs_from_parent": new_files[0].name.split("-%s" % (meta.get("id") or ""))[0] != rollout.name.split("-%s" % conv_id)[0]})
    else:
        report.fact("exec fork writes a parent link into session_meta", "not testable", {"error": why, "new_files": len(new_files)})
    report.expect(rc.sha256(rollout) == original_hash, "`codex exec fork` changed the parent's rollout.")
    original = rollout.read_bytes()
    original_hash = rc.sha256(rollout)

    # --- fork by hand before turn 2, no row in state_5 and no projection
    fork_id = str(uuid.uuid4())
    forked = fork_lines(lines, turns[1], conv_id, fork_id)
    fork_path = rollout.with_name(rollout.name.replace(conv_id, fork_id))
    fork_path.write_bytes(b"".join(forked))
    row_before = rc.sqlite_rows(home_a / "state_5.sqlite", NEW_ROW, (fork_id,))
    mark = server.mark()
    out, why = resume(ws_a, env_a, flags, fork_id, prompts[3], "Codex fork resume")
    seen = rc.history_seen(server.bodies_since(mark), prompts)
    ok = out is not None and seen == [True, False, False, True] and thread_of(out) == fork_id
    report.fact("codex exec resume takes a hand fork's rollout that has no threads row and no projection", "yes" if ok else "no", {"row_before": row_before, "history_seen_p1_p2_p3_p4": seen, "error": why})
    report.expect(ok, "Codex did not resume the hand fork with only turn 1 as history (history seen: %r, error: %s)." % (seen, why))
    row_after = rc.sqlite_rows(home_a / "state_5.sqlite", NEW_ROW, (fork_id,))
    projected = rc.sqlite_rows(home_a / "thread_history_1.sqlite", TURNS, (fork_id,))
    report.fact("the CLI indexes a hand fork on resume (threads row)", "yes" if row_after else "no", {"row_after": row_after})
    report.fact("the CLI re-projects a rollout it had not indexed (thread_history turns)", "yes" if isinstance(projected, list) and len(projected) >= 2 else "no", {"projected_turn_rows": projected})
    report.fact("a fork's file name may keep the parent's time", "yes" if ok else "no", {"parent": rollout.name, "fork": fork_path.name})
    report.expect(rc.sha256(rollout) == original_hash, "Forking and resuming the fork changed the parent's rollout.")

    # --- carry the original to folder B under another CODEX_HOME
    rel = rollout.relative_to(home_a)
    pairs = rc.path_pairs(home_a, home_b) + rc.path_pairs(ws_a.project, ws_b.project)
    placed = home_b / rel
    placed.parent.mkdir(parents=True)
    placed.write_text(rc.rewrite_text(original.decode("utf-8"), pairs))
    mark = server.mark()
    out, why = resume(ws_b, env_b, flags, conv_id, prompts[3], "Codex carry resume")
    bodies = server.bodies_since(mark)
    seen = rc.history_seen(bodies, prompts)
    ok = out is not None and seen == [True, True, True, True] and thread_of(out) == conv_id
    report.fact("a carried rollout resumes in folder B under another CODEX_HOME with all three turns", "yes" if ok else "no", {"history_seen_p1_p2_p3_p4": seen, "error": why, "b_had_no_state_db": True})
    report.expect(ok, "Codex did not resume the carried rollout in folder B with all three turns (history seen: %r, error: %s)." % (seen, why))
    row_b = rc.sqlite_rows(home_b / "state_5.sqlite", NEW_ROW, (conv_id,))
    report.fact("a placed carry whose rollout_path the db did not hold is indexed on resume", "yes" if row_b else "no", {"row": row_b})
    needles = [str(ws_a.project), str(ws_a.home), str(ws_a.root), str(home_a)]
    naming = rc.files_naming(ws_b.home, needles)
    leaked = [n for n in needles if any(n in b for b in bodies)]
    report.fact("nothing under the new HOME or in the requests sent from B names folder A, A's CODEX_HOME or A's HOME", "yes" if not any(naming.values()) and not leaked else "no", {"files_naming_a": {k: v[:5] for k, v in naming.items() if v}, "requests_naming_a": leaked})
    report.expect(not any(naming.values()) and not leaked, "After the carry something under folder B's HOME still names folder A (see the report's evidence).")

    # --- in-place rewrite of the original to a folder of another length
    folder_c = ws_a.root / "moved" / "a much longer folder name for the rewrite"
    folder_c.mkdir(parents=True)
    report.expect(len(str(folder_c)) != len(str(ws_a.project)), "Folder C must differ in length from folder A to move line lengths.")
    before = history_state(home_a, conv_id, rollout)
    report.observe("thread_history_before_rewrite", before)
    row_start = rc.sqlite_rows(home_a / "state_5.sqlite", NEW_ROW, (conv_id,))
    rewritten = rc.rewrite_text(original.decode("utf-8"), rc.path_pairs(ws_a.project, folder_c))
    rollout.write_text(rewritten)
    import sqlite3
    state = home_a / "state_5.sqlite"
    updated = None
    if state.is_file():
        connection = sqlite3.connect(str(state))
        try:
            with connection:
                updated = connection.execute("UPDATE threads SET cwd = ?, rollout_path = ? WHERE id = ?", (str(folder_c), str(rollout), conv_id)).rowcount
        finally:
            connection.close()
    after_rewrite = history_state(home_a, conv_id, rollout)
    report.observe("thread_history_after_rewrite_before_resume", after_rewrite)
    stale = bool(after_rewrite.get("offsets_off_a_line_start")) or after_rewrite.get("projection_matches_size") is False
    report.fact("thread_history's byte offsets go stale after a rewrite that changes a line's length", "yes" if stale else "no", {"before": {k: before.get(k) for k in ("offsets_off_a_line_start", "projection_matches_size")}, "after": {k: after_rewrite.get(k) for k in ("offsets_off_a_line_start", "projection_matches_size", "rollout_size")}, "rollout_size_before": before["rollout_size"], "row_updated": updated})
    mark = server.mark()
    out, why = resume(ws_a, env_a, flags, conv_id, prompts[3], "Codex rewritten resume", cwd=folder_c)
    bodies = server.bodies_since(mark)
    seen = rc.history_seen(bodies, prompts)
    ok = out is not None and seen == [True, True, True, True] and thread_of(out) == conv_id
    report.fact("codex exec resume works on a rewritten rollout with its thread row updated and thread_history untouched", "yes" if ok else "no", {"history_seen_p1_p2_p3_p4": seen, "error": why, "offsets_were_stale": stale})
    report.expect(ok, "Codex did not resume the rewritten rollout with all three turns (history seen: %r, error: %s)." % (seen, why))
    report.expect(not any(str(ws_a.project) in b for b in bodies), "A request after the in-place rewrite still named the old folder.")
    final = history_state(home_a, conv_id, rollout)
    report.observe("thread_history_after_resume", final)
    report.fact("the CLI repairs thread_history by itself after a resume of a rewritten rollout", "yes" if not final.get("offsets_off_a_line_start") and final.get("projection_matches_size") is not False else "no", {k: final.get(k) for k in ("offsets_off_a_line_start", "projection_matches_size", "rollout_size")})
    row_end = rc.sqlite_rows(state, NEW_ROW, (conv_id,))
    report.fact("the thread row's cwd follows the rewrite and the CLI keeps it", "yes" if isinstance(row_end, list) and row_end and row_end[0][1] == str(folder_c) else "no", {"row_before": row_start, "row_after_resume": row_end})


if __name__ == "__main__":
    common.main(run)
