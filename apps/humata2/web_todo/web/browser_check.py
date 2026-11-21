#!/usr/bin/env python3
"""Headless Chrome acceptance check for WEBTODO (D905).

Starts `smoke_server` serving the real compiled GPUI Wasm To-do bundle on
127.0.0.1:0, opens it in headless Chrome via CDP, verifies:
  1. GPUI initializes and renders a non-empty <canvas> (phase == 'running').
  2. The browser receives the initial live To-do board over WebSocket push.
  3. Triggering a live board update on the server pushes to the browser
     immediately without polling.
"""

import base64
import hashlib
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request


CHROME_CANDIDATES = [
    os.environ.get("CHROME_BIN", ""),
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    shutil.which("google-chrome") or "",
    shutil.which("chromium") or "",
]


def find_chrome():
    for path in CHROME_CANDIDATES:
        if path and os.path.isfile(path) and os.access(path, os.X_OK):
            return path
    raise RuntimeError("No Chrome/Chromium binary found for headless browser check")


class MinimalWebSocket:
    """Tiny RFC 6455 text-frame client using Python stdlib only (no pip deps)."""

    def __init__(self, ws_url: str):
        assert ws_url.startswith("ws://")
        rest = ws_url[len("ws://") :]
        host_port, _, path = rest.partition("/")
        path = "/" + path
        host, _, port_s = host_port.partition(":")
        port = int(port_s or "80")
        self.sock = socket.create_connection((host, port), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        req = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host_port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(req.encode("ascii"))
        resp = b""
        while b"\r\n\r\n" not in resp:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("CDP WebSocket handshake closed early")
            resp += chunk
        header, _, leftover = resp.partition(b"\r\n\r\n")
        if b"101" not in header.splitlines()[0]:
            raise RuntimeError(f"CDP WebSocket handshake failed: {header!r}")
        expected_accept = base64.b64encode(
            hashlib.sha1(
                (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode("ascii")
            ).digest()
        ).decode("ascii")
        if expected_accept.encode("ascii") not in header:
            raise RuntimeError("CDP WebSocket accept mismatch")
        self.buf = bytearray(leftover)

    def _recv_exact(self, n: int) -> bytes:
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("CDP WebSocket closed")
            self.buf.extend(chunk)
        out = bytes(self.buf[:n])
        del self.buf[:n]
        return out

    def send_text(self, text: str):
        payload = text.encode("utf-8")
        mask = os.urandom(4)
        header = bytearray([0x81])
        length = len(payload)
        if length < 126:
            header.append(0x80 | length)
        elif length < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack("!H", length))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack("!Q", length))
        header.extend(mask)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.sock.sendall(bytes(header) + masked)

    def recv_text(self) -> str:
        while True:
            b1, b2 = self._recv_exact(2)
            opcode = b1 & 0x0F
            masked = (b2 & 0x80) != 0
            length = b2 & 0x7F
            if length == 126:
                length = struct.unpack("!H", self._recv_exact(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._recv_exact(8))[0]
            mask = self._recv_exact(4) if masked else None
            data = self._recv_exact(length)
            if mask:
                data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
            if opcode == 0x8:
                raise RuntimeError("CDP WebSocket received close frame")
            if opcode == 0x9:
                # Pong
                continue
            if opcode == 0x1:
                return data.decode("utf-8")

    def close(self):
        try:
            self.sock.close()
        except Exception:
            pass


def cdp_eval(ws: MinimalWebSocket, req_id: int, expr: str):
    ws.send_text(
        json.dumps(
            {
                "id": req_id,
                "method": "Runtime.evaluate",
                "params": {"expression": expr, "returnByValue": True},
            }
        )
    )
    while True:
        msg = json.loads(ws.recv_text())
        if msg.get("id") == req_id:
            if "error" in msg:
                raise RuntimeError(f"CDP error: {msg['error']}")
            return msg.get("result", {}).get("result", {}).get("value")


def pick_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: browser_check.py <smoke_server_bin> <bundle_dir>")
    smoke_server_bin, bundle_dir = sys.argv[1], sys.argv[2]

    # Always verify the compiled GPUI Wasm bundle files exist and are valid.
    wasm_path = os.path.join(bundle_dir, "web_todo_bg.wasm")
    js_path = os.path.join(bundle_dir, "web_todo.js")
    html_path = os.path.join(bundle_dir, "index.html")
    for p in (wasm_path, js_path, html_path):
        assert os.path.isfile(p) and os.path.getsize(p) > 0, f"missing bundle file: {p}"
    with open(wasm_path, "rb") as f:
        assert f.read(4) == b"\x00asm", "invalid Wasm magic header"

    chrome_bin = find_chrome()
    server = subprocess.Popen(
        [smoke_server_bin, bundle_dir],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    user_data_dir = tempfile.mkdtemp(prefix="web_todo_chrome_")
    chrome = None
    ws = None
    try:
        url = server.stdout.readline().strip()
        assert url.startswith("http://127.0.0.1:"), f"unexpected server url: {url!r}"
        with urllib.request.urlopen(url + "health", timeout=3) as resp:
            assert resp.status == 200, f"health status {resp.status}"

        debug_port = pick_free_port()
        chrome = subprocess.Popen(
            [
                chrome_bin,
                "--headless=new",
                f"--remote-debugging-port={debug_port}",
                "--remote-allow-origins=*",
                f"--user-data-dir={user_data_dir}",
                "--no-first-run",
                "--no-default-browser-check",
                "--use-mock-keychain",
                "--password-store=basic",
                "--disable-gpu-sandbox",
                "--use-gl=angle",
                "--use-angle=swiftshader",
                "--enable-unsafe-swiftshader",
                "--window-size=1280,800",
                url,
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )

        # Wait for Chrome's CDP JSON endpoint to expose a page target.
        ws_url = None
        last_targets = None
        deadline = time.time() + 15.0
        while time.time() < deadline:
            try:
                with urllib.request.urlopen(
                    f"http://127.0.0.1:{debug_port}/json/list", timeout=1
                ) as resp:
                    targets = json.loads(resp.read().decode("utf-8"))
                last_targets = targets
                for t in targets:
                    if t.get("type") == "page" and t.get("webSocketDebuggerUrl"):
                        ws_url = t["webSocketDebuggerUrl"]
                        break
                if ws_url:
                    break
            except Exception:
                time.sleep(0.1)
        if not ws_url:
            raise RuntimeError(f"Chrome CDP target did not become ready; targets={last_targets}")

        ws = MinimalWebSocket(ws_url)
        req_id = 1
        # Wait briefly if Chrome is still navigating from about:blank, otherwise navigate explicitly.
        req_id += 1
        cur_href = cdp_eval(ws, req_id, "location.href")
        if cur_href == "about:blank":
            req_id += 1
            ws.send_text(
                json.dumps(
                    {
                        "id": req_id,
                        "method": "Page.navigate",
                        "params": {"url": url},
                    }
                )
            )
            while True:
                msg = json.loads(ws.recv_text())
                if msg.get("id") == req_id:
                    break

        # 1. Wait for GPUI Wasm to render a live <canvas> and receive the initial board snapshot.
        initial_state = None
        raw = None
        deadline = time.time() + 25.0
        while time.time() < deadline:
            req_id += 1
            raw = cdp_eval(
                ws,
                req_id,
                """(() => {
                    const c = document.querySelector('canvas');
                    const inspect = typeof window.webTodoInspect === 'function' ? JSON.parse(window.webTodoInspect()) : null;
                    return JSON.stringify({
                        href: location.href,
                        loading: document.getElementById('loading')?.textContent || null,
                        webTodo: window.webTodo || null,
                        canvas: c ? { width: c.width, height: c.height } : null,
                        inspect,
                    });
                })()""",
            )
            if raw:
                state = json.loads(raw)
                wt = state.get("webTodo") or {}
                if wt.get("phase") == "failed":
                    raise RuntimeError(f"webTodo failed in browser: {wt}")
                insp = state.get("inspect") or {}
                canvas = state.get("canvas") or {}
                if (
                    wt.get("phase") == "running"
                    and canvas.get("width", 0) > 1
                    and canvas.get("height", 0) > 1
                    and insp.get("connected") is True
                    and insp.get("item_count") == 2
                ):
                    initial_state = state
                    break
            time.sleep(0.15)

        assert initial_state is not None, f"Timed out waiting for GPUI canvas + live board; last={raw}"
        insp = initial_state["inspect"]
        assert insp["keys"] == ["PAY-1", "OPS-2"], f"unexpected keys: {insp}"
        assert insp["header"] == "2 open across 2 channels", f"unexpected header: {insp}"
        print(f"PASS initial load: canvas={initial_state['canvas']} inspect={insp}")

        # 2. Trigger a live push from the server (no polling!) and verify the browser updates.
        server.stdin.write("push\n")
        server.stdin.flush()

        pushed_state = None
        deadline = time.time() + 10.0
        while time.time() < deadline:
            req_id += 1
            raw = cdp_eval(
                ws,
                req_id,
                "typeof window.webTodoInspect === 'function' ? window.webTodoInspect() : ''",
            )
            if raw:
                insp2 = json.loads(raw)
                if insp2.get("item_count") == 3:
                    pushed_state = insp2
                    break
            time.sleep(0.1)

        assert pushed_state is not None, f"Timed out waiting for live push update; last={raw}"
        assert pushed_state["keys"] == ["PAY-1", "OPS-2", "PAY-3"], f"unexpected pushed keys: {pushed_state}"
        assert pushed_state["revision"] == 2, f"unexpected revision: {pushed_state}"
        print(f"PASS live push: inspect={pushed_state}")
    finally:
        if ws is not None:
            ws.close()
        if chrome is not None:
            chrome.terminate()
            try:
                chrome.wait(timeout=5)
            except subprocess.TimeoutExpired:
                chrome.kill()
        if server.stdin:
            try:
                server.stdin.write("quit\n")
                server.stdin.flush()
            except Exception:
                pass
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
        shutil.rmtree(user_data_dir, ignore_errors=True)


if __name__ == "__main__":
    main()
