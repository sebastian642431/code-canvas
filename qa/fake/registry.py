"""Package registries that answer only what Code Canvas asks of them, on this computer.

python qa/fake/registry.py <port> <folder of the run>
A scenario gets them with a header line `registry: on` (the token of the private
npm registry is in QA_NPM_TOKEN) or `registry: no-token` (it is not). The app is
pointed at them with CODE_CANVAS_REGISTRY=http://127.0.0.1:<port>: every request
for https://<host>/<path> comes here as /<host>/<path>.

Each request is written to registry.log in the folder of the run, with whether it
carried an authorization (never the authorization itself). A package of the
private scope asked of the public npm registry, or of the private one in a run
without the token, writes leak.txt there: a scenario says
`absent-file: {WORK}/leak.txt`.
"""
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote

PORT = int(sys.argv[1])
WORK = sys.argv[2]
NO_TOKEN = len(sys.argv) > 3 and sys.argv[3] == "no-token"
SCOPE = "@latin-securities/"
TOKEN = "Bearer qa-npm-token"

NPM = {
    "react": (["17.0.2", "18.2.0", "18.2.5", "18.3.1", "19.0.0", "19.1.0", "19.2.0-rc.1"], "19.1.0"),
    "left-pad": (["1.0.0", "1.1.0", "1.3.0"], "1.3.0"),
    "vite": (["5.0.0", "5.0.12", "5.4.11", "6.0.0", "6.0.3"], "6.0.3"),
    "lodash": (["4.17.20", "4.17.21"], "4.17.21"),
}
PRIVATE = {SCOPE + "widgets": (["1.0.0", "1.2.0", "2.0.0"], "2.0.0")}
CRATES = {
    "serde": ["1.0.100", "1.0.228"],
    "log": ["0.3.9", "0.4.0", "0.4.22"],
    "anyhow": ["1.0.75", "1.0.93"],
}
PYPI = {"requests": ["2.28.1", "2.28.2", "2.31.0", "3.0.0b1"], "django": ["4.2.0", "4.2.7", "5.0.1"]}
NUGET = {"newtonsoft.json": ["12.0.3", "13.0.1", "13.0.3"]}
GO = {"github.com/spf13/cobra": ["v1.6.0", "v1.7.0", "v1.8.1"]}


def packument(name, table):
    versions, latest = table[name]
    return {"name": name, "dist-tags": {"latest": latest}, "versions": {v: {} for v in versions}}


class Fake(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def send(self, status, body, kind="application/json"):
        data = body.encode() if isinstance(body, str) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        path = unquote(self.path)
        authorized = self.headers.get("Authorization") is not None
        with open(os.path.join(WORK, "registry.log"), "a", encoding="utf8") as log:
            log.write(f"{path} authorized={authorized}\n")
        host, _, rest = path.lstrip("/").partition("/")
        if host == "registry.npmjs.org":
            if rest.startswith(SCOPE):
                with open(os.path.join(WORK, "leak.txt"), "a", encoding="utf8") as leak:
                    leak.write(rest + "\n")
                return self.send(404, {"error": "not found"})
            if rest in NPM:
                return self.send(200, packument(rest, NPM))
        elif host == "npm.pkg.github.com":
            if NO_TOKEN:
                with open(os.path.join(WORK, "leak.txt"), "a", encoding="utf8") as leak:
                    leak.write(rest + "\n")
            if self.headers.get("Authorization") != TOKEN:
                return self.send(401, {"error": "unauthorized"})
            if rest in PRIVATE:
                return self.send(200, packument(rest, PRIVATE))
        elif host == "index.crates.io":
            name = rest.rsplit("/", 1)[-1]
            if name in CRATES:
                lines = [json.dumps({"name": name, "vers": v, "yanked": False}) for v in CRATES[name]]
                return self.send(200, "\n".join(lines) + "\n", "text/plain")
        elif host == "pypi.org":
            name = rest.split("/")[1] if rest.count("/") >= 2 else ""
            if name in PYPI:
                releases = {v: [{"yanked": False}] for v in PYPI[name]}
                return self.send(200, {"info": {"version": PYPI[name][-1]}, "releases": releases})
        elif host == "api.nuget.org":
            name = rest.split("/")[1] if rest.count("/") >= 2 else ""
            if name in NUGET:
                return self.send(200, {"versions": NUGET[name]})
        elif host == "proxy.golang.org":
            module = rest.split("/@v/")[0]
            if module in GO:
                return self.send(200, "\n".join(GO[module]) + "\n", "text/plain")
        self.send(404, {"error": "not found"})


ThreadingHTTPServer(("127.0.0.1", PORT), Fake).serve_forever()
