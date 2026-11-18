# Notes

The source code of Code Canvas carries **no comments, by design**. What a reader
cannot get from the code and its names lives here: why something is done in a
non-obvious way, invariants, traps, platform quirks, budgets, file formats and how
to run things. These notes were rescued from the comments the code used to have.

- Notes are filed by crate and source file and name the function, type or constant
  they belong to. There are no line numbers: search the file for the name.
- The big picture is in [ARCHITECTURE.md](ARCHITECTURE.md), extensions in
  [EXTENSIONS.md](EXTENSIONS.md). What those explain is only linked from here.
- When you change behaviour described here, change the note in the same commit.

## Contents

- [Quick reference](#quick-reference):
  [environment variables](#environment-variables) ·
  [files in the data directory](#files-in-the-data-directory) ·
  [tests and benchmarks that need the real world](#tests-and-benchmarks-that-need-the-real-world)
- [canvas-core](#canvas-core):
  [cache.rs](#cratescanvas-coresrccachers) ·
  [deps.rs](#cratescanvas-coresrcdepsrs) ·
  [layout.rs](#cratescanvas-coresrclayoutrs) ·
  [refs.rs](#cratescanvas-coresrcrefsrs) ·
  [scan.rs](#cratescanvas-coresrcscanrs) ·
  [store.rs](#cratescanvas-coresrcstorers) ·
  [symbols.rs](#cratescanvas-coresrcsymbolsrs) ·
  [syntax.rs](#cratescanvas-coresrcsyntaxrs) ·
  [wrap.rs](#cratescanvas-coresrcwraprs)
- [canvas-render](#canvas-render):
  [lib.rs](#cratescanvas-rendersrclibrs) ·
  [scene.rs](#cratescanvas-rendersrcsceners) ·
  [shaders](#cratescanvas-rendersrcshaders) ·
  [curve.rs](#cratescanvas-rendersrccurvers) ·
  [gpu_util.rs](#cratescanvas-rendersrcgpu_utilrs) ·
  [text.rs](#cratescanvas-rendersrctextrs) ·
  [theme.rs](#cratescanvas-rendersrcthemers)
- [code-canvas](#code-canvas):
  [build.rs](#cratescode-canvasbuildrs) ·
  [exthost/host.js](#cratescode-canvasexthosthostjs) ·
  [main.rs](#cratescode-canvassrcmainrs) ·
  [app.rs](#cratescode-canvassrcapprs) ·
  [bench.rs](#cratescode-canvassrcbenchrs) ·
  [codicons.rs, icons.rs](#cratescode-canvassrccodiconsrs-and-iconsrs) ·
  [commit_ai.rs](#cratescode-canvassrccommit_airs) ·
  [editor.rs](#cratescode-canvassrceditorrs) ·
  [ext/host.rs](#cratescode-canvassrcexthostrs) ·
  [ext/market.rs](#cratescode-canvassrcextmarketrs) ·
  [ext/mod.rs](#cratescode-canvassrcextmodrs) ·
  [ext/views.rs](#cratescode-canvassrcextviewsrs) ·
  [ext/when.rs](#cratescode-canvassrcextwhenrs) ·
  [files.rs](#cratescode-canvassrcfilesrs) ·
  [git.rs](#cratescode-canvassrcgitrs) ·
  [grammars.rs](#cratescode-canvassrcgrammarsrs) ·
  [icon_theme.rs](#cratescode-canvassrcicon_themers) ·
  [keys.rs](#cratescode-canvassrckeysrs) ·
  [markdown.rs](#cratescode-canvassrcmarkdownrs) ·
  [media.rs](#cratescode-canvassrcmediars) ·
  [overlay.rs](#cratescode-canvassrcoverlayrs) ·
  [page.rs](#cratescode-canvassrcpagers) ·
  [paths.rs](#cratescode-canvassrcpathsrs) ·
  [plist.rs](#cratescode-canvassrcplistrs) ·
  [prefs.rs](#cratescode-canvassrcprefsrs) ·
  [script.rs](#cratescode-canvassrcscriptrs) ·
  [session.rs](#cratescode-canvassrcsessionrs) ·
  [agents.rs](#cratescode-canvassrcagentsrs) ·
  [codex.rs](#cratescode-canvassrccodexrs) ·
  [share](#cratescode-canvassrcshare) ·
  [themes.rs](#cratescode-canvassrcthemesrs) ·
  [ui.rs](#cratescode-canvassrcuirs) ·
  [update.rs](#cratescode-canvassrcupdaters) ·
  [versions.rs](#cratescode-canvassrcversionsrs) ·
  [watch.rs](#cratescode-canvassrcwatchrs) ·
  [webview.rs](#cratescode-canvassrcwebviewrs)
- [Repository files](#repository-files)
- [scripts](#scripts)

## Quick reference

### Environment variables

| Variable | Read in | Effect |
|---|---|---|
| `CODE_CANVAS_SCRIPT` | `script.rs`, `app.rs` | Steps to run inside the app, see [script.rs](#cratescode-canvassrcscriptrs). Also makes the run *quiet* (below). |
| `CODE_CANVAS_BENCH` | `bench.rs`, `app.rs` | `CODE_CANVAS_BENCH=1 code-canvas <folder>` runs the frame-time benchmark. Also makes the run quiet. |
| `CODE_CANVAS_DATA_DIR` | `session::data_dir` | Replaces the data directory, so a test leaves the real session alone. |
| `CODE_CANVAS_NO_CACHE` | `page::scan_cache` | Set to anything: scans neither read nor write what a scan found. |
| `CODE_CANVAS_TEXT_MB` | `page::text_budget` | How many megabytes of file text are kept in memory before the least recently used is let go and read again on demand (768 by default). |
| `CODE_CANVAS_BACKEND` | `app.rs` | `vulkan\|dx12\|metal\|gl` overrides the graphics API (default on Windows: Direct3D 12). |
| `CODE_CANVAS_NO_VSYNC` | `app.rs` | See "Measuring" in [ARCHITECTURE.md](ARCHITECTURE.md). |
| `CODE_CANVAS_PRETEND_VERSION` | `update::current` | Replaces the version of this build, to see how an offered update looks without waiting for a release. |
| `CODE_CANVAS_CHECK_UPDATES` | `App::new` | Lets a quiet run look for updates anyway (for testing the updater). |
| `CODEX_HOME` | `codex::home` | Codex's own: where its sessions are (`~/.codex` otherwise). The QA runner points it at a folder of the run. |

A *quiet* run (scripted or benchmark) does not install the default extensions and
does not look for updates: such runs must not reach for the network.

### Files in the data directory

`session::data_dir` is `%APPDATA%\code-canvas` on Windows, `~/.config/code-canvas`
on Linux and `~/Library/Application Support/code-canvas` on macOS, unless
`CODE_CANVAS_DATA_DIR` says otherwise.

| File | Owner | What |
|---|---|---|
| `session.json` | `session.rs` | Opened folders, cameras, collapsed and hidden paths, panel state. |
| `theme.json` | `themes.rs` | Built-in theme, colours changed on top of it, file icon theme. |
| `keybindings.json` | `keys.rs` | The user's changes to the shortcuts. |
| `prefs.json` | `prefs.rs` | Preferences that are neither looks nor keys. |
| `commit.json` | `commit_ai.rs` | Who writes commit messages. |
| `defaults.json` | `ext/mod.rs` | The default extensions this installation already got. |
| `settings.json` | extension host | VS Code-style settings, see [EXTENSIONS.md](EXTENSIONS.md). |
| `extensions/` | `ext/market.rs` | Installed extensions, unpacked. |
| `scans/<hash of the folder's path>.bin` | `canvas-core/store.rs`, `page.rs` | What the last scan of that folder found. Safe to delete: the folder is read whole the next time. One not used for 30 days is removed at start. |
| `code-canvas.log` | `main.rs` | The log, see [ARCHITECTURE.md](ARCHITECTURE.md). |

The extension host script is embedded in the executable and written here at start.

### Tests and benchmarks that need the real world

The tests below are `#[ignore]`d because they need a folder, a program or the
network; the last rows are not tests but belong to the same toolbox.

| What | Command |
|---|---|
| Time each phase of loading a folder | `cargo run --release -p canvas-core --example scan_bench -- <folder>` |
| Time one file typed, read again, made and deleted, against building everything again | `cargo run --release -p canvas-render --example update_bench -- <folder> [--write] [--check]` (`--write` makes and deletes a file in the folder: never on a folder that is not yours to write; `--check` compares each step with everything built again) |
| Time dependency analysis on a real repository | `CODE_CANVAS_DEPS_REPO=<folder> cargo test --release -p canvas-core deps::tests::bench -- --ignored --nocapture` |
| Ask a real assistant for a commit message about this repository | `COMMIT_AI_TEST=claude cargo test -p code-canvas real_assistant -- --ignored --nocapture` |
| What the Codex of this computer did, as the canvas would be told it (reads its sessions, writes nothing; `CODEX_SESSIONS_SHOW=1` lists each place) | `CODEX_SESSIONS_TEST=%USERPROFILE%\.codex\sessions cargo test -p code-canvas real_sessions -- --ignored --nocapture` |
| Rail icons of a real extensions folder | `EXT_RAIL_TEST=<extensions folder> cargo test -p code-canvas rail_of_a_real -- --ignored --nocapture` |
| `git::load` on a real repository | `GIT_LOAD_TEST=<folder> cargo test -p code-canvas load_a_real -- --ignored --nocapture` |
| Draw what a real icon theme gives a few names, side by side, to look at | `ICON_THEME_TEST=<theme.json> ICON_THEME_SHEET=<out.png> cargo test -p code-canvas real_icon_theme -- --ignored --nocapture` |
| Latest release, from GitHub | `cargo test -p code-canvas real_latest -- --ignored --nocapture` |
| Latest versions, from npm and crates.io | `cargo test -p code-canvas real_registries -- --ignored --nocapture` |
| Frame-time benchmark | `CODE_CANVAS_BENCH=1 code-canvas <folder>` |
| Drive the real app | `CODE_CANVAS_SCRIPT="goto src/main.rs; wait 800; select main; locate" code-canvas <folder>` |

---

## canvas-core

Everything the app knows about a repository that does not need a GPU. Coordinates,
loading, the pre-order layout and the symbol index are explained in
[ARCHITECTURE.md](ARCHITECTURE.md).

### crates/canvas-core/src/cache.rs

- `TokenCache::begin_frame` must be called once per frame before
  `TokenCache::chunk`.
- `max_chunks` is a soft limit. Above it, `begin_frame` evicts the least recently
  used chunks down to 3/4 of the limit.
- **Painted tokens** are the ones a grammar of an extension gave
  (`code-canvas/src/grammars.rs`). The cache has a `Painter` (which files a grammar
  colours, from the app) and keeps a `Painted` per file: one slice of `Piece`s
  (byte offset in the line, length, kind) per line, how many lines the file has,
  and the `SourceFile::serial` of the text they belong to. `chunk` builds a chunk's
  `Token`s from the pieces when every line of the chunk is there, the serial is the
  file's and each piece lies inside its line on character boundaries; otherwise the
  chunk is the app's own tokens, and the file goes in `asked` (the app sends it to
  the grammar thread, `take_asked`).
- Pieces are per line, not per chunk, so after an edit only the lines that changed
  are new: the lines after it are shared with the version before even though their
  byte offsets moved.
- `reset` (an edit, a diff shown) forgets the chunks but keeps the painted files:
  the serial tells which are still right, so the other files of the page keep their
  colours while one is typed in. `set_painter` (grammars installed or removed, the
  setting changed) forgets everything.
- `paint` returns the chunks that are newly covered, which the app takes off the
  shaped text (`TextLayer::drop_chunks`).
- Above `PAINTED_LINES` (a million lines) painted in all, the files that have no
  chunk in the cache any more are let go; they are asked again when seen.

### crates/canvas-core/src/deps.rs

What it is and how fast: see "Dependencies between files" in
[ARCHITECTURE.md](ARCHITECTURE.md). The rest is its limits and its rules.

- **The disk is never touched.** Statements are resolved with nothing but the list
  of paths the scan produced.
- **A statement only counts when its line *starts* with the keyword.** So an import
  inside a block comment or a string that happens to start a line is taken for
  real. There is no parser, no build configuration (include directories, `tsconfig`
  paths, `sys.path`, crate names) and no type information.
- Whatever needs the build system to be found (generated headers, other crates of a
  workspace) ends up in `Deps::unresolved` or `Deps::external`. `unresolved`:
  points inside the repository by its form (relative, quoted, aliased) but matches
  no file. `external`: names something outside (`react`, `<vector>`, `std::…`). One
  statement counts once, however many paths it lists.
- Go, C# and the other languages are skipped.
- `Deps::edges` is sorted by `(from, to, line)` and holds at most one edge per
  `(from, to, kind)`: the first line it appears on. `line` is 0-based.
  `Deps::outgoing` is ordered by `(to, line)`, `Deps::incoming` by `(from, line)`
  (the counting sort by `to` is stable, which is what keeps that order).
- `Index::by_path` gives the same answers as `Repo::file_index`, in one hash
  instead of a binary search over paths: an order of magnitude cheaper, and
  resolution is all lookups. The ignored `bench` test prints what one lookup costs
  each way. `Index::lean` has no hash (and builds the Java suffixes only when a
  Java file is resolved): for a handful of files the binary search is cheaper than
  hashing every path. It checks that the path found is the one asked for, because
  `Path` comparison ignores `.` components.
- **One file at a time.** What a file imports depends only on its own text and on
  which paths exist, so:
  - its text changed (`Deps::update_files`): only its statements are resolved again.
    When it still points at the same files with the same kinds (only lines moved,
    what typing in a body does) its edges are rewritten in place and nothing else
    changes: the order of `by_to` and the cycles depend on targets, not lines.
    Otherwise its edges are spliced in and the indexes and the cycles are made
    again from the sorted edges (`Deps::reindex`, linear, no sort).
  - a file made (`Deps::insert_file`): file numbers after it move up, then its own
    imports are resolved, and so are those of every file whose text holds its stem
    (`naming`, through the `threes` filter first, then the text): only a statement
    that names it can come to point at it, whether it pointed nowhere before or at
    another file it now shadows (`x.ts` beside `x/index.ts`, `a/b/c.py` for `from
    a.b import c`). For `index`, `mod` and `__init__` the folder's name counts too.
  - a file deleted (`Deps::remove_file`): its edges go both ways, and its importers
    and the files that name it are resolved again.
  - `reaches_far`: making or deleting a `Cargo.toml`, `package.json`,
    `tsconfig.json`, `lib.rs` or `main.rs` changes how many files resolve (package
    roots, `crate::`), so the caller analyses everything again (`None`).
  - `Deps::per_file` keeps each file's unresolved and external counts, so the
    totals follow without counting again.
- `MAX_STATEMENT_LINES` = 40: a statement that spans several lines is followed for
  that many lines.
- **Import cycles** are found once, when the edges are built (`cycles`, Tarjan's
  strongly connected components without recursion: a ring of 200,000 files must
  not overflow the stack). `Deps::in_cycle(edge)`: both ends are in the same
  component of two or more files, so a cycle of two files marks both edges.
  `Module` edges (`mod x;`) are left out: a module using an item of its parent is
  how Rust is written, not a cycle. Every kind counts whatever the switches of
  the panel say.
- `bundle` merges the edges whose ends fall in the same pair of groups into one
  `Bundle` with a count, biggest first, and drops those within one group. A
  bundle is `cyclic` when an edge in it is in a cycle or a bundle goes the other
  way between the same groups (the two folders import each other). `first` is
  the index of one of its edges, for where a file end sits. It sorts packed
  `u64` keys (in parallel above 100,000): Chromium's 1.35 million edges in tens of
  milliseconds.

Resolution rules per language:

- **JavaScript / TypeScript**
  - `JS_SWAPS`: `./x.js` written for `./x.ts` (TypeScript with ES modules) and
    `./x.ts` for `./x.d.ts`. `JS_ALIASES`: `@/`, `~/`, `src/` and Node's
    package-private `#` stand for the sources of the package.
  - `package_roots`: the ancestors that hold a `package.json` or `tsconfig.json`,
    nearest first, then the repository root.
  - `js_statement`: `import(`, `import.meta`, `imports` and `exports.x` are not
    statements. `import type from 'x'` imports a default called `type`. Only
    `export {…} from` and `export * from` can name a module.
  - `require('x')` / `import('x')` can sit anywhere on a line, so they are searched
    for only in files whose text contains `require(` or `import(` (most have
    neither). One in a trailing `//` comment is skipped; `://` is a URL, not a
    comment.
  - A query is dropped from the specifier (`./icon.svg?raw`).
- **C / C++ / Objective-C**: an include is looked up next to the including file,
  then from the root (`"base/logging.h"`).
- **Rust**
  - `lib.rs`, `main.rs` and `mod.rs` keep their child modules next to them;
    `foo.rs` keeps them in `foo/`. `rust_mod` tries a second base for crate roots
    that are not called `main.rs` (`src/bin/tool.rs`, `tests/it.rs`): their modules
    are their siblings.
  - `#[path = "…"]` is relative to the folder of the file, whatever the file is
    called.
  - Inline `mod x { … }` blocks are tracked by indentation, and end by it. Inside
    one (`mod tests { … }`), `super` is the file itself. `$` is accepted in a
    module name for the inline modules a macro generates (`mod $name {`).
  - `crate_src`, where `crate::` paths start: `src/` of the nearest ancestor with a
    `Cargo.toml` (the folder itself when the crate root sits next to the manifest);
    without a manifest, the nearest folder that holds a `lib.rs` or `main.rs`.
  - `find_rust` takes the longest prefix of a path that names a module file
    (`a/b.rs` or `a/b/mod.rs`); the rest are items inside it. A glob stands for the
    module in front of it. An item of the crate root, used from the root itself, is
    no dependency.
- **Python**: one leading dot is the file's package, every further dot its parent.
  An absolute import is tried from the root, then from every folder above the
  file, nearest first. In `from a.b import c`, `c` may be the module `a/b/c.py`;
  otherwise it is a name of `a.b`. `from . import x` with no `x.py` is a name of
  the package itself.
- **Java / Kotlin**: files are indexed by their last 1..=`JAVA_SEGMENTS` (6) path
  segments, without extension (`com/acme/Main`); `AMBIGUOUS` marks a suffix shared
  by several files. `find_java` drops a trailing member or nested class and tries
  again, but never down to a single segment: that says too little.

### crates/canvas-core/src/layout.rs

- `Node::gutter` is the room for line numbers to the left of *each* column of a
  card (`GUTTER_COLS` cells, none for a card that is not text). It is inside
  `col_stride` and before `code_origin`, so everything that goes from a cell to a
  place or back keeps working; what measures a column's width takes it off.
  `set_line_numbers` is for the whole app: the layout reads it when it is built.
- `Node::fold` is how many lines a column of the card holds before the text goes
  on in the next column: `FOLD_LINES` for every file, unless `SourceFile::fold`
  says less (`rows_per_column`). The shader gets it per file in the fourth value
  of its origin. It is what puts the two sides of a diff next to each other. On a
  wrapped card (`Node::wrap`, below) it counts *rows*: those of its tallest
  column.
- `FOLD_LINES` (2048) must stay a multiple of `CHUNK_LINES` and `BLOCK_LINES`, so
  chunks and blocks never straddle two columns of a folded file. The shader has
  its own copy, see [shaders](#cratescanvas-rendersrcshaders).
- Files longer than `FOLD_LINES` are folded into side-by-side columns, like a
  newspaper, so one huge data file cannot stretch the whole canvas.
- `MAX_COLS` (160): a card grows with its longest line up to that; longer lines
  are clipped, and the rest of one is shown beside its card under the pointer
  or the caret (`rest_of_line`, below), unless they are wrapped.
- **Wrapping** (`set_wrap`, `Prefs::wrap`, off by default) is decided per card
  when the layout is built (`wrap_of`): a text file wider than `MAX_COLS` that
  is not a side-by-side diff gets a `Wrap` in `Node::wrap`, made once from its
  text (read from disk for a moment if it was let go), never per frame. Every
  place-to-cell function of `Node` (`cell_rect`, `cell_rects`, `line_rects`,
  `cell_at`, `visible_lines`, `column_rows`, `first_row`) goes through it, so
  whatever is drawn at a line and column (carets, selections, finds, washes,
  problems, decorations, relation ends, assistant marks) lands on its row
  without knowing. What spans columns must ask `cell_rects`: one rectangle per
  row it crosses. Rows are *virtual*: column `k` of a long file starts at row
  `k × fold`, so the shader's arithmetic and `FOLD_LINES` columns of lines stay
  as they were; a shorter column just leaves rows empty at its foot.
  `Page::show_edit` makes the edited file's `Wrap` again on each edit
  (`Layout::resize_file`, below).
- `rest_of_line` is the geometry of that rest, in pixels of the screen: rows that
  start where the card cuts the line (`Node::shown_cols`) and go on to the right
  edge of the canvas, wrapping under the first row when the rest is wider. With
  less than `REST_MIN_COLS` of room beside the card it sits under the line (over
  it when there is more room above), against the right edge. It never leaves
  the canvas: a line not wholly on it gets none, and rows that do not fit are
  dropped. `RestRow::text_left` is where column 0 of the line would be drawn for
  that row, so each row is the line's own shaped text moved left by a whole
  number of columns.
- Paths in `LayoutFilter` are relative to the repository root with `/` separators;
  the root itself is `""`.
- A chain of single-child directories is one box labelled `a/b/c`. `Node::rel_path`
  is the deepest directory of the chain. `Layout::dir_node` maps every directory of
  the chain, and every directory inside a collapsed box, to the node that shows it.
  Consequence in the app: a merged chain may have been collapsed through any of its
  folders, so `App::apply` looks at the whole chain.
- `Layout::file_anchor[file]`: the card, else the collapsed folder that contains
  the file, else `NONE` (hidden or filtered out). `Layout::anchor` spreads the
  sites of a collapsed folder with a hash of the file, so the spots are
  deterministic.
- A collapsed box grows with what it holds, but stays much smaller than expanded:
  its side is `sqrt(lines) × 6`, clamped to 140..1600.
- `Layout::groups(min_size, keep)`: for every node, the node it is drawn into
  when dependencies are bundled: the outermost folder whose box is smaller than
  `min_size` on both sides holds everything in it; anything else is itself. The
  root and the folders around `keep` (the selection) never fold, so a selected
  file and its neighbours are seen one by one. One pass over the pre-order.
- Children are packed tallest first, which packs best; ties keep the alphabetical
  order. A file's slot includes the label strip above its card, and `Layout::hit`
  counts that strip as part of the card.
- **One file at a time.** Every node keeps the size of its place in its folder
  (`Node::slot`, the label strip included) and where that place is from the
  folder's corner (`Node::offset`), so a folder can be packed again from its
  children's slots alone (`pack`, which `lay_dir` uses too) and a node placed again
  from its parent with the very additions the full build makes: the results are
  equal to the bit, which the tests of `updates.rs` compare.
  - `Layout::resize_file` (a file's text changed): the card (or the collapsed box
    it is counted in) takes its new size; if that size is the same nothing else
    moves (only `lines` up the chain change). Otherwise its folder is packed again
    (`repack`: its children in the order `lay_dir` would see them, dirs then files),
    then the folder above, and so on until a folder keeps its size. Then, from the
    top, the chain and the children that changed place are placed again with
    their subtrees. Node order is **not** changed: a card that grew taller than its
    neighbours keeps its index, though the full build would put it first. Nothing
    depends on the order of siblings (they never overlap), and selections, the
    hovered node and the scene's ranges stay valid. Compare layouts by path
    (`Layout::differences`), not by index.
  - `Layout::insert_file` / `remove_file` (a file made or deleted, the repository
    already changed): `lay_dir` runs again with a `Relay`: the folders on the
    file's path are laid out again, every other folder is the old node it was
    (`kept`, with its slot, its counts and the part of its name that is its own
    chain), and so is every other card of those folders. `Flat::keep` moves the
    kept subtrees into the new list, renumbered, with file numbers moved past the
    one made or deleted. `dir_node` and `file_anchor` are carried over by mapping
    old nodes to new ones. It costs moving every node once (a few ms for 30,000)
    and packing the folders on the path; no card is measured again. It returns, for
    each new node, the old one it was (`NONE` for new), which is what the scene
    needs.
  - With `only_files` set (only changed, only connected, a commit shown) the file
    numbers in the filter are the caller's, so these return `None` and the caller
    builds everything again.

### crates/canvas-core/src/refs.rs

- `SymbolIndex` is two sorted arrays per kind of name (`Named`): the hash of each
  name and, beside it, the symbol. A name's symbols are the run of its hash, found
  by bisection; no string is kept and nothing is allocated per name, which is what
  made building it slow (0.64 s for 3.3 M symbols, 0.17 s now, on all cores). Two
  names with the same hash are told apart when it is built and kept in
  `clashing`, by name; a name that is asked for and *defined nowhere* could in
  principle meet another name's hash and be given its symbols.
- **One file at a time.** A file's names are taken out and put in without building
  the index again (`SymbolIndex::replace_file`, `insert_file`, `remove_file`): a
  name taken out of the sorted arrays is only marked (`Named::gone`, positions), a
  name put in goes to a small sorted list beside them (`Named::added`). `defs` and
  `impls` therefore give a `Cow`: the slice itself while nothing changed for that
  hash, otherwise a merged copy. Past `COMPACT_AT` (16,384) changes, and before
  file numbers move (a file made or deleted), both are folded back into the arrays
  in one pass (`compact`). A file whose names are the same as before (typing in a
  body) changes nothing at all.
  - Should a name put in meet the hash of another name, or a hash in `clashing`, the
    index is built again whole: rare, and it keeps the clash rule exact.
  - `SymbolIndex::tidied` is the index with every change folded in, equal to one
    built from the same files; the tests and the `count` step compare them.
- `relations` skips a file whose `idents` filter rules the name out; `search_text`
  one whose `threes` filter rules the phrase out, and once enough hits are found
  it stops reading files it would have to read from disk. Both are exact for
  files whose text is in memory.
How usages are bound to definitions: see "Symbols and relations" in
[ARCHITECTURE.md](ARCHITECTURE.md).

- `MAX_PER_KIND` = 1500 relations kept per symbol and kind; `Relations::truncated`
  says some were dropped.
- `Repo::resolve`: mentioning a candidate's file stem (`./atlas`, `atlas::`) is a
  strong hint, but it is only checked when there are at most 16 candidates. Ties
  are broken deterministically: the earlier file wins.
- `Repo::qualifier`: only a type-like qualifier says anything, that is `X::name`
  or a capitalised `X.name`; `foo.bar()` has an unknown type. `Self`, `self` and
  `this` become the enclosing type's name.
- `is_call`: names the language-agnostic tokenizer tags as keywords (`new`,
  `default`, …) count as callees when they are qualified (`X::new(`, `x.default(`).
- `Repo::relations`: definition sites, of the target or of a namesake, are not
  usages.
- `Repo::sym_line_start` is the indentation column of the definition's line, zero
  width. Curves and name chips attach there so they do not run over the text in
  front of the name (`pub fn`, `export function`).
- `Repo::search_text` is smart case (an all-lowercase query ignores case), returns
  at most `max` hits in file order, and reports columns in characters, not bytes.
  `find_all` with `ignore_case` needs `needle` already in lower case.

### crates/canvas-core/src/scan.rs

Text on demand

- A `SourceFile` does not always hold its text. `text()`, `line()`, `line_starts()`
  and `tokenize_chunk()` read the file again when it is not there (`loaded`, a
  `OnceLock`, so any thread may be the one that reads it); `peek` looks at the
  text without keeping it, which is what a search over every file must do.
  `line_count()` never needs the text.
- Text read again is forced to the number of lines the scan saw (`with_lines`):
  if the file changed on disk a moment ago, everything that indexes by line still
  finds a line there until the watcher reloads the file.
- `root` is `None` for text that was given by hand (`Repo::replace_source`: what
  is being typed, a diff): that text is all there is, so it is never let go.
- `Repo::trim_text(keep)` lets go of the least recently used text until three
  quarters of `keep` are held. Each file it lets go first gets its two filters,
  made from the text it still has: `idents` (every identifier, for `may_contain`)
  and `threes` (every three bytes in a row, lower case, for `may_hold`). They can
  only say "not here". While a file has no filters both answer "maybe", which is
  right: its text is in memory. Making them at the scan would cost every folder a
  tenth of its load time for something only huge ones use.
- `LOADED_BYTES` counts the text held by every folder; the app compares it with
  the budget (`page::text_budget`: 768 MB, or `CODE_CANVAS_TEXT_MB`) every two
  seconds, and a scan trims before it hands the folder over.
- Symbol names are kept beside the text (`names`, `name_at`, `implemented`), so
  the index is patched after every save without reading a file, and what draws a
  symbol's name does not read its file. `Symbol::name_start` still points into the
  text: the relations compare it with token offsets.
- `Repo::warm` reads the text of files that were taken from the store, a batch at
  a time, while the app is idle (`App::poll_background`, only for folders whose
  text fits the budget): a moment after opening, a folder is as it was when every
  file was read at the scan. Until then a file changed from outside has no "text
  before" to compare with, and the canvas goes to its top instead of to the line.
- `ScanOptions::cache` is where what the scan finds is kept (`store.rs`);
  `reuse: false` reads everything and only writes. `Repo::reused` is how many
  files were not read; `Repo::kept_deps` their imports, for `deps::analyze_keeping`.
The phases are in "Loading a folder" in [ARCHITECTURE.md](ARCHITECTURE.md).

- `scan` leaves **two cores free**: a scan of a huge folder must not starve the
  window that shows its progress, or anything else on the machine.
- `Found` carries the file size only when the directory listing gives it for free.
  It does on Windows; asking again costs one system call per file, which adds up
  to seconds. Elsewhere no extra call is made either. `read_sized` then reads the
  file with one open and no question about its size.
- `BINARY_EXTENSIONS`: files that are never text become a card without being
  opened at all.
- Paths are sorted component-wise (like a file explorer) after the walk, so the
  layout does not depend on thread timing. `Repo::files` stays sorted by path,
  which is why `Repo::file_index` can be a binary search.
- `SourceFile::rel_path` always uses `/`. `SourceFile::text` is normalised: `\n`
  line endings, tabs expanded to `TAB_WIDTH` (4) spaces. `normalise` does not let a
  trailing newline create an extra empty line.
- `clean_path`: `canonicalize` on Windows returns `\\?\C:\…`; the prefix is
  stripped.
- `Repo::replace_source` (an edit not saved yet) deliberately leaves the symbol
  index as it was. Every file put in place of another goes through `set_file`,
  which keeps the version the index last saw in `Repo::stale`; `reindex_symbols`
  takes those files' names out and puts the new ones in (`SymbolIndex::replace_file`)
  and gives their numbers back, so the caller can resolve their imports too. The
  app calls it when the typing pauses (after 700 ms); `reload_file` calls it at once.
- `Repo::add_file` and `Repo::remove_file`: one file made or deleted without a scan.
  `files` stays sorted (the new one goes where a scan would put it), the numbers in
  `tree` after it move by one (`shift_tree`), a new folder is put among its
  siblings in the order the scan makes, and a folder left empty is taken out
  (`unhang`). Both flush `stale` first, because the symbol index moves its file
  numbers. `kept_deps` follows `files` while it is not empty.
- `Repo::files_under(dir)`: the files of a folder are one run of the sorted list,
  right after the folder's own path (component order puts `a/b/x` before `a/b-c`).
- `LOAD_PROFILE`: CPU nanoseconds per loading step, summed over all threads, as
  `[stat, read, normalise, analyse]`. Only `scan_bench` reads it.
- `ScanOptions::skip_files` are lockfiles: huge and rarely useful.
- `SourceFile::serial` is a number no other `SourceFile` made by this run has.
  Every new text (`replace_source`, `replace_shown`, a scan) makes a new
  `SourceFile`, so the serial says "this very text" without hashing it: tokens a
  grammar made are only used while it matches.

### crates/canvas-core/src/store.rs

What a scan found out about each file, kept on disk so the folder opens again
without reading the files that did not change.

- A file is taken from it when its **size and modification time** are what they
  were (`SourceFile::stamp`, from the directory walk, which costs no open). Then
  its `SourceFile` is made with no text (`SourceFile::kept`): everything the far
  views, the layout, the symbol index and the two filters need is in the record,
  and the text is read when it is first asked for.
- `made_with` is written at the start and must match: the format number, the
  crate's version, the tab width and the limits of `ScanOptions`. **Change
  `FORMAT` whenever what `syntax::analyze` or the symbol scanner produce changes
  between two builds of the same version**, or an old record will be believed.
- A file under `WHOLE` (512 MB) is read in one go, which is three times faster
  than fetching its records one by one; a bigger one never is.
- Layout of the file: the header, one record per file, then a table (path, stamp,
  where its record is) and, last, where the table starts. Records are written as
  they are made, `BATCH` files at a time, and read one by one with positional
  reads from any thread: neither side ever holds the whole file, which for a huge
  folder is gigabytes.
- The imports of a file are kept with it, already resolved to file numbers. They
  are reused only when the set of files is exactly the same (`same_files`, a hash
  of every path): with a file more or less, what a path resolves to may differ,
  and every file's imports are worked out again, which reads every file.
- `keep` writes after a scan in which anything was read, and otherwise only
  touches the file, so `page::forget_old_scans` knows it is in use.
- A record that does not decode is a file that is read. Nothing here can make the
  scan fail.

### crates/canvas-core/src/symbols.rs

Intentionally approximate; exact answers are the job of language servers behind
the same data types.

- The tokenizer is language-agnostic, so `new`, `default`, `type`, `match` … may be
  tagged as keywords where they are plain identifiers. `name_like` accepts them
  after a definition keyword.
- A pending definition can be `tentative`: removed again if the statement ends
  with `;` before any `{` (C prototypes, forward declarations). Or `expirable`:
  dropped at the end of a line that clearly ends the statement. That is for
  JavaScript without semicolons, where a statement that visibly ended must not
  capture a later, unrelated `{`; `type X int` uses it too, having no body.
- Lines inside brackets (continuations) do not close anything.
- `foo(): void;` in an interface is a declaration and is kept.
- C family: `struct foo *make(void) {` is a function returning a struct, not a
  struct. `Type name(args) {` is only recognised outside function bodies, and the
  name must follow a type unless it is a constructor.
- A macro name keeps its bang out of the symbol name (`println!` → `println`).
- `Symbol::body_end` is one past the last line of the body, `line + 1` without a
  body. `Symbol::parent` is `u32::MAX` at top level.
- `symbol_at_line` needs `symbols` in source order. It starts from the last symbol
  that begins at or before the line and walks up the parent chain: anything else
  containing the line also contains that symbol's first line, so it is one of its
  ancestors.

### crates/canvas-core/src/syntax.rs

- `Token` is 12 bytes because large repositories have tens of millions. `len`,
  `col` and `cols` are `u16` and saturate at 65535; only the colouring of such a
  line is affected.
- `Block { lo, hi }`: `hi <= lo` means empty.
- `Analysis::chunk_in_comment[i]` is the tokenizer state at the start of chunk
  `i`. It is what lets any chunk be tokenized on demand without starting from the
  top of the file.
- `tokenize_range` expects normalised text (no `\r`, no tabs).
- These tokens are the app's own. A file whose language only an extension's
  grammar knows (or any language, with `grammars_everywhere`) is coloured by that
  grammar instead, through `TokenCache` (see `cache.rs`); everything else that reads
  tokens (`analyze` and its blocks, symbols, `refs.rs`) keeps using these.
- `dominant_kind`: most character cells wins, but comments and punctuation only
  win when little else is there.
- A block comment's opener is consumed before looking for its close, so `/*/` is
  not taken as open plus close. In Rust, `'a` is a lifetime and `'a'` a char.

### crates/canvas-core/src/wrap.rs

- `Wrap` is where the rows of a wrapped file begin: for each line, the column
  each further row starts at and how far it is indented (`breaks`), and the
  first of them (`first`, an index per line, so a line's row is found without a
  search). Only lines wider than `width` have any. A card numbers *lines*
  everywhere else; this is the one place that knows rows.
- A row breaks after a blank, a comma or a semicolon in its second half, else
  at the width. Rows after the first are indented as the line is, plus
  `WRAP_INDENT` (2), never more than a quarter of the width.
- `place` (line, column → row, column on screen) and `line_at` + `col_at` (back)
  are exact inverses for every column of a line (tested). A column of a row's
  indent is the row's first character; past the end of a row that is not the
  last is the start of the next.
- `pieces` cuts a span of a line into one piece per row; it starts at the row
  of the span's first column (a minified line has thousands of rows, and it is
  called for every token bar each frame).
- `step` is Up and Down by rows: the goal is a column *on screen*, so the caret
  stays under where it was as it goes through the rows of one line.

---

## canvas-render

### crates/canvas-render/src/lib.rs

- Line numbers are one shaped block per chunk, shared by every file: lines 129 to
  256 read the same in all of them. A file with `SourceFile::numbers` (a card
  showing a diff: the line of the real file each row is, 0 for none) gets blocks
  of its own, and so does a wrapped card (`CodeText::wrap`): a number, then a
  blank for each further row of its line.
- Code text is shaped in chunks of `CHUNK_LINES` lines that know nothing about
  columns, so each column is clipped to its own rows as well as to its width: a
  chunk that straddles the end of a column would otherwise show its next lines in
  the padding under it.
- One render pass, in this order: frames (directory boxes, file cards) → `under`
  rectangles (symbol bodies, outline regions) → code bars at the current `Lod` →
  text (code chunks, file and folder names) → relation curves → `over` rectangles
  (hover, selection, usage markers) → chips (screen-space rectangles and their
  labels).
- **Several panes share one frame, and the calls share buffers.** Each
  `Renderer::render` call needs an encoder of its own that is submitted before the
  next call. `Renderer::end_frame` is called once per frame, after the frame's
  encoder was submitted.
- `Pane::clear` is for the first call of a frame. `Pane::curves` is for one page
  only: the curves on the GPU belong to one page.
- `Renderer::set_curves` replaces the curves; they stay on the GPU until replaced,
  so call it only when they change.
- `ScenePass::clip` keeps code text from bleeding over the UI bars; the scissor
  keeps a page inside its pane and inside the target.
- When `theme::revision` moved, `render` uploads the syntax colours again and
  clears the shaped text, which carries the old ones.

### crates/canvas-render/src/scene.rs

The performance model (static frames and blocks in pre-order, per-frame lines and
tokens, 12-byte file-relative instances) is in "Rendering" in
[ARCHITECTURE.md](ARCHITECTURE.md).

- `MAX_DYNAMIC_INSTANCES` = 4,000,000 per-frame instances: a safety valve.
- `SceneData` is pure data so it can be built on the scan thread; only the upload
  (`SceneGpu::new`) runs on the UI thread.
- **Instances name a slot, not a file.** `CodeInstance::file` is an index into
  `origins`, and `SceneGpu::slots[file]` says which (the file's own number when the
  scene is built). A file made gets a free slot or a new one at the end; a file
  deleted gives its slot back. So when file numbers move, the block instances
  already on the GPU stay right, and `prepare` maps the file to its slot for the
  instances it makes each frame.
- **One file at a time** on the GPU:
  - `SceneGpu::card_changed` (after `Layout::resize_file`): the frames of the nodes
    that moved are written where they are, and their origins (the span of slots
    they touch). The card's blocks are written in place when there are as many as
    before; otherwise a new block buffer is filled by copying what is before and
    after the card on the GPU (`copy_buffer_to_buffer`, it needs `COPY_SRC`) and
    writing the card's blocks between (`splice_blocks`), and `block_start` moves.
  - `SceneGpu::relaid` (after `Layout::insert_file` / `remove_file`): frames and
    origins are written whole (32 and 16 bytes a node: about a megabyte for 30,000
    files), and the new block buffer is copied from the old one in runs of nodes
    that were kept, in their new order (`move_blocks`); only new cards are made on
    the CPU. A buffer cannot be copied into itself, hence a new one each time the
    count changes.
  - `Moves` is what both write through: the GPU, or a `Vec` in the test that checks
    them against a scene built again.
- `push_lines` merges tokens separated by small gaps into one bar.
- On a wrapped card the instances carry rows, not lines: `push_bar` cuts a bar
  into one per row (`Wrap::pieces`) and `push_blocks` stretches a block over the
  rows of its lines, in pieces of at most 255 (the span has 8 bits). The shader
  is the same.

### crates/canvas-render/src/shaders

- `code.wgsl`: the constant `FOLD_LINES` **must match**
  `canvas_core::layout::FOLD_LINES`. `BAR` = 0.72 is the fraction of the line
  height a bar covers. `origins[file]`: `x, y` the first character of the file,
  `z` the x distance between folded columns.
- `rect.wgsl`: rectangles thinner than a pixel stay one pixel wide but fade out by
  their real coverage. Geometry is grown by 1 px for anti-aliasing (`curve.wgsl`
  too).
- `curve.wgsl`: vertices carry a world position plus a normal; the ribbon is
  extruded in screen space, so its width stays constant in pixels at any zoom.
  `along` (world distance from the start) times the zoom gives dashes of a
  constant length in pixels; `dash` 0 is a solid line. A vertex with `arrow` > 0
  belongs to an arrowhead: its `half_width` and `shift` are in arrowhead lengths,
  and that length is `arrow` × zoom clamped to 3.2..7 times the line's width (at
  least 6 px), the width travelling in `dash`. So a head is never huge nor lost,
  and it needs no new tessellation when the zoom changes.

### crates/canvas-render/src/curve.rs

- `control_points`: a horizontal S when the ends are side by side, an arc bulging
  to the left when they are stacked (two places in the same file or column).
- `push_curve`: neighbouring quads share their edge exactly, so there are no gaps
  and no overlap. A degenerate curve produces nothing.
- `push_edge` is `push_curve` with a `Stroke`: dashed, and an arrowhead at `b`
  pointing the way the curve arrives (a tapered ribbon: anti-aliased like the
  line). `arrow_px` is the shader's sizing, kept here for its test.

### crates/canvas-render/src/gpu_util.rs

- `DynBuffer` grows to the next power of two when the data does not fit.
  `write_buffer` needs a multiple of 4 bytes; our instances always are.

### crates/canvas-render/src/text.rs

- Shaping is the expensive part, hence `NEW_CHUNKS_PER_FRAME` (see "Text" in
  [ARCHITECTURE.md](ARCHITECTURE.md)) and `TextLayer::pending`, which keeps the
  app redrawing.
- `EVICT_AFTER` = 300 frames an unused cache entry survives.
- `TextLayer::clear` exists for theme changes: colours are part of the shaped
  text. `TextLayer::drop_page` forgets a page's chunks; the app calls it when a
  page closes and before it shows new contents of a file.
- `CodeStrip` (the rest of a long line) draws the chunk the canvas already shaped
  once more per row, at the same zoom, with the row as its bounds: the glyphs
  and colours are the canvas's own. It goes with the interface text, after the
  chips, so it is over the strip's background. Nothing is drawn while the chunk
  is not shaped yet.
- A wrapped chunk is shaped with a line break and the row's indent put in where
  each row starts (`cuts_of`, `Spans`), cutting a token in two when it must; the
  tokens are the line's own, so a string or a comment keeps its colour on every
  row. The chunk is drawn from the row of its first line. Turning wrapping on or
  off drops the shaped text of every page (`Action::SetPrefs`).
- `TextLayer::drop_chunks` forgets some chunks of one file: those a grammar has
  just coloured, which were shaped with the app's own tokens.

### crates/canvas-render/src/theme.rs

- Every colour is `0xRRGGBBAA` in sRGB. The surface is non-sRGB, so values go to
  the screen untouched (the same blending as most UI toolkits).
- **Colours get baked** into GPU buffers and shaped text. Whoever keeps such a
  copy compares `revision()` with the one it was built at. `set` does nothing when
  the colours are the same, so the revision only moves on real changes.
- The `theme!` macro declares `Theme`, the built-in themes `DARK` and `MIDNIGHT`,
  and `FIELDS` from one table: `key: "Name" = <dark>, <midnight>;` under a group
  heading. `Field::key` is the name in `theme.json`.
- `PRESETS`: `(key in the settings file, name on screen, colours)`; the first is
  the default.
- `Theme::token_colors` has 16 entries because the shader's palette has 16.
  `Theme::rel_colors` is indexed by `RelKind::index()`: definitions,
  implementations, references, incoming calls, outgoing calls. Directory borders
  cycle through four colours by depth.
- `get()` is cheap, but take it once per function, not per item.
- `parse_hex` reads `#rgb`, `#rrggbb` and `#rrggbbaa`, the `#` optional. `to_hex`
  writes `#rrggbb` or `#rrggbbaa`.

---

## code-canvas

### crates/code-canvas/build.rs

- Embeds the icon and version info into the Windows executable. The icon is
  cosmetic: failing to embed it is a warning, never a failed build.

### crates/code-canvas/exthost/host.js

What the host is, the mapping of VS Code's ideas onto the canvas and how to drive
it without the app: [EXTENSIONS.md](EXTENSIONS.md).

Protocol

- One JSON object per line on stdin and stdout. **stdout belongs to the
  protocol**: `process.stdout.write` is replaced, and whatever extensions print
  becomes a log line.
- Incoming messages run one at a time, in order: documents must open before
  requests on them. Messages of type `request` are the exception: they may take
  long (a formatter starting up) and do not hold the queue.
- Anything an extension asks for that the API lacks gets a harmless stand-in
  (`permissive`, and the `Proxy` that `require('vscode')` returns) and is reported
  to the app once, as `unsupported`.
- Positions cross the wire as line + **display column**: tabs expand, one cell per
  code point, the second half of a UTF-16 pair does not count. `displayColumn`,
  `characterAt` and `positionAt` convert. Locations are sent back the same way, so
  the app can point at a place without the file's text.
- Colours cross the wire as `#rrggbbaa` or `theme:<id>` (`colorName`).
- A message with choices waits in `pendingMessages` until the user picks one or
  dismisses it (dismissed: no action).
- What a command returns stays in the host: it is rarely something that can be
  sent.

Behaviour

- `scope` (an `AsyncLocalStorage`) says which extension is running and follows
  async continuations. Event listeners remember who subscribed, so what a listener
  does is attributed to its extension.
- `fileKey` is the same for a file however its path is spelled; documents are
  found by it.
- `applyEdits` takes edits in any order; an overlapping edit is skipped rather
  than corrupt the text.
- Settings: on top of what [EXTENSIONS.md](EXTENSIONS.md) lists, the user's own
  VS Code `settings.json` is read as a base when VS Code is installed (the app
  sends its path as `vscodeSettingsFile`), so formatters and linters behave here
  as they do there. The host's own defaults make formatting work with no set-up:
  `editor.formatOnSave` is on and Prettier formats where it is installed and knows
  the language; prose is left as typed. `parseJsonc` accepts comments and trailing
  commas.
- A format request made by saving is skipped when `editor.formatOnSave` is off.
- Language of a file: the longest matching extension wins (`.d.ts` before `.ts`).
- `reloadDocument`: when the text typed here is now what is on disk, the document
  keeps its text and only stops being dirty. `editDocument`: the document's text
  is what the app holds, not the disk's.
- Activation: a `workspaceContains` pattern with a glob is assumed to match rather
  than walk the tree. `activateForView` starts the extension that contributes a
  view (`onView:<id>`, which VS Code implies for every view of a manifest);
  `resolveWebview` calls it because nobody has registered the view while its
  extension has not started.
- `setContext` keys are forwarded: the app keeps them and evaluates the `when` of
  each view. `vscode.open` and `revealInExplorer` are forwarded too: the app shows
  the file (a picture in its viewer, code on the canvas).
- `installVscodeModule` makes `vscode` resolvable for both `require` and `import`;
  bundles do either.
- Not wired yet: file system watchers exist but never fire.
- **Code lenses.** `codeLenses` asks every provider of the file (`askUntil`, 10 s),
  resolves those that came without a command (`resolveCodeLens`) and answers plain
  rows with an id; the lens objects stay in `lensTable`, because their arguments
  (Uris, positions, a language server's own objects) cannot cross the wire.
  `runLens` runs one by its id. Only the last answer of a file is kept
  (`lensesOf`): a lens of an older answer is gone, and so are a closed file's.
  `wait` lets a file being typed in rest first; a newer request for the same file
  makes the waiting one answer `superseded`, so typing asks once.
- `lensesChanged` is sent (at most every 200 ms) when a lens provider is registered
  and when one fires `onDidChangeCodeLenses`: the app asks again for what it shows.
- `editor.action.showReferences` and its kin (`SHOW_USES`) are what "N references"
  runs: the host sends `showUses` with the place, and the canvas selects the name.
- **Tree views.** `createTreeView` and `registerTreeDataProvider` both make an entry
  in `trees` and say `treeView`. `treeChildren` asks the provider for the children
  of a handle (none: the root) and gives each one a handle: `i/<id>` for an item with
  an id, otherwise its parent's handle, `/` and its label (`#2`… for a repeat). The
  same element gets the same handle when the tree is asked again, so the app keeps
  what was open. The first time the root is asked the view becomes `visible`.
  `onDidChangeTreeData` becomes `treeChanged` for the whole view (the app asks again
  for every list it has); setting a view's `title`, `description` or `message` sends
  `treeInfo`. `treeClick` runs a row's command and selects it, `treeExpand` fires
  the expand and collapse events, `treeCommand` runs a button's command, with the
  row's element as the argument for a row's button. A tree asked for before its
  extension has started starts it (`activateForView`); `missing` means nothing
  registered it even then.
- Webview pages are served by the app from `RESOURCE_ORIGIN`
  (`https://ccres.localhost`); the host only owns the object the extension talks
  to.
- `contentChange`: a change event carries the smallest span that was replaced
  (common start and end trimmed, never cutting a `\r\n` or a surrogate pair). A
  change whose range reaches past the end of the document is dropped in silence
  by language servers (rust-analyzer does), and nothing typed would be checked.
- **When the app is gone the host must stop writing.** `appGone` is reached by a
  closed stdin, a failed write, an error on stdout or the parent process no
  longer existing (`startedBy`, looked at every 3 s: a killed app does not always
  close the pipe). From then on `send` writes nothing, the extensions are asked
  to stop, and a `send` more than 3 s later ends the process on the spot. Before
  this, an extension that kept logging into the broken pipe had every failed
  write reported as an error, which was written to the same pipe: a loop that
  starves the timers, so the host stayed behind using a whole core.
- `closeWebview` disposes a webview panel as VS Code does when its tab is closed:
  the extension hears `onDidDispose` and stops what it started for it.

### crates/code-canvas/src/main.rs

- Usage: `code-canvas [FOLDER ...]`. Without arguments the previous session is
  restored.
- `code-canvas --install-extension <publisher.name | file.vsix> ...` installs
  without opening a window. The outcome goes to the log; the exit code is the
  number of failures.
- Release builds on Windows have no console (`windows_subsystem = "windows"`), so
  logs go to a file (`Tee` also writes to stderr when there is one) and the panic
  hook logs the panic and tells the user. Without it a panic would vanish
  silently.
- `update::clean_up()` runs at start: it removes what an update left next to the
  program.
- The event loop proxy is the waker background work uses to wake the UI thread.

### crates/code-canvas/src/app.rs

Showing a path

- `App::show_path` is the one way a path reaches the canvas, whoever names it (an
  extension's `showTextDocument`, a definition, a link, a row of the explorer). A
  file on a canvas is travelled to, and to the line when one comes with it. A file
  inside an open folder that has no card (git ignores it) is added to that
  folder's `Page::extra` and the folder is scanned again. A file outside every
  open folder gets a page of its own kind: `Page::of_files`, rooted at the file's
  folder but with only the files asked for (`ScanOptions::only`), so naming one
  file of a home directory does not read the home directory.
- Such a page is not a place of work: it is not kept in the session, its folder
  is watched without its subfolders, new files in it are not news, extensions are
  not told about it and its scan is not kept on disk.
- `show_when_read`: the path waits here while the folder it is in is being read,
  and is shown when the scan arrives (`poll_pages`).
- The host answers `showTextDocument` with an editor even for a file the canvas
  has not opened, because extensions go on to call `revealRange` on it; that call
  is what carries the line.

Pictures and what plays

- Nothing of a folder opens beside the canvas: a picture is drawn in its card and
  a video, a sound or a PDF plays in its card. The side panel that used to show
  them only reads Markdown now.
- Media files get a card of a fixed size from the scan (`scan::media_kind`,
  `MEDIA_LINES` by `MEDIA_COLS`): their own size is not known without opening
  them, which a scan must not do, so the picture is fitted inside.
- `pictures.rs` reads a picture on another thread the first time its card is tall
  enough to be worth it, never more than three at once, shrunk to 1024 pixels on
  its longer side, and keeps the 96 most recently drawn. They are painted by egui
  under everything it draws but over the canvas, clipped to it: what the canvas
  draws over a card (curves) goes under a picture.
- The cameras move before the interface is built in `App::redraw`, not after as
  they used to: pictures are placed from the cameras, and a frame late they
  would lag behind their cards while the canvas travels.
- What plays is one webview (`media::PLAYER`) placed over the card each frame
  (`App::player_place`), hidden and paused when the card is too small or off the
  canvas. It is a window of its own: it covers the toolbar where they overlap.
- **The player is the whole card, cut at the edge of the canvas.** The window of
  the player has the place and the size of its card even where the card goes
  beyond the canvas, and what is beyond is cut out of it like anything that
  floats over a page (`beyond` in `App::redraw`). It used to be only the part of
  the card in view, and the video fitted itself into that part: zoomed in, or
  with the card half out, the video was somewhere else than its card and changed
  size as the canvas moved. The video fills its page (`object-fit: contain`), so
  a small one grows with its card too.
- **While its card changes size a still stands in for what plays**
  (`App::stand_still`, `Still`). A browser window that is resized draws itself
  at the new size some frames late, and a zoom is a resize every frame: the
  video trembled and stretched. Now, when the card changes size, the page is
  asked for a picture of what it shows (`media::ASK_STILL`, sent back as a
  `PageNote::Still`); as soon as it comes the window is veiled
  (`WebViews::veil`: all of it cut out, yet still there and drawing) and the
  picture is painted in the card by egui, like the pictures, asked again every
  150 ms. A quarter of a second after the last change the page is asked to say
  when it has drawn itself (`ASK_SETTLED`, two frames of its own), and only then
  is the veil lifted; the picture stays under it a moment more. A card too small
  for the player keeps the picture. The wheel over a veiled player is the
  window's own, so the cursor is noted from the page's wheel before.

Saving and new builds

- **What is typed is saved by itself** (`Prefs::autosave`, on by default,
  "Save while typing" in Settings): 0.7 s after the last keystroke every
  unsaved file is written (`App::typed`, `save_some(.., quietly)`), without
  formatting. A file another program changed meanwhile is left for the person
  to decide (Ctrl+S asks), never overwritten quietly.
- **A new build put in place of the running one is picked up by itself**
  (`App::watch_own_build`, `own_build`): every 5 s the app looks at the size
  and time of its own .exe (the loop wakes for it even when idle). When they
  change, the status bar offers "Restart into the new build"; the person
  chooses when. It never restarts by itself: an assistant's chat running
  inside the app would be cut (it was tried, and it cut the person's
  conversation each time). Script and bench runs never look.

Drawing over the canvas

- The toolbar has three tools: the pointer (edit, what the pencil used to be),
  the hand (move) and the pencil, which draws. The pencil is not a preference:
  `UiState::pen` (`doodle::Pen`) is on until another tool is taken, and the app
  starts without it. With it the left button draws and the right one still
  moves the canvas; the wheel is as always.
- What is drawn belongs to the canvas, not to the screen: a `doodle::Stroke`
  keeps its points and its width in the canvas's own measure (`App::ink`), in
  `Page::doodles`, so it travels and grows with the code it points at. It is
  never thinner than a pixel. It is not saved: it lasts while the folder is
  open, like a scribble on a whiteboard during a call.
- They are painted by egui on the canvas, after the pictures
  (`App::place_doodles` turns the ones in view into points of the screen each
  frame). A page that is a window of its own (a video playing, an extension)
  is over them.
- The bar of the pencil (`ui::pen_bar`, over the toolbar while the pencil is
  taken) has six colours, three widths, the rubber and the bin. The rubber
  takes away what it passes over and no more (`doodle::rub`): a stroke rubbed
  in the middle becomes two, and a long straight stretch is first cut into
  short ones near the rubber so that only the part under it goes. The bin
  clears every canvas in view.

The rest of a long line

- `App::long_line_now` picks the line each frame: the caret's when it is beyond
  the cut, else the one under the pointer, else the one shown while the pointer
  is inside its strip (following the line to the right takes the pointer off the
  card and onto the strip, which must not close it). `update_hover` redraws when
  the pointer crosses the strip's edge.
- Under the pointer it waits for the canvas to be still for `LONG_LINE_STILL`
  (no travel, no zoom settling, no drag, the same camera): while the canvas
  moves the line under the pointer changes every frame, and strips would flash.
  `long_line_waits` asks `poll_background` for the redraw that shows it after.
- `caret_cell` gives the caret's place in the strip when there is one, so the
  completion list opens there and `keep_caret_visible` does not move the canvas
  to the clamped caret at the edge.
- A click in the strip is a click on its line: the caret goes to that column
  (`RestOfLine::col_at`) and its card is selected, whatever card is under the
  strip. A drag that starts in it moves the canvas (`code_under` finds no code
  there).

Back and forward

- A `Place` is not a camera: it is the node under the middle of the canvas, where
  the camera was from that node's corner, and what was selected. After files grow
  or the layout changes the node is somewhere else and the place still means the
  same code.
- `remember_place` is called by whatever makes the canvas jump (`frame_node`,
  `frame_site`, `frame_selection`, `frame_every_folder`), never by panning or
  zooming by hand. Jumps the app makes by itself (following an outside change,
  walking the matches of a search) are remembered once in a row, or they would
  bury the place the user was at.
- `Ctrl+Z` and `Ctrl+Y` are these two only while no file has the caret: the editor
  takes them first for its own undo and redo.

The frame and the resize rules are in "The app" and "Windows, resizing and the
graphics API" in [ARCHITECTURE.md](ARCHITECTURE.md). What follows adds to them.

Graphics and the frame

- **Never reconfigure a surface while one of its textures is held.** In
  `App::frame` a `Suboptimal` surface texture is still usable (the window is being
  resized and the size event has not arrived yet) and must be used as it is:
  configuring there is an error that takes the app down. The resize event
  reconfigures it.
- `Gfx::resized`: a size change is only recorded and applied once at the start of
  the next frame. While an edge is dragged many sizes arrive between two frames,
  and each reconfiguration costs 15-30 ms.
- `Gfx` field order matters: `window` is dropped last, because the surface must go
  before the window.
- `device.on_uncaptured_error` logs GPU errors. By default such an error ends the
  program; here it costs a frame at most.
- The surface format is the first non-sRGB one: colours are authored in sRGB and
  blended like most UIs.
- **egui's texture updates must not be lost when a frame is skipped.** New and
  changed textures go to the GPU right away, whether or not a frame follows. The
  ones egui no longer needs are freed after the frame that may still show them, or
  right away when there is no frame. The delta is taken out of the egui output
  (`full`), which checks that none is forgotten.
- **Several folders share one canvas.** Each page keeps its own world and its own
  camera, but only the active page's camera is moved by the user: every frame
  `App::follow_active_camera` copies it to the others, shifted by where each folder
  sits (`camera::origins`: left to right, tops aligned, a gap of 6% of the widest
  folder and at least 240 world units). So a point of the shared world is at the
  same place on screen for every page, and nothing else in the app has to know.
  - `Page::extent` is the last known size of a folder. It is kept while the folder
    is scanned again, so its neighbours do not jump during a rescan.
  - A camera animation (`Page::target`) belongs to the active page only;
    `App::switch_to` hands it over to the page that becomes active.
  - `App::focus_folder_at` makes the folder under the pointer the active one (a
    press, the wheel, a pinch). `App::show_folder` (the folder menu, the keyboard)
    also flies to it when none of it is on screen.
  - `Fit` frames every folder (`App::frame_every_folder`); the other framing
    commands stay within the active one.
  - The folders beside the active one are rendered first, each with an encoder of
    its own (see [canvas-render lib.rs](#cratescanvas-rendersrclibrs)). Relation
    curves are the active folder's only.
  - Leaving the shared canvas (`UiState::single`) frames the other folders again
    when they are next shown: their cameras were looking at the active one.
- When the canvas's part of the window moves (a panel opens), what was in the
  middle of it stays in the middle (`Page::pane_center`).
- **Questions are asked inside the window** (`ui::Ask`, drawn by `ask_modal`):
  deleting a file, discarding changes, closing with files not saved. A dialog of
  the system stops the event loop until it is answered and can end up behind the
  window, which looks like a frozen app. An `Ask` stops nothing: the action that
  needs the answer puts it in `UiState::ask` and returns, and the answer comes
  back as `Action::Answered(Asked, Answer)`, where `App::answered` does the rest.
  Closing is the one with a state: `confirm_exit` puts the question and says no;
  the answer sets `exit_requested` again, with `exit_confirmed` when the files
  are to be left as they are. Only the panic hook and the pickers of files are
  still the system's, and the pickers have the window as their parent.
- `show_when_read` waits for the page of the file to be read and is only then
  taken: taking it first and looking after lost it whenever the scan ended a
  frame later, and the canvas never went to a file just let in (a video picked
  among the ignored ones, say).
- A description under an option of the settings wraps at the width of its box
  (`settings::option`): in a row laid out sideways a label does not wrap by
  itself, and one long sentence pushed every box of the page out of the window.
- **The wheel moves the canvas, it does not zoom it**: up and down, sideways with
  Shift (down is to the right of the folder, as in any editor), and it zooms with
  Ctrl, at the pointer. A touchpad's two fingers come as the same event with both
  directions and move the canvas as they go; its pinch zooms. `WHEEL_NOTCH` is
  how far one notch of a mouse goes.
- The wheel with the right button held zooms too, with one hand. That press is
  a drag already (`Drag::button`); the wheel marks it as moved, so letting the
  button go does not open the menu of the card. The menu, if it is open, closes
  as soon as the canvas moves by any means.
- Over a video playing in its card the wheel belongs to the page of the player,
  not to the window: the page sends it on (`media::WHEEL`, `PageWheel`) and
  `App::wheel` does with it what it does with the window's own.
- **The workspace** (`App::workspace`, kept in the session as `Known`) is every
  folder that was opened and not forgotten; the pages are the ones of them on
  the canvas. Closing a page takes its folder off the canvas and leaves it in
  the list with the number of files it had. Putting one on turns the shared
  canvas on (`single` off): the point is to have it next to the others. A row
  dragged out of the list is dropped on the canvas if the pointer is over where
  the canvas was a frame ago (`UiState::canvas_was`): the list is drawn before
  the canvas has its place.
- A folder just put on the canvas is framed by its own first frame; all of
  them at once (`PutAll`) are framed together when the last one is read
  (`frame_all_when_read`), not before, when there is nothing to frame yet.
- A file is renamed in its own row of the explorer (`rename_in_place`), the name
  without its extension chosen, as editors do; Enter takes it, leaving the field
  or Esc does not. New files and folders still ask in the box above the list.
- `Put` of the workspace always turns the shared canvas on, also for a folder
  that was already open: with one folder at a time, dropping it only changed
  which one was looked at, which is not what dropping it on the canvas says.
- The offer to connect Claude Code (`offer_claude`) comes by itself a few
  seconds after the app starts, once, when Claude Code is on the computer and
  not connected: a feature that waits behind a button in the settings is a
  feature nobody sees. `--connect-claude` and `--disconnect-claude` do the same
  from a command line.
- **The menu of a right click** (`ui::CardMenu`, `MenuOn`) is for a file, a
  folder or the canvas, whatever is under the pointer, and is there with no
  folder open too. What it offers ends with what is always there: hiding the
  interface and seeing the whole canvas.
- **The interface hidden** (`UiState::bare`): the bars, the panels, the status
  bar and the toolbar are not drawn; the title bar stays, it is how the window
  is moved and closed. Hiding says once how to bring it back.
- **The explorer follows the selection**: when the selected node changes
  (`explorer_shown`), its folders are opened in the tree and the list scrolls to
  its row if it is out of sight (`explorer_seen` is what the list showed a frame
  ago). Only then: a list that scrolled back on every frame could not be
  scrolled by hand. `explorer_reveal` asks the same for a path by name.
- **A question remembers what it was about.** `Asked::Delete`, `Asked::Git` and
  `Asked::Close` carry the folder, and the answer acts on that folder, not on
  the one that is active when the button is pressed. A press on a button of the
  app's own (a question, a menu) does not choose the folder under it
  (`focus_folder_at` leaves what `over_ui` covers alone), and while a question is
  up no key reaches the editor behind it (`on_key`).
- Closing a folder with files not saved asks (`close_folder`): the cross of the
  workspace, taking it off the canvas, forgetting it, the shortcut.
- Discarding drops the unsaved edits of the files git will put back, not of new
  files git does not know (`drop_edits`), and not before knowing git is free.
- **A batch of the watcher is not dropped** when the folder cannot be read again
  right now (a scan just done, one still running): its files are reloaded one
  by one and the scan waits (`rescan_later`). Text that is not saved does not
  hold a scan back any more: the scan keeps those editors (`rescan_quietly`)
  and `poll_pages` puts their text on the new cards. It used to wait for a
  save, so with one file unsaved, or with a guest typing, new and deleted files
  were never seen.
- **What another program writes into a file with unsaved text joins that text**
  (`Editor::take_in`): the editor remembers the text the file had
  (`Editor::on_disk`), what the program did is the edit from that to what is on
  disk now, and it is passed over what was typed here the way an edit of
  someone else in a shared session is (`TextOp::transform`). So an assistant
  can work on the file you are typing in, and a guest's save cannot erase what
  a program wrote at the host. It says so once per file and the file stays
  unsaved. After it, undo never calls the text clean (`saved_at` is none): what
  it would go back to is not what the file has.
  - Only when that cannot be done (the file was deleted, or is not text any
    more) is the file marked (`changed_under`) and saving asks
    (`ask_overwrite`: Save mine, Leave it deleted or Take theirs, Cancel).
    `settle_changed_under` reads such a file again once nothing is unsaved in
    it.
  - A file deleted from the app drops its unsaved text with it: closing the
    folder must not write it back.
- **Undo takes back what was done here.** An edit that comes from elsewhere
  (`Editor::apply_remote`: someone in the session, a program on disk) is not a
  step of undo. It is applied one changed place at a time (`TextOp::splices`),
  and each step of undo that is after or before that place is moved to where
  its text is now (`keep_history`). A step whose own text the edit went into
  cannot be told apart from it any more: that step and the older ones are let
  go. More than 64 places at once are applied as one, for speed. Redo is
  emptied by any edit from elsewhere.
- An edit between two texts that changes as many lines as it removes pairs them
  (`TextOp::lines`): each line keeps what is the same at its start and end, so
  a caret in a line that Replace all touched stays in its line instead of going
  to the end of everything that was replaced.
- Going to a file the canvas has but is not showing (only changed files, only
  connected ones, a commit) shows everything again; it does not scan.
- git is run with `--literal-pathspecs` (a path like `app/[slug]/page.tsx` is
  that path, not a pattern) and the diff a change is cut from does not follow
  the user's `diff.noprefix` or text conversions.
- **A file keeps its bytes where nothing was typed.** Every line of an editor
  remembers its own end (`Editor::ends`), so a file with both kinds keeps both;
  a new line gets the kind most lines have. The mark some programs put at the
  start of a file (U+FEFF) is not text: `scan::normalise`, `share::body` and
  `Editor::open` take it off, so column 1 is the first letter, and
  `Editor::text` puts it back. Saving a file with nothing changed writes
  nothing (`save_file`).
- **Find compares letter by letter** (`App::found_in`): a letter is lowered
  only when that gives one letter, so the columns found are the columns of the
  text (`İ` lowers to two, and lowering the whole line moved every match after
  it). The list shown stops at 5000 matches; Replace all asks again without
  that stop and is one step of undo (`Editor::replace_each`: the lines from the
  first match to the last are rewritten as one change). After replacing one
  match the next one is the first that starts after what was put, so putting
  `aa` for `a` goes on to the next `a` instead of into its own text.
- A click puts the caret at the nearest gap between letters, not before the
  letter under the pointer (`on_canvas_click`, `cell_in`); a double click still
  takes the word under it.
- A path taken from the list of paths closes the list unless it is a folder,
  whose files come next. It used to open again on the name just taken, and the
  next Enter took it once more.
- **The colours are the concept video's** (`theme::DARK`, measured from its
  frames): almost no colour in the interface or on the canvas (a ground of
  `#171619`, boxes of `#0a090c`, a black canvas, grey outlines and names), so
  that colour means something where it shows: green is new, blue is changed,
  red is removed, and the relations keep theirs. Folder borders are greys that
  get fainter with depth, not four hues.
- **Nothing of VS Code's own is read.** The app does not need VS Code installed
  and does not look at its settings, its extensions or its sign-ins: settings
  are `<data dir>/settings.json` and a folder's `.vscode/settings.json` (a file
  of the project, which extensions expect there). It once took the user's VS
  Code settings as defaults; an app of its own does not. There is no account in
  the app either: an extension's `authentication` and `secrets` give nothing,
  the commit message is written by the `claude` or `codex` the person has
  installed and signed into, and an assistant's extension uses that person's
  own sign-in on their computer.
- **Zoom rests on whole pixels** (`Camera::rested`): when the wheel or a pinch
  has been still for a moment (`App::zoom_rest`), and for the zoom buttons at
  once, the camera glides the last bit to where a line of code is a whole
  number of pixels tall and the world's origin is on a pixel, about the point
  that was being zoomed at. Text drawn between pixels is what shimmers. Below
  six pixels a line nothing is read, and nothing is moved.
- **The middle of the canvas stays in the middle** when the window or the
  canvas's part of it changes size (`Camera::keep_middle`). The camera is
  anchored on the window's middle, which already moves by half the change, so
  the move of that middle is taken off the move of the pane's middle; adding
  the whole of it moved the canvas by the full change of size.
  `Page::pane_viewport` is the window size `pane_center` was measured in, so a
  session saved between a resize and the next frame is still right.
- **A web page is a child window, always above what the app draws**, so what
  floats over one is cut out of it (`ui::note_covers` lists every rectangle of
  the interface drawn this frame above the canvas, egui's own menus and hints
  included; `WebViews::show`, `holes_in`, `cut_out` set a window region with
  those holes). A page wholly covered is hidden. This is the only `unsafe` code
  of the app (four Win32 calls).
- **A card is not over the page it holds.** A card on the canvas is itself a
  rectangle of the interface above the canvas, and the page of an extension or
  of a Markdown file is inside one: counted as a cover, each card covered the
  whole of its own page, so the page was never made and the card stayed empty
  (the panel of Claude Code, with nothing in the log). `ui::cards` notes which
  pages were placed while each card was drawn (`CardPlace::pages`),
  `note_covers` leaves the cards out of `UiState::covers`, and
  `ui::covers_of` gives each page the covers plus every card but its own. A
  card still covers what plays on the canvas under it.
- **Some settings of extensions start the way Code Canvas wants**, not the
  extension's (`OURS` in host.js; a person's `settings.json` still wins): the
  Claude Code panel does not attach the open file to every message
  (`claudeCode.attachOpenFile: false`). `qa/host/settings.test.js` checks it.
- `progressBar-background` is given the accent by name. Guessed from its name
  it was a background, so the colour of the card: the bar of the weekly usage
  of Claude Code was filled with the colour of what it stood on, and only the
  orange one (above 90%) could be seen.
- The interface font has no arrows: keys are named in words (`Chord::label`:
  Left, Right, Up, Down) and arrows in text come from the icon font.
- Deep rows of the explorer keep their names: the indent of a level shrinks
  from 16 to 6 points so the deepest open row keeps room, then is capped, and a
  nested row shows its path as a hint (`explorer`).
- A symbol's tag that would cover the caption of a collapsed folder goes just
  above or below it (`overlay::build`). Names wider than their card or folder
  end in an ellipsis (`overlay::fitted`). A card with nothing to show says what
  it is (`scan::binary_note`, `Pictures::problem`).
- Line numbers are clipped to the lines each column of a card has
  (`Renderer::render`, `CodeText::numbers`): the shared block of numbers runs on
  to the end of its 128 lines, which showed one number too many under short
  files and thirty after the end of long ones.
- With no folder open the canvas menu has no folder actions
  (`CardMenu::nothing_open`), and any command run from the keyboard closes the
  card menu (`App::run_command`).
- **A minimised window draws nothing.** Windows reports it as 160 by 28 pixels,
  not as nothing, and it stays that size for a few frames after it is restored.
  One frame of the interface that small left the side panels at their narrowest
  for good, which is what "the canvas shrinks every time I minimise" was.
  `can_show` is the one test: `Resized` ignores a size that fails it (the surface
  keeps its last good size), `redraw` returns, and `about_to_wait` stops asking
  for frames, which would otherwise come back at once and spin a core.
- A file dropped on the window is copied on a thread (`files::copy_into`);
  `App::poll_copies` hears when it is done. A copy that git ignores is not on the
  canvas, so `App::show_dropped` says so, and opens it in the viewer when it is
  media. The same happens when the file was already there.
- **What git ignores** is not scanned, so it is not on the canvas, but the file
  tree lists it dimmed and without a checkbox (`GitState::ignored`, read with the
  rest of the git state; `page::build_tree` hangs each path from the nearest
  folder that is on the canvas). An ignored folder is one row: it is not walked
  (`--directory`), which keeps `target/` or `node_modules/` cheap. At most
  `MAX_IGNORED` (400) paths. A click opens media or Markdown in the viewer and
  shows anything else in the system's file manager (`Action::OpenIgnored`).
- Another frame is requested while text is still being shaped in any pane, while a
  notice is up (they fade by themselves), and once more when the stats changed:
  the status bar shows the previous frame's stats and would otherwise stay stale.
- `App::about_to_wait` ticks every 100 ms while a scan runs, to refresh its
  progress; otherwise it sleeps until input arrives or egui's next scheduled
  repaint. `user_event` is empty on purpose: `about_to_wait` picks the work up.
- `WindowEvent::RedrawRequested` is not input and is not passed to egui;
  everything else goes to egui first.

Window

- The window has no system frame (`with_decorations(false)`): the header is ours,
  and so are the edges (`App::edge_at` finds the edge or corner under the pointer).
- The window icon is set explicitly. The taskbar and Alt+Tab use the big icon, and
  setting it keeps them from falling back to whatever Windows cached for the file.
- **Where it opens** (`window_place::place`, called from `App::resumed`). The
  first time: centred on the main screen at 90% × 92% of what the taskbar leaves,
  never the 1440×900 it used to ask for whatever the screen was. After that: the
  size and place it was left in.
  - A saved size larger than the screen it opens on is cut to fit (minus
    `MARGIN`, and `TASKBAR` in height, because winit does not report the work
    area and a window as tall as the screen ends up under the taskbar).
  - A saved place is kept only if a screen still holds enough of the title bar to
    grab (`holds`); a second screen that is gone sends the window back to the
    main one, centred.
  - Maximized is saved as its own flag (`Session::maximized`). While maximized,
    `save_session` keeps the size and place from before, so "restore" after the
    next start gives the old window back and not a screen-sized one.
  - A maximized window on Windows reports a rectangle 8 px larger than the screen
    on every side. That is the invisible resize frame, not our content: the
    client area ends at the screen's edges, so nothing needs padding. (A padding
    was tried and it only added a band.)

Start-up and shutdown

- The previous session always comes back; folders given on the command line are
  added to it and the first of them is shown.
- The colours are loaded before anything else is built, so everything takes them.
- Only the folder that will be on screen is scanned at start. The others wait
  until they are shown (`PageState::Waiting`): a huge one would take seconds and
  every core for a canvas nobody is looking at. A folder beside the active one is
  on screen too and is scanned.
- The session is saved as soon as the folders are open, so what is open is
  remembered even if the app is later killed.
- The session saves the panel each side last showed by its key (`Panel::key`),
  and where each icon sits (`Rails`). At start a side reopens that panel unless it
  is a web page (an extension's view or the media viewer): those wait to be asked
  for.
- `App::ui_events` holds pointer events a script asked for (`ui ...` steps), one
  batch per frame, added to what egui gets from the window.
- After an update is installed (`restart_as`), the app leaves the usual way, so
  unsaved files are asked about, and the new version starts once this one has
  closed. If the user cancels leaving, nothing is restarted; the new version is in
  place all the same and starts next time.

Keyboard and focus

- **AltGr reports Ctrl+Alt.** That is typing (`@`, `{`, `[` on many layouts), not a
  shortcut: in `App::edit_key` a command key is Ctrl without Alt, or Super.
- With the caret in a file, keys go to the editor first. A shortcut with Alt is
  the exception (Shift+Alt+F formats, it does not type); Ctrl+Alt is AltGr again,
  which does type.
- `Scope::Anywhere` shortcuts work wherever the keyboard is, a text field
  included.
- While Settings waits for the new shortcut of a command (`UiState::rebinding`),
  the next key is it. Esc gives up; a modifier on its own, or a key that cannot be
  used for that command, keeps it waiting.
- While Settings is open no key reaches the canvas or the editor
  (`App::on_key`). Esc and the Settings shortcut close it; `Scope::Anywhere`
  shortcuts still run, and the two that open the "go to" box close Settings
  first, so two things never fight over which is on top.
- Each command has one shortcut by default. The second ones (Ctrl+T to open a
  folder, Ctrl+R to reload, F1 for the palette) and Ctrl+1 … Ctrl+9 to go to
  folder N were removed at the user's request: duplicates nobody needed. A
  second shortcut can still be given to any command in Settings.
- `App::page_chord` handles chords pressed while a web page had the keyboard: the
  same shortcuts as in the window, except the canvas's own, which are not for a
  page that is being typed in. Palette, go to file and search in files open a text
  field of the app, so the keyboard is taken back from the page first.
- **A mouse press that reaches the window calls `webviews.focus_app()`.** Such a
  press is on the app's own interface, not on a web page. Without this, a box
  clicked while an extension's panel had the keyboard opened but could not be
  typed in.
- Only presses that start on the canvas belong to the canvas.
- While a completion list is up, the keys that walk and take it are the list's.
  Any other key goes on to the editor, which then offers again for what is at the
  caret.

Editing and files

- Typing is debounced twice in `App::poll_background`: the names of the typed files
  go into the symbol index and their imports are resolved again 700 ms after the
  last edit (not for a card that shows a diff: its text is not the file's), and the
  extensions get the text 250 ms after it, not per keystroke. The card itself, its
  folders and its blocks follow each keystroke (`Page::show_edit`).
- `App::save_edit` reloads the file from disk after writing, so the symbol index
  and git see the saved file; then the extensions format it if the settings say
  so.
- `App::write_file` (a formatter or a fix produced new contents): an open editor
  without changes of its own takes the new text.
- `App::on_disk_changes`:
  - paths with a skipped directory in them (build output, dependencies) and
    lockfiles (`ScanOptions::skip_files`) are ignored;
  - new paths count only when git would not ignore them;
  - up to `ONE_BY_ONE` (24) files changed, made or deleted (a rename is one of
    each) are taken one by one: `Page::reload_file`, `Page::file_made`,
    `Page::file_gone`. Nothing else is read or laid out again. A folder that is
    gone counts as all its files gone. Directories reported as changed are
    ignored: Windows reports the folder of every file that changes;
  - more than that, or a folder still being read, mean a rescan: none for a huge
    folder (it takes seconds: that stays the user's call, F5), right after another
    one, or while the page is loading;
  - a file made by another program is lit and travelled to like one that changed;
  - a file with unsaved edits is not reloaded: what is being typed here wins over
    what changed there;
  - a changed `package.json` or `Cargo.toml` has its dependencies read again.
- `App::file_op`: paths come relative to the open folder and cannot leave it
  (`files::inside`). The path is rebuilt with this system's separators, whatever
  the tree uses.
- `App::drop_file`: a file dropped on the window is copied into the folder
  selected on the canvas or in the explorer (the one a selected file is in), else
  the open folder itself. Nothing is ever overwritten.
- `App::locate_file` slices the path by the length of the root's key. That is
  valid only because the key differs from the path in case alone, so lengths
  match.
- Discarding changes cannot be undone, so `App::apply` asks first.
- `App::go_to_definition` (F12): language extensions answer exactly; without one,
  the built-in index does.
- Hover: the language extensions are asked about a code cell only once the pointer
  has rested on it a moment (`hover_probe`).

Camera

- `App::frame_node`, for a file: a readable width, with the top of the file at the
  top of the screen. `App::frame_site` puts the site 30 % across and 35 % down the
  canvas.

### crates/code-canvas/src/bench.rs

- The camera sweeps from "whole repository" to readable text and back over
  `PERIOD` = 12 s, eased, while drifting sideways by about 30 % of the screen so
  new content keeps entering. That is the worst case for the streaming paths:
  token chunks, text shaping, glyph rasterisation.
- Frame-interval statistics are logged every few seconds. The first sample of each
  window includes the report itself and is skipped.

### crates/code-canvas/src/codicons.rs and icons.rs

- Both are **generated: do not edit by hand.** `codicons.rs` comes from
  `scripts/make_codicons.py`, `icons.rs` from `scripts/make_icons.py`
  ([scripts](#scripts)).
- Codicons are VS Code's icon set, CC BY 4.0 (`assets/CODICON-LICENSE.txt`).
  Lucide is ISC (`assets/LUCIDE-LICENSE.txt`).
- `codicons::TABLE` is sorted by name. `codicons::render` replaces every `$(name)`
  with its glyph; `$(sync~spin)` is the icon `sync`; an unknown name is dropped.

### crates/code-canvas/src/commit_ai.rs

- `commit.json` chooses the provider (the settings panel edits it):

  | `provider` | What runs |
  |---|---|
  | `auto` (default) | The first of Claude Code and Codex that is installed, Claude Code first. |
  | `claude` | `claude -p`, with no tools and no session. |
  | `codex` | `codex exec --sandbox read-only --skip-git-repo-check --ephemeral`. |
  | `custom` | Any command, as typed in a terminal: `ollama run llama3`, a script. |

  `claude_model` and `codex_model` name the model asked for (empty: the program's
  own default; the built-in default is a small, fast model, plenty for one line).
  `custom` holds the command.
- Every provider gets the request on its standard input and the message is what
  it prints. Codex is the exception: what it prints also carries its progress, so
  its answer goes to a file (`Invocation::answer_file`).
- The request (`prompt`) is the rules, the latest commit subjects for the style,
  and the diff: of what is staged, or of everything when nothing is. `MAX_DIFF`
  cuts the diff at 30,000 characters.
- `LIMIT`: an assistant that has not answered in 120 s is stopped.
- `run` feeds stdin and drains the output on threads of their own: a full pipe
  would stall the child. On failure the error is the last three lines it printed,
  the ones that say what went wrong.
- `write` blocks until the assistant answers: call it off the UI thread.
- `find_program` looks through `PATH` as the system does. On Windows that includes
  `.cmd` shims (what `npm` installs), and a file without an extension is not a
  match: Windows could not start it.
- `clean` takes the first line that says something, without the fences or quotes
  some assistants add anyway.

### crates/code-canvas/src/editor.rs

- The canvas shows a *normalised* copy of every file (tabs expanded, `\n` only).
  An `Editor` holds the file as it really is, with its own tabs and line endings,
  so saving changes nothing the user did not type.
- Positions are `(line, character)`. `Editor::display_col` converts to the columns
  the canvas draws (tabs go to the next tab stop); `Editor::pos_at_display` goes
  back, and a click on the right half of a tab lands after it.
- `Editor::splice` does not record undo; `Editor::replace` is one undo step.
- `Editor::tab` indents in the file's own style: a tab in a file indented with
  tabs, spaces to the next step otherwise. `Editor::home` goes to the first
  non-blank character, then to the very start.
- `Editor::dirty` is exact: `saved_at` is how deep the undo history was when the
  text was read or saved, so undoing back to it is "no changes" again. Typing
  after undoing past that point makes the saved text unreachable (`None`).
- **Several carets.** `caret`/`anchor`/`goal` are the main caret (the one the
  camera follows, completions and the status bar use, and the others see in a
  shared session); `others` are the rest. The newest caret is the main one.
  - `edit_each` asks every caret for the range it replaces and the text, all
    against the text as it is, then applies them from the top down, moving
    each next range by what the ones before it did (`Shift`). Each is one
    `Change`; all but the first are `joined`, so `undo` and `redo` take the
    whole group, and the first remembers every caret as it was
    (`others_before`). Ranges that would overlap are cut at the end of the one
    before. With one caret it is plain `replace`.
  - Up, Down, Page Up and Page Down on a wrapped card go by rows
    (`vertical_in` with the card's `Wrap`, from `Page::edited_wrap`): the goal
    column is then one on screen. Without a `Wrap` it is plain `vertical`.
  - Movements run the one-caret code once per caret (`each`), then `settle`
    sorts the carets and merges those that overlap or meet (a caret at the edge
    of a selection joins it). A click, Ctrl+Home/End, select all, a double
    click and any single-range edit (`replace`: completion, Replace all,
    imports, another program's text) go back to one caret.
  - Ctrl+D (`add_next_occurrence`): with no selection it selects the word at
    every caret, and from then on it looks for whole words only
    (`whole_words`), as VS Code does; a selection made by hand is found inside
    other words too. The next one is the first free one after the main caret,
    starting again at the top. Ctrl+Shift+L takes every one at once.
    `MOST_CARETS` (5000) is the most carets there can be.
  - Paste with as many lines as carets gives each caret its line; copy joins
    the selections with line breaks.
  - An edit from elsewhere (`apply_remote`) moves every caret, not only the
    main one.
  - Completions are offered only with one caret (`App::update_completion`,
    `ask_completion`, `offer_code`, `paths_offered`). Saving, autosave and a
    shared session need nothing: they look at the text, not at the carets.

### crates/code-canvas/src/ext/host.rs

- **Writing to the host must never happen on the UI thread.** `Host::out` is a
  channel to a writer thread: when the host is busy its pipe fills up, and a write
  would block whoever does it.
- Dropping the channel ends the writer and closes the pipe, and a closed pipe is
  what tells the host to stop. `Host::stop` asks the host to deactivate its
  extensions and leave, kills it if it has not within a few seconds, and does not
  block the caller.
- **Leaving the app waits for the host** (`Host::stop_and_wait`, from
  `Extensions::shutdown`): a thread that waits is gone the moment the process
  ends, the request to stop may not even have been written, and whatever the host
  started (language servers, an assistant's processes) would be left running. The
  window is hidden first, so the wait of at most three seconds is not seen.
- `kill_tree` kills the host and whatever it started (language servers).
- `HOST_JS` is embedded with `include_str!`, so an installed app needs no extra
  files; `Host::start` writes it to disk and runs it.
- `Stderr` lines are what the host wrote outside the protocol: Node's own errors.

### crates/code-canvas/src/ext/market.rs

- Extensions come from Open VSX because Microsoft's marketplace may only be used
  by Microsoft's own products.
- A `.vsix` is a zip; the extension is the `extension/` folder inside.
- `MAX_VSIX` = 400 MiB: extensions that bundle a language server are big.
- Unpacking uses `enclosed_name`, which refuses entries whose path would leave the
  folder.
- Installing replaces other versions of the same extension.
- An extension is identified by `publisher.name` in lower case
  (`MarketItem::id`, `dir_id`).
- `install` takes the latest version, in the build for this platform (`platform`,
  e.g. `win32-x64`) when the extension ships one per platform. `search` with an
  empty query lists the most downloaded.

### crates/code-canvas/src/ext/mod.rs

Tabs

- A webview *panel* is VS Code's editor tab. An extension that also has views of
  its own (Claude Code: the chat, the list of sessions) gets one more icon,
  `tabs:<extension id>`, whose panel shows the panels it has opened as tabs
  (`Extensions::tabs_of`, `shown`, `front_tab`); closing one tells the host
  (`close_tab`). An extension with no views keeps its single panel as before.
- Two icons of one extension with the same title get their view's name added
  (`view_words`), so the tooltips tell them apart.

Default extensions

- `DEFAULTS` and what they are: [EXTENSIONS.md](EXTENSIONS.md).
  `defaults_to_install` offers each one once. `defaults.json` lists those this
  installation already got; one already installed by hand counts as got; one that
  failed to arrive (no network) is tried again; one the user removed afterwards
  does not come back.

Languages and grammars

- `read_installed` keeps `contributes.languages` (id, extensions and file names,
  lower case) and `contributes.grammars` (language, scope name, file, `injectTo`)
  of every extension, with or without code: grammars need no host. `languages()`
  and `grammars()` are those of the enabled ones. `installed_revision` moves each
  time the list is read again (install, removal, enabling, disabling), which is
  what tells `grammars.rs` to load them again.

Host and documents

- `MAX_OPEN` = 48 documents kept open in the host; the least recently seen go
  first. Documents wanted before a (re)start of the host are opened again.
- Before an extension's folder is removed or replaced the host is stopped: it may
  hold the files open. `remove_dir_retrying` exists because Windows keeps files
  locked for a moment after the process using them ends.
- `own_decorations` (what the app itself draws on files, in the same form as
  extensions' decorations) survives a restart of the host.

Paths and positions

- Diagnostics and decorations are keyed by `path_key`, the same for a path however
  it is spelled. `diagnostics_under` and `decorations_under` slice paths by the
  length of the folder's key, valid only because the key differs in case alone.
- `Diagnostic::col` and everything else on the canvas is in display columns (tabs
  expanded).
- A `diagnostics` message with an empty list clears the file.

Colours

- `parse_color` reads `#rgb`, `#rrggbb`, `#rrggbbaa`, or `theme:<id>`. A theme
  colour is guessed from the last part of its name
  (`errorLens.warningForeground`) until VS Code themes are supported.
- In `Decoration::color`, zero alpha means "not set".

Manifests and the rail

- A manifest string of the form `%key%` points into `package.nls.json`
  (`localized`).
- `manifest_commands` leaves out the commands an extension hides from the command
  palette. `manifest_containers` does not take the containers of VS Code's own places (the
  explorer, source control): the trees a manifest adds there go together in one
  container of its own, `<publisher.name>:views`, with no title (the extension's name
  stands in); webviews there are left out.
- `Extensions::rail_items`: the rule is in "Webviews" in
  [EXTENSIONS.md](EXTENSIONS.md). In addition, an extension none of whose
  containers gives an icon still gets one for a webview it has open or registered:
  a panel it opened itself, or a view under a condition this app cannot follow.
  Claude Code offers the same view in two places; only one of them is shown. A
  container with no webview shown but a tree whose `when` holds gets an icon
  `tree:<container id>` (`TREE`, `ui::Panel::Tree`), after the extension's webview
  icons, so the webview fallback above still looks at webviews only.
- `Container::codicon` is set when the manifest names a codicon (`$(github)` →
  `github`). Otherwise the icon is an image file, and the extension's own icon
  stands in.
- `Extensions::ensure_view`: a view of a manifest has no `WebviewEntry` until its
  extension starts and registers it, so one is created to be able to show it.
  `WebviewEntry::failed`: the extension was asked and gave no page; it is not
  asked again.
- `Extensions::icon_theme(wanted)`: that theme if installed, none for `"none"`,
  otherwise one that is there (Symbols first), so installing an icon theme is
  enough to see it.

Notices

- At most 5 plain notices are kept, so the corner stays small; questions stay
  until answered.
- Plain notices fade after a while. An extension's question waits much longer, and
  left unanswered counts as dismissed. The app's own questions (`Extensions::ask`)
  wait 3600 s: nobody is blocked on them. Asking again about the same thing
  replaces the notice; answers arrive through `Extensions::take_answers`.
- Status bar texts go through `codicons::render` (`$(check) Prettier`).

### crates/code-canvas/src/ext/views.rs

Code lenses and tree views on the app's side; the host's half is in host.js above.

- **Lenses are asked only for files readable on screen**: `show_documents` hands the
  visible files to `lenses_for`, which asks those not in `lens_fresh`. An answer is
  kept per file (`Views::lenses`) and drawn until a newer one comes. What makes a
  file asked again: an edit (`lenses_edited`, asked at once with `LENS_REST_MS` of
  rest, so the host waits for the typing to stop), a change on disk or a save
  (`lenses_stale`: asked when next seen), `lensesChanged` from the host (every file,
  when next seen). A file pushed out of the host's open documents forgets its lenses.
- `lens_asked` keeps the last request of each file: an answer to an older one, or a
  `superseded` one, is dropped.
- "Show code lenses" off (`set_show_lenses`, from the preferences every frame) drops
  everything and asks nothing.
- `plain_title` takes `$(codicon)` out of a title: the canvas's font has no codicons.
- **Trees.** `TreeState` per view: the rows of each handle loaded (`""` is the root),
  which are expanded (a row that comes `Expanded` is opened the first time it is
  seen, never again), which requests are in flight and which lists are `stale`.
  `tree_rows` is what the card draws and also asks for what is missing or stale: the
  root and the children of every open row. `treeChanged` marks every list loaded
  (and every one in flight) stale, so the card keeps its rows until the new ones
  arrive and does not blink.
- Rows of a handle the host no longer knows (`gone`) are dropped and closed.
- `manifest_menus` reads `view/title` and `view/item/context` with the icon of each
  command (a codicon, else the title is the button). `MenuItem::is_button`: the
  title's `navigation` group, a row's `inline` group; `view_buttons` evaluates their
  `when` with `view` and `viewItem` (the row's `contextValue`) on top of the context
  keys.

### crates/code-canvas/src/ext/when.rs

- Supported: `!`, `&&`, `||`, parentheses, `==` `!=` (also `===` `!==`),
  `<` `<=` `>` `>=` on numbers, `true` and `false`. `&&` binds tighter than `||`.
- **Anything else makes the whole clause false** (`=~`, `in`, a regular
  expression), and so is a key nobody set. The caller then treats the thing as not
  shown and has to cope with that.
- A key nobody set equals nothing: `key == x` is false and `key != x` is true.
- An empty clause is true: it is what "no condition" looks like.
- The parser does not short-circuit: both sides of `&&` and `||` are always read,
  because the position has to move on.

### crates/code-canvas/src/files.rs

Boundary for everything the explorer's menu does:

- Everything stays inside the open folder. `inside(root, rel)` only returns a path
  that stays inside `root`; `named` accepts a name with `/` in it (`a/b.txt` makes
  `a` too) but not one that leaves its folder.
- Nothing existing is ever overwritten. Renaming onto something that exists is
  refused.
- Nothing is deleted for good: "delete" is `recycle`, a move to the recycle bin.
- The result shows on the canvas through the file watcher, not directly.

Details:

- `rename`: on Windows "Readme" and "README" are the same file, so changing only
  the case is a rename, not a clash.
- `copy_into` runs on a thread of its own (the file may be a large video); what
  goes wrong there is only logged. It returns `Err` with an empty string when the
  file is already in that folder: nothing to say.
- `reveal`: Explorer wants the comma glued to the switch and backslashes:
  `/select,C:\path`.
- `terminal`: Windows Terminal (`wt.exe -d <dir>`) where it is installed, else
  the classic console.

### crates/code-canvas/src/git.rs

- `compare` gives the hunks `git diff -U0` would, from two texts in memory, with
  the same meaning of `Hunk::line` as `apply_diff`. Line endings are not a
  difference. It gives up refining after 60 ms on huge inputs.
- `head_text` is the file as committed (`HEAD:./path`, relative to the folder);
  `None` for a file git does not have yet.
- `FileChange::unsaved`: the entry describes text that is typed but not on disk
  (see `Page::preview_unsaved`).
- `head_texts` reads many files as committed with one `git cat-file --batch`. The
  names are written from another thread: git answers while it is still being
  asked, and both pipes are small.
- `unified` is a file with its removed lines put back above what replaced them,
  and with `around` the unchanged stretches replaced by one `Gap` row. `Row::line`
  is the line of the real file the row is, or for a row that is not one the next
  real line. `DiffText::lines` says which row each line of the file is on (a line
  hidden in a fold is on its `Gap` row); `line_of` goes back.
- `side_by_side` is the same pieces as two aligned lists, written one after the
  other: all the rows of what the file was, then all the rows of what it is, with
  `DiffText::fold` rows each. A card whose column is that tall (`SourceFile::fold`)
  shows them next to each other, and each side is highlighted as the code it is.
  `Before` is an unchanged line on the old side, `Blank` the filler opposite a
  line the other side has alone. Past `FOLD_LINES` rows the two lists alternate
  in blocks of that many.
- Everything runs the user's `git`, off the UI thread. A folder outside a
  repository, or a missing git, simply means no changes (`load` returns `None`).
- **`GIT_OPTIONAL_LOCKS=0` in `command` is load-bearing.** Reading the status must
  not rewrite `.git/index`, or the file watcher would see a change and ask for the
  status again. `command` also sets `core.quotepath=off` and opens no console
  window.
- `load` runs its reads side by side: they do not depend on each other, and
  starting git costs more than what it then does, on Windows above all.
- Paths in the status are relative to the repository, not to the open folder; the
  prefix is asked for and removed.
- Status parsing: the branch line is `main...origin/main [ahead 1, behind 2]` or
  `No commits yet on main`. A renamed or copied entry is followed by the path it
  came from. Untracked files are `??` and exist only in the working tree.
- `apply_diff` reads `git diff -U0`. A hunk header is
  `@@ -old_start,old_count +new_start,new_count @@` with a count of 1 left out. A
  pure removal (`new_count` 0) sits *after* line `new_start`. Hunk lines are
  0-based.
- `MAX_CHANGES` = 5000 changes kept (a fresh `node_modules` that is not ignored…);
  `GitState::truncated` says more existed.
- `not_ignored` reads the exit code: 0 some paths are ignored (and listed), 1 none
  is, anything else means not a repository, where nothing is ignored.
- `log` follows renames.
- What the operations mean (`GitOp`): `Pull` and `Sync` are fast-forward only,
  because histories that diverged are the user's to merge. `Discard` puts a
  tracked file back to the last commit. `DiscardAll` does that for every tracked
  file and leaves untracked ones. `Stash` puts every change away, untracked files
  included. `Commit { amend: true }` with an empty message keeps the message the
  commit had.
- `change_for_message`: the diff of what is staged, or of everything when nothing
  is (new files by name), plus the latest subjects. `None` when there is nothing
  to commit.

### crates/code-canvas/src/grammars.rs

TextMate grammars of extensions (`contributes.grammars`) colour the languages the
app's own tokenizer does not know (`Lang::Plain`), or every language with
`Prefs::grammars_everywhere`.

- **Why `irosashi`.** The grammars of real extensions are written for Oniguruma
  (`\G`, look-behind, possessive quantifiers, `\h`, back-references from `begin` in
  `end`...), so the engine has to be Oniguruma or a faithful copy. Looked at:
  `syntect` reads Sublime's `.sublime-syntax`, not `.tmLanguage.json`, and its
  pure-Rust backend (`fancy-regex`) lacks part of Oniguruma; the `onig` crate is C
  (a C build, and the engine on top would be ours to write); `vscode-textmate` +
  `vscode-oniguruma` in the extension host is the reference but needs Node
  (grammars are declarative and should work without it), a WASM blob to ship and
  every token of every file sent over the pipe; `giallo` is EUPL. `irosashi`
  (MIT) is a Rust port of `vscode-textmate` on `ferroni` (BSD-2-Clause), a pure
  Rust port of Oniguruma: no C, no Node, about 2 MB of executable, and its authors
  check it token for token against `vscode-textmate` on 234 grammars. Its embedded
  grammars and themes are left out (`default-features = false`): only the ones of
  installed extensions are loaded. It needs Rust 1.94, hence the workspace's
  `rust-version`.
- `Matcher`: the languages of `contributes.languages` that have a grammar, by file
  name and by extension (`.d.zed` before `.zed`: every suffix from a dot, the
  longest first), lower case. `paints` is the `Painter` of every page's
  `TokenCache`: also no note (but "truncated") and at most `MAX_LINES` (200,000)
  lines; bigger files keep the app's tokens.
- `Grammars::update` runs at the start of every frame and does something only when
  the installed extensions (`Extensions::installed_revision`) or the setting
  changed: a new matcher, a new `generation` (results of the old one are dropped),
  and the grammar thread loads the grammars again. The app then gives every page
  the new painter and drops its shaped text (`App::paint_grammars`).
- One thread, `grammars`, does all the work (`Worker`); the UI thread only sends a
  file's text (`ask`, at the end of a frame for what `TokenCache` asked) and
  installs what comes back (`take`, at the start of the next one, then
  `TokenCache::paint`). The thread is started with the first grammar.
- The grammar files are read on that thread: `.json` as they are (a BOM taken
  off), `.tmLanguage` / `.plist` / `.xml` through `plist.rs`, anything else is
  left out with a warning. A grammar without `language` (an injection, or one only
  included by others) is registered under its scope name; `injectTo` of the
  manifest is written into the grammar, where `irosashi` looks for it. A grammar
  that does not load, or whose session cannot be made, is logged once and its files
  keep the app's tokens.
- **Work is sliced.** `step` runs the task with the least left (an edit first) for
  `SLICE` (15 ms) or until an edit arrives (`hurry`), then reads the channel again.
  A long file sends what it has every `SEND_EVERY` (150 ms), so its top is coloured
  while the rest is still being read; chunks not covered yet are the app's own.
  Lines longer than `MAX_LINE_BYTES` (4,000) are not tokenized (no colour, the
  state goes on unchanged), so a minified file does not stall it.
- **Edits.** The thread keeps, per page and file (`Memo`, the least recently used
  let go above `MEMO_LINES` in all), each line's hash, the state after it and its
  pieces. A new version keeps the lines in front of the first changed one, and
  from the end of the change on, as soon as the state before a line is the one the
  same line had before, takes the rest of the old lines as they are: typing in a
  long file tokenizes a line or two. The state is shared between equal lines, so
  keeping it per line is cheap.
- **No flicker while typing.** `App::after_edit` sends the edited file as urgent
  and waits for it up to `EDIT_WAIT` (8 ms) before the frame is drawn (`wait`; the
  work is still on the other thread). Late, the file shows the app's tokens for a
  frame or two. Other results that arrive meanwhile go to `backlog`.
- `flying` remembers which version of which file is being worked on, so a file
  asked again every frame is sent once; a finished one is forgotten there.
- **Scopes to kinds** (`rule_of`): the innermost scope that one of `RULES` fits
  (the rule is the scope or its start up to a dot; the longest rule wins) decides;
  `meta.*`, `source.*` and unknown names say nothing and the next scope out is
  asked. A rule has a kind for words and one for the rest: `keyword.operator` is a
  keyword when it is a word (`typeof`, `new`) and punctuation otherwise; a token no
  scope says anything about is a name (words) or punctuation, or prose under
  `text.*`. The theme's colours of each kind come from the same scope names
  (`themes.rs`).
- `cut` makes pieces like the app's own tokens: no blanks (strings and comments
  are trimmed at both ends and joined over the blanks between their parts, other
  tokens are split into words), and pieces of the same kind that touch are one.

### crates/code-canvas/src/icon_theme.rs

- VS Code's own format: a JSON file where `iconDefinitions` names image files and
  tables say which definition a file or folder gets, by its whole name
  (`fileNames`, `folderNames`) or by its extension (`fileExtensions`). Image paths
  are relative to the folder of the theme file.
- Only SVG icons are drawn (with `resvg`). Font icons and the per-language table
  (`languageIds`) are not read; a file without a match gets the theme's plain file
  icon.
- `IconTheme::file`: the whole name wins over the extension, and a longer
  extension over a shorter one (`x.test.ts` is `test.ts` before `ts`). A theme may
  name a definition it does not define, or one that is a font character; `image`
  takes the first that has an image.
- `IconTheme::folder`: a theme without open variants shows the closed one.
- `rasterize` draws the SVG whole and centred into a square and returns RGBA with
  **premultiplied** alpha. The explorer rasterizes at 32 px, twice the size it
  shows, to stay sharp on dense screens.

### crates/code-canvas/src/keys.rs

- `keybindings.json` holds only the user's changes: the id of a command and the
  chords it answers to, replacing its own. An empty list leaves the command
  without a shortcut.

  ```json
  { "goToFile": ["ctrl+e"], "rescan": [] }
  ```

- The keys of the text editor itself (arrows, Ctrl+C, Ctrl+Z, Ctrl+S…) are not in
  the table. The carets ones are (`Command::is_caret`: Ctrl+D, Ctrl+Shift+L,
  Ctrl+Alt+Up/Down): they are run before the editor sees the key, whatever
  chord they are given, because the editor would take Ctrl+Alt+Up as Up.
  Alt+click (add or take away a caret) and Esc (back to one caret, then stop
  typing) are fixed.
- `Scope`: `Anywhere` (also in a text field or an extension's web page), `Window`
  (not while typing in one of the app's text fields), `Canvas` (keys nothing else
  wanted, plain letters among them).
- `Chord::key` is lower case: a character (`"p"`, `"+"`) or a name (`"f5"`,
  `"tab"`, `"left"`). `Chord::parse` reads `ctrl+shift+p`, `f5`, `ctrl++`.
- `Chord::new` normalises what was pressed:
  - Shift is dropped for symbols. It is how many symbols are typed at all (`+` is
    Shift+`=` on some keyboards and a key of its own on others), so it says
    nothing. Letters and named keys keep it: Ctrl+Shift+P arrives as an upper case
    "P" with Shift held.
  - Browsers say `arrowleft` where the app says `left`.
- `KeyFile::bind` takes the chord away from whichever command had it, and refuses
  a chord that cannot be used: a plain letter (`Chord::is_typing`) outside the
  canvas would make typing impossible.
- `KeyFile::store` forgets a command whose chords are simply its own, so the file
  only lists real changes.
- `Keymap`: a chord means one thing. If two commands list it, the first in
  `COMMANDS` keeps it.

### crates/code-canvas/src/markdown.rs

- A Markdown file is shown rendered in the media viewer's browser view (`page`),
  with the app's colours. It is not a `media::Kind`: a click on a `.md` file is
  for editing it, so the preview is asked for (`App::open_preview`: the *Preview*
  word next to its name on the canvas, the file tree's menu, the palette).
- **The file is not trusted.** Three things keep a README from doing more than
  being read:
  - the page's policy allows no script but the one of the page itself, which
    carries a nonce made for each render;
  - markup that acts without a click (`ACTS_ON_ITS_OWN`: `<meta>` refreshes,
    `<base>`, frames, forms, scripts, styles) is shown as text, while the rest of
    the HTML GitHub allows (`<details>`, `<img>`, `<kbd>`...) is kept;
  - a link never navigates the view: `WebViews` hands it to the app
    (`App::follow_link`), which opens a file of the folder itself and gives a web
    link to the browser through `files::open_web_link`, without a shell.
- `<base>` points at the file's folder, so relative images and links resolve as on
  GitHub. Images load from the folder (the roots the views may read) and from the
  web.
- Headings get the anchors GitHub gives them (`slug`, repeats numbered), so a
  table of contents works.
- The preview follows the editor: `App::refresh_preview` runs when an edit is sent
  to the extensions (250 ms after typing stops) and renders again only if the text
  changed. The page keeps its scroll position across renders in `sessionStorage`.

### crates/code-canvas/src/media.rs

- Images, video, audio and PDF are shown by the same browser view that shows
  webviews, which already knows every common format. The page is a few lines of
  HTML pointing at the file through the app's protocol (`file_url`); its handle
  among the webviews is `HANDLE` = `media-viewer`.

### crates/code-canvas/src/overlay.rs

- A card shown as a diff draws its rows (added, removed, the folds) instead of
  git's hunks, which count lines of the real file.
- Changed lines are washed in their git colour over the whole line, and from far
  away the wash is stronger and at least a pixel and a half tall: a changed file
  is seen on the map of the whole folder. `Touch` (a file just changed from
  outside) lights its lines in the accent colour and fades in `Touch::SHOWN`
  seconds.
- `SYMBOL_HOVER_MIN_PX` = 0.6: symbols react to the pointer once a line is at
  least that many pixels tall.
- `MAX_DEP_CURVES` = 4000: most dependency curves drawn for one selection, and for
  "every edge"; with bundles the biggest come first.
- Dependency curves: a file hidden inside a collapsed folder counts as being where
  that folder is. Each curve starts at the line of the import and ends in an
  arrowhead at the top of the file imported; one in an import cycle is dashed.
- **Bundles.** Above `BUNDLE_FROM` (300) edges (every edge, or a big folder's), the
  edges are merged per pair of `Layout::groups`: folders smaller than `GROUP_PX`
  (150 px) on screen are one end. A bundle is thicker with its count (log scale,
  at most 7 px) and ends on the side of the folder's box that faces the other
  end; a count chip sits in its middle (`Page::dep_marks`, at most
  `MAX_DEP_MARKS`). Zooming in opens the folders, and selecting a file opens the
  folders around it. The groups follow the zoom in steps of √2 (`zoom_step`):
  `Page::dep_zoom` is the step the curves were made for, and `App` makes them
  again when the camera, at rest, is on another one.
- A relation that ends at a definition attaches to the start of its line (see
  `Repo::sym_line_start`). A plain mention outside any function gets no chip.
- Labels: screen rectangles of placed labels are remembered; parents are visited
  first and win, overlapping descendants are skipped. The root's name sits above
  its box, so it never fights with the first folder inside for the same corner.
  The root cannot be collapsed.
- Chips have a fixed pixel size, are anchored at a world point, sit just above the
  line they name, and never overlap.
- Source control and problems are marked at two distances: from far away a
  coloured strip or edge on the card and a dot; close up, a bar next to each
  changed line and an underline per problem. A pure removal is a short mark
  between two lines.
- Decorations from extensions are drawn only at the text level of detail. Several
  texts on one line (two problems) follow each other.
- **Code lenses** (`code_lenses`, `lens_place`) are drawn from `LENS_MIN_PX` (a line
  6.5 px tall), at 0.72 of the code's size, the lenses of a line joined by `|`. The
  card has no room between lines, so they go on the line above: at the line's
  indentation when the line above is empty there (most functions follow an empty
  line), after the text above when it fits in the card, and otherwise (the first
  line, a long line above) after the end of the line itself. A lens that runs
  nothing is fainter and cannot be clicked; the one under the pointer
  (`Views::lens_hover`) is in the accent colour and underlined. `lens_hits` are in
  screen pixels and a click on one comes before anything else of the card.
- The selection of the file being edited reaches one cell past the end of a line,
  for the line break. Every caret is drawn the same, with its line lit and its
  selection; only those on screen are drawn.
- `long_line` / `LongLine::draw`: the rest of a line cut at the card's edge, only
  at the text level of detail. Its background is the card's colour made opaque
  over the canvas, with the card's border, as the last chip; the line in the
  card gets the faint wash of the edited line. With the caret beyond the cut,
  the caret is drawn in the strip too (the one on the card stays at the edge).
  A wrapped card cuts nothing, so it never has one.
- On a wrapped card the edited line's wash covers all its rows, and a selection,
  a match, a problem or a decoration that crosses the end of a row is one
  rectangle per row (`Node::cell_rects`).

### crates/code-canvas/src/page.rs

- `scan_cache`: one file per folder path in `scans/` of the data directory.
  `Page::read_again` (F5, "Rescan") makes the next scan ignore it and read every
  file; the scans the app starts by itself, when files appear or go, use it.
- **Source control shows what is typed, not only what is saved.**
  `Page::preview_unsaved` replaces the entry of every unsaved file in
  `GitState::changes` with one made from its text against `HEAD` (`heads`, read
  once per file by `want_head`, thrown away whenever git is asked again because
  `HEAD` may have moved). `previewed` keeps what git itself said for those paths,
  to put it back when the file is saved, discarded or undone to no changes. A
  file whose text equals `HEAD` leaves the list even if the disk still differs.
- `changed_dirs`: every folder above a changed file, for the explorer's colours.
- `SearchResults::current`: the match the find bar is on, drawn stronger.
  `App::find_refresh` searches the text the card shows (smart case: exact when
  the text typed has a capital), so its columns are the canvas's; replacing goes
  through `Editor::pos_at_display` and starts from the last match so the earlier
  ones keep their place.
- **The diff view swaps the text of the card.** `Page::sync_diff` gives every
  changed file (`MAX_DIFFS` at most) the text `git::unified` makes of it, with
  `Repo::replace_source`, and keeps the rows in `diffs`: the layout, the symbols
  and the drawing stay line based and need no second kind of card. The price is
  that such a card numbers *rows*, not lines of the file: whatever comes from
  outside in file lines (problems, decorations, where a definition is, a line to
  travel to) goes through `row_of`, and what goes out (hover, go to definition,
  the caret of a click) through `line_of`.
- **A commit on the canvas is the same swap with other texts.** `Page::show_commit`
  reads, off the thread, the files a commit changed and what each was before and
  after it (`git::commit_files`: one `diff-tree` for the names, one `cat-file
  --batch` for both texts of all of them; `MAX_DIFFS` files at most, a merge
  against its first parent). While `Page::showing` is there `sync_diff` diffs
  those two texts instead of `HEAD` against the file, `relayout` keeps only those
  files, and the overlay marks them instead of the working changes. A file of
  the commit that is not in the folder now has no card: `Showing::here` counts
  the ones that have. `frame_wanted` tells the app the layout became another one:
  it frames the commit, or goes back to `App::commit_from`, the place the canvas
  was at before the first commit was shown.
- Rows of a commit are lines of the file *as the commit left it*. Nothing that
  works in lines of the file on disk is right on such a card, so the card menu
  offers no change there, and editing a file takes its diff away as always.
- **One change of a file** (`GitOp::Part`): `git::part` takes the `-U0` diff of the
  file (index to worktree, or `--cached` to unstage), cuts it in its hunks
  (`cuts_of`), and gives the one at the line to `git apply --unidiff-zero`
  (`--cached` to stage, `--cached -R` to unstage, `-R` to discard). The line asked
  is one of the file on disk, as the canvas has it, so the hunks of the staged
  diff, which are in lines of the index, go through `in_worktree` first. It all
  runs at the top of the repository: `git apply` in a folder under it reads the
  paths of a patch its own way. `holds` is the one rule for "this line is in that
  change", shared with the menu: a change that only removes lines belongs to the
  line after it and the one before.
- The card menu (`ui::CardMenu`, right click without moving) offers a change only
  where `FileChange::hunks` has one and the file has nothing unsaved: the hunks
  of an unsaved buffer are not what git would cut.
- **Completions** are asked after every edit that leaves a word or a mark other
  than a blank or a closing one before the caret (`App::ask_completion`): the text
  goes to the host first, without the pause other edits wait, then `complete` with
  the cell of the caret and, when what was typed is not part of a word, that
  character as the trigger. The host asks the providers that declared it (all of
  them for a word), keeps the items whose filter text has the typed letters in
  order, sorts them (the preselected, the ones that start with what was typed, then
  `sortText`) and sends `MAX_COMPLETIONS` of them as plain rows: label, detail,
  kind, the text to put, where it starts (a cell of the caret's line) and, for a
  snippet, where the caret goes and how much to select (`plainSnippet` drops the
  stops and keeps the first one's place). An item that replaces text of another
  line is left out.
- A reply is taken only if it is for the last request (`Extensions::completing`)
  and the caret is still where it was asked from (`App::offer_code`). Until the
  next reply comes the list shown is the old one without the names that stopped
  fitting (`update_completion`), so it does not blink while typing.
- Taking one puts its text from its own start to the caret, each further line
  indented as the caret's, and then asks `completionExtra` for that item: the
  host resolves it (`resolveCompletionItem`) and sends its `additionalTextEdits`,
  which is how an import arrives. They are applied only above the caret's line
  (`Editor::edit_before`), where they cannot move what was just typed sideways.
  The host keeps the items of the last `complete` only (`offered`), by a serial
  the app gives back: an older list answers with nothing.
- **Exact relations.** `Page::toggle_symbol` selects with the relations the names
  give and leaves the symbol in `want_exact`; `Extensions::request_relations` asks
  the host three things at the definition, each answered on its own as an
  `Event::Relations` (`ext::Found`), which `Page::exact_relations` hands to
  `Repo`:
  - where it is used (`locations`, kind `Reference`) → `refine`: references and
    incoming calls not confirmed go, confirmed places the names missed are added
    (a call if the token there is one);
  - the calls it makes (`outgoingCalls`: `prepareCallHierarchy`, then
    `provideCallHierarchyOutgoingCalls` for each item) → `refine_calls`: every
    guessed outgoing call goes; one relation per callee, at its first call, the
    target calling itself left out, as the guess does;
  - what implements it (`locations`, kind `Implementation`) → `refine_implementations`:
    the guessed implementations go; the target itself is left out.
  `symbol_named_at` finds the symbol a server points at: the one whose name starts
  there, else the first defined on that line (the place may be another part of an
  `impl` line). A callee with no symbol there still gets its curve, nameless.
  Places outside the folder (the standard library) are dropped by the app
  (`Found::with_paths`); a call with either end there goes.
- `Relations::exact` is per kind; the foot of the Symbols panel (`ui::exact_note`)
  names the ones a server gave and says "the rest by name". Definitions are never
  replaced.
- No answer leaves the guess: no extension for the language, one still starting,
  an error, an empty list of uses or implementations. Outgoing calls are the
  exception: when a server prepared the function and answered (`known`), an empty
  list means it calls nothing and the guesses go.
- Every relation request carries `deadline` (`RELATIONS_WAIT_MS`, 20 s): the host
  gives each provider a token of its own (`askUntil`), cancels it when the time
  is up and answers as if there were nothing. Until an answer comes the guesses
  are drawn; a late answer for a name no longer selected is dropped. Nothing is
  kept between selections: selecting again asks again.
- Places come in lines of the file and go through `row_of` for a card that shows
  a diff.
- A file that changes on disk does not drop the selection any more:
  `Page::reload_file` selects the same names again (`select_again`: same file,
  nearest line), which also asks for their exact uses again.
- The file being edited is never shown as a diff: its caret is in file lines.
  `diff_due` notices that the edited file changed and asks for a new pass.
- `Diffed::plain` is the file's real text as the canvas would show it, kept to
  tell what an outside change touched (`App::on_disk_changes`).
- `heads` are thrown away only when `HEAD` moves (`heads_at` against
  `GitState::head`), not at every look at git.
- Side by side, a line to travel to is aimed at the same row of the old column,
  so both sides are on screen (`App::follow_now`).
- `diff_settled`: whether a file already looks the way it is going to. Travelling
  to a changed line waits for it (`App::follow_now`), or the fold that arrives a
  moment later would move the line away from where the camera went.
- `reveal`: a file that appeared on disk; when the scan it caused is done the
  canvas goes to it.
- `PageState::Waiting`: opened but not read, a folder restored from the last
  session that has not been shown. `Page::ensure_scanned` reads it when the user
  goes to it.
- `Page::portable_camera` / `Page::place_camera`: the camera in a form that does
  not depend on where in the window the page is drawn, namely the world point in
  the middle of its pane and the zoom in logical pixels. This is what the session
  stores, and what makes cameras survive panels opening and closing
  (`Page::pane_center`).
- `Page::begin_edit` only edits files that are valid UTF-8, text that survives a
  round trip, so nothing is lost on save.
- `Page::editors` keeps files with edits in memory until saved, so leaving a file
  does not lose what was typed.
- **`Page::show_edit`, `Page::reload_file`, `Page::file_made` and `Page::file_gone`
  expect the caller to drop the page's cached text first** (`TextLayer::drop_page`).
- `show_edit` clears the selections (symbols and lines of that file moved, so what
  pointed at them is stale); the other three select the same names again.
- **One file at a time.** A change of one file costs that file and the folders
  around it, not the folder:
  - its text (`show_edit`, `reload_file` → `card_changed`): the repository's file
    and symbol names (`Repo::reload_file`), its imports (`Deps::update_files`), its
    card and the folders up its chain (`Layout::resize_file`), and on the GPU the
    frames and origins of what moved and the card's blocks
    (`SceneGpu::card_changed`). The tokens of that file only are forgotten.
  - a file made or deleted (`file_made`, `file_gone`): `Repo::add_file` /
    `remove_file`, `Deps::insert_file` / `remove_file`, `Layout::insert_file` /
    `remove_file`, `SceneGpu::relaid`. File numbers move, so the token cache, the
    search and the hovered cell are dropped and the selected node is found again
    by path. The explorer's tree is made again.
  - With only changed or only connected files shown, or a commit, the layout is
    built whole as before (`layout_filter` has `only_files`).
  - Each logs `updated '<path>' (<why>) alone in <ms>` with what each part took;
    `with everything laid out again` when it could not. The scenario
    `edit/big-folder-one-file-at-a-time` checks those lines and, with the `count`
    step, that the layout, the symbol index and the dependencies are what building
    them again gives.
  - Measured, release build, the UI thread, before (everything built again, a
    rescan for a file made or deleted) and after:

    | | before | after |
    |---|---|---|
    | 30,721 files (`big-folder` of the QA fixtures): a line typed, the card grows | 40–43 ms, and 12–15 ms more for the symbol index when the typing pauses | 0.23 ms (layout 0.04, GPU 0.16), and 0.03 ms for names and imports |
    | the same: saved | 73 ms | 8.7 ms, of which 8.6 reading the file just written back from the disk |
    | the same: written by another program (40 lines more) | 85 ms | 1.0–1.4 ms (1,296 of 31,161 boxes placed again) |
    | the same: a file made | 755 ms (a rescan, the scan on its thread) | 8–11 ms |
    | the same: a file deleted | 735 ms | 9–11 ms |
    | the same: a file renamed | 805 ms | 18 ms (deleted, then made) |
    | cargo registry, 53,028 files / 19.4 M lines / 3.37 M symbols (`update_bench`, no GPU): a line typed | 356 ms (layout 68, scene 36, symbol index 252) | 2.0 ms (layout 1.9: the registry's root has 954 folders to pack again; names and imports 0.1) |
    | the same: a file read again | 342 ms | 3.2 ms (read 0.6, imports 0.08, layout 2.5) |

    A file made or deleted costs moving every node once and looking for its name in
    every file's text (to resolve again the imports that may name it): a few ms
    each on 30,000 files, linear but with a small constant.
- `Page::rebuild_scene`: the scene's colours come from the theme, so a new theme
  needs a new scene. `Page::relayout` keeps the selection by path.
- "Only changed" and "only connected" together: the changed files and what the
  selection connects them to.
- `Page::changed_files` leaves out files git reports as deleted.
- `RelFilters`: outgoing calls start at opacity 0.45, being usually the noisiest
  kind.

### crates/code-canvas/src/paths.rs

- A path is offered for completion when the caret is inside quotes (`"`, `'` or
  `` ` ``) and what is typed has a slash in it. The open quote is the last one
  before the caret, if an odd number of its kind came before.
- Relative paths are read from the folder of the file being edited; one that
  starts with `/` from the open folder. Only names are read from disk. `tidy`
  resolves `..` without touching the disk and gives up when the path climbs out of
  everything.
- `candidates`: at most `MAX` = 60 names, folders first, then files, each
  alphabetical, matched whatever the case. Hidden names are offered only once a
  dot is typed. A file does not offer itself.
- `completion`: scripts are imported without their extension from other scripts
  (`import x from "./file"`), as their tools expect; everything else is written
  whole. A folder ends in a slash so the next name can be typed straight away.
- Columns (`Partial::start`) are in characters, not bytes.

### crates/code-canvas/src/plist.rs

- `to_json` reads an XML property list (the older `.tmLanguage` grammars) into
  JSON: `dict`, `array`, `string`, `integer`, `real`, `true`, `false`, and `data`
  and `date` as strings; comments, `<?xml?>` and `<!DOCTYPE>` are skipped, the five
  named entities, numeric ones and CDATA are read. Anything else makes it `None`:
  the grammar is left out rather than half read.

### crates/code-canvas/src/prefs.rs

`prefs.json`, edited by the settings panel:

| Field | Meaning |
|---|---|
| `check_versions` | Ask npm and crates.io for the latest version of each dependency and show it next to its line (`versions.rs`). Only package names are sent. |
| `complete_paths` | Offer file and folder names while a path is typed in a string (`paths.rs`). |
| `complete_code` | Offer what the language's extension proposes while code is typed (`App::ask_completion`). |
| `check_updates` | Look for a newer release when the app starts (`update.rs`). |
| `follow_chat` | Keep an assistant's panel scrolled to what it is writing, until the user scrolls up (`webview.rs`). |
| `follow_changes` | Travel to the lines a file changed from outside (an assistant, another editor) has new, and light them (`App::follow_change`). |
| `edit` | The tool of the toolbar: `false` is the hand (clicks select, nothing is edited), `true` the pencil (a click in readable code puts the caret there). |
| `unwanted` | Extensions offered for a language and answered "Do not ask again" (`App::offer_checker`). |
| `share_name`, `share_writers` | The name the others see in a shared session, and whether those who join can edit from the start. |
| `line_numbers` | Number the lines of every card (`canvas_core::layout::set_line_numbers`). |
| `wrap` | Wrap lines wider than their card into the rows under them instead of cutting them (`canvas_core::layout::set_wrap`, off by default). |
| `codex`, `asked_codex` | Follow Codex from the session files it writes (`codex.rs`), and whether the app already offered it. |
| `grammars_everywhere` | Let a grammar of an extension colour the languages the app's own tokenizer knows too (off by default: only the ones it does not know, see `grammars.rs`). |
| `skip_dirs`, `skip_files` | The names of folders and files a scan never reads, when they are not the usual ones of `ScanOptions::default` (absent: the usual ones). `page::scan_options` is where every scan gets them; changing them reads the open folders again. |

### qa

Scenarios that run the real program: `qa/README.md` says how. The steps are the
ones of `script.rs`; a behaviour that needed a new step to be tested gets the
step. `qa/host/*.test.js` are run with node against `exthost/host.js`.

### crates/code-canvas/src/script.rs

Scripted driving, for tests and demos that must not touch the real mouse or
keyboard. An example and `scripts/screenshot.ps1` are in "Testing the real app" in
[ARCHITECTURE.md](ARCHITECTURE.md).

- Steps are separated by `;` and run in order. The table below is every step
  `parse_step` knows.
- `App::run_script` runs the next step only when the active page has finished
  loading, the camera has no target left, and the page has been framed.
- `Script::next` leaves 120 ms between steps so each one draws. `wait` steps are
  consumed there.

| Step | Syntax | What it does |
|---|---|---|
| `wait` | `wait <ms>` | Pause. |
| `fit` | `fit` | Show the whole canvas. |
| `locate` | `locate` | Frame the selected symbols. |
| `goto` | `goto <path>` | Select a file or folder and fly to it. |
| `select` | `select <name>` | Select the first symbol with that name. |
| `search` | `search <text>` | Search in files. |
| `left` | `left files\|search\|extensions\|problems\|changes\|help` or `left <publisher.name>`; `left none` | Open that panel on the side its icon is on, or that extension's own panel. `none` closes the left side. |
| `left settings` | `left settings`, `left settings:general\|appearance\|colours\|keyboard\|commits`, `left settings:<text>`, `left settings:close` | Open Settings, on a category, or searching for the text; close it. |
| `right` | `right symbols\|deps` or `right <view id or extension id>`; `right none` | Open that panel, or that extension's webview, on the side its icon is on. `none` closes the right side. |
| `preview` | `preview <path>` | Show a Markdown file of the folder rendered, or close it if it is the one shown. `open <path>` shows one from anywhere. |
| `drop` | `drop <path>` | What dropping that file on the window does: a copy in the selected folder. |
| `mark` | `mark <name>` | Leave an empty file `marks/<name>` in the data directory. `scripts/screenshot.ps1 -Marks` takes a picture at each. |
| `move` | `move <panel> left\|right [panel to sit before]` | Move an icon to a bar. `<panel>` is a key (`files`, `symbols`, `media`...), a view id or an extension id. |
| `click` | `click <x> <y>`, `click <path> <line> <column>` | A click on the canvas, in physical pixels of the window, or in the middle of a cell of a card (1-based, as `hover`): on a wrapped card that is the row the column is on. |
| `hover` | `hover <x> <y>`, `hover <path> <line> <column>` | The pointer resting on the canvas, in physical pixels or on a cell of a card (1-based; a column past the card's edge is where it would be if the card went on): what is under it reacts (the rest of a long line). |
| `ui` | `ui click\|right\|move\|down\|up <x> <y>`, `ui drag <x> <y> <x> <y>` | The pointer on the interface, in points: what a button, a menu or a dragged icon gets. The steps after it wait until it has played out. |
| `folders` | `folders` | Show the list of opened folders. |
| `collapse-all` | `collapse-all` | No argument. |
| `expand-all` | `expand-all` | No argument. |
| `connected` | `connected on\|off` | Show only connected files. |
| `deps` | `deps all\|selected on\|off`, `deps report` | The switches of the Dependencies panel: every edge, the edges of the selection. `report` logs "script: deps N of M edges drawn, bundled\|one by one, K counts" for the selection. |
| `changed` | `changed on\|off` | Show only the files git reports as changed. |
| `tab` | `tab <n>` | Switch to tab `n`, 1-based. |
| `zoom` | `zoom <factor>` | Zoom by a factor. |
| `quick` | `quick <text>` | Open "go to file" with a query typed in. |
| `palette` | `palette <text>` | Open the command palette with a filter typed in. |
| `edit` | `edit <path> <line> <column>` | Put the caret in a file; line and column are 1-based. |
| `type` | `type <text>` | Type at the caret. |
| `key` | `key enter\|backspace\|delete\|tab\|left\|right\|up\|down\|home\|end\|undo\|redo\|save\|escape` | Press a key in the editor. With a completion list up, the keys that walk and take it go to the list, as when typed. `save` with no caret saves every unsaved file. `escape` with several carets leaves one. |
| `caret` | `caret <line> <column>` | What Alt+click does there in the file being edited (1-based): adds a caret, or takes away the one that is there. Ctrl+D and the others are `press ctrl+d`. |
| `back`, `forward` | `back`, `forward` | Where the canvas was before its last jump, and forward again. |
| `account` | `account signin\|cancel\|sync\|signout\|menu` | The GitHub account: start signing in, give up, sync now, sign out, open its menu. |
| `tool` | `tool move\|edit\|draw\|rub\|clear\|colour <n>\|width <n>` | The pointer (`edit`), the hand (`move`) or the pencil (`draw`) of the toolbar; `rub` is the pencil rubbing out, `clear` takes away everything drawn, `colour` and `width` pick one of the pencil's by its place. `edit <path>` puts a caret whatever the tool is. |
| `diff` | `diff on\|off\|split\|together\|fold\|unfold`, `diff around <lines>` | Show what changed inside the changed files: removed lines above the new ones or old and new side by side, with or without the unchanged lines folded, and how many of them stay around a change. |
| `agent` | `agent <JSON of an event>`, `agent <name>` | What an assistant would send, as if it had arrived (a relative `path` is of the folder in front, such as a guest's shared one); or follow the one with that name. |
| `workspace` | `workspace put\|off\|forget <folder>`, `workspace all` | Put a folder of the workspace on the canvas, take it off, forget it, or put them all. |
| `share` | `share start [writers]`, `share stop`, `share join <invitation> [name]`, `share follow\|writer\|reader <name>`, `share unfollow`, `share panel [close]` | Share the session or join one, follow someone, let someone edit or only look. |
| `theme` | `theme <key or name>` | Change the theme: a built-in one or one of an installed extension. |
| `commit` | `commit <place in the graph>`, `commit <start of a hash>`, `commit off` | Show on the canvas what a commit changed, or the files as they are again. |
| `rename` | `rename <path>` | Start renaming a file in its row of the explorer. |
| `delete` | `delete <path>` | Ask to delete a file or folder (answer with `answer Delete` or `answer Cancel`). |
| `ignored` | `ignored <path>` | A click on a file git ignores, in the explorer: it comes onto the canvas. |
| `minimize`, `restore` | `minimize`, `restore` | Minimise the window and bring it back. |
| `interface` | `interface` | Hide the interface, or show it again. |
| `menu` | `menu <x> <y>` | The menu of a right click on a card, in physical pixels. |
| `part` | `part stage\|unstage <path> <line>` | Stage or unstage the one change at a line (1-based) of a file. |
| `drag` | `drag <x> <y> <x> <y>` | A drag of the left button on the canvas, in physical pixels: it draws (or rubs out) with the pencil, selects code with the pointer over a file, and moves the canvas otherwise. |
| `find` | `find <text>`, `find next\|previous\|close`, `find replace <text>`, `find replace-all <text>` | Find in the file being edited or selected, walk the matches, replace the current one or all of them (the pencil must be on). |
| `console` | `console <a line>` | Open the console and type that line and Enter into its shell (no `;` in it: that ends a step). |
| `press` | `press <keys>` | The command bound to those keys, or the key itself sent to the interface (`press escape` closes a menu). |
| `where` | `where` | Logs "script: the middle of the canvas shows (x, y) at zoom z": what a resize or a panel must not move. |
| `colours` | `colours <path> <line>` | Logs "script: colours of <path> line N: keyword let · ident x · …": the kind of each token of that line (1-based) as the canvas draws it, from a grammar or the app's own. |
| `touch` | `touch <path> <text>` | Append a line to a file behind the app's back (`\n` in the text is a line break): what another program changing it looks like. |
| `remove` | `remove <path>` | Delete a file behind the app's back. |
| `renamed` | `renamed <path> <new path>` | Rename (or move) a file behind the app's back. |
| `count` | `count` | Logs "script: N files on the canvas, M cards", then whether the layout, the symbol index and the dependencies are what building them again gives ("script: not what building again gives: …" with what differs). |
| `page` | `page <javascript>` | Run a line of script in the webview on screen: a click on something of an extension's page without the mouse. No `;` in it: that ends the step. |
| `git` | `git stage [path]`, `git unstage [path]`, `git discard [path]`, `git commit <message>` | Source control. Discarding asks first. |
| `install` | `install <publisher.name>` | Install an extension from Open VSX. |
| `ext-search` | `ext-search <text>` | Search the extension registry. |
| `format` | `format <path>` | Format a file with the installed formatter. |
| `command` | `command <id>` | Run an extension's command. |
| `lens` | `lens <path> <line> [n]`, `lens report <path>` | What a click on the n-th code lens (1-based) of a line (1-based) does; log every line's lenses as "script: lenses of <path> line N: a \| b". |
| `tree` | `tree <row>`, `tree expand\|collapse <row>`, `tree button <command> <row>`, `tree title <command>`, `tree report` | In the tree cards open: a click on the row with that label (opens or closes it, runs its command), a row's button or a title's button by command id; log the rows of each view as "script: tree <view>: a / v b /   c". |
| `answer` | `answer <choice>` | Press that button of the question the app is asking, or of the notice that is. |
| `open` | `open <path>` | Show an image, video or PDF in the viewer; a path in the folder, or anywhere on the disk. |
| `focus` | `focus page\|app` | Give the keyboard to the webview on screen, or back to the window: what clicking one or the other does. |
| `updates` | `updates` | Look for a newer version of the app now. |
| `resize` | `resize <width> <height>` | Resize the window, in physical pixels. |
| `maximize` | `maximize` | Maximize the window. |
| `quit` | `quit` | Leave. |

`UiState::open_folder_menu` exists for `folders`: a script has no click to open
the menu with.

### crates/code-canvas/src/session.rs

- What survives between launches, as JSON in the data directory (locations in
  [Quick reference](#files-in-the-data-directory)).
- `PageSession::camera` is the portable camera of `Page::portable_camera`: the
  world point in the middle of the page's part of the window, and the zoom in
  logical pixels per world unit.
- `Session::window` is the logical window size. `Session::left_last` and
  `Session::right_last` are keys of panels (`Panel::key`). `Session::rails` is the
  order of the icons of each bar. `Session::single` shows one folder at a time
  instead of all on one canvas.

### crates/code-canvas/src/agents.rs

Assistants seen on the canvas while they work (`app/assistants.rs` draws them).

- **The app listens, it does not ask.** At startup it takes a UDP port of this
  computer only and writes its number to `<data dir>/agents.port`. Whoever wants
  to be seen sends one JSON object per datagram to it. Nothing is kept: an
  assistant that says nothing for `GONE_AFTER` is gone.
- **The plain form**, for any program:
  `{"agent": "<an id of its own>", "name": "Codex", "action": "read|edit|search|run|think|wait|done", "path": "<file or folder>", "line": 120, "lines": 30, "note": "why"}`.
  Only `agent` is needed; `line` counts from 1. Two more are optional: `find`, a
  line of text to look for in the file when the line number is not known (an
  edit), and `session`, the assistant a helper belongs to.
- **Claude Code** is heard through its hooks. `code-canvas --agent-hook claude`
  is the command of the hook: it reads the hook's JSON from its input, cuts the
  long texts out of it (`shortened`: a datagram holds 64 KB and an edit can be
  larger), sends it wrapped as `{"source", "hook"}` and ends, before anything of
  the app proper starts. `connect_claude` writes the hooks into
  `~/.claude/settings.json` (`with_hooks`: only the entries whose arguments have
  `--agent-hook` are ours, the rest of the file is kept, and a copy of the file is
  left beside it), as commands run without a shell and `async`, so a tool of
  Claude never waits for the app.
- One session is one assistant; a tool used inside an agent of the session comes
  with `agent_id` and `agent_type`, and is another one. `PreToolUse` says where
  it is going (`Read` with its `offset` and `limit` is a range of lines, `Grep`
  and `Glob` a folder), `PostToolUse` of an edit what it wrote, `Stop` that it
  waits for the user, `SubagentStop` and `SessionEnd` that it leaves.
- **An edit has no line**: the hook gives the text written, not where. The first
  line worth looking for is kept in `Agent::find` and `poll_agents` looks for it
  in the file the canvas has, for `LOOKED_FOR`: the file on the canvas changes a
  moment after the hook says so.
- **Claude Code's settings are someone else's file.** `read_settings` gives an
  empty object only for a file that is not there; one that cannot be read, is
  not UTF-8 or is not JSON is an error and nothing is written. `with_hooks`
  refuses a shape it does not understand instead of replacing it, takes out only
  the hooks that are ours (also from an entry shared with the user's own), and
  keeps the order of the keys (`serde_json` with `preserve_order`, for the whole
  app). The copy beside the file is made once, of what was there before the
  first change, and the write is a new file renamed over the old one.
- The hook runs the program that connected it. A copy of the app started from
  another place repairs the path at startup (`offer_claude`), unless it is a
  build in a `target` folder: a developer's build never takes the hooks.
- **More than one app can listen**: each writes its port as a file name in
  `<data dir>/agents/`, the hook sends to all of them, a port nobody is bound
  to is cleaned at the next start and the own one on exit.
- Hooks are separate processes and arrive out of order: a tool event for someone
  who has left is not theirs to come back with, and a `SubagentStop` that names
  no agent is nobody's.
- Faces: fifty mascots made with Codex for the app (`assets/agents`, rounded
  squares of 128 pixels cut from sheets of nine), each with the colour of its
  ground in `COLORS`. **An assistant keeps its face**: the key of an assistant
  (its session, or session/helper) and the face it took are remembered in
  `faces.json` in the data folder (the last 600), so a session that comes back
  has the same face. A new one takes, of the faces nobody on the canvas has,
  the one given longest ago or never.
- **Following an assistant always reaches it**: when the file it works on is not
  on the canvas (in a folded or hidden folder, ignored, or outside every open
  folder, such as another working copy), `bring_into_view` unfolds and shows
  the folders on the way and `show_path` puts the file on the canvas, then the
  canvas goes there. A folder opened that way (outside every open folder) is
  taken off again when the assistant moves outside it or is no longer followed
  (`App::follow_page`, `let_go_of_follow_page`), unless something was typed in
  it, so following does not leave a folder behind for every place it went.
  The name taken from a prompt skips markup such as
  `<agent-message …>` (`spoken`).
- **Assistants have no names; they are called by what they are doing**
  (`Agent::task`, `Agent::name`): the main assistant by the first line of what
  it was last asked (`UserPromptSubmit`'s `prompt`), a helper by the
  description of the task it was handed (the `Agent`/`Task` tool's
  `description`, queued per session in `Agents::handed` and taken by the next
  helper of that session), a plain event by its `task`. With none it is called
  by its kind (`Claude Code`, `Explore`, `Codex`). `agent <text>` in a script
  follows the one whose name or kind it is.
- Following one sets `App::following`, the same travel a change on disk causes,
  each time it moves (`Agent::moved`) or its line is found.
- **A trail of where it has been** (`Agent::trail`, `Step`): each place read,
  edited or searched is a step, the last `TRAIL` (12) kept, each dropped by
  `poll` after `TRAIL_LASTS` (10 minutes). The same place again is the same step;
  a `PostToolUse` joins the step its `PreToolUse` made (`Agent::stepped`), and the
  line an edit is found on later goes to its step too (`Agent::found`, only while
  the step is the agent's latest move). It goes with the assistant: one that leaves
  takes its trail. `trails_view` puts on screen only the steps whose file is on a
  card shown (one lookup per file a frame, not per step); `agent_trails` in
  `ui.rs` draws a ring (read, searched) or a dot (edited) left of the first line,
  the lines barely washed, a dotted curve from each step to the next, all fading
  with `Step::left`, and the tip of the one under the pointer. With trails on
  screen it asks for a frame every 20 s, so they fade with nothing else happening.
  `Prefs::trails` turns it off.
- **Assistants of others in a shared session** are in the same `Agents::list`
  with `Agent::from` set to the person they belong to (see the share section):
  `Agents::agent` never takes one of them for one heard here, and only the ones
  with no `from` are sent to the others.

### crates/code-canvas/src/codex.rs

Codex followed without touching it: the app reads the session files Codex
writes as it works and tells `Agents` what each line says, as plain events.

- **Why the session files.** Codex (0.169 here) keeps every session, in the
  terminal, in `codex exec` and in its VS Code extension alike, in
  `$CODEX_HOME/sessions/<year>/<month>/<day>/rollout-<time>-<id>.jsonl` (`CODEX_HOME`
  is `~/.codex` unless set), one JSON object a line, written as it goes. Its other
  ways out were weighed: `notify` in `config.toml` only runs at the end of a turn;
  its hooks (`hooks.json`, shaped like Claude Code's) would mean writing into the
  user's Codex configuration, which the app leaves alone; `codex exec --json`
  only exists for a session the app itself starts. The files need nothing from
  the user and nothing of Codex is changed.
- **What a line says** (`Session::hear`, all of it unit-tested on lines shaped like
  the real ones): `session_meta` gives the session's id, folder and source
  (`exec` leaves when its task is complete; a `subagent` with `thread_spawn` is a
  helper of `parent_thread_id`, named by its `agent_role`; the internal ones,
  `compact`, `memory_consolidation`, guardians, are not shown). `turn_context`
  moves the folder. The name is the first thing asked (`item_completed`
  `UserMessage`, `user_message`, or a user `message` that is not one of the texts
  Codex adds itself: `<environment_context>`, `# AGENTS.md`…; the IDE's
  "## My request for Codex:" is cut to the request). Where it is: a
  `CommandExecution` item (or `exec_command_begin`) with Codex's own reading of the
  command (`parsed_cmd`: `read` a file, `search`, `list_files`); a `FileChange`
  item (or `patch_apply_begin`) with each file changed and its unified diff, whose
  first hunk gives the line; `apply_patch` (`*** Update File:`… and the first line
  written, looked for like Claude's edits); the older `shell`, `shell_command`,
  `exec_command`, `read_file`, `list_dir`, `grep_files`, `view_image` calls; and the
  code-mode `exec` tool, whose JavaScript is read for its string literals (a
  `cmd:` is a command, a patch is a patch). `task_complete` is waiting for the user.
- **A command line is read like a person would** (`shell::understood`): split on
  `;`, `&&`, `||`, new lines and pipes, quotes kept together, redirections dropped;
  `cat`, `type`, `Get-Content`, `head`, `sed -n 'a,bp'` are reads (with the range
  of lines when the command or a `| head` / `| Select-Object -Skip -First` says it),
  `rg`, `grep`, `Select-String`, `findstr`, `git grep` searches (`for <pattern>`),
  `ls`, `dir`, `Get-ChildItem`, `find`, `fd` searches of a folder, anything else is
  running a command. A path with `*` stands for its folder.
- **Whatever is not understood is let be**: an unknown line, field or tool gives
  nothing, or "thinking" with the tool's name; a line that is not JSON is skipped.
- **Following the files** (`Tail`, `Follower`, a thread of its own): the folder of
  sessions is watched (`notify`), and every 1.5 s the newest two day folders are
  listed and the files known are measured. **The size, not the time**: on Windows a
  rollout keeps the modification time of its creation while Codex writes to it,
  and change events can come late, so a file is known to have grown by its length
  (read through an open handle) and only the bytes after the last whole line are
  read; half a line waits for the rest. A file that has not grown for five minutes
  is measured every 15 s. Files are opened sharing everything and closed at once,
  so Codex is never in the way.
- **What was said long ago is not told again.** At start, a file whose last line
  is older than 15 minutes is not read (its beginning is read silently if it grows
  again, `codex resume`). Lines older than a minute are read for what they set
  (folder, name) but not told, except the last thing said when it is less than 15
  minutes old, so a session at work before the app started is where it was.
- On in `Prefs::codex` (off until the user connects it, or accepts the offer made
  once when `~/.codex` is found: `App::offer_codex`, after Claude Code's own offer).
  Turning it off lets the faces of Codex leave. An assistant of Codex is keyed
  `codex:<session id>` (`codex:<parent>/<id>` for a helper), so it keeps its face.

### crates/code-canvas/src/share

A session shared between apps: `ot.rs` is how two texts edited at once agree,
`wire.rs` what crosses the network, `mod.rs` the session, and
`app/sharing.rs` what the app does with it.

- **The one who shares is the centre.** Everyone else has one connection, to
  them; what a guest does reaches the other guests through the host. A folder
  has the id of the host's page, a person the number of their connection (the
  host is 0), and the host never takes a guest's word for who they are.
- **A guest's folder is a real folder**: the files of the host's page written
  under `<data dir>/shared/<process>/<folder id>/<name>` and opened as any other
  page (`Page::shared`). So everything a page does works on it with no second
  kind of page, and what arrives is a file changing on disk, which the canvas
  already lights up and travels to. The folder is deleted when the session ends
  and whatever is left of an earlier run at startup (`share_setup`). Only text
  the canvas shows is sent: no `note`, at most `LARGEST_FILE` each and
  `MOST_TEXT` in all, and a page of more than `MOST_FILES` is not shared.
- **The text of a file is its lines joined by `\n`** (`share::body`,
  `Editor::display_source`): line ends and the last newline stay with whoever
  has the file. Both ends keep that text for every file (`Doc::text`), which is
  what an edit is made against.
- **Edits are found by looking, not by being told.** Every pass (`poll_share`,
  with the rest of the background work) the text of each editor whose `rev`
  moved, and of each file the watcher named, is compared with the kept one;
  `TextOp::between` makes the edit: what is between the common start and end,
  or, when that spans lines on both sides, a diff by lines, so a formatter that
  touches the top and the bottom of a file does not replace its middle under
  someone's caret. Inside a line (or the one changed place) what is left is
  diffed by letters when it is small (`TextOp::middle`, up to 4096 letters):
  typing at several carets of one line is one insertion per caret, and what
  someone else typed between them at the same time is not removed and put back. Nothing in the editor knows it is shared. `share_look_over`
  compares the files of the page with the ones shared every two seconds: that is
  how a new file, or a deleted one, is noticed.
- **Two edits made at once** end the same on both sides by the usual transform
  (`TextOp::transform`, the host's insertion first when both are at one place)
  and a `Link` per file and connection: each message says how many of the
  other's edits its sender had seen, the receiver drops the ones of its own that
  were seen and passes the incoming one over the rest. An edit that does not fit
  (a reader's, or a bug) is answered with the whole file and a new `epoch`,
  under which the old messages still on their way are ignored.
- An edit that arrives goes into the editor of the file (`Editor::apply_remote`:
  the caret keeps its text, and it is not a step of undo). At the host an
  editor is made for it if there is none: what a guest types is *not saved* at
  the host until someone saves, which is `Msg::Save` from a guest. At a guest
  with no editor the kept text is written to the file and the watcher shows it;
  with one, the file is written too, told to the watcher as its own
  (`Watcher::expect`), and the editor counts as saved, **unless it had text of
  the guest's own not saved yet**: then it stays not saved (`Sharing::typed`,
  noted before the first edit of a pass is applied, `Editor::written_as` in
  `share_settle`). Before, an edit from the host (another program appending
  to the file there) made the guest's editor "saved", its `Ctrl+S` did nothing,
  no `Msg::Save` went, and what the guest typed was never written on the
  host's disk. Files not saved of a shared folder do not hold the guest's app
  from closing (`App::unsaved`), but the guest's saving while typing does save
  them (`share_unsaved`, added to what autosave saves), which sends a save to
  the host like `Ctrl+S`. `Msg::Save` says whether to format (`Ctrl+S` does,
  autosave and a file changed on the guest's disk do not).
- **A guest's save keeps what another program wrote on the host's disk.** The
  host's editor takes in what changed on disk under the text not saved
  (`Editor::take_in`), as for its own files; and when `Msg::Save` comes before
  the watcher has said so, `Event::Save` reads the file, and if it is not what
  the editor read (`Editor::read_as`), takes it in first (`on_disk_changes`),
  then saves. The text taken in reaches everyone as one more edit.
- **Readers**: `Page::read_only` refuses to start editing and to make, rename
  or delete files, and the host refuses the edits anyway (`Session::edit`),
  sending the file back. Nothing a reader's disk does is sent: a file changed
  or deleted there is written again as the session has it (`share_flush`), so
  a reader never looks at text nobody else has.
- What is typed into a shared text has no control characters but the line
  break and the tab (`Session::edit`), and a name has none at all, nor the
  marks that turn the direction of text (`wire::turns_text`): a name is shown
  in notices and beside carets.
- **Where someone is** (`Seen`) is said in terms of the layout, not of pixels or
  world places, because two apps lay the same folder out differently (folded
  folders, hidden files, a diff open): the node at the middle of the view and
  the offset in it, how much of the world the view takes, the node under the
  pointer and the offset in it, and the caret as a cell of a file. The other
  side finds the node by its path (`share_place`). Following fits what they see
  into the own canvas (`share_follow`); a drag, the wheel or Esc lets go.
- **The wire**: a greeting in clear (`CCSH1` and 16 random bytes each way),
  then frames of length and a sealed body (ChaCha20-Poly1305, one key each way
  made by HKDF from the secret of the invitation and both greetings, a counter
  as nonce). The body is a flag and JSON, deflated from `PACKED_FROM` bytes on.
  Who has not the secret cannot open the first frame and is dropped; there is no
  other password. The invitation is `cc1:<address>:<port>:<secret in hex>`;
  the address is the one of this network unless another is written in the panel.
- **Sharing is not limited, it is warned about.** Someone who shares with their
  own second computer wants all of it to work there, so a guest's folder is a
  folder like any other: extensions and assistants get it, and what they (or
  any program) write in it reaches the host. What that opens up is said once
  each time, in the app's own question, before sharing and before joining
  (`share_ask`, `Asked::Share`): only with someone you trust. For a while a
  guest's folder never reached an extension; that made the guest's right bar
  useless and was taken out.
  - **What changes on a guest's disk** goes the way the host's does, the other
    way round (`share_flush`, the part for a guest who can edit): a file whose
    text is not the session's is sent as an edit and a save, a file the session
    does not have is `Msg::Make`, one that is gone is `Msg::Drop`. The host
    writes or recycles the file and its own watcher does the rest, so everyone,
    the maker included, hears of it the usual way. A file made is taken only
    where there is none (`Sharing::made` lets its maker write it again until it
    is shared), and only one the guest's own canvas shows, so what is ignored
    stays where it was built.
  - A guest's folder is written by the session too, and the watcher names those
    writes like any other. `Sharing::mirrored` keeps a mark of the last text
    put in each file: a file that still has it was not changed by anyone here,
    however far the session's text has moved on since.
  - What stays checked is not a limit on anyone's work: a path that arrives is
    checked before it is anything (`wire::safe_path`): no `..`, no drive, no
    `.git` in any spelling Windows takes for it. A page under
    `<data dir>/shared` does not run git (`Page::no_git`): there is no
    repository there to ask; source control for a guest has to be asked of the
    host, which is not done yet.
  - Before the secret is proven nothing is set aside: the greeting has a deadline
    for the whole of it (`Hurried`), the first message is at most `wire::FIRST`
    bytes, a message is read as it comes and not into a buffer of the size it
    claims, and at most `MOST_KNOCKING` connections are being greeted at once.
  - What waits to be sent to one connection is counted (`Conn::waiting`); past
    `MOST_WAITING` that connection is closed: someone who asks for files and
    does not read them cannot fill the memory.
  - A guest takes at most what a host would send (`LARGEST_FILE`, `MOST_FILES`,
    `MOST_TEXT`, each with room) and ends the session past that. Where someone
    is (`Seen`) must be finite numbers and short names (`Seen::sane`), or one
    message would leave every follower's camera nowhere.
  - A save from a guest is for a file that is shared, not for any page whose
    number is guessed. Sending someone away changes the secret: the invitation
    they have stops working.
- **When the two texts disagree the host's is the true one.** Remote edits of a
  pass are applied one by one (`share_remote_edit`) and compared once at the end
  (`share_settle`): a guest takes the session's text, a host sends its own as
  one more edit. Comparing after each edit against the text after all of them
  replaced whole files and lost the caret.
- What keeps the host's text and the session's the same: every editor has a mark
  no other editor ever had (`Editor::rev` comes from one counter, so a file
  reloaded is not taken for the one before it), files the watcher names while a
  page is being read again wait for it (`touched`), and what the app itself
  writes for an extension is named too (`write_file`).
- A text never has a carriage return (`body`), and an edit that brings one is
  refused: the editor would drop it and never again agree with the session.
- `Link::waiting` is let go when the other side says what it has (`Msg::Said`,
  every `SAID_EVERY` edits it receives without sending): someone who only reads
  made the other keep every edit of the session.
- The wire has a version (4 now: `Agents`, and `format` in `Save`; 3 brought
  `Make` and `Drop`): an app of another version is told so, instead of failing
  on a message it does not know.
- **Everyone's assistants on everyone's canvas** (`share_agents_out`,
  `share_agents_in`). Each app sends `Msg::Agents` with the list of its own
  assistants (`Agent::from` is `None`) that are at or have been in a shared
  folder: a `Helper` per assistant, its place and trail as a folder id and a
  path in it (`Place`, `share_place_of`), what it does as `Doing::code`, and
  how long ago each thing was heard in milliseconds (`ago`, so no two clocks
  need to agree). Steps outside the shared folders are left out, and so is an
  assistant with nothing in them. It is sent only when the list without the
  ages changed, at most every `AGENTS_EVERY` (400 ms), and again after
  `AGENTS_AGAIN` (8 s) if someone was heard since, so a remote one does not
  turn quiet while it works. A list is a few hundred bytes.
- The host passes a guest's list on to the others (`who` is the sender's
  number, which the host sets itself), keeps the last list of each and its own
  (`Session::agents`, `own_agents`), and gives them to who joins, after the
  people (`agents_to`), with the ages grown by the time they were kept. A list
  that is not `Helper::sane` (too many, long texts, a path that is not safe) is
  dropped; one of someone who left goes with them (`Event::Agents`).
- On arriving, every remote assistant becomes an `Agent` like the ones heard
  here, with `from` the person's number, key `<who>:<key>`, its place a path
  under the page of that folder here and `at` set back by `ago`: drawing,
  following, the trail and the search of an edited line need nothing else.
  They are made anew from `Session::agents` on each list, on `FolderReady` and
  `FolderGone`, and all go when the session ends. `share_owner` gives the
  person's name and colour: a small circle with their initial on the face (in
  the rail and on the canvas), their name after the tag, in the tooltip and in
  the following bar.
- A guest's folder is held by a file it keeps open (`.held`); a second app
  starting cleans only the folders nobody holds.
- One thread reads each connection and one writes it, so the app never waits on
  a socket; they wake the loop as the extension host does. `Session::close`
  says `Bye`, and it is called when the app exits and when the session is
  dropped.
- `CODE_CANVAS_SHARE_ON` is the address to listen on instead of every one of the
  computer: tests use `127.0.0.1`, which the firewall of Windows does not ask
  about. Under a script the invitation is also written to `<data dir>/invite.txt`.

### crates/code-canvas/src/themes.rs

- `theme.json`:

  ```json
  { "theme": "dark", "colors": { "card_border": "#4b5362" }, "icons": "symbols" }
  ```

- `theme`: key of a built-in theme (`theme::PRESETS`), or of a theme of an
  extension: `ext:<extension id>:<label>` (`ColorThemeDecl::key`); unknown or
  empty means the default one.
- A theme of an extension is read when it is chosen and at startup
  (`theme_of_extension`) and kept in `ThemeFile::from_extension`, which is not
  written: `base` gives it while the key is one of an extension, and the default
  theme if the extension is gone.
- `from_vscode` reads the file and the ones it `include`s (the included first, so
  the including one wins), through `plain_json`: theme files carry comments and
  commas before a closing bracket. `theme_of` makes a `Theme` out of it:
  - `editor.background` is the colour of the *cards* (it is where code is); the
    canvas behind them is `editorPane.background` or, for want of it, the same
    colour darkened, so cards stand out as they do in the built-in themes.
  - Panels are the side bar, the window's frame the activity bar. Rows, buttons
    and borders are the side bar's colour moved towards the text's: themes set
    their own borders to transparent or to their accent too often to trust them.
  - A colour with alpha is laid over what it will be on (`over`): the theme
    struct has no place for "see-through over the side bar".
  - Code: for each kind of token the app tells apart, the scopes it could be, most
    telling first (`token`). A rule fits a scope it is or begins (`keyword` fits
    `keyword.control`); the longest fitting rule wins, the later of two as long.
    Of `a b c` only `c` counts, and a rule with ` - ` is left out. A kind with no
    rule is the plain foreground.
  - Grammars of extensions (`grammars.rs`) go the other way, from a token's
    scopes to one of these kinds, with the same scope names, so what a theme says
    about `storage.type` or `entity.name.function` lands about where VS Code puts
    it. Code has these eleven colours, not one per scope.
  - Relations keep the built-in colours: they have to be told apart from each
    other, which no theme promises.
- `colors`: colours changed by the user, key → `#rrggbb` or `#rrggbbaa`. Keys that
  no longer exist and values that are not colours are ignored. `set_color` forgets
  a change when the colour is set back to the base theme's value, so the file only
  lists what really differs. `set_base` switches theme and keeps the colours
  changed by hand.
- `icons`: the id an extension gives its file icon theme (`"symbols"`), `"none"`
  for the app's own glyphs, or empty to use one that is installed
  (`Extensions::icon_theme`).

### crates/code-canvas/src/ui.rs

The `Action` model is in "The app" in [ARCHITECTURE.md](ARCHITECTURE.md).

- The empty parts of the header move the window. That drag area is added *first*,
  so every button placed afterwards wins over it.
- `UiState::floating` lists interactive floating areas over the canvas (the
  completion list, for one): clicks on them are for them, not for the canvas
  underneath. For notices, only questions take clicks; plain ones let them through
  to the canvas.
- egui is always dark, whatever the system theme is (`apply_theme`).
- Lucide and Codicons are added as fallback fonts, so icon code points render
  anywhere text does. Codicons are needed because extensions name them in their
  texts, and a few panels that mirror VS Code's use the icons it does.
- **Settings is a window over the app, not a panel** (`ui/settings.rs`,
  `UiState::settings`): the user asked for room to see everything at once. It has
  its categories on the left and a search box that, while it has text, shows what
  matches from every category.
  - It is `egui::Modal`, which puts the backdrop and the content in one area. Two
    separate areas would swap places the first time the backdrop is clicked.
  - Esc is not egui's to handle here (`ModalResponse::should_close` is not used):
    the same key gives up on a shortcut that is waiting for its keys, and that
    must not close the window too.
  - While it is open the whole screen is in `UiState::floating`, so the canvas
    gets no pointer, and `page_places` is emptied, so web pages (which are real
    child windows, always above what we draw) hide instead of covering it.
  - Each section function says whether it drew anything; nothing drawn is "No
    setting matches".
  - Shortcut rows are sorted by the place of their group in `keys::COMMANDS`,
    whose rows are not grouped.
- The "go to file" box becomes the command palette when the query starts with `>`,
  as in VS Code. `quick_score` is a subsequence match on a lower-case query
  (`appts` → `src/app/types.ts`).
- **The window is boxes on a dark ground** (the concept video `git.mp4`): the
  ground is `chrome`; on it sit the left box (the bar of icons and, joined to
  it, the files), the canvas in a box of its own, black, with round corners,
  and the right bar when it has icons. `EDGE` is the strip of ground between
  them and `ROUND` their corners.
  - **The files are docked, everything else floats.** `UiState::files` is the
    one panel that is part of the window (`files_dock`); it has no cross and is
    hidden with `Ctrl+B` or the button at the top. Any other panel is a card
    over the canvas (`UiState::cards`, `cards`): several at once, side by side
    from the top right corner, the newest further in, each with a cross and an
    edge that is dragged to make it wider (`card_widths`). A card that does not
    fit waits until one is closed. The canvas goes on under and between them.
  - Opening a card moves nothing on the canvas; going somewhere does take the
    cards into account (`App::clear`: fit, a file, a symbol, the caret are
    framed in what the cards leave free, `UiState::cards_reach`, unless that is
    less than a third of the canvas).
  - The canvas is drawn on the whole window and the interface over it, so its
    round corners are the ground painted over its square ones (`enclose`,
    `corner`: a fan of triangles from the corner to the arc), with the border
    drawn last.
  - The version (`v0.4.3`) is the last thing at the right of the status bar;
    a click on it opens Settings, where updates are.
  - The status bar is a band of the ground under everything, 22 points tall
    (`status_bar`, `STATUS_H`): what is under the pointer on the left; on the
    right what extensions report, problems, what is not saved, the branch, an
    update. It was a small card over the bottom right of the canvas, and it hid
    what was under it. The version is in Settings; what a frame costs is the
    hint of the zoom number.
  - A card's cross is drawn over its top right corner after its content, so
    `panel_title` leaves room for it when it is told one is coming (`CARD_X`).
    The cross and the icons of a title are all 24 points wide with a glyph of
    14, on one line: the menu of Source Control is a `MenuButton` made from a
    button of that size, not `menu_button`, whose own padding made it narrower
    than its neighbours.
  - **A card can be put anywhere on the canvas and sized.** Dragged by its
    title it leaves the row of docked cards and keeps a place of its own
    (`Rails::cards`: the key of its panel to x, y, width and height from the
    corner of the canvas), saved with the bars. Its right side, bottom and
    corner make it larger or smaller (no smaller than 250 by 150); its left
    side too. A double click on its title docks it again. A free card is kept
    inside the canvas when the window shrinks, and it is not counted in the
    room the docked ones take (`cards_reach`).
  - **Files, Search, Source Control and Extensions are tabs of the dock**, not
    cards (`Panel::docks`, `UiState::dock`; `files` says whether the dock is
    open): each takes the place of the other. Its icon on the bar swaps the dock to it, with the
    folder switcher and the workspace above it (`folder_row`), so choosing
    another folder shows that repository's changes. Which one is docked is
    saved with the open cards (`UiState::open_keys`).
  - **Each folder of the workspace shows its branch** where its count of files
    was (`WorkspaceRow::branch`; the count is in its hint): from the folder's
    git when it is open (with `*` when something changed), else read from
    `.git/HEAD` every 5 s (`git::head_of`, which also follows a `.git` file
    and shows a detached head as its short hash). On the folder being looked
    at, a click on the branch opens the list to switch to another. Source
    Control has no section for the repository any more; Sync (pull, then
    push) is in its menu with Pull, Push and Fetch.
  - Nothing draws a line between the bar of icons, the dock and the header:
    the seam of their outlines is painted over in their own fill, the header
    has no separator, and the dock's resize line is drawn with no stroke (the
    pointer still turns into the resize arrows there). The tab of the icon
    whose panel is docked flares into the dock at both ends (`rail_tab`).
  - Problems is not an icon of the bar: its count is always in the status
    bar, and a click there opens it.
  - The faces at the bottom of the bar overlap like a stack of cards, each
    21 points above the one below with a ring of the bar's colour around it;
    the one above is drawn last, so it is on top.
  - **The bottom of the bar of icons** holds, from the bottom up: the GitHub
    account, Settings, Help, sharing, and then the faces of the assistants
    and the people in the session, one above the other, as many as fit. They
    were in the header, side by side; the share panel opens beside its button.
  - **The GitHub account** is each person's own, signed in from the app
    (`account.rs`, `app/github_sync.rs`, `github.rs`): GitHub's device flow.
    The app asks for a code, shows it, the person enters it on
    github.com/login/device with their account, and the app waits for the
    sign-in (`Waited`). It asks only for the `gist` scope. The app's id with
    GitHub (`github::CLIENT_ID`) is the only thing about it in the program:
    it is public by design, like the one every desktop app carries, and
    names no person. Without it the window says the version cannot sign in.
  - The sign-in is kept in `github.token` in the data folder, sealed with
    Windows' DPAPI for that Windows user (`sync::keep_token`); it is never
    written as it is, never logged, never in the repository.
  - **Settings sync** (`sync.rs`): `prefs.json`, `theme.json`,
    `keybindings.json`, `commit.json` and the list of extensions installed and
    switched off (`extensions.json`) go to a secret gist of the person's own,
    found again by its `code-canvas.json`. `github.json` keeps the gist and a
    digest of what was last in step (`State::synced`; a digest ignores
    spacing). On sign-in and at start `sync::plan` decides: nothing on GitHub
    sends this computer's; only one side changed since the last time, that
    side wins; a computer never set up takes GitHub's; both changed asks
    (`Asked::Sync`). Afterwards every 4 seconds what changed here is sent,
    not while extensions are being installed. Taking GitHub's writes the
    files and loads them again, installs the missing extensions and switches
    the others on or off.
  - The scenarios sign in to a GitHub of their own on this computer
    (`qa/fake/github.py`, header `github:`); the app is pointed at it with
    `CODE_CANVAS_GITHUB_SITE`, `CODE_CANVAS_GITHUB_API` and
    `CODE_CANVAS_GITHUB_CLIENT`.
  - **Customize the layout** (the button left of the window's buttons,
    `layout_menu`): the bar of icons, the dock, the status bar and the toolbar
    of the canvas are shown or hidden (`Prefs::rail`, `status`, `toolbar`;
    the dock is `UiState::files`), a button puts every part back, and two
    modes: full screen (`Action::ToggleFullScreen`, borderless) and only the
    canvas (`Action::ToggleUi`). The dock and its bar of icons go to either
    side (`Prefs::dock_right`): the bars swap sides, the dock becomes a right
    panel, and the rounded corners, the seam and the tab of the docked icon
    follow (`rail_frame`, `rail_tab`).
  - The Timeline under the files folds away by its title (`Prefs::timeline`):
    closed, it is a row of its own height that cannot be dragged taller.
  - **The question before deleting shows what is about to go** (`ui::Preview`,
    `App::preview_of`): a picture is drawn in the question, a video, a sound or
    a PDF is a page of its own placed over it (`media::PEEK`, not started, shut
    when the question goes), a text file shows its first twelve lines and a
    folder what it holds. Each question is a modal of its own identity, so one
    is not as tall as the tallest before it.
    The viewer has its own cross and a web page gets a title row to carry it.
  - `Panel` names every panel and `UiState::show` opens one, wherever it goes.
    Which bar an icon is on is `UiState::side_of`: the app's own start on the
    left, what an extension brings (`Panel::Webview`) on the right
    (`Panel::home`), and any icon can be dragged to the other bar. `Help` and
    the gear stay at the bottom of the left bar. The right bar is not there
    while it has no icon, unless one is being dragged.
  - The icon of the files is a tab of them (`rail_tab`); the icon of an open
    card is lit.
  - The homes changed once (symbols and dependencies were on the right). An
    arrangement saved before that is taken home one time (`Rails::at_home`,
    `Rails::homes`); what is moved afterwards stays where it was put. For one
    version (0.3.3) the sides were fixed and a drop on the other bar did
    nothing: people arrange the bars as they like, so that went.
  - `rail_entries` lists the icons there are this frame; `Rails::arrange` puts
    them in order. An icon never moved sits on its side after the moved ones.
    One that is not there now (an extension's view whose condition is false, the
    media viewer before anything was looked at) keeps its place for when it is.
  - An icon is dragged by hand (`RailDrag`, `rail_drop_target`,
    `finish_rail_drag`): a bar takes a drop within `RAIL_REACH` points of it, and
    the button being up ends the drag even if the release itself was missed.
  - The move is applied after both bars are drawn (`UiState::rail_move`), so the
    drag, the icon's menu and a script all go the same way.
  - Several web pages can be on screen, one per card: each adds its place to
    `UiState::page_places` and `WebViews::show` takes them all.
- The filter rows of the symbols panel give the slider what room is left and drop
  the percentages below 244 points: the panel can be on the narrower left side.
- `text_fit` cuts what does not fit at the end, which then reads "…".
- The folder menu lays its rows out by hand, as wide as its longest name needs
  within limits, so that the menu does not stretch.
- **One change of a file is applied with the header a diff of only that change
  would have** (`git::alone`). `git apply` starts looking at the line the new
  side of the `@@` header names, and that number counts every change above,
  also the ones not being applied: with another change higher up, staging,
  unstaging or discarding one change landed lines away (and discarding
  damaged the file). The side the patch is applied to is exact already, so the
  other side is rewritten from it: a zero count means "after this line", so the
  start is one more for an insertion and one less for a deletion.
- **Discarding** reads the status again when it runs and sorts files by whether
  the last commit has them (`git::discard`): those it has go back to it, staged
  part included; those it does not have (a staged new file, the new name of a
  staged rename) go back to what is staged, and only if they changed since.
  The question lists exactly what will change (`FileChange::goes_back`,
  `git::what_is_lost`); with nothing to discard it says so and asks nothing.
  Paths go to git through `--pathspec-from-file`.
- `FileChange::from` is the old path of a rename: the committed text is read
  from it (`Page::want_heads`), so a renamed and changed file has a diff.
- **What the app wrote itself is told apart by what it wrote**
  (`Watcher::write` keeps a hash of the bytes; `Watcher::poll` drops a notice
  only while the file on disk is those bytes). It used to ignore the path for
  two seconds, and a program that wrote the file right after a save was not
  seen and then lost at the next save.
- **A card can show one text while its symbols are another's**
  (`Repo::replace_shown`): with the diff on, or a commit shown, the card has the
  diff's rows, but search in files, symbols and their uses are those of the
  real text, placed on the rows; a symbol in folded lines sits on the row that
  folds it. Hits keep the row for the canvas and the line for the panel
  (`Page::found_lines`). Find in the file being looked at still walks the rows.
- The wish to draw diffs again is kept while a commit is shown, even when no
  card shows one right now (`Page::diff_due`): leaving the editor of a commit's
  only file gave plain text for good. The timeline is read again when the head
  moves (`timeline_stale`), keeping its rows meanwhile.
- The palette's diff commands take the page's `DiffView` and only turn it on or
  off; they used to put side by side, folding and "around" back to default.
- Git panel: discarding a new file has nothing to go back to, so it removes the
  file (to the recycle bin). Of the commit actions, amending is the one that works
  without a message.

### crates/code-canvas/src/term.rs, ui/console.rs

The console: a card like any other (`Panel::Console`, `` Ctrl+` ``), with bash
in the folder that is open.

- **It is bash everywhere** (`term::bash`): the same commands work on every
  system and in what assistants and READMEs say to run. On Windows that is the
  one Git for Windows brings (`Git\bin\bash.exe`; the `bash.exe` of
  `System32` and `WindowsApps` only starts WSL and is passed over). When there
  is none the card says so and offers to install Git with `winget`
  (`Console::install`, on a thread, told when it ends) or to open its download
  page; nothing is installed without that click.
- The shell runs on a pseudo terminal (`portable-pty`: ConPTY on Windows) as a
  login shell, with `CHERE_INVOKING` so that Git's profile stays in the folder
  instead of going home. One thread reads what it prints; `vt100` keeps the
  screen and 5000 lines above it; `Terminal::poll` feeds it with the rest of
  the background work.
- **A terminal is asked things and must answer**: ConPTY starts by asking where
  the cursor is (`ESC [ 6 n`) and prints nothing until told, so a terminal that
  only listens shows an empty screen for ever. `Terminal::poll` answers that
  and "what are you" (`ESC [ c`).
- The screen is drawn a row at a time as one laid out line of runs of the same
  colours (`row_of`), in the interface's monospace font; the card's size in
  letters is what the shell is told (`Terminal::resize`).
- **Keys go to the shell while the screen has the focus** (a click on it gives
  it, a click elsewhere takes it): egui holds the focus, which is also what
  keeps the app's own shortcuts quiet (`on_key` returns while something is
  being typed into), and Tab, the arrows and Escape are kept from moving the
  focus (`set_focus_lock_filter`). `typed` turns a key into what a terminal
  sends. Ctrl+C arrives from egui as "copy": with nothing selected it is the
  interrupt. The commands of scope `Anywhere` still run (the palette, go to
  file, the console's own key).
- Shift+PageUp, Shift+PageDown and the wheel look at what scrolled away; typing
  comes back.
- "Open in terminal" of the files menu starts the console's shell there. It used
  to start a window of another program.
- Not there yet: several consoles, selecting text with the mouse, a console of
  the one who shares for those who join.

### crates/code-canvas/src/update.rs

- The latest release is read from GitHub (`REPOSITORY` =
  `sebastian642431/code-canvas`) when the app starts and every `EVERY` = 6 h while
  it stays open: one small request, nothing about the user in it. A newer one is
  offered once per version, in a notice. `check_now` works with the automatic look
  switched off too.
- **Replacing a running program on Windows:** its file cannot be overwritten, but
  it can be renamed. `swap` renames the running one to `code-canvas.old.exe`,
  gives the new one its name, and `clean_up` removes the old file the next time
  the app starts. If the second rename fails the first is undone: better the old
  version than none.
- `code-canvas.old.exe` can be taken: it is a program still running when the app
  was updated before and not closed since, or when two of them are open. Then
  nothing can be renamed onto it ("access denied") and the update failed for
  good. `swap` goes on to `code-canvas.old-1.exe` and the next ones (`ASIDES`),
  and `clean_up` removes all of them that nobody runs.
- What is trusted:
  - `parse_release` takes the download only from the releases of this repository,
    whatever GitHub's answer says. A program from somewhere else is not taken,
    whatever it is called. The asset must be `ASSET` = `code-canvas.exe`.
  - `install` rejects a download smaller than 1 MiB or not starting with `MZ`: it
    has to be a program, whole, not an error page or half a download.
- `newer`: pre-releases are never newer.
- `install` blocks and runs on a thread of its own.

### crates/code-canvas/src/versions.rs

- `package.json` and `Cargo.toml` are read for their dependencies; the latest
  version of each is asked from npm's registry or the crates.io index on
  background threads, and shown after its line: `✓ latest`, or `↑ 2.1.0`.
- **Nothing but the names of the dependencies leaves the machine**, and only when
  `check_versions` is on.
- `Checker`: `WORKERS` = 4 requests at once; an answer is good for an hour. A name
  asked and not found (no network) is remembered as unknown and nothing is said
  about it.
- Only what a registry can answer for is asked: a requirement that starts with a
  digit or one of `^~>=<v*`. Files, links, git and tags are skipped.
- `Cargo.toml`: in a `[...dependencies]` table every line is a dependency; in a
  `[dependencies.name]` table the `version` line is that crate's. A table with a
  `package` key renames the crate, and `package` is the name to ask for.
- `standing(wanted, latest)`:
  - versions compare as far as they are written: `1.2` is `1.2.x`;
  - with nothing to compare the answer is `Latest`: do not cry wolf;
  - `Newer` is something newer the manifest already accepts, `Major` something it
    does not. A caret range accepts the same first number that is not zero. `~1.2`,
    `=` and ranges with `<` do not take a new minor by themselves;
  - pre-releases (`2.0.0-rc.1`) are never "the latest".
- npm: a scope's slash is part of the name, not of the path, and is sent as `%2f`.
- crates.io: the index keeps a crate at a path made from its name
  (`se/rd/serde`, `3/s/syn`; see `crates_index_path`). Its answer is one JSON
  object per line, one per version, some of them withdrawn.

### crates/code-canvas/src/watch.rs

- What a batch does to the canvas is in `App::on_disk_changes`. A file changed
  from outside is compared with what the canvas had (`git::compare`), its new
  lines are lit (`Page::touch`) and, with `follow_changes`, the canvas travels to
  the first of them: never while a file is being edited, a drag is going on or
  within a second and a half of the user's last key or click (`App::last_input`).
- The operating system reports changes per path. `Watcher::poll` hands them over
  in batches once things have been quiet for `SETTLE` = 350 ms, already split into
  `Batch::files` and `Batch::git` (roots whose `.git` changed: status and history
  may differ).
- Access events are dropped: reads are not changes.
- `Watcher::expect(path)` must be called before the app writes a file itself, so
  its own write is not reported back as news.
- Inside `.git`, `*.lock` files are ignored: they come and go while git works.
  Anything else there means "look again".
- `Watcher::busy` is true while changes wait to settle: keep polling.

### crates/code-canvas/src/webview.rs

The overall design is in "Webviews" in [EXTENSIONS.md](EXTENSIONS.md).

What a page may load

- `serve` answers `/view/<handle>` with the HTML the extension set and
  `/file/<path>` with a file. A file is served only if it lies under an allowed
  root (the extensions folder, the open folders) or is one of the single files the
  media viewer was allowed. **The path is canonicalized first and the resolved
  path is what gets compared**, so `..` cannot leave the roots.
- `WebViews::allow_file` allows one file wherever it is, for the media viewer.
  Only the 64 most recent stay allowed.
- Video and audio are asked for in pieces (`Range: bytes=a-b` or `bytes=a-`) and
  answered with `206 Partial Content`. That is what makes seeking work, and a long
  video is never read whole.
- `Shared` is read by the protocol handler from the browser's thread, hence the
  mutex.

Messages and lifetime

- Page → `ipc` → `WebViews::poll`; extension → `WebViews::post` →
  `window.postMessage` in the page.
- Messages posted before a page finished loading are queued in order and delivered
  when it is `ready`. A message for a webview that is not on screen yet is
  dropped: nothing listens, and VS Code drops these too.
- `WebViews::show` does nothing for a handle whose extension has not produced the
  page yet.
- `WebViews::retheme` gives the theme to pages that have not had the current one.
  A page still loading gets it when it is ready, so `App::follow_theme` calls it
  again then; it is cheap when nothing is due.

Keyboard

- `with_browser_accelerator_keys(false)`: the browser's own shortcuts (print,
  find, reload) are not ours.
- `WebViews::focus_app`: **a page that was clicked keeps the keyboard, even
  hidden, until someone takes it.** Any of the views can hand it back to the
  window they all live in. See "Keyboard and focus" in
  [app.rs](#cratescode-canvassrcapprs).
- Pages hand key chords over to the app (`PageKey`, collected by `poll`, taken
  with `take_keys`) so that the app's shortcuts work wherever the caret is.
  `PageKey::key` is lower case, as browsers name keys: `"p"`, `"f1"`, `"tab"`.

Looks

- The view's background is the panel's own colour (the theme's `card`): that is
  what shows until the page paints.
- `theme_json` is the theme as `INIT_JS` reads it: every colour of
  `canvas_render::theme` by its key, plus two that pages need and are derived from
  those. It is put in front of the script, which reads it while it sets itself up.

`INIT_JS`, which runs in every page before the page's own scripts

- **Theme variables.** VS Code gives every webview its colours as `--vscode-*` CSS
  variables, and pages use hundreds of them. The common ones are taken from the
  app's theme (the object `T`, by the keys of `canvas_render::theme`); the rest
  are guessed from their name, so that nothing is left transparent or black on
  black. A "tint" of a tone is its `#rrggbb` with a low alpha.
- **Default style sheet.** VS Code also gives every webview a default style sheet,
  the scroll bars among it; without one, pages show the browser's own grey bar
  with arrows. The standard scroll bar properties switch those rules off where a
  page sets them. The sheet is inserted last in the head, after the page's own, so
  that between equal rules ours win.
- **Claude Code panel.** The second half of the sheet is this app's taste for that
  panel: text that does not fit is cut at a whole line instead of fading out. Its
  classes end in a hash that changes with every version, hence the
  `[class*=...]` selectors. The rules: a long message of the user,
  collapsed, shows three whole lines with no veil; what a tool printed, collapsed,
  shows three whole lines of its smaller type (0.85em at 1.5), minus the 4px of
  bottom padding, which would otherwise show what overflows; the message pinned at
  the top gets a solid edge over what scrolls under it.
- **Theme changes.** The script exposes a function the app calls when its theme
  changes while the page is open.
- **Follow-scroll** (`follow_chat`, `WebViews::set_follow`). While the
  conversation of an assistant's panel grows it stays scrolled to its end, as a
  terminal does. Scrolling up by hand lets go; coming back to the end takes hold
  again. A scroll away from the end that the page itself caused is not the user's
  doing and does not let go.
- **Key chords.** Chords belong to the app (Ctrl+P, Ctrl+Shift+P…), as in VS Code,
  where a webview hands them to the workbench. The ones text editing needs stay in
  the page, and so does AltGr, which arrives as Ctrl+Alt. For a chord handed over,
  the page's own handlers still run; only the browser's default does not.

Platforms

- `AVAILABLE` is `cfg!(windows)`. Elsewhere there is no browser view to use yet
  and webviews stay empty (the non-Windows `WebViews` is a stand-in).

---

## Repository files

Configuration carries no comments either; what they said:

- `.github/workflows/ci.yml` and `release.yml` use a fixed Rust version: a new
  compiler brings new lints, and they should arrive when the version is raised on
  purpose, not break an unrelated push.
- **Releasing.** Pushing a tag like `v0.3.0` (`git tag v0.3.0 && git push origin
  v0.3.0`) builds the Windows app and publishes it as a GitHub release, so nobody
  needs Rust to try it. "Run workflow" builds the same zip without publishing a
  release; it is kept as an artifact of the run. The version in `Cargo.toml` must
  match the tag: the app compares it with the latest release to offer an update.
- `Cargo.toml`: `resvg` is built without text, fonts or embedded images, because
  file icon themes are SVG and icons have none of those. `wry` (the browser view of
  extensions' webviews, WebView2) is a dependency on Windows only; anything the
  rest of the code uses must not go in that section.
- `.gitattributes` gives everyone the same line endings whatever their git
  settings (`*.ps1` keeps CRLF), and marks fonts, icons and images as binary so
  they are never touched.
- `.gitignore` leaves out local media (demo recordings, screenshots): `*.mp4`,
  `*.mov`.

## scripts

- `scripts/install.ps1` installs for the current user, with no admin rights: it
  copies the release build to `%LOCALAPPDATA%\Programs\Code Canvas`, adds Start
  Menu and Desktop shortcuts and, with `-ContextMenu`, "Open with Code Canvas" on
  folders. From the repository root:

  ```powershell
  powershell -ExecutionPolicy Bypass -File scripts\install.ps1
  powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -ContextMenu
  powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -NoBuild -NoDesktop
  ```

  - The installed copy cannot be replaced while it is running; the script looks
    for such a process first.
  - It runs `ie4uinit.exe -show` because Windows keeps the icons of programs in a
    cache; without that an update can keep showing the previous icon on shortcuts
    and the taskbar.
  - The context menu is two per-user registry keys:
    `Directory\shell\CodeCanvas` with `%1` (right-click a folder) and
    `Directory\Background\shell\CodeCanvas` with `%V` (right-click inside one).
- `scripts/uninstall.ps1` removes what `install.ps1` added. Settings and the
  session in `%APPDATA%\code-canvas` are kept unless `-Purge` is given.
- `scripts/screenshot.ps1` starts a test instance with a data folder of its own,
  runs a script in it and saves a picture of **its window only**, not of the
  screen:

  ```powershell
  scripts\screenshot.ps1 -Exe target\debug\code-canvas.exe -Data $env:TEMP\cc-test `
      -Script "left settings; wait 4000; quit" -Out shot.png -After 4500 -Folders "C:\some\folder"
  ```

  Several folders go in one `-Folders` value, separated by commas. It calls
  `PrintWindow` with flag 2 (`PW_RENDERFULLCONTENT`), which captures what the GPU
  drew and not just the frame.
- `scripts/draw_icon.py` draws the app's icon (a C whose mouth holds a card) into
  `assets/icon_source.png`, four times too big and shrunk, so its edges are
  smooth. Change the colours or the shapes there, then run `make_icon.py`.
- `scripts/install.ps1` saves the taskbar's pinned shortcut again when it points
  at the app: Windows keeps the icon a pin was made with, and a new icon would
  not show there.
- `scripts/make_icon.py` builds every icon asset from one square image. Requires
  Pillow.

  ```sh
  python scripts/make_icon.py                      # from assets/icon_source.png
  python scripts/make_icon.py path/to/icon.png     # from another image
  python scripts/make_icon.py --placeholder        # draw a simple placeholder
  ```

  | Output in `crates/code-canvas/assets/` | Size | Used for |
  |---|---|---|
  | `icon.png` | 256 px | README, packaging |
  | `icon.ico` | 16-256 | the `.exe`, shortcuts, taskbar |
  | `icon_128.rgba` | 128 px | the logo in the app's header |
  | `icon_64.rgba` | 64 px | the window icon |

  - The artwork should be a square PNG, ideally 1024 px, with a transparent
    background around the icon shape.
  - Transparent margins around the artwork are trimmed, keeping 2 %, so the icon
    is not smaller than its neighbours in the taskbar. A result that is not square
    is padded with transparency.
  - The placeholder (nested folder boxes with code bars and a relation curve) is
    drawn at 1024 px and downscaled, for anti-aliasing.
- `python scripts/make_codicons.py path/to/codicon.css` generates
  `crates/code-canvas/src/codicons.rs`. The font
  (`crates/code-canvas/assets/codicon.ttf`) and the CSS come from the
  `@vscode/codicons` npm package.
- `python scripts/make_icons.py path/to/lucide.css` generates
  `crates/code-canvas/src/icons.rs`. The font
  (`crates/code-canvas/assets/lucide.ttf`) and the CSS come from the
  `lucide-static` npm package.
