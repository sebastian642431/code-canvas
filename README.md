# Code Canvas

**See an entire codebase at once.** Code Canvas lays out every folder and file of a
repository on an infinite, GPU-rendered canvas. Zoom out to see the shape of the whole
project, zoom in until you are reading real, syntax-coloured code — in the same place.
Click a function and curves show where it is defined, referenced and called.

Written in Rust with [wgpu](https://wgpu.rs), [winit](https://github.com/rust-windowing/winit)
and [egui](https://github.com/emilk/egui). Built to stay fast on very large repositories.

> Leer en español: [README.es.md](README.es.md)

## What it does

**Canvas**
- Folders are nested boxes, files are cards sized after their code; five levels of
  detail from "whole repository" down to readable text.
- Folders collapse (`−` / `+`), single-child chains merge into `a/b/c/`, files and
  folders can be hidden, long files fold into columns.
- Several folders open at once, each on its own canvas (e.g. a frontend and a backend).

**Symbols — "where is this used?"**
- Click a name in the code: curves go to its definition, references, incoming and
  outgoing calls and implementations, each kind in its own colour.
- Name chips mark the functions at the other end; function bodies are highlighted.
- Filters per kind with opacity; **show only connected files** re-lays the canvas out
  with just the files involved.
- Works instantly with a built-in name-based index (Rust, TypeScript/JavaScript,
  Python, Go, C-family). It is approximate by design; language servers are the plan
  for exact answers (see the roadmap).

**Finding things**
- Files panel: tree with checkboxes, filter, click to fly to a file.
- `Ctrl+P` go to a file or folder; `Ctrl+Shift+F` search text in every file, with the
  hits marked on the canvas.

**Claude Code**
- A chat panel that runs your own `claude` command-line tool, so it uses the account
  or subscription you are already signed in with — Code Canvas stores no API key.
- Read-only by default; files mentioned in an answer become buttons that fly to them.

**An app, not a demo**
- Own title bar and panels, icon rails on both sides, remembers your folders, cameras
  and layout between launches.

## Install and run

Windows (per user, no admin rights):

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

This builds the release binary, copies it to `%LOCALAPPDATA%\Programs\Code Canvas` and
adds Start Menu and Desktop shortcuts (`-ContextMenu` also adds "Open with Code
Canvas" for folders; `scripts\uninstall.ps1` removes everything).

Any platform, from source:

```sh
cargo run --release -- path/to/frontend path/to/backend
```

Without arguments the previous session is restored. Folders can also be opened with
**Ctrl+O** or by dropping them on the window.

| Action | Input |
|---|---|
| Zoom / pan | Mouse wheel, drag, `+` `-`, arrow keys |
| Fly to a file or folder | Double-click, or click it in the files panel |
| Show everything / go to the selection | `F` or `Home` / `L` |
| Zoom out to the parent folder | `Backspace` |
| See where a name is used | Click it (zoomed in to text); click inside a function when zoomed out |
| Deselect | Click it again, `Esc` for everything |
| Go to a file or folder | `Ctrl+P` |
| Search in files | `Ctrl+Shift+F` |
| Open a folder in a new canvas | `Ctrl+O` |
| Next / previous canvas, canvas N | `Ctrl+Tab` / `Ctrl+Shift+Tab`, `Ctrl+1…9` |
| Close the canvas / reload from disk | `Ctrl+W` / `F5` |

## Performance

Performance is a feature, not an afterthought. Current numbers on a stress test — the
whole local cargo registry, **33,500 files / 14.1 M lines / 2.27 M symbols**
(Windows, RTX 5070 Ti, 240 Hz display):

| | |
|---|---|
| Open (walk + read + analyse + index + layout) | **~2.8 s**, off the UI thread |
| Benchmark sweep through every zoom level | **240 fps**, worst frame ~8 ms |
| CPU time per frame, UI included | **0.7 ms** typical, 1.1 ms p99 |
| Relations of a symbol in a 90k-line repository | **~1 ms** (231 usages) |
| Text search in a 90k-line repository | **< 1 ms** |

How (details in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)): parallel walk and load;
tokens produced on demand per visible chunk instead of stored; static geometry uploaded
once and culled by contiguous folder ranges; near views built only from visible lines;
text shaped within a per-frame budget; frames drawn only when something changes.

Measure it yourself: `CODE_CANVAS_BENCH=1` logs frame statistics while the camera
sweeps the canvas; `cargo run --release -p canvas-core --example scan_bench -- <folder>`
times each loading phase.

## Roadmap

Contributions are very welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

- [ ] **Editing**: click into a file and type, with undo and save. Needs a real text
      buffer that keeps each file's own tabs and line endings.
- [ ] **Diagnostics and formatting**: lint errors on the canvas and in a problems
      panel, format on save — using the project's own ESLint / Prettier / rustfmt
      configuration, so results match its CI.
- [ ] **Soft wrapping** for lines longer than the card width (160 columns today).
- [ ] **Language servers** (rust-analyzer, typescript-language-server, pyright …) as an
      exact provider for the same relations the name-based index gives today.
- [ ] VS Code **themes and grammars** loaded from `.vsix` files; an extension model for
      everything else — see [docs/EXTENSIONS.md](docs/EXTENSIONS.md).
- [ ] Other AI providers behind the chat panel (API keys), streaming of partial answers.
- [ ] Dependency edges between files and folders (imports), bundled for collapsed folders.
- [ ] Git: changed files on the canvas, history.
- [ ] File watching with incremental re-layout; a layout cache for instant re-open.
- [ ] Faster symbol indexing (parallel, lazy) and memory-mapped sources for 1M+ file monorepos.
- [ ] Line numbers, a settings panel, configurable skip lists.
- [ ] Linux / macOS packaging, a web build (WebGPU).

## Project layout

```
crates/
  canvas-core/    scanning, tokenizer, symbols, relations, search, layout   (no GPU, unit-tested)
  canvas-render/  wgpu renderer: rects, code bars, curves, text (glyphon), LOD, culling
  code-canvas/    the app: window, UI (egui), pages, camera, Claude Code, sessions
docs/             ARCHITECTURE.md, EXTENSIONS.md
scripts/          install / uninstall, icon generation
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Contributions are accepted under the same terms.

Bundled third-party assets: the [Lucide](https://lucide.dev) icon font (ISC, see
`crates/code-canvas/assets/LUCIDE-LICENSE.txt`) and egui's default fonts (Ubuntu font
licence / OFL, via the `epaint_default_fonts` crate).
