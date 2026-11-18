"""Installed Claude Code: fork by hand, carry to another HOME, resume (ROAM X2).

A three-turn conversation in folder A, a fork of it before turn 2 made the way
the conversation adapter makes one, and a carry of the original to folder B
under another HOME and CLAUDE_CONFIG_DIR. The report answers the facts the
adapter lists as unproven. Loopback Messages API only.
"""
import contextlib
import json
import os
from pathlib import Path
import shutil
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common
import roam_common as rc

PATH_KINDS = ("user", "assistant", "system", "attachment")
FLAGS = ["-p", "--verbose", "--output-format", "stream-json", "--model", "fake", "--setting-sources", ""]


def environment(ws, server):
    config = ws.home / ".claude"
    config.mkdir(mode=0o700, exist_ok=True)
    env = ws.env({
        "CLAUDE_CONFIG_DIR": str(config),
        "ANTHROPIC_BASE_URL": "http://127.0.0.1:%d" % server.server_port,
        "ANTHROPIC_API_KEY": "sk-ant-local-fixture-not-a-real-credential",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
        "DISABLE_AUTOUPDATER": "1",
    })
    return env, config


def session_ids(text):
    return {r["session_id"] for r in common.json_lines(text, "output") if r.get("session_id")}


def record_of(line):
    try:
        value = json.loads(line)
    except json.JSONDecodeError:
        return {}
    return value if isinstance(value, dict) else {}


def fork_lines(lines, prompt_two, old_id, new_id, only_path_records):
    """The records before the turn whose prompt is `prompt_two`, with the id swapped."""
    cut = None
    for index, line in enumerate(lines):
        record = record_of(line)
        if record.get("type") == "user" and prompt_two in json.dumps(record.get("message", {})):
            cut = index
            break
    if cut is None:
        raise common.LayoutFailure("The Claude transcript has no user record holding turn 2's prompt, so it cannot be forked before turn 2.")
    kept = lines[:cut]
    last_path = max((i for i, line in enumerate(kept) if record_of(line).get("type") in PATH_KINDS), default=-1)
    out = []
    for index, line in enumerate(kept):
        kind = record_of(line).get("type")
        if only_path_records and kind not in PATH_KINDS:
            continue
        if kind == "queue-operation" and index > last_path:
            continue
        out.append(line.decode("utf-8").replace(old_id, new_id).encode("utf-8"))
    return out


def conversation_files(config, folder_dashed, conv_id, slug):
    """(path relative to the config dir) of every file this conversation owns."""
    found = [Path("projects") / folder_dashed / (conv_id + ".jsonl")]
    bases = [Path("projects") / folder_dashed / conv_id, Path("file-history") / conv_id]
    for base in bases:
        if (config / base).is_dir():
            found += sorted(p.relative_to(config) for p in (config / base).rglob("*") if p.is_file())
    todos = config / "todos"
    if todos.is_dir():
        found += sorted(p.relative_to(config) for p in todos.glob(conv_id + "-agent-*.json"))
    if slug and (config / "plans" / (slug + ".md")).is_file():
        found.append(Path("plans") / (slug + ".md"))
    return found


def run():
    binary = common.binary_or_skip("Claude Code", ["TERMINAL_CLAUDE_BIN", "CLAUDE_BIN"])
    prompts = rc.roam_prompts()
    report = rc.Report("claude")
    with contextlib.ExitStack() as stack:
        server = stack.enter_context(rc.recording_server())
        ws_a = rc.new_workspace(stack, "claude")
        ws_b = rc.new_workspace(stack, "claude")
        try:
            body(binary, prompts, report, server, ws_a, ws_b)
        finally:
            report.finish([("A", ws_a), ("B", ws_b)])


def body(binary, prompts, report, server, ws_a, ws_b):
    env_a, config_a = environment(ws_a, server)
    env_b, config_b = environment(ws_b, server)
    report.data["version"] = common.run_cli("claude --version", [binary, "--version"], ws_a, env_a, timeout=20).strip()

    # --- three turns in folder A
    conv_id = None
    for number in range(3):
        argv = [binary] + FLAGS + (["--resume", conv_id] if number else []) + [prompts[number]]
        out = common.run_cli("Claude turn %d" % (number + 1), argv, ws_a, env_a)
        ids = session_ids(out)
        if len(ids) != 1:
            raise common.LayoutFailure("Claude turn %d did not report exactly one session_id (got %r)." % (number + 1, sorted(ids)))
        if number == 0:
            conv_id = next(iter(ids))
        elif ids != {conv_id}:
            raise common.LayoutFailure("Resuming %s reported another session_id, so turn %d did not continue the conversation." % (conv_id, number + 1))
    folder_dashed = rc.dashed(ws_a.project)
    transcript = config_a / "projects" / folder_dashed / (conv_id + ".jsonl")
    on_disk = sorted(p.name for p in (config_a / "projects").glob("*"))
    report.fact("dashed folder rule (every non-alphanumeric to a dash)", "yes" if transcript.is_file() else "no", {"expected": folder_dashed, "found_under_projects": on_disk})
    if not transcript.is_file():
        raise common.LayoutFailure("Claude did not store the conversation at projects/<dashed folder>/<id>.jsonl; see the report's evidence for what is under projects/.")
    original_lines = rc.jsonl_lines(transcript)
    original_hash = rc.sha256(transcript)
    records = [record_of(line) for line in original_lines]
    report.observe("record_types", sorted({r.get("type", "<none>") for r in records}))
    report.observe("config_tree_top_level", sorted(p.name for p in config_a.iterdir()))

    # --- facts about files the loopback model cannot make the CLI write
    slug = next((r["slug"] for r in records if isinstance(r.get("slug"), str)), None)
    owned = conversation_files(config_a, folder_dashed, conv_id, slug)
    report.observe("owned_files", [str(p) for p in owned])
    report.fact("transcript records carry a slug field", "yes" if slug else "no", {"slug_seen": bool(slug), "note": "a plain text -p run; a plan-mode run may write one"})
    todos = sorted(p.name for p in (config_a / "todos").glob("*")) if (config_a / "todos").is_dir() else []
    plans = sorted(p.name for p in (config_a / "plans").glob("*")) if (config_a / "plans").is_dir() else []
    if slug and plans:
        report.fact("plans/<slug>.md is the plan file's name", "yes" if (slug + ".md") in plans else "no", {"slug": bool(slug), "plans": plans})
    else:
        report.fact("plans/<slug>.md is the plan file's name", "not testable", "The loopback model makes no plan-mode tool call, so the CLI wrote no plans/ file.")
    if todos:
        report.fact("todos/<id>-agent-*.json is a todo file's name", "yes" if any(t.startswith(conv_id + "-agent-") for t in todos) else "no", {"todos": todos})
    else:
        report.fact("todos/<id>-agent-*.json is a todo file's name", "not testable", "The loopback model makes no TodoWrite call, so the CLI wrote no todos/ file.")
    uuids = [r["uuid"] for r in records if isinstance(r.get("uuid"), str)]
    report.fact("no file holds two records with one uuid", "yes" if len(uuids) == len(set(uuids)) else "no", {"records_with_uuid": len(uuids), "distinct": len(set(uuids))})
    last_assistant = [r for r in records if r.get("type") == "assistant"][-1:]
    stop = last_assistant[0].get("message", {}).get("stop_reason") if last_assistant else None
    report.fact("finished-turn rule holds (last assistant record has stop_reason end_turn)", "yes" if stop == "end_turn" else "no", {"stop_reason": stop})
    for key in ("rewind and edited-prompt branching through parentUuid", "compaction branching through logicalParentUuid", "inline side conversations and tool-call records in KINDS and FIELDS"):
        report.fact(key, "not testable", "Needs a rewind, a compaction or a tool call; the loopback model answers plain text and the fixture runs with -p only.")

    # --- fork by hand before turn 2 (the full record set), then resume it
    fork_id = str(uuid.uuid4())
    fork_lines_full = fork_lines(original_lines, prompts[1], conv_id, fork_id, only_path_records=False)
    fork_path = transcript.with_name(fork_id + ".jsonl")
    fork_path.write_bytes(b"".join(fork_lines_full))
    for base in (Path("projects") / folder_dashed / conv_id, Path("file-history") / conv_id):
        if (config_a / base).is_dir():
            shutil.copytree(config_a / base, config_a / str(base).replace(conv_id, fork_id))
    for p in (config_a / "todos").glob(conv_id + "-agent-*.json") if (config_a / "todos").is_dir() else []:
        shutil.copyfile(p, p.with_name(p.name.replace(conv_id, fork_id)))
    report.expect(not any(prompts[1].encode() in l or prompts[2].encode() in l for l in fork_lines_full), "The hand fork still holds turn 2 or 3.")
    mark = server.mark()
    out, why = rc.try_run(report, "Claude fork resume", [binary] + FLAGS + ["--resume", fork_id, prompts[3]], ws_a, env_a)
    seen = rc.history_seen(server.bodies_since(mark), prompts)
    ok = out is not None and seen == [True, False, False, True]
    report.fact("--resume takes a file whose sessionIds were rewritten, and resumes the current path", "yes" if ok else "no", {"history_seen_p1_p2_p3_p4": seen, "error": why})
    report.expect(ok, "Claude did not resume the hand fork with only turn 1 as history (history seen: %r, error: %s)." % (seen, why))
    if out is not None:
        report.fact("resume of a fork keeps the fork's new id", "yes" if session_ids(out) == {fork_id} else "no", {"reported": sorted(session_ids(out))})
        fork_text = fork_path.read_text()
        report.expect(prompts[0] in fork_text and prompts[3] in fork_text and prompts[1] not in fork_text, "The fork's transcript after its resume should hold turns 1 and 4 only.")
    report.expect(rc.sha256(transcript) == original_hash, "Forking and resuming the fork changed the parent's transcript.")

    # --- a fork with only the path records: are the session records needed?
    minimal_id = str(uuid.uuid4())
    minimal = fork_lines(original_lines, prompts[1], conv_id, minimal_id, only_path_records=True)
    transcript.with_name(minimal_id + ".jsonl").write_bytes(b"".join(minimal))
    mark = server.mark()
    out, why = rc.try_run(report, "Claude minimal fork resume", [binary] + FLAGS + ["--resume", minimal_id, prompts[3]], ws_a, env_a)
    seen = rc.history_seen(server.bodies_since(mark), prompts)
    needed = not (out is not None and seen == [True, False, False, True])
    report.fact("--resume needs the session records (queue-operation, api-request, cost-state, mode, last-prompt, snapshots)", "yes" if needed else "no", {"resumed_with_path_records_only": not needed, "history_seen": seen, "error": why})

    # --- carry the original to folder B under another HOME and config dir
    slug_value = slug
    files = conversation_files(config_a, folder_dashed, conv_id, slug_value)
    folder_b_dashed = rc.dashed(ws_b.project)
    pairs = []
    for old, new in [(config_a / "projects" / folder_dashed, config_b / "projects" / folder_b_dashed)]:
        pairs += rc.path_pairs(old, new)
    pairs += rc.path_pairs(ws_a.project, ws_b.project)
    pairs += [(folder_dashed, folder_b_dashed)]
    pairs += rc.path_pairs(config_a, config_b, extra=[(ws_a.home, ws_b.home)])
    for rel in files:
        target_rel = Path(str(rel).replace(folder_dashed, folder_b_dashed))
        target = config_b / target_rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(rc.rewrite_text((config_a / rel).read_text(), pairs))
    placed = config_b / "projects" / folder_b_dashed / (conv_id + ".jsonl")
    mark = server.mark()
    out, why = rc.try_run(report, "Claude carry resume", [binary] + FLAGS + ["--resume", conv_id, prompts[3]], ws_b, env_b)
    bodies = server.bodies_since(mark)
    seen = rc.history_seen(bodies, prompts)
    ok = out is not None and seen == [True, True, True, True] and session_ids(out) == {conv_id}
    report.fact("a carried conversation resumes in folder B under another HOME with all three turns", "yes" if ok else "no", {"history_seen_p1_p2_p3_p4": seen, "error": why, "placed": placed.is_file()})
    report.expect(ok, "Claude did not resume the carried conversation in folder B with all three turns (history seen: %r, error: %s)." % (seen, why))
    needles = [str(ws_a.project), folder_dashed, str(ws_a.home), str(ws_a.root)]
    naming = rc.files_naming(ws_b.home, needles)
    leaked_bodies = [n for n in needles if any(n in b for b in bodies)]
    report.fact("nothing under the new HOME or in the requests sent from B names folder A, A's dashed folder or A's HOME", "yes" if not any(naming.values()) and not leaked_bodies else "no", {"files_naming_a": {k: v[:5] for k, v in naming.items() if v}, "requests_naming_a": leaked_bodies})
    report.expect(not any(naming.values()) and not leaked_bodies, "After the carry something under folder B's HOME still names folder A (see the report's evidence).")

    # --- a folder name too long for one dashed directory name
    long_folder = ws_a.root / "long" / ("x" * 100) / ("y" * 100) / ("z" * 100)
    long_folder.mkdir(parents=True)
    expected = rc.dashed(os.path.realpath(long_folder))
    before = {p.name for p in (config_a / "projects").glob("*")}
    out, why = rc.try_run(report, "Claude long folder turn", [binary] + FLAGS + [prompts[0]], ws_a, env_a, cwd=long_folder)
    created = sorted({p.name for p in (config_a / "projects").glob("*")} - before)
    report.fact("a very long folder is dashed whole, not shortened or hashed", "yes" if expected in created else "no", {"expected_length": len(expected), "created_lengths": [len(c) for c in created], "created_ends_with_hash_suffix": [c[-7:] for c in created], "error": why})


if __name__ == "__main__":
    common.main(run)
