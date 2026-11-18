"""An Open VSX that answers only what Code Canvas asks of it, on this computer.

python qa/fake/openvsx.py <port>
A scenario gets one with a header line `openvsx: on`; the app is pointed at it with
CODE_CANVAS_OPEN_VSX=http://127.0.0.1:<port>/api. It knows three extensions:

- qa-tools.bru-lite: verified, rated, with an icon, a README with a picture beside it, a
  changelog, and things Code Canvas cannot do yet (a custom editor, keyboard shortcuts,
  editor menu entries).
- qa-tools.lint-plus: everything it brings works (commands, settings, a language).
- qa-tools.web-only: code only for VS Code in a browser, deprecated, no icon.

Each request is printed on stderr (the runner keeps it as openvsx.log).
"""
import io
import json
import struct
import sys
import zipfile
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

PORT = int(sys.argv[1])
API = f"http://127.0.0.1:{PORT}/api"


def png(size, back, front):
    """A square picture: `back` with a `front` square in the middle (RGB tuples)."""
    rows = []
    for y in range(size):
        row = bytearray([0])
        for x in range(size):
            inside = size // 4 <= x < 3 * size // 4 and size // 4 <= y < 3 * size // 4
            row.extend(front if inside else back)
        rows.append(bytes(row))
    raw = b"".join(rows)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    head = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", head) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


EXTENSIONS = {
    "bru-lite": {
        "displayName": "Bru Lite",
        "version": "1.2.0",
        "description": "Send API requests from .bru files and keep collections next to your code",
        "verified": True,
        "downloadCount": 82841,
        "averageRating": 4.5,
        "reviewCount": 12,
        "timestamp": "2026-09-16T19:24:15.326253Z",
        "first": "2024-02-03T10:00:00Z",
        "categories": ["Testing", "Programming Languages"],
        "tags": ["__ext_bru", "API Client", "REST", "HTTP", "GraphQL"],
        "license": "MIT",
        "repository": "https://github.com/qa-tools/bru-lite.git",
        "homepage": "https://example.com/bru-lite",
        "bugs": "https://github.com/qa-tools/bru-lite/issues",
        "icon": png(64, (40, 40, 48), (245, 180, 40)),
        "readme": "# Bru Lite\n\nSend API requests without leaving the editor.\n\n![The request editor](media/shot.png)\n\n"
        "## Features\n\n- **Git-friendly**: collections are plain text files\n- **Offline-first**: nothing in the cloud\n"
        "- Environments and secrets\n\n| Shortcut | What it does |\n|---|---|\n| `Ctrl+Enter` | Send the request |\n"
        "| `Ctrl+S` | Save it |\n\n```bru\nget {\n  url: https://example.com/users\n}\n```\n\n"
        "## Requirements\n\nNothing: it brings its own runtime.\n",
        "changelog": "# Changelog\n\n## 1.2.0\n\n- Collections open faster\n- GraphQL variables\n\n## 1.1.0\n\n"
        "- Environments\n",
        "manifest": {
            "main": "./dist/extension.js",
            "activationEvents": ["workspaceContains:**/bruno.json", "onLanguage:bru"],
            "contributes": {
                "commands": [
                    {"command": "bru.send", "title": "Send Request", "category": "Bru"},
                    {"command": "bru.newCollection", "title": "New Collection", "category": "Bru"},
                ],
                "customEditors": [{"viewType": "bru.requestEditor", "displayName": "Bru Request Editor",
                                   "selector": [{"filenamePattern": "*.bru"}]}],
                "keybindings": [{"command": "bru.send", "key": "ctrl+enter"}, {"command": "bru.save", "key": "ctrl+s"}],
                "languages": [{"id": "bru", "aliases": ["Bru Request"], "extensions": [".bru"]}],
                "grammars": [{"language": "bru", "scopeName": "source.bru", "path": "./syntaxes/bru.json"}],
                "viewsContainers": {"activitybar": [{"id": "bru", "title": "Bru", "icon": "media/side.svg"}]},
                "views": {"bru": [{"id": "bru.collections", "name": "Collections", "type": "webview"}]},
                "menus": {"editor/title": [{"command": "bru.send"}], "view/title": [{"command": "bru.newCollection"}]},
            },
        },
    },
    "lint-plus": {
        "displayName": "Lint Plus",
        "version": "3.0.1",
        "description": "Finds problems in plain text files as you write them",
        "verified": False,
        "downloadCount": 1520,
        "timestamp": "2025-05-01T08:00:00Z",
        "first": "2023-11-20T08:00:00Z",
        "categories": ["Linters"],
        "tags": ["lint"],
        "license": "Apache-2.0",
        "repository": "github:qa-tools/lint-plus",
        "icon": png(64, (30, 90, 160), (230, 240, 250)),
        "readme": "# Lint Plus\n\nIt underlines what looks wrong.\n",
        "changelog": "",
        "manifest": {
            "main": "./out/main.js",
            "activationEvents": ["onLanguage:plaintext"],
            "contributes": {
                "commands": [{"command": "lintPlus.fix", "title": "Fix All", "category": "Lint Plus"}],
                "configuration": {"properties": {
                    "lintPlus.enable": {"type": "boolean", "default": True, "description": "Lint as you type."},
                    "lintPlus.maxProblems": {"type": "number", "default": 100, "description": "At most this many."},
                }},
                "languages": [{"id": "plaintext", "extensions": [".txt"]}],
            },
        },
    },
    "web-only": {
        "displayName": "Web Only Helper",
        "version": "0.3.0",
        "description": "Helps in vscode.dev and github.dev",
        "verified": False,
        "deprecated": True,
        "downloadCount": 87,
        "timestamp": "2024-01-10T08:00:00Z",
        "first": "2024-01-10T08:00:00Z",
        "categories": ["Other"],
        "tags": [],
        "icon": None,
        "readme": "# Web Only Helper\n\nFor the browser.\n",
        "changelog": None,
        "manifest": {"browser": "./dist/web.js", "contributes": {"commands": [{"command": "web.go", "title": "Go"}]}},
    },
}
SHOT = png(320, (230, 232, 236), (90, 110, 200))


def base(name):
    e = EXTENSIONS[name]
    return f"{API}/qa-tools/{name}/{e['version']}"


def files(name):
    e = EXTENSIONS[name]
    out = {"download": f"{API}/vsix/qa-tools.{name}.vsix", "manifest": f"{base(name)}/file/package.json"}
    if e["icon"]:
        out["icon"] = f"{base(name)}/file/icon.png"
    if e["readme"] is not None:
        out["readme"] = f"{base(name)}/file/README.md"
    if e["changelog"] is not None:
        out["changelog"] = f"{base(name)}/file/CHANGELOG.md"
    if e.get("license"):
        out["license"] = f"{base(name)}/file/LICENSE.txt"
    return out


def summary(name):
    e = EXTENSIONS[name]
    out = {"namespace": "qa-tools", "name": name, "displayName": e["displayName"], "version": e["version"],
           "description": e["description"], "downloadCount": e["downloadCount"], "verified": e["verified"],
           "deprecated": e.get("deprecated", False), "timestamp": e["timestamp"], "files": files(name),
           "url": f"{API}/qa-tools/{name}"}
    if "averageRating" in e:
        out["averageRating"] = e["averageRating"]
        out["reviewCount"] = e["reviewCount"]
    return out


def details(name):
    e = EXTENSIONS[name]
    out = summary(name)
    out.update({"namespaceDisplayName": "QA Tools", "publishedBy": {"fullName": "QA Tools", "loginName": "qa"},
                "categories": e["categories"], "tags": e["tags"], "license": e.get("license", ""),
                "repository": e.get("repository", ""), "homepage": e.get("homepage", ""), "bugs": e.get("bugs", ""),
                "allVersions": {"latest": f"{API}/qa-tools/{name}/latest", e["version"]: base(name)},
                "downloadSizes": {"universal": 1_234_567}, "engines": {"vscode": "^1.80.0"},
                "downloads": {"universal": f"{API}/vsix/qa-tools.{name}.vsix"}})
    return out


def manifest(name):
    e = EXTENSIONS[name]
    out = {"publisher": "qa-tools", "name": name, "displayName": e["displayName"], "version": e["version"],
           "description": e["description"], "engines": {"vscode": "^1.80.0"}}
    out.update(json.loads(json.dumps(e["manifest"])))
    return out


def vsix(name):
    e = EXTENSIONS[name]
    data = io.BytesIO()
    with zipfile.ZipFile(data, "w") as z:
        package = manifest(name)
        package.pop("main", None)
        package.pop("browser", None)
        z.writestr("extension/package.json", json.dumps(package))
        if e["readme"] is not None:
            z.writestr("extension/README.md", e["readme"])
        if e["changelog"]:
            z.writestr("extension/CHANGELOG.md", e["changelog"])
    return data.getvalue()


class Fake(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def send(self, status, body, kind):
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def json(self, status, body):
        self.send(status, json.dumps(body).encode(), "application/json")

    def do_GET(self):
        url = urlparse(self.path)
        query = parse_qs(url.query)
        parts = [p for p in url.path.split("/") if p]
        print("GET", self.path, file=sys.stderr, flush=True)
        if parts[:1] != ["api"]:
            return self.json(404, {"error": "no such place"})
        parts = parts[1:]
        if parts == ["-", "search"]:
            words = (query.get("query") or [""])[0].lower()
            found = [summary(n) for n in EXTENSIONS if words in f"qa-tools.{n}"
                     or words in EXTENSIONS[n]["description"].lower() or words in EXTENSIONS[n]["displayName"].lower()]
            return self.json(200, {"offset": 0, "totalSize": len(found), "extensions": found})
        if len(parts) == 2 and parts[0] == "vsix":
            name = parts[1].removeprefix("qa-tools.").removesuffix(".vsix")
            if name in EXTENSIONS:
                return self.send(200, vsix(name), "application/octet-stream")
        if len(parts) < 2 or parts[0].lower() != "qa-tools" or parts[1].lower() not in EXTENSIONS:
            return self.json(404, {"error": f"Extension not found: {'.'.join(parts[:2])}"})
        name = parts[1].lower()
        e = EXTENSIONS[name]
        rest = parts[2:]
        if not rest:
            return self.json(200, details(name))
        if rest == ["latest"]:
            return self.json(200, details(name))
        if rest == ["versions"]:
            return self.json(200, {"offset": 0, "totalSize": 2, "versions": {"0.0.1": f"{API}/qa-tools/{name}/0.0.1"}})
        if rest == ["0.0.1"]:
            return self.json(200, dict(summary(name), version="0.0.1", timestamp=e["first"]))
        if len(rest) == 3 and rest[1] == "file":
            wanted = rest[2]
            if wanted == "package.json":
                return self.json(200, manifest(name))
            if wanted == "icon.png" and e["icon"]:
                return self.send(200, e["icon"], "image/png")
            if wanted == "README.md" and e["readme"] is not None:
                return self.send(200, e["readme"].encode(), "text/markdown")
            if wanted == "CHANGELOG.md" and e["changelog"] is not None:
                return self.send(200, e["changelog"].encode(), "text/markdown")
            if wanted == "LICENSE.txt":
                return self.send(200, b"License text", "text/plain")
        if len(rest) == 4 and rest[1] == "file" and rest[2:] == ["media", "shot.png"]:
            return self.send(200, SHOT, "image/png")
        self.json(404, {"error": "not found"})


ThreadingHTTPServer(("127.0.0.1", PORT), Fake).serve_forever()
