import os
import random


def write(root, rel, text):
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf8", newline="") as out:
        out.write(text)


LAYERS = ["app", "ui/pages", "ui/widgets", "api", "data/stores", "data/models", "core", "util"]
SUBFOLDERS = 4
PER_FOLDER = 30


def module(layer, sub, i):
    short = layer.split("/")[-1]
    return f"src/{layer}/{short}{sub}/{short}{i}"


def relative(source, target):
    rel = os.path.relpath(target, os.path.dirname(source)).replace(os.sep, "/")
    return rel if rel.startswith(".") else "./" + rel


def deps_folders(root):
    """About a thousand TypeScript files in eight layers of four folders: each imports
    files of the layers under it and of its own folder, so there are thousands of edges.
    Two import cycles are planted: core0 and pages0 import each other, util1 -> util2 ->
    util3 -> util1."""
    rng = random.Random(7)
    imports = {}
    for depth, layer in enumerate(LAYERS):
        for sub in range(SUBFOLDERS):
            for i in range(PER_FOLDER):
                me = module(layer, sub, i)
                chosen = []
                lower = LAYERS[depth + 1 :]
                for _ in range(4 if lower else 0):
                    other = rng.choice(lower)
                    chosen.append(module(other, rng.randrange(SUBFOLDERS), rng.randrange(PER_FOLDER)))
                for _ in range(2 if i > 0 else 0):
                    chosen.append(module(layer, sub, rng.randrange(i)))
                imports[me] = chosen
    core0, page0 = module("core", 0, 0), module("ui/pages", 0, 0)
    imports[core0].append(page0)
    imports[page0].append(core0)
    u1, u2, u3 = (module("util", 0, i) for i in (1, 2, 3))
    imports[u1].append(u2)
    imports[u2].append(u3)
    imports[u3] = [u for u in imports[u3] if u != u1] + [u1]
    for me, chosen in imports.items():
        lines = []
        seen = set()
        for target in chosen:
            if target == me or target in seen:
                continue
            seen.add(target)
            lines.append(f'import {{ run as run{len(lines)} }} from "{relative(me, target)}";')
        name = me.rsplit("/", 1)[1]
        body = [
            "",
            "export function run(): string {",
            f'    const parts: string[] = ["{name}"];',
        ]
        body += [f"    parts.push(String(run{n}).length.toString());" for n in range(len(lines))]
        body += ['    return parts.join(" ");', "}", ""]
        write(root, me + ".ts", "\n".join(lines + body))
    write(root, "package.json", '{ "name": "deps-folders", "private": true }\n')


FIXTURES = {
    "deps-folders": deps_folders,
}
