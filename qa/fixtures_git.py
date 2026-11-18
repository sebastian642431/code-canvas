import os
import subprocess


def write(root, rel, text, newline="\n"):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf8", newline="") as out:
        out.write(text.replace("\n", newline))


def git(root, *args):
    subprocess.run(["git", "-C", root, *args], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def init(root):
    os.makedirs(root, exist_ok=True)
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "core.autocrlf", "false")
    git(root, "config", "user.name", "qa")
    git(root, "config", "user.email", "qa@example.invalid")


def commit(root, message):
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", message)


HEADER = "// one\n// two\n// three\n// four\n"
LINES = "".join(f"line {i}\n" for i in range(1, 13))
SAME = "\n".join(f"fn {name}() -> u32 {{\n    0\n}}\n" for name in "abcde")


def git_shifted(root):
    """Every file gets four lines at its top (not staged), and one more change below them.

    insert.txt  a line added after "line 8"            (line 13 of the file on disk)
    delete.txt  "line 8" removed                       (between lines 11 and 12 on disk)
    same.rs     the body of c() goes from 0 to 3       (line 14 on disk); every body was "    0"
    """
    init(root)
    write(root, "insert.txt", LINES)
    write(root, "delete.txt", LINES)
    write(root, "same.rs", SAME)
    commit(root, "first")
    write(root, "insert.txt", HEADER + LINES.replace("line 8\n", "line 8\ninserted\n"))
    write(root, "delete.txt", HEADER + LINES.replace("line 8\n", ""))
    write(root, "same.rs", HEADER + SAME.replace("fn c() -> u32 {\n    0\n", "fn c() -> u32 {\n    3\n"))


def git_spaces(root):
    """A changed file whose folder and name have spaces, one with accents, and one with no final newline."""
    init(root)
    write(root, "with space/a file.js", "export function hello(name) {\n  return `hi ${name}`;\n}\n")
    write(root, "año ñ/día €.txt", "uno\ndos\ntres\n")
    write(root, "no_newline.txt", "one\ntwo")
    write(root, "plain.txt", "a\nb\nc\n")
    commit(root, "first")
    write(root, "with space/a file.js", "export function hello(name) {\n  return `hello ${name}`;\n}\n\nexport const more = 1;\n")
    write(root, "año ñ/día €.txt", "uno\nDOS\ntres\n")
    write(root, "no_newline.txt", "one\nTWO")


def git_renamed(root):
    """History with a rename: old_name.rs became new_name.rs in the second commit, then changed again."""
    init(root)
    write(root, "src/old_name.rs", "pub fn kept() -> u32 {\n    1\n}\n\npub fn also() -> u32 {\n    2\n}\n")
    write(root, "src/main.rs", "fn main() {}\n")
    commit(root, "first")
    git(root, "mv", "src/old_name.rs", "src/new_name.rs")
    commit(root, "renamed")
    write(root, "src/new_name.rs", "pub fn kept() -> u32 {\n    10\n}\n\npub fn also() -> u32 {\n    2\n}\n")
    commit(root, "ten")
    write(root, "src/fresh.rs", "pub fn fresh() {}\n")


def git_mixed(root):
    """Staged, unstaged and untracked at once, a staged new file and a staged deletion."""
    init(root)
    write(root, "a.txt", "a1\na2\na3\n")
    write(root, "b.txt", "b1\nb2\nb3\n")
    write(root, "c.txt", "c1\nc2\nc3\n")
    write(root, "gone.txt", "gone\n")
    commit(root, "first")
    write(root, "a.txt", "a1\nA2 staged\na3\n")
    write(root, "added.txt", "added and staged\n")
    os.remove(os.path.join(root, "gone.txt"))
    git(root, "add", "-A")
    write(root, "a.txt", "a1\nA2 staged\na3\na4 not staged\n")
    write(root, "b.txt", "b1\nB2 not staged\nb3\n")
    write(root, "untracked.txt", "nobody knows me\n")
    write(root, "sub/also untracked.txt", "nor me\n")


def git_subfolder_history(root):
    """A repository whose app/ folder is the project, with three commits: one inside and outside app/, one only outside, and the first."""
    init(root)
    write(root, "README.md", "# Whole repository\n")
    write(root, "app/src/main.rs", 'fn main() {\n    println!("one");\n}\n')
    write(root, "app/src/util.rs", "pub fn double(n: u32) -> u32 {\n    n * 2\n}\n")
    write(root, "other/thing.txt", "one\ntwo\nthree\n")
    commit(root, "first")
    write(root, "other/thing.txt", "one\nTWO\nthree\n")
    commit(root, "only outside the folder")
    write(root, "app/src/util.rs", "pub fn double(n: u32) -> u32 {\n    n + n\n}\n")
    write(root, "other/thing.txt", "one\nTWO\nTHREE\n")
    write(root, "app/notes.txt", "a new file\n")
    commit(root, "inside and outside")
    write(root, "app/src/main.rs", 'fn main() {\n    println!("two");\n}\n')


def git_rename_staged(root):
    """A rename staged with git mv and the renamed file changed afterwards (not staged), and a file deleted without git rm."""
    init(root)
    write(root, "src/old_name.rs", "pub fn kept() -> u32 {\n    1\n}\n\npub fn also() -> u32 {\n    2\n}\n")
    write(root, "src/main.rs", "fn main() {}\n")
    write(root, "src/removed.rs", "pub fn removed() {}\n")
    commit(root, "first")
    git(root, "mv", "src/old_name.rs", "src/new_name.rs")
    write(root, "src/new_name.rs", "pub fn kept() -> u32 {\n    100\n}\n\npub fn also() -> u32 {\n    2\n}\n")
    os.remove(os.path.join(root, "src", "removed.rs"))


def git_no_commits(root):
    """A repository just made: files, no commit yet."""
    init(root)
    write(root, "a.txt", "a1\na2\na3\n")
    write(root, "src/b.rs", "pub fn b() -> u32 {\n    1\n}\n")


def git_remote_branches(root):
    """A repository with a bare remote (a folder next to it): main is one commit ahead of origin/main,
    feature/a is on both sides, feature/b/deep and fix/c are only on the remote."""
    init(root)
    remote = root + "-remote.git"
    os.makedirs(remote, exist_ok=True)
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", remote], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    write(root, "a.txt", "a1\na2\n")
    commit(root, "first")
    git(root, "remote", "add", "origin", remote)
    git(root, "push", "-q", "-u", "origin", "main")
    for name in ["feature/a", "feature/b/deep", "fix/c"]:
        git(root, "switch", "-q", "-c", name)
        write(root, name.replace("/", "-") + ".txt", name + "\n")
        commit(root, "work on " + name)
        git(root, "push", "-q", "-u", "origin", name)
        git(root, "switch", "-q", "main")
    git(root, "branch", "-q", "-D", "feature/b/deep", "fix/c")
    write(root, "a.txt", "a1\na2\na3\n")
    commit(root, "local only")


FIXTURES = {
    "git-remote-branches": git_remote_branches,
    "git-shifted": git_shifted,
    "git-spaces": git_spaces,
    "git-renamed": git_renamed,
    "git-mixed": git_mixed,
    "git-subfolder-history": git_subfolder_history,
    "git-rename-staged": git_rename_staged,
    "git-no-commits": git_no_commits,
}
