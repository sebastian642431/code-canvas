import glob
import importlib.util
import os
import subprocess
import sys


def write(root, rel, text, newline="\n"):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf8", newline="") as out:
        out.write(text.replace("\n", newline))


def write_bytes(root, rel, data):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as out:
        out.write(data)


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


STEPS = "\n".join(f"fn step_{i}() -> u32 {{\n    {i}\n}}\n" for i in range(1, 15))
MAIN = 'mod steps;\nmod util;\n\nfn main() {\n    println!("{} {}", steps::step_1(), util::double(2));\n}\n'
UTIL = "pub fn double(n: u32) -> u32 {\n    n * 2\n}\n\npub fn triple(n: u32) -> u32 {\n    n * 3\n}\n"


def rust_small(root):
    write(root, "Cargo.toml", '[package]\nname = "small"\nversion = "0.1.0"\nedition = "2021"\n')
    write(root, "src/main.rs", MAIN)
    write(root, "src/steps.rs", STEPS)
    write(root, "src/util.rs", UTIL)
    write(root, ".gitignore", "target\n")


def git_clean(root):
    init(root)
    rust_small(root)
    commit(root, "first")


def git_changes(root):
    git_clean(root)
    text = STEPS
    text = text.replace("fn step_2() -> u32 {\n    2\n}", "fn step_2() -> u32 {\n    let two = 2;\n    two\n}")
    text = text.replace("fn step_6() -> u32 {\n    6\n}\n\n", "")
    text = text.replace("    11\n", "    eleven()\n")
    write(root, "src/steps.rs", text)
    write(root, "src/new_file.rs", "pub fn fresh() {}\n")


def git_history(root):
    init(root)
    rust_small(root)
    commit(root, "first")
    write(root, "src/util.rs", UTIL + "\npub fn label(n: u32) -> String {\n    format!(\"value {n}\")\n}\n")
    commit(root, "a label")
    write(root, "docs/notes.md", "# Notes\n\nOne line.\n")
    write(root, "src/steps.rs", STEPS.replace("    3\n", "    3 + 0\n"))
    commit(root, "notes and a step")
    git(root, "switch", "-q", "-c", "side")
    write(root, "src/side.rs", "pub fn side() {}\n")
    commit(root, "on the side")
    git(root, "switch", "-q", "main")
    write(root, "src/main.rs", MAIN.replace("util::double(2)", "util::triple(2)"))
    commit(root, "triple")
    git(root, "merge", "-q", "--no-ff", "-m", "merge the side", "side")
    os.remove(os.path.join(root, "docs", "notes.md"))
    commit(root, "notes removed")


def git_subfolder(root):
    init(root)
    write(root, "README.md", "# Whole repository\n")
    rust_small(os.path.join(root, "app"))
    write(root, "other/thing.txt", "one\ntwo\nthree\n")
    commit(root, "first")
    write(root, "app/src/util.rs", UTIL.replace("n * 2", "n + n").replace("n * 3", "n + n + n"))
    write(root, "other/thing.txt", "one\nTWO\nthree\n")


def odd_files(root):
    init(root)
    write(root, "crlf.txt", "first line\nsecond line\nthird line\n", newline="\r\n")
    write(root, "tabs.go", "package main\n\nfunc main() {\n\tif true {\n\t\tprintln(\"tabs\")\n\t}\n}\n")
    write(root, "no_final_newline.txt", "one\ntwo")
    write(root, "empty.txt", "")
    write(root, "unicode ñ €.md", "# Título con ñ y €\n\nLínea con emoji 🙂 y 日本語.\n")
    write(root, "with space/a file.js", "export function hello(name) {\n  return `hi ${name}`;\n}\n")
    write_bytes(root, "latin1.txt", "caf\xe9 no es utf8\n".encode("latin-1"))
    write_bytes(root, "binary.bin", bytes(range(256)) * 8)
    write(root, "long_line.txt", "x" * 5000 + "\nshort\n")
    write(root, "big.rs", "".join(f"pub fn f{i}() -> usize {{ {i} }}\n" for i in range(12000)))
    write_bytes(
        root,
        "pixel.png",
        bytes.fromhex(
            "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489"
            "0000000d49444154789c6360f8cfc0000003010100c9fe92ef0000000049454e44ae426082"
        ),
    )
    commit(root, "first")
    write(root, "crlf.txt", "first line\nSECOND line\nthird line\nfourth line\n", newline="\r\n")
    write(root, "tabs.go", "package main\n\nfunc main() {\n\tif true {\n\t\tprintln(\"tabs changed\")\n\t}\n}\n")


def empty(root):
    os.makedirs(root, exist_ok=True)


def plain(root):
    rust_small(root)


def typescript(root):
    write(root, "package.json", '{\n  "name": "ts-small",\n  "version": "1.0.0",\n  "dependencies": { "left-pad": "1.0.0" }\n}\n')
    write(root, "src/index.ts", 'import { greet } from "./greet";\n\nconsole.log(greet("canvas"));\n')
    write(root, "src/greet.ts", "export function greet(name: string): string {\n  return `hello ${name}`;\n}\n")
    write(root, "README.md", "# ts-small\n\nSee [the code](src/index.ts).\n")


FIXTURES = {
    "rust-small": git_clean,
    "plain": plain,
    "git-changes": git_changes,
    "git-history": git_history,
    "git-subfolder": git_subfolder,
    "odd-files": odd_files,
    "empty": empty,
    "typescript": typescript,
}


def more_fixtures():
    here = os.path.dirname(os.path.abspath(__file__))
    for file in sorted(glob.glob(os.path.join(here, "fixtures_*.py"))):
        spec = importlib.util.spec_from_file_location(os.path.basename(file)[:-3], file)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        FIXTURES.update(module.FIXTURES)


def main():
    target, names = sys.argv[1], sys.argv[2:]
    more_fixtures()
    for name in names:
        make = FIXTURES.get(name)
        if make is None:
            print(f"no fixture named {name}: {', '.join(sorted(FIXTURES))}")
            return 1
        make(os.path.join(target, name))
    return 0


if __name__ == "__main__":
    sys.exit(main())
