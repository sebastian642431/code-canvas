# Architecture

Code Canvas is three crates with a one-way dependency chain:

```
canvas-core  ──►  canvas-render  ──►  code-canvas (app)
(no GPU)          (wgpu)              (winit, tabs, input)
```

## Coordinates

Everything on the canvas is in **world units**. One monospace cell is
`CHAR_W × LINE_H` = `6 × 12` world units, so at zoom `1.0` code is drawn at roughly a
10 px font. `zoom` is *physical pixels per world unit*. A camera is
`(center, zoom)`; `View` converts between world and screen.

## Loading a folder (`canvas-core::scan`) — runs on a background thread

1. **Walk** — `ignore::WalkParallel` (the ripgrep engine) on all cores, honouring
   `.gitignore`. Paths are collected per thread and sorted afterwards so the result
   does not depend on thread timing.
2. **Load** — `rayon` reads every file, normalises it (`\n`, tabs → spaces) and runs
   `syntax::analyze` once, which keeps only:
   - a `Block` per 8 lines (column span + dominant token kind) for the far view, and
   - the tokenizer state at the start of each 128-line chunk.

   Tokens themselves are **not** stored (they would be ~12 B each — over 1 GB for
   14 M lines).
3. **Tree + layout** — directories become boxes, files become cards sized after their
   code. Children are packed with a skyline packer, tallest first.
4. **Static GPU data** (`canvas-render::SceneData`) is built on the same thread; the UI
   thread only uploads it.

### Pre-order layout

`Layout::nodes` is in pre-order and every node stores `subtree_end`, so a subtree is
the slice `nodes[i..subtree_end]`. This gives:

- `hit()` — descend only into the child that contains the point.
- `visit_visible()` — skip an invisible folder in O(1).
- range culling in the renderer (below).

## Rendering (`canvas-render`)

One render pass per frame, only when something changed:

| Step | Data | Lives |
|---|---|---|
| frames (boxes, cards) | `RectInstance` 32 B, one per node | static buffer |
| code bars | `CodeInstance` 12 B, file-relative | static (blocks) or per frame (lines, tokens) |
| overlays | hover / selection | tiny per-frame buffer |
| world text | code chunks + labels (glyphon) | cached buffers |
| UI | tab bar, status bar | per frame |

### Levels of detail

Picked from pixels per line (`zoom × LINE_H`):

| px / line | LOD | Source |
|---|---|---|
| < 0.08 | cards | frames only |
| < 1 | blocks | static, 1 bar / 8 lines |
| < 2.5 | lines | generated for visible lines |
| < 6.5 | tokens | generated for visible lines |
| ≥ 6.5 | text | glyphon, shaped per visible chunk |

### Range culling

Because instances are emitted in layout pre-order, the instances of a subtree are a
contiguous range. Culling walks the tree: a folder fully inside the view becomes one
range, a folder outside is skipped, a partially visible one emits its own instance and
recurses. Adjacent ranges merge, so a whole repository is typically 2–60 draw calls.

### Code instances

`CodeInstance { file, line:20|kind:4|span:8, col:16|cols:16 }`. The shader reads the
file origin from a storage buffer, so moving a file only rewrites one `vec2`.
Sub-pixel bars are drawn one pixel wide with alpha scaled by their real coverage.

### Text

`TextLayer` wraps glyphon. Code is shaped in 128-line chunks (`Shaping::Basic`, no
wrapping) using colours from the token cache. At most 12 new chunks are shaped per
frame; if more are needed the frame is marked `text_pending` and the app redraws.
The font size is chosen at start-up so the monospace advance equals `CHAR_W`
exactly, which keeps text aligned with the cards and bars.

### Token cache (`canvas-core::TokenCache`)

Per page, keyed by `(file, chunk)`, LRU-evicted above a soft limit. Tokenizing a chunk
costs ~30 µs, so line/token LODs and text can always fetch what is visible.

## Symbols and relations (`canvas-core::symbols`, `canvas-core::refs`)

`symbols::SymbolScanner` rides along the load pass: it sees every line's tokens and
recognises definitions with small per-language patterns (Rust, JS/TS, Python, Go,
C-family), finding bodies by brace matching or indentation. A `Symbol` is 20 bytes and
its name is a slice of the file text.

`refs::SymbolIndex` maps names to definitions for the whole repository.
`Repo::relations(symbol)` answers "where is this used":

1. every file containing the name is scanned in parallel; matches must be whole
   identifiers that are real code tokens (not comments or strings);
2. when several definitions share the name, a usage binds to the closest one: an
   explicit `Type::name` / `this.name` qualifier, then the same file, then a file whose
   name is mentioned (imports), then the nearest folder;
3. usages become `Reference` or `IncomingCall` relations, calls inside the body become
   `OutgoingCall`, `impl` blocks become `Implementation`.

This is name-based and approximate. The `Relation` / `Site` types are deliberately
independent of how they were found, so a language-server client can provide exact
results later without touching the canvas or the panels.

## Layout filter

`LayoutFilter { collapsed, hidden, only_files }` decides what is on the canvas.
Collapsed folders become one box (their files still *anchor* to it, so relation curves
end there); hidden paths disappear; `only_files` implements "show only connected
files". `Layout::anchor(site)` turns a place in the code into a world position.
Changing the filter rebuilds the layout and the static GPU buffers (tens of ms even
for huge repositories); cameras and selections survive by path.

## The app (`code-canvas`)

- `Page` = one opened folder: scan state, camera, filter, selected symbols, chat,
  search results.
- `ui.rs` builds the chrome with egui: our own title bar (the window has no system
  frame), an icon rail per side, and panels. It never mutates pages; it returns
  `Action`s that `App::apply` executes.
- `overlay.rs` builds what is drawn over the code each frame: names, collapse
  markers, highlights, chips, body rectangles. Relation curves are tessellated only
  when the selection, filters or layout change and stay on the GPU.
- A frame is: egui pass → apply actions → canvas render pass → egui paint pass, all
  in one command encoder.
- `claude.rs` runs the user's `claude` CLI (`-p --output-format stream-json`), reads
  one JSON event per line on a thread, and resumes the session for follow-ups.
- `ext/` runs VS Code extensions: `market.rs` installs them from Open VSX,
  `host.rs` starts the extension host (`exthost/host.js`, a Node process that gives
  extensions the `vscode` API), `mod.rs` keeps what they report. See
  [EXTENSIONS.md](EXTENSIONS.md).
- `git.rs` runs `git status`, `git diff HEAD -U0` and `git log` off the UI thread:
  changed files, changed lines and the timeline of the selected file.
- `session.rs` persists folders, cameras, collapsed/hidden paths and panel state.
- `script.rs` drives the app from `CODE_CANVAS_SCRIPT` (see below).

## Measuring

- Status bar: LOD, instances, draw calls, cached chunks and CPU frame time.
- `cargo run --release -p canvas-core --example scan_bench -- <folder>` times
  each loading phase.
- `CODE_CANVAS_NO_VSYNC=1` disables vsync; `CODE_CANVAS_BENCH=1` sweeps the camera and
  logs frame statistics.

## Testing the real app without touching the mouse

`CODE_CANVAS_SCRIPT` runs steps inside the app, in order, waiting for loads and
camera animations:

```sh
CODE_CANVAS_DATA_DIR=/tmp/cc-test CODE_CANVAS_SCRIPT="goto src/main.rs; wait 800; select main; locate; wait 1000; quit"   cargo run --release -- .
```

Steps: `wait <ms>`, `fit`, `locate`, `goto <path>`, `select <symbol>`, `search <text>`,
`left files|search|extensions|problems|changes|<publisher.name>|help|none`,
`right symbols|claude|none`, `folders`, `collapse-all`, `expand-all`, `connected on|off`,
`changed on|off`, `tab <n>`, `zoom <f>`, `quick <text>`, `chat <text>`,
`install <publisher.name>`, `ext-search <text>`, `format <path>`, `command <id>`,
`maximize`, `quit`. `CODE_CANVAS_DATA_DIR` keeps the test's session and log away from
the real ones. Logs go to `code-canvas.log` in the data directory.
