import os
import shutil
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
VIDEO = os.path.join(os.path.dirname(HERE), "asd.mp4")


def write(root, rel, text):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf8", newline="") as out:
        out.write(text)


def write_bytes(root, rel, data):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as out:
        out.write(data)


def png(width, height, colour_at):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows.extend(colour_at(x, y))
    head = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", head) + chunk(b"IDAT", zlib.compress(bytes(rows), 6)) + chunk(b"IEND", b"")


def checks(x, y):
    on = (x // 16 + y // 16) % 2 == 0
    return (230, 90, 40) if on else (30, 60, 200)


def bands(x, y):
    return ((x * 5) % 256, (y * 3) % 256, 120)


SVG = (
    '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120" viewBox="0 0 200 120">'
    '<rect width="200" height="120" fill="#104060"/>'
    '<circle cx="100" cy="60" r="40" fill="#f0c020"/></svg>\n'
)

CODE = "pub fn shown() -> &'static str {\n    \"beside the pictures\"\n}\n"


def media(root):
    """Pictures of several shapes and kinds, one that is not a picture, a video, and code."""
    write(root, "src/lib.rs", CODE)
    write(root, "README.md", "# Media\n\n![checks](img/checks.png)\n")
    write_bytes(root, "img/checks.png", png(160, 96, checks))
    write_bytes(root, "img/wide.png", png(600, 20, bands))
    write_bytes(root, "img/tall.png", png(20, 600, bands))
    write_bytes(root, "img/tiny.png", png(1, 1, lambda x, y: (255, 255, 255)))
    write_bytes(root, "img/broken.png", b"\x89PNG\r\n\x1a\nthis is not a picture at all")
    write_bytes(root, "img/empty.png", b"")
    write(root, "img/drawing.svg", SVG)
    write(root, ".gitignore", "ignored\n")
    write_bytes(root, "ignored/hidden.png", png(64, 64, checks))
    if os.path.exists(VIDEO):
        os.makedirs(os.path.join(root, "video"), exist_ok=True)
        shutil.copyfile(VIDEO, os.path.join(root, "video", "clip.mp4"))
        os.makedirs(os.path.join(root, "ignored"), exist_ok=True)
        shutil.copyfile(VIDEO, os.path.join(root, "ignored", "kept out.mp4"))


def deep_tree(root):
    """Fourteen folders one inside the other, a file in each."""
    rel = ""
    for level in range(14):
        rel += f"d{level:02}/"
        write(root, rel + f"file_{level:02}.rs", f"pub fn at_level_{level}() -> u32 {{\n    {level}\n}}\n")
    write(root, "top.rs", "pub fn top() {}\n")


def many_files(root):
    """Four hundred small files in twenty folders."""
    for folder in range(20):
        for file in range(20):
            body = "".join(f"pub fn f_{folder}_{file}_{n}() -> u32 {{ {n} }}\n" for n in range(1 + (file * 7) % 30))
            write(root, f"mod_{folder:02}/unit_{file:02}.rs", body)
    write(root, "Cargo.toml", '[package]\nname = "many"\nversion = "0.1.0"\nedition = "2021"\n')


LONG = "a_very_long_name_that_goes_on_and_on_and_fits_in_no_row_of_a_panel"


def long_names(root):
    """A file and a folder whose names are far longer than a panel is wide (short enough for a 260 character path)."""
    write(root, f"{LONG}.rs", "pub fn long_named() {}\n")
    write(root, f"{LONG}_d/in.rs", "pub fn inside() {}\n")
    write(root, "short.rs", "pub fn " + LONG + "_function_name_too() -> u32 {\n    1\n}\n\npub fn s() {}\n")
    write(root, "a b/c d.rs", "pub fn spaced() {}\n")


FIXTURES = {
    "media": media,
    "deep-tree": deep_tree,
    "many-files": many_files,
    "long-names": long_names,
}
