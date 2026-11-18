"""Installed Copilot CLI: where does it store a two-turn conversation?

Offline, loopback Chat Completions, throwaway HOME, COPILOT_HOME and
COPILOT_CACHE_HOME, project folder with a space and a dot. Writes
copilot_layout.json plus sample testdata.
"""
import re
import os
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common


def session_ids(records):
    found = set()

    def visit(value):
        if isinstance(value, dict):
            if value.get("type") == "session.start":
                data = value.get("data", value)
                identity = data.get("sessionId") if isinstance(data, dict) else None
                if isinstance(identity, str):
                    uuid.UUID(identity)
                    found.add(identity)
            for nested in value.values():
                visit(nested)
        elif isinstance(value, list):
            for nested in value:
                visit(nested)

    visit(records)
    return found


def run():
    binary = common.binary_or_skip("Copilot", ["TERMINAL_COPILOT_BIN", "COPILOT_BIN"])
    prompts = common.new_prompts()
    with common.model_server() as server, common.workspace("copilot") as ws:
        copilot = ws.home / ".copilot"
        cache = ws.home / ".cache" / "copilot"
        for folder in (copilot, cache):
            folder.mkdir(mode=0o700, parents=True)
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
        version = common.run_cli("copilot --version", [binary, "--no-auto-update", "--version"], ws, env, timeout=30).strip()
        flags = [binary, "--no-auto-update", "--disable-client-telemetry", "--disable-builtin-mcps", "--no-custom-instructions", "--no-bash-env", "--no-ask-user", "--sync-session=off", "--output-format=json", "--stream=off", "--log-level=error"]
        first = common.json_lines(common.run_cli("Copilot first turn", flags + ["--prompt", prompts[0]], ws, env), "Copilot first turn")
        # Only UUID-named folders are sessions; Copilot also keeps entries such as
        # ".session-operation-locks" in session-state/.
        uuid_name = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
        stored = {p.name for p in (copilot / "session-state").glob("*") if p.is_dir() and uuid_name.match(p.name)} if (copilot / "session-state").is_dir() else set()
        candidates = session_ids(first) | stored
        if len(candidates) != 1:
            raise common.LayoutFailure("Copilot's first turn did not leave exactly one session id (got %r)." % sorted(candidates))
        identity = candidates.pop()
        uuid.UUID(identity)
        second = common.json_lines(common.run_cli("Copilot second turn", flags + ["--resume=" + identity, "--prompt", prompts[1]], ws, env), "Copilot second turn")
        if session_ids(second) - {identity}:
            raise common.LayoutFailure("Resuming %s started a different session; the second turn did not continue the same conversation." % identity)
        common.finish("copilot", version, ws, identity, prompts)


if __name__ == "__main__":
    common.main(run)
