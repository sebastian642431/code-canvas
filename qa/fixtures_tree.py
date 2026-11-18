import os
import subprocess


def write(root, rel, text):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf8", newline="") as out:
        out.write(text)


def git(root, *args):
    subprocess.run(["git", "-C", root, *args], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def tree_move(root):
    """Files and folders to drag around: a name that exists in two places, a folder with a folder in it."""
    os.makedirs(root, exist_ok=True)
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "core.autocrlf", "false")
    git(root, "config", "user.name", "qa")
    git(root, "config", "user.email", "qa@example.invalid")
    for name in ("a", "b", "c", "d", "extra"):
        write(root, f"{name}.txt", f"root {name}\n")
    write(root, "notes.md", "# Notes\n")
    write(root, "src/main.rs", "fn main() {}\n")
    write(root, "src/a.txt", "src a\n")
    write(root, "docs/readme.md", "# Readme\n")
    write(root, "docs/guide/intro.md", "# Intro\n")
    write(root, "lib/keep.txt", "keep\n")
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", "first")


def tree_other(root):
    os.makedirs(root, exist_ok=True)
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.name", "qa")
    git(root, "config", "user.email", "qa@example.invalid")
    write(root, "other.txt", "other\n")
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", "first")


def tree_outside(root):
    write(root, "from-explorer.txt", "dropped from the system\n")


FIXTURES = {"tree-move": tree_move, "tree-other": tree_other, "tree-outside": tree_outside}
