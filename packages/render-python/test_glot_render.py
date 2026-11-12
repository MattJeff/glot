"""python3 test_glot_render.py — a local HTTP server stands in for glot."""
import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

from glot_render import Glot, GlotError

seen, answer = [], {"status": 200, "body": {"status": "translated", "content": "Bonjour Zoë"}}


class H(BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        seen.append((self.path, dict(self.headers), body))
        out = json.dumps(answer["body"]).encode()
        self.send_response(answer["status"])
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def log_message(self, *a):
        pass


srv = HTTPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()
glot = Glot("glot_rk_test", f"http://127.0.0.1:{srv.server_port}/", timeout=2)

r = glot.render(locale="fr", content="Hello {{name}}", variables={"name": "Zoë"}, idempotency_key="k1")
assert r["content"] == "Bonjour Zoë", r
path, headers, body = seen[-1]
assert path == "/v1/render" and headers["Authorization"] == "Bearer glot_rk_test" and headers["Idempotency-Key"] == "k1"
assert body["variables"] == {"name": "Zoë"} and body["type"] == "text"

answer.update(status=503, body={})
r = glot.render(locale="fr", type="html", content="<p>Hi {{name}} {{{raw}}} {{later}}</p>", variables={"name": "A<b>", "raw": "<i>x</i>"})
assert r["status"] == "fallback" and r["content"] == "<p>Hi A&lt;b&gt; <i>x</i> {{later}}</p>", r
assert r["fallback_reason"].startswith("client_fallback:")
assert [i["status"] for i in glot.render_batch([{"locale": "de", "content": "x"}])] == ["fallback"]

answer.update(status=402, body={"error": {"code": "quota_exceeded", "message": "quota reached"}})
try:
    glot.render(locale="fr", content="x")
    raise SystemExit("expected GlotError")
except GlotError as e:
    assert (e.status, e.code, str(e)) == (402, "quota_exceeded", "quota reached")

down = Glot("k", "http://127.0.0.1:1", timeout=1)
assert down.render(locale="fr", content="Hi {{n}}", variables={"n": 1})["content"] == "Hi 1"
srv.shutdown()
print("ok")
