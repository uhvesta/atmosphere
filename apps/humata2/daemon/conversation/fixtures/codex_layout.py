"""Installed Codex: where does it store a two-turn conversation?

Loopback Responses API, throwaway HOME and CODEX_HOME, project folder with a
space and a dot. Writes codex_layout.json plus sample testdata.
"""
import os
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common


def thread_id(records, stage):
    values = [r["thread_id"] for r in records if r.get("type") == "thread.started"]
    if len(values) != 1:
        raise common.LayoutFailure("%s did not print exactly one thread.started event." % stage)
    uuid.UUID(values[0])
    return values[0]


def run():
    binary = common.binary_or_skip("Codex", ["TERMINAL_CODEX_BIN", "CODEX_BIN"])
    prompts = common.new_prompts()
    with common.model_server() as server, common.workspace("codex") as ws:
        codex_home = ws.home / ".codex"
        codex_home.mkdir(mode=0o700)
        env = ws.env({"CODEX_HOME": str(codex_home)})
        version = common.run_cli("codex --version", [binary, "--version"], ws, env, timeout=20).strip()
        provider = 'model_providers.fixture={name="fixture",base_url="http://127.0.0.1:%d/v1",wire_api="responses",requires_openai_auth=false,supports_websockets=false,request_max_retries=0,stream_max_retries=0}' % server.server_port
        flags = ["-c", 'model_provider="fixture"', "-c", 'model="fixture-layout"', "-c", provider, "-c", "features.plugins=false"]
        first = common.json_lines(common.run_cli("Codex first turn", [binary] + flags + ["exec", "--json", "--skip-git-repo-check", prompts[0]], ws, env), "Codex first turn")
        identity = thread_id(first, "Codex first turn")
        second = common.json_lines(common.run_cli("Codex second turn", [binary] + flags + ["exec", "resume", "--json", "--skip-git-repo-check", identity, prompts[1]], ws, env), "Codex second turn")
        if thread_id(second, "Codex second turn") != identity:
            raise common.LayoutFailure("Resuming %s reported a different thread; the second turn did not continue the same conversation." % identity)
        common.finish("codex", version, ws, identity, prompts)


if __name__ == "__main__":
    common.main(run)
