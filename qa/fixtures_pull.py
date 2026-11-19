import os
import shutil
import stat
import subprocess
import tempfile


def write(root, rel, text):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf8", newline="") as out:
        out.write(text)


def git(root, *args):
    # Paths of a run's work folder are long: objects of the remote could pass Windows' 260 characters.
    subprocess.run(["git", "-c", "core.longpaths=true", "-C", root, *args], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def init(root):
    os.makedirs(root, exist_ok=True)
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "core.autocrlf", "false")
    git(root, "config", "core.longpaths", "true")
    git(root, "config", "user.name", "qa")
    git(root, "config", "user.email", "qa@example.invalid")


def commit(root, message):
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", message)


def unlock(function, path, _info):
    os.chmod(path, stat.S_IWRITE)
    function(path)


def behind(root, diverge):
    """A repository whose remote (a bare folder next to it) has one more commit than it, already fetched:
    from-remote.txt is new there and a.txt has a third line. With `diverge` it has a commit of its own too."""
    init(root)
    remote = root + "-remote.git"
    os.makedirs(remote, exist_ok=True)
    subprocess.run(["git", "-c", "core.longpaths=true", "init", "-q", "--bare", "-b", "main", remote], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    write(root, "a.txt", "a1\na2\n")
    write(root, "src/lib.rs", "pub fn one() -> u32 {\n    1\n}\n")
    commit(root, "first")
    git(remote, "config", "core.longpaths", "true")
    git(root, "remote", "add", "origin", remote)
    git(root, "push", "-q", "-u", "origin", "main")
    other = tempfile.mkdtemp(prefix="qa-other-")
    try:
        subprocess.run(["git", "-c", "core.longpaths=true", "clone", "-q", remote, other], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        git(other, "config", "user.name", "someone")
        git(other, "config", "user.email", "someone@example.invalid")
        write(other, "from-remote.txt", "pulled from the remote\n")
        write(other, "a.txt", "a1\na2\na3 from the remote\n")
        commit(other, "remote work")
        git(other, "push", "-q", "origin", "main")
    finally:
        shutil.rmtree(other, onerror=unlock)
    if diverge:
        write(root, "local-only.txt", "only here\n")
        commit(root, "local work")
    git(root, "fetch", "-q", "origin")


def git_behind(root):
    behind(root, False)


def git_diverged(root):
    behind(root, True)


FIXTURES = {
    "git-behind": git_behind,
    "git-diverged": git_diverged,
}
