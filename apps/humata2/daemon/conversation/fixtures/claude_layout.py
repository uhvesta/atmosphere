"""Installed Claude Code: where does it store a two-turn conversation?

Loopback Messages API, throwaway HOME and CLAUDE_CONFIG_DIR, project folder
with a space and a dot. Writes claude_layout.json plus sample testdata.
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common


def run():
    binary = common.binary_or_skip("Claude Code", ["TERMINAL_CLAUDE_BIN", "CLAUDE_BIN"])
    prompts = common.new_prompts()
    with common.model_server() as server, common.workspace("claude") as ws:
        config = ws.home / ".claude"
        config.mkdir(mode=0o700)
        env = ws.env({
            "CLAUDE_CONFIG_DIR": str(config),
            "ANTHROPIC_BASE_URL": "http://127.0.0.1:%d" % server.server_port,
            "ANTHROPIC_API_KEY": "sk-ant-local-fixture-not-a-real-credential",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
            "DISABLE_AUTOUPDATER": "1",
        })
        version = common.run_cli("claude --version", [binary, "--version"], ws, env, timeout=20).strip()
        flags = ["-p", "--verbose", "--output-format", "stream-json", "--model", "fake", "--setting-sources", ""]
        first = common.json_lines(common.run_cli("Claude first turn", [binary] + flags + [prompts[0]], ws, env), "Claude first turn")
        ids = {r["session_id"] for r in first if r.get("session_id")}
        if len(ids) != 1:
            raise common.LayoutFailure("Claude's first turn did not report exactly one session_id (got %r)." % sorted(ids))
        identity = ids.pop()
        second = common.json_lines(common.run_cli("Claude second turn", [binary] + flags + ["--resume", identity, prompts[1]], ws, env), "Claude second turn")
        if {r["session_id"] for r in second if r.get("session_id")} != {identity}:
            raise common.LayoutFailure("Resuming %s reported a different session_id; the second turn did not continue the same conversation." % identity)
        common.finish("claude", version, ws, identity, prompts)


if __name__ == "__main__":
    common.main(run)
