"""Serves the Humata 2 UX prototype on 127.0.0.1 and keeps its feedback in feedback.json.

Run: python3 -I apps/humata2/prototype/serve.py  (then open http://127.0.0.1:8737)
"""

import http.server
import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
STORE = HERE / "feedback.json"
PORT = 8737


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(HERE), **kwargs)

    def _json(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/api/load":
            body = json.loads(STORE.read_text()) if STORE.exists() else {}
            return self._json(200, body)
        return super().do_GET()

    def do_POST(self):
        if self.path != "/api/save":
            return self._json(404, {"error": "unknown path"})
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        STORE.write_text(json.dumps(body, indent=2))
        return self._json(200, {"ok": True})

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    print(f"Humata 2 prototype on http://127.0.0.1:{PORT}  (feedback -> {STORE})")
    http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
