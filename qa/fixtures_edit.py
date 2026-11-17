import os
import stat


def write_bytes(root, rel, data):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as out:
        out.write(data)


def write(root, rel, text):
    write_bytes(root, rel, text.encode("utf8"))


WORDS = (
    "Alpha beta alpha BETA\n"
    "a.b a*b a+b (x) [y] a.b\n"
    "foo foofoo food\n"
    "path\\to\\file $1 ^start end$\n"
    "alpha\n"
)

TABS = "package main\n\nfunc main() {\n\tvalue := 1\n\t\tvalue = value + 1\n\tprintln(value)\n}\n"


def edit_odd(root):
    write(root, "unicode.md", "# Título con ñ y €\n\nLínea con emoji 🙂 y 日本語.\n")
    write(root, "mixed_eol.txt", "one\r\ntwo\nthree\r\nfour\n")
    write_bytes(root, "bom.txt", b"\xef\xbb\xbf" + b"first\nsecond\n")
    write(root, "cr_only.txt", "one\rtwo\rthree\r")
    write(root, "only_newline.txt", "\n")
    write(root, "trailing_ws.txt", "a  \nb\t\n\n\n")
    write(root, "words.txt", WORDS)
    write(root, "tabs_find.go", TABS)
    write(root, "turkish.txt", "İstanbul foo bar\nfoo İİ foo\nplain foo\n")
    write(root, "a.txt", "first of a\nsecond of a\n")
    write(root, "b.txt", "first of b\nsecond of b\n")
    write(root, "c.txt", "first of c\nsecond of c\n")


def edit_deep(root):
    write(root, "top.rs", "pub fn top() {}\n")
    write(root, "one/two/three/four/leaf.rs", "pub fn leaf() -> u32 {\n    4\n}\n")
    write(root, "one/two/side.rs", "pub fn side() {}\n")
    write(root, "one/other/near.rs", "pub fn near() {}\n")
    write(root, "zeta/last.rs", "pub fn last() {}\n")


def edit_paths(root):
    write(root, "web/app.js", 'import { util } from "./lib/util.js";\n\nconst data = "";\nconsole.log(util, data);\n')
    write(root, "web/lib/util.js", "export const util = 1;\n")
    write(root, "web/lib/data.json", '{ "a": 1 }\n')
    write(root, "web/assets/logo.svg", '<svg xmlns="http://www.w3.org/2000/svg"/>\n')
    write(root, "web/about.html", "<html></html>\n")
    write(root, "src/main.rs", 'mod other;\n\nfn main() {\n    let text = "";\n    println!("{text}");\n}\n')
    write(root, "src/other.rs", "pub fn other() {}\n")
    write(root, "src/data/table.txt", "one\n")


def edit_readonly(root):
    write(root, "locked.txt", "locked one\nlocked two\n")
    write(root, "free.txt", "free one\nfree two\n")
    os.chmod(os.path.join(root, "locked.txt"), stat.S_IREAD)


FIXTURES = {
    "edit-readonly": edit_readonly,
    "edit-odd": edit_odd,
    "edit-deep": edit_deep,
    "edit-paths": edit_paths,
}
