import os
import shutil
import struct
import subprocess
import tempfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
# A video of your own beside the repository wins; otherwise one is made with ffmpeg once
# and kept in the temporary folder.
OWN_VIDEO = os.path.join(os.path.dirname(HERE), "asd.mp4")
MADE_VIDEO = os.path.join(tempfile.gettempdir(), "code-canvas-qa-clip.mp4")


def video():
    """The path of a short H.264 clip for the video scenarios, made with ffmpeg the first time."""
    if os.path.exists(OWN_VIDEO):
        return OWN_VIDEO
    if os.path.exists(MADE_VIDEO) and os.path.getsize(MADE_VIDEO) > 0:
        return MADE_VIDEO
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise SystemExit("the media fixture needs a video: put asd.mp4 beside the repository or install ffmpeg")
    making = MADE_VIDEO + ".part.mp4"
    subprocess.run(
        [ffmpeg, "-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=4:size=640x360:rate=25",
         "-f", "lavfi", "-i", "sine=frequency=440:duration=4", "-c:v", "libx264", "-pix_fmt", "yuv420p",
         "-c:a", "aac", "-shortest", making],
        check=True, stdin=subprocess.DEVNULL, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    os.replace(making, MADE_VIDEO)
    return MADE_VIDEO


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
    clip = video()
    os.makedirs(os.path.join(root, "video"), exist_ok=True)
    shutil.copyfile(clip, os.path.join(root, "video", "clip.mp4"))
    os.makedirs(os.path.join(root, "ignored"), exist_ok=True)
    shutil.copyfile(clip, os.path.join(root, "ignored", "kept out.mp4"))


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


WORDS = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa", "lambda", "mu", "nu"]


def long_lines(root):
    """A Rust file with lines of 400, 600 and 4000 columns between short ones."""
    pairs = ", ".join(f'("{WORDS[i % len(WORDS)]}", {i}, {i}.5)' for i in range(20))
    names = ", ".join(f'"{WORDS[i % len(WORDS)]}_{i}"' for i in range(50))
    data = ", ".join(str(i * 7) for i in range(700))
    lines = [
        "// The rest of a long line is shown beside the card.",
        "pub fn table() -> Vec<(&'static str, u32, f64)> {",
        f"    vec![{pairs}] // end of the pairs",
        "}",
        "",
        f"pub const NAMES: [&str; 50] = [{names}]; // end of the names",
        "",
        f"pub static DATA: [u32; 700] = [{data}]; // end of the data",
        "",
        "pub fn short() -> u32 {",
        "    1",
        "}",
    ]
    write(root, "src/table.rs", "\n".join(lines) + "\n")


QUILL = """{- Quill is made up: no app knows it,
   only the grammar of its extension. TODO: more -}
module Greeter

let greeting: Text = "hello, \\"world\\""
let count: Number = 42

fun greet(name: Text) -> Text {
  when count > 10 {
    give shout(greeting ++ name)
  }
  give greeting
}

-- a line comment, FIXME later
let story = script <<INK
=== start ===
Hello there. -> meet
INK

fun shout(text: Text) -> Text { give upper(text) }
"""

INK = """// A story in ink, read by a grammar written as a property list.
=== meet ===
The door opens.
* Knock -> knock
+ Walk away -> END

=== knock ===
Nobody answers. -> END
"""


def grammars(root):
    """Files of languages only a grammar from an extension knows: Quill (.quill, Quillfile), Ink (.ink), a Quill file of 6000 lines with a comment opened deep inside it, and Rust beside them."""
    write(root, "src/hello.quill", QUILL)
    write(root, "Quillfile", '-- what to build\nlet target = "all"\ngive build(target, 3)\n')
    write(root, "notes/story.ink", INK)
    write(root, "src/main.rs", 'fn main() {\n    let answer = 42; // the answer\n    println!("{answer}");\n}\n')
    lines = []
    for i in range(6000):
        if i == 3000:
            lines.append("{- a comment opened deep in the file")
        elif i == 3004:
            lines.append("   and closed four lines later -}")
        else:
            lines.append(f"let value_{i}: Number = {i} -- line {i + 1}")
    write(root, "big/long.quill", "\n".join(lines) + "\n")


FIXTURES = {
    "grammars": grammars,
    "media": media,
    "deep-tree": deep_tree,
    "many-files": many_files,
    "long-names": long_names,
    "long-lines": long_lines,
}
