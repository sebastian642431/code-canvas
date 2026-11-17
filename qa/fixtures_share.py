import os


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


NOTES = "".join(f"line {i:02d}\n" for i in range(1, 13))


def share_text(root):
    write(root, "notes.txt", NOTES)
    write(root, "other.txt", "alpha\nbeta\ngamma\n")
    write(root, "docs/deep/inner.txt", "inner one\ninner two\n")
    write(root, "src/lib.rs", "pub fn one() -> u32 {\n    1\n}\n\npub fn two() -> u32 {\n    2\n}\n")


def share_second(root):
    write(root, "second.txt", "second one\nsecond two\n")
    write(root, "more/third.txt", "third\n")


def share_odd(root):
    write_bytes(root, "bom.txt", b"\xef\xbb\xbfone\r\ntwo\r\n")
    write_bytes(root, "mixed.txt", b"a\r\nb\nc\r\nd\n")


FIXTURES = {
    "share-text": share_text,
    "share-second": share_second,
    "share-odd": share_odd,
}
