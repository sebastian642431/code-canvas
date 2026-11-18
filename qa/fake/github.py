"""A GitHub that answers only what Code Canvas asks of it, on this computer.

python qa/fake/github.py <port> [<files of a gist to start with, as JSON>]
A scenario gets one with a header line `github: empty` or `github: <file in qa/fake>`.
Point the app at it with CODE_CANVAS_GITHUB_SITE and CODE_CANVAS_GITHUB_API
(http://127.0.0.1:<port>) and CODE_CANVAS_GITHUB_CLIENT=fake.
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
GISTS = {}
ASKED = {"token": 0}
if len(sys.argv) > 2:
    GISTS["g1"] = {"id": "g1", "files": json.load(open(sys.argv[2], encoding="utf8"))}


def gist_out(gist):
    files = {name: {"filename": name, "content": text, "truncated": False} for name, text in gist["files"].items()}
    return {"id": gist["id"], "public": False, "files": files}


class Fake(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def answer(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def body(self):
        size = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(size).decode() if size else ""

    def signed(self):
        return self.headers.get("Authorization") == "Bearer fake-token"

    def do_POST(self):
        text = self.body()
        if self.path == "/login/device/code":
            return self.answer(200, {"device_code": "dev", "user_code": "FAKE-1234",
                                     "verification_uri": f"http://127.0.0.1:{PORT}/login/device",
                                     "expires_in": 900, "interval": 1})
        if self.path == "/login/oauth/access_token":
            ASKED["token"] += 1
            if ASKED["token"] < 3:
                return self.answer(200, {"error": "authorization_pending"})
            return self.answer(200, {"access_token": "fake-token", "token_type": "bearer", "scope": "gist"})
        if self.path == "/gists" and self.signed():
            sent = json.loads(text)
            gist = {"id": f"g{len(GISTS) + 1}", "files": {n: f["content"] for n, f in sent["files"].items()}}
            GISTS[gist["id"]] = gist
            dump()
            return self.answer(201, gist_out(gist))
        self.answer(401, {"message": "Bad credentials"})

    def do_PATCH(self):
        text = self.body()
        gid = self.path.rsplit("/", 1)[-1]
        if self.signed() and gid in GISTS:
            sent = json.loads(text)
            GISTS[gid]["files"].update({n: f["content"] for n, f in sent["files"].items()})
            dump()
            return self.answer(200, gist_out(GISTS[gid]))
        self.answer(404, {"message": "Not Found"})

    def do_GET(self):
        if not self.signed():
            return self.answer(401, {"message": "Bad credentials"})
        if self.path == "/user":
            return self.answer(200, {"login": "tester"})
        if self.path.startswith("/gists?"):
            page = 1 if "page=1" in self.path else 2
            return self.answer(200, [gist_out(g) for g in GISTS.values()] if page == 1 else [])
        gid = self.path.rsplit("/", 1)[-1]
        if gid in GISTS:
            return self.answer(200, gist_out(GISTS[gid]))
        self.answer(404, {"message": "Not Found"})


def dump():
    pass


ThreadingHTTPServer(("127.0.0.1", PORT), Fake).serve_forever()
