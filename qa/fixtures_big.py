import os
import random

TOP = ["app", "core", "data", "net", "ui", "util"]
MIDDLE = 8
LOW = 8
PER_FOLDER = 80


def write(root, rel, text):
    path = os.path.join(root, *rel.split("/"))
    with open(path, "w", encoding="utf8", newline="") as out:
        out.write(text)


def body(rng, name, imports):
    lines = [f'import {{ {other} }} from "./{other}";' for other in imports]
    lines.append("")
    for k in range(rng.randrange(3, 12)):
        lines.append(f"export function {name}_{k}(input: number): number {{")
        for step in range(rng.randrange(2, 9)):
            lines.append(f"    input = input * {step + 2} + {k};")
        lines.append("    return input;")
        lines.append("}")
        lines.append("")
    lines.append(f"export const {name} = {name}_0;")
    return "\n".join(lines) + "\n"


def big_folder(root):
    """About thirty thousand TypeScript files in three levels of folders (6 x 8 x 8
    folders of 80 files), each importing two others of its folder: what a change of
    one file must not make the app read or lay out again."""
    rng = random.Random(11)
    for top in TOP:
        for m in range(MIDDLE):
            for low in range(LOW):
                folder = f"src/{top}/{top}{m}/part{low}"
                os.makedirs(os.path.join(root, *folder.split("/")), exist_ok=True)
                names = [f"{top}{m}_{low}_{i}" for i in range(PER_FOLDER)]
                for i, name in enumerate(names):
                    imports = sorted({names[rng.randrange(PER_FOLDER)] for _ in range(2)} - {name})
                    write(root, f"{folder}/{name}.ts", body(rng, name, imports))
    write(root, "package.json", '{ "name": "big-folder", "private": true }\n')


FIXTURES = {
    "big-folder": big_folder,
}
