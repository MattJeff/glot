"""Client of the glot render API (`POST /v1/render`). Standard library only, Python 3.9+.

    from glot_render import Glot
    glot = Glot(key=os.environ["GLOT_RENDER_KEY"], base_url="https://api.glotlang.com")
    r = glot.render(locale="fr", type="text", content="Your order {{id}} has shipped", variables={"id": "A-42"})
    send(r["content"])            # r["status"] == "fallback" means: still in the source language, see r["fallback_reason"]

Fail-open: when glot cannot be reached (network, timeout, 5xx) the source content is returned with simple
`{{name}}` variables filled and `status: "fallback"`. Refusals that need a fix (400, 401, 402, 429) raise GlotError.
"""
import html
import json
import re
import urllib.error
import urllib.request


class GlotError(Exception):
    def __init__(self, status, code, message):
        super().__init__(message)
        self.status, self.code = status, code


_TOKEN = re.compile(r"\{\{(\{?)\s*([\w.-]+)\s*\}?\}\}")


def fill_local(s, variables, escape=False):
    """Fill `{{name}}` with plain values (typed values need the server's locale data: they stay as tokens)."""

    def sub(m):
        v = variables.get(m.group(2))
        if v is None or isinstance(v, (dict, list)):
            return m.group(0)
        v = str(v).lower() if isinstance(v, bool) else str(v)
        return html.escape(v, quote=True) if escape and not m.group(1) else v

    return _TOKEN.sub(sub, s)


class Glot:
    def __init__(self, key, base_url, timeout=20.0, fail_open=True):
        self.key, self.base_url, self.timeout, self.fail_open = key, base_url.rstrip("/"), timeout, fail_open

    def _post(self, path, body, idempotency_key=None):
        headers = {"Authorization": f"Bearer {self.key}", "Content-Type": "application/json"}
        if idempotency_key:
            headers["Idempotency-Key"] = idempotency_key
        req = urllib.request.Request(self.base_url + path, json.dumps(body).encode(), headers, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=self.timeout + 2) as res:
                return json.load(res)
        except urllib.error.HTTPError as e:
            try:
                err = json.load(e).get("error", {})
            except ValueError:
                err = {}
            raise GlotError(e.code, err.get("code", "http_error"), err.get("message", e.reason)) from None

    def _fallback(self, item, reason):
        v, t, c = item.get("variables") or {}, item.get("type", "text"), item["content"]
        if t == "email":
            c = {k: fill_local(s, v, k == "html") if isinstance(s, str) else s for k, s in c.items()}
        elif t in ("text", "markdown", "html"):
            c = fill_local(c, v, t == "html")
        return {"id": item.get("id"), "locale": item["locale"], "type": t, "status": "fallback", "content": c,
                "cache": "none", "units": {"total": 0, "cached": 0, "translated": 0, "fallback": 0},
                "fallback_reason": reason, "warnings": []}

    def _open(self, e):
        if not self.fail_open or (isinstance(e, GlotError) and e.status < 500):
            raise e
        return f"client_fallback: glot was not reachable ({e})"

    def render(self, *, locale, content, type="text", variables=None, idempotency_key=None, **more):
        item = {"locale": locale, "type": type, "content": content, **more}
        if variables is not None:
            item["variables"] = variables
        try:
            return self._post("/v1/render", {**item, "timeout_ms": int(self.timeout * 1000)}, idempotency_key)
        except (GlotError, OSError) as e:
            return self._fallback(item, self._open(e))

    def render_batch(self, items, idempotency_key=None):
        try:
            body = {"items": items, "timeout_ms": int(self.timeout * 1000)}
            return self._post("/v1/render/batch", body, idempotency_key)["items"]
        except (GlotError, OSError) as e:
            reason = self._open(e)
            return [self._fallback(i, reason) for i in items]
