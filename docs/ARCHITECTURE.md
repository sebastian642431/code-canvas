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

This is name-based and approximate. The `Relation` / `Site` types are independent
of how they were found: when an extension of the language answers "where is this
used", "what does it call" or "what implements it" (`Extensions::request_relations`),
`Repo::refine`, `refine_calls` and `refine_implementations` put its answer in place
of the guessed relations of that kind, and the canvas and the panels draw the
result as they drew the guess.

## Dependencies between files (`canvas-core::deps`)

`deps::analyze(repo)` reads the import statements of every file (ES imports and
`require`, `#include`, Rust `mod` / `use`, Python and Java imports) and resolves them
to files of the repository, in parallel: Chromium's 1.35 million edges take half a
second. The result is a sorted edge list with O(1) `outgoing(file)` / `incoming(file)`.
The app draws the edges of the selected node as curves (the same pipeline as symbol
relations, plus arrowheads and dashes) and lists them in the dependencies panel. The
import cycles (strongly connected components) are found with the edges, and when
there are many edges `deps::bundle` merges them per pair of folders too small to see
into (`Layout::groups`). It is import-based and approximate: no build system is
consulted.

## Layout filter

`LayoutFilter { collapsed, hidden, only_files }` decides what is on the canvas.
Collapsed folders become one box (their files still *anchor* to it, so relation curves
end there); hidden paths disappear; `only_files` implements "show only connected
files". `Layout::anchor(site)` turns a place in the code into a world position.
Changing the filter rebuilds the layout and the static GPU buffers (tens of ms even
for huge repositories); cameras and selections survive by path.

## The app (`code-canvas`)

- `Page` = one opened folder: scan state, camera, filter, selected symbols, git changes,
  search results.
- The Search panel (`ui/search_panel.rs`, `search.rs`) searches in a background thread:
  `canvas_core::grep` reads the files on the canvas from disk in parallel (the text of an editor
  with unsaved changes, or of the commit shown, is handed to it), matches line by line with the
  regex crate (plain text is escaped; whole word is checked around each match) and stops at
  5000 results. A new query cancels the search running. Files written or typed in are searched
  again one by one a moment later; a folder read again is searched again whole. Results keep
  real line numbers and are turned into places on the canvas (rows of a diff) only to mark them.
  Replacing rewrites the files in the background; the app then writes them through the
  watcher, or puts the text in the editor when the file has unsaved changes.
- `ui.rs` builds the chrome with egui: our own title bar (the window has no system
  frame), an icon rail per side, and panels. It never mutates pages; it returns
  `Action`s that `App::apply` executes.
- `overlay.rs` builds what is drawn over the code each frame: names, collapse
  markers, highlights, chips, body rectangles. Relation curves are tessellated only
  when the selection, filters or layout change and stay on the GPU.
- A frame is: egui pass → apply actions → canvas render pass → egui paint pass, all
  in one command encoder.
- `ext/` runs VS Code extensions: `market.rs` installs them from Open VSX,
  `host.rs` starts the extension host (`exthost/host.js`, a Node process that gives
  extensions the `vscode` API), `mod.rs` keeps what they report. See
  [EXTENSIONS.md](EXTENSIONS.md).
- `git.rs` runs the user's `git` off the UI thread: status (staged and unstaged
  sides), changed lines, the latest commits and the timeline of the selected file,
  plus what the source control panel does (stage, unstage, commit, sync).
- `webview.rs` shows extensions' webviews in a child browser view.
- `editor.rs` is the text being edited (the file as it is on disk, with its own tabs
  and line endings), the caret, selection and undo. After each edit the file's card
  is rebuilt from that text; the layout only when its size changes.
- `watch.rs` reports files that change on disk; a few are reloaded one by one, new or
  removed files mean a rescan (left to the user for huge folders).
- `churn.rs` counts, over the last minute, writes per file, git reads and scans per
  folder, and says which one keeps the app busy (a "Busy" item in the status bar and
  one notice per cause every ten minutes).
- `session.rs` persists folders, cameras, collapsed/hidden paths and panel state.
- `script.rs` drives the app from `CODE_CANVAS_SCRIPT` (see below).

## Windows, resizing and the graphics API

On Windows the app draws with Direct3D 12 (`CODE_CANVAS_BACKEND=vulkan|dx12|gl`
overrides it). Measured while the window is resized hundreds of times: Vulkan rebuilds
its swap chain for every size (15-30 ms) and, with a browser view in the window (an
extension's webview), stalls for half a second per frame waiting for an image;
Direct3D 12 does neither. Two rules in the frame loop come from the same work:

- A size change is only recorded; the surface is reconfigured once, at the start of
  the next frame, however many sizes arrived in between.
- A surface is never reconfigured while one of its textures is held (wgpu treats
  that as a fatal error).

`resize <w> <h>` in a script resizes the window from inside the app; resizing it from
another process, as a user's drag does, is what found these problems.

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

Steps: `wait <ms>`, `fit`, `locate`, `goto <path>`, `select <symbol>`, `search <text>` (the Search panel,
with its query), `search-option case|word|regex|list on|off`, `search-option include|exclude|replace <text>`,
`search-option replace-all|next|previous` (the options and buttons of the Search panel; `replace-all` asks first),
`left files|search|extensions|problems|changes|settings|<publisher.name>|help|none`,
`right symbols|<extension id>|none`, `folders`, `collapse-all`, `expand-all`, `connected on|off`,
`changed on|off`, `tab <n>`, `zoom <f>`, `quick <text>`, `palette <text>`, `edit <path> <line> <column>`, `type <text>`,
`key enter|backspace|undo|redo|save|…`, `git stage|unstage [path]`, `git commit <message>`, `git pull`,
`install <publisher.name>`, `ext-search <text>`, `format <path>`, `command <id>`,
`answer <choice>` (press that button of a notice that asks),
`resize <width> <height>`, `open <path>` (media viewer), `updates`, `maximize`,
`click <x> <y>` (a click on the canvas, in physical pixels of the window),
`ui click|right|move|down|up <x> <y>` and `ui drag <x> <y> <x> <y>` (the pointer on the interface, in
points: what a button, a menu or a drag of an icon gets),
`move <panel> left|right [panel to sit before]` (an icon to the other bar),
`preview <path>` (a Markdown file rendered), `drop <path>` (a file dropped on the window),
`tool move|edit|draw|rub|clear` (the hand, the pointer, the pencil, its rubber, its bin), `back`, `forward`, `diff on|off|split|together|fold|unfold|around <lines>` (what changed
inside the files), `drag <x> <y> <x> <y>` (a drag on the canvas), `find <text>|next|previous|replace <text>|replace-all <text>|close`,
`touch <path> <text>` (a line appended to a file behind
the app's back, as another program would), `page <javascript>` (a line of script run in the
webview on screen),
`mark <name>` (leaves an empty file `marks/<name>` in the data directory: a sign for whoever watches the
instance, see below), `quit`. `CODE_CANVAS_DATA_DIR` keeps the test's session and log away from
the real ones. Logs go to `code-canvas.log` in the data directory.

To see the result, `scripts/screenshot.ps1` (Windows) starts such an instance and
saves a picture of its window: enough to check a panel or a theme without clicking.
With `-Marks` it saves one picture per `mark <name>` step instead (`<name>.png`, next
to `-Out`), a third of a second after each: a whole walk through the interface in one
run. Follow each `mark` with a `wait` of half a second or more.
