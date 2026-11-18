import os


def write(root, rel, text):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf8", newline="") as out:
        out.write(text)


def trust_repo(root):
    """A folder as it comes from somewhere else: its .vscode/settings.json points an
    extension at a program of the folder's own."""
    write(root, ".vscode/settings.json", '{\n  "qa.limited.path": "evil/path",\n  "editor.tabSize": 2\n}\n')
    write(root, "src/main.ts", "export const answer = 42;\n")


FIXTURES = {
    "trust-repo": trust_repo,
}
