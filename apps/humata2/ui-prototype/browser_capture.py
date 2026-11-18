#!/usr/bin/env python3
"""Capture the local Rust canvas with CDP; no desktop input or real profile."""
import argparse
import base64
import json
from pathlib import Path
import subprocess
import time
import urllib.request

try:
    import websocket
except ImportError:
    raise SystemExit("Install websocket-client in .tools/browser-qa, then run this script with that environment's Python.")

parser = argparse.ArgumentParser()
parser.add_argument("--url", default="http://127.0.0.1:8740/?revision=fast-release")
parser.add_argument("--output", default="target/ui-captures/browser-release.png")
parser.add_argument("--software", action="store_true")
args = parser.parse_args()
root = Path(__file__).resolve().parent
profile = root / "target/chrome-cdp"
profile.mkdir(parents=True, exist_ok=True)
port_file = profile / "DevToolsActivePort"
port_file.unlink(missing_ok=True)
command = ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "--headless=new", "--no-first-run", "--no-default-browser-check", "--disable-default-apps", "--disable-background-networking", "--disable-sync", "--remote-debugging-port=0", "--remote-allow-origins=*", f"--user-data-dir={profile}", "--window-size=1512,960", "about:blank"]
if args.software:
    command += ["--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader", "--disable-webgpu"]
errors = []
with (root / "target/browser-chrome.log").open("w") as log:
    process = subprocess.Popen(command, stdout=log, stderr=log)
    connection = None
    try:
        deadline = time.monotonic() + 30
        while not port_file.exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise SystemExit("Headless Chrome did not expose its local debugging port. Read target/browser-chrome.log.")
            time.sleep(0.1)
        port = port_file.read_text().splitlines()[0]
        targets = json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list"))
        target = next(target for target in targets if target["type"] == "page")
        connection = websocket.create_connection(target["webSocketDebuggerUrl"], timeout=20)
        number = 0
        def request(method, params=None):
            global number
            number += 1
            current = number
            connection.send(json.dumps({"id": current, "method": method, "params": params or {}}))
            while True:
                reply = json.loads(connection.recv())
                if reply.get("method") in ("Runtime.exceptionThrown", "Runtime.consoleAPICalled", "Log.entryAdded"):
                    if len(errors) < 100:
                        errors.append(reply)
                if reply.get("id") == current:
                    if "error" in reply:
                        raise RuntimeError(f"CDP {method} failed: {reply['error']}")
                    return reply.get("result", {})
        request("Runtime.enable")
        request("Log.enable")
        request("Page.enable")
        request("Network.enable")
        request("Network.setCacheDisabled", {"cacheDisabled": True})
        request("Emulation.setDeviceMetricsOverride", {"width": 1512, "height": 960, "deviceScaleFactor": 1, "mobile": False})
        request("Page.navigate", {"url": args.url})
        # Real elapsed time is intentional: virtual-time budgets can finish before
        # asynchronous Wasm compilation and GPU initialization paint a frame.
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            time.sleep(0.25)
            startup = request("Runtime.evaluate", {"expression": "window.__humataPrototype?.phase", "returnByValue": True})
            phase = startup.get("result", {}).get("value")
            if phase in ("running", "failed"):
                time.sleep(1)
                break
        state = request("Runtime.evaluate", {"expression": "JSON.stringify({url:location.href,text:document.body.innerText,canvases:[...document.querySelectorAll('canvas')].map(c=>({width:c.width,height:c.height})),fonts:document.fonts.status,startup:window.__humataPrototype})", "returnByValue": True})
        print(state.get("result", {}).get("value"))
        screenshot = request("Page.captureScreenshot", {"format": "png", "captureBeyondViewport": False})
        output = root / args.output
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(base64.b64decode(screenshot["data"]))
        diagnostic = output.with_suffix(".browser.json")
        diagnostic.write_text(json.dumps({"state": state, "events": errors}, indent=2))
        print(f"Screenshot: {output}\nDiagnostics: {diagnostic}")
        observed = json.loads(state.get("result", {}).get("value", "{}"))
        if observed.get("startup", {}).get("phase") != "running":
            raise SystemExit("The browser prototype did not start. Inspect the saved browser diagnostics.")

    finally:
        if connection:
            connection.close()
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
