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
  [syntax.rs](#cratescanvas-coresrcsyntaxrs)
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
  [ext/when.rs](#cratescode-canvassrcextwhenrs) ·
  [files.rs](#cratescode-canvassrcfilesrs) ·
  [git.rs](#cratescode-canvassrcgitrs) ·
  [icon_theme.rs](#cratescode-canvassrcicon_themers) ·
  [keys.rs](#cratescode-canvassrckeysrs) ·
  [markdown.rs](#cratescode-canvassrcmarkdownrs) ·
  [media.rs](#cratescode-canvassrcmediars) ·
  [overlay.rs](#cratescode-canvassrcoverlayrs) ·
  [page.rs](#cratescode-canvassrcpagers) ·
  [paths.rs](#cratescode-canvassrcpathsrs) ·
  [prefs.rs](#cratescode-canvassrcprefsrs) ·
  [script.rs](#cratescode-canvassrcscriptrs) ·
  [session.rs](#cratescode-canvassrcsessionrs) ·
  [agents.rs](#cratescode-canvassrcagentsrs) ·
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
| Time dependency analysis on a real repository | `CODE_CANVAS_DEPS_REPO=<folder> cargo test --release -p canvas-core deps::tests::bench -- --ignored --nocapture` |
| Ask a real assistant for a commit message about this repository | `COMMIT_AI_TEST=claude cargo test -p code-canvas real_assistant -- --ignored --nocapture` |
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
  each way.
- `MAX_STATEMENT_LINES` = 40: a statement that spans several lines is followed for
  that many lines.

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
  of its origin. It is what puts the two sides of a diff next to each other.
- `FOLD_LINES` (2048) must stay a multiple of `CHUNK_LINES` and `BLOCK_LINES`, so
  chunks and blocks never straddle two columns of a folded file. The shader has
  its own copy, see [shaders](#cratescanvas-rendersrcshaders).
- Files longer than `FOLD_LINES` are folded into side-by-side columns, like a
  newspaper, so one huge data file cannot stretch the whole canvas.
- `MAX_COLS` (160): a card grows with its longest line up to that; longer lines
  are clipped. Soft wrapping is the planned fix.
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
- Children are packed tallest first, which packs best; ties keep the alphabetical
  order. A file's slot includes the label strip above its card, and `Layout::hit`
  counts that strip as part of the card.

### crates/canvas-core/src/refs.rs

- `SymbolIndex` is two sorted arrays per kind of name (`Named`): the hash of each
  name and, beside it, the symbol. A name's symbols are the run of its hash, found
  by bisection; no string is kept and nothing is allocated per name, which is what
  made building it slow (0.64 s for 3.3 M symbols, 0.17 s now, on all cores). Two
  names with the same hash are told apart when it is built and kept in
  `clashing`, by name; a name that is asked for and *defined nowhere* could in
  principle meet another name's hash and be given its symbols.
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
  the index is rebuilt after every save without reading a file, and what draws a
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
  index as it was: rebuilding it per keystroke would be slow. Call
  `Repo::reindex_symbols` when the typing pauses (the app does, after 700 ms).
- `LOAD_PROFILE`: CPU nanoseconds per loading step, summed over all threads, as
  `[stat, read, normalise, analyse]`. Only `scan_bench` reads it.
- `ScanOptions::skip_files` are lockfiles: huge and rarely useful.

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
- `dominant_kind`: most character cells wins, but comments and punctuation only
  win when little else is there.
- A block comment's opener is consumed before looking for its close, so `/*/` is
  not taken as open plus close. In Rust, `'a` is a lifetime and `'a'` a char.

---

## canvas-render

### crates/canvas-render/src/lib.rs

- Line numbers are one shaped block per chunk, shared by every file: lines 129 to
  256 read the same in all of them. A file with `SourceFile::numbers` (a card
  showing a diff: the line of the real file each row is, 0 for none) gets blocks
  of its own.
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
- `push_lines` merges tokens separated by small gaps into one bar.

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

### crates/canvas-render/src/curve.rs

- `control_points`: a horizontal S when the ends are side by side, an arc bulging
  to the left when they are stacked (two places in the same file or column).
- `push_curve`: neighbouring quads share their edge exactly, so there are no gaps
  and no overlap. A degenerate curve produces nothing.

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
- Not wired yet: file system watchers exist but never fire; tree views exist and
  stay empty.
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

- Typing is debounced twice in `App::poll_background`: the symbol index is rebuilt
  700 ms after the last edit, and the extensions get the text 250 ms after it, not
  per keystroke.
- `App::save_edit` reloads the file from disk after writing, so the symbol index
  and git see the saved file; then the extensions format it if the settings say
  so.
- `App::write_file` (a formatter or a fix produced new contents): an open editor
  without changes of its own takes the new text.
- `App::on_disk_changes`:
  - paths with a skipped directory in them (build output, dependencies) are
    ignored;
  - new paths count only when git would not ignore them;
  - a few changed files are reloaded one by one; many, or new and removed ones,
    mean a rescan;
  - no automatic rescan for a huge folder (it takes seconds: that stays the user's
    call, F5), right after another one, while the page is loading, or while any
    editor has unsaved changes;
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
  palette. `manifest_containers` leaves out views a manifest adds to VS Code's own
  containers (the explorer, source control): those places are not an extension's.
- `Extensions::rail_items`: the rule is in "Webviews" in
  [EXTENSIONS.md](EXTENSIONS.md). In addition, an extension none of whose
  containers gives an icon still gets one for a webview it has open or registered:
  a panel it opened itself, or a view under a condition this app cannot follow.
  Claude Code offers the same view in two places; only one of them is shown. A
  container that only has trees gets no icon: there is nothing to open.
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
  the table.
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
  "every edge" (a sample of them in a huge repository).
- `dep_curves`: a file hidden inside a collapsed folder counts as being where that
  folder is. Each curve starts at the line of the import.
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
- The selection of the file being edited reaches one cell past the end of a line,
  for the line break.

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
- **Exact uses.** `Page::toggle_symbol` selects with the relations the names give
  and leaves the symbol in `want_exact`; the app asks the host where it is used
  (`locations`, kind `Reference`, at the definition) and `Page::exact_uses` gives
  the answer to `Repo::refine`: references and incoming calls not confirmed go,
  confirmed places the names missed are added (a call if the token there is one),
  and `Relations::exact` says so in the panel. Definitions, outgoing calls and
  implementations stay as guessed. No answer, or an empty one (no extension for
  the language, or one still starting), leaves the guess. Places come in lines of
  the file and go through `row_of` for a card that shows a diff.
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
- **`Page::show_edit` and `Page::reload_file` expect the caller to drop the page's
  cached text first** (`TextLayer::drop_page`).
- Both clear the selections: symbols and lines of that file moved, so what pointed
  at them is stale. `show_edit` rebuilds the layout only when the card's size
  changes (a line added, removed, or outgrowing it).
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
| `click` | `click <x> <y>` | A click on the canvas, in physical pixels of the window. |
| `ui` | `ui click\|right\|move\|down\|up <x> <y>`, `ui drag <x> <y> <x> <y>` | The pointer on the interface, in points: what a button, a menu or a dragged icon gets. The steps after it wait until it has played out. |
| `folders` | `folders` | Show the list of opened folders. |
| `collapse-all` | `collapse-all` | No argument. |
| `expand-all` | `expand-all` | No argument. |
| `connected` | `connected on\|off` | Show only connected files. |
| `changed` | `changed on\|off` | Show only the files git reports as changed. |
| `tab` | `tab <n>` | Switch to tab `n`, 1-based. |
| `zoom` | `zoom <factor>` | Zoom by a factor. |
| `quick` | `quick <text>` | Open "go to file" with a query typed in. |
| `palette` | `palette <text>` | Open the command palette with a filter typed in. |
| `edit` | `edit <path> <line> <column>` | Put the caret in a file; line and column are 1-based. |
| `type` | `type <text>` | Type at the caret. |
| `key` | `key enter\|backspace\|delete\|tab\|left\|right\|up\|down\|home\|end\|undo\|redo\|save\|escape` | Press a key in the editor. With a completion list up, the keys that walk and take it go to the list, as when typed. `save` with no caret saves every unsaved file. |
| `back`, `forward` | `back`, `forward` | Where the canvas was before its last jump, and forward again. |
| `tool` | `tool move\|edit` | The hand or the pencil of the toolbar. `edit <path>` puts a caret whatever the tool is. |
| `diff` | `diff on\|off\|split\|together\|fold\|unfold`, `diff around <lines>` | Show what changed inside the changed files: removed lines above the new ones or old and new side by side, with or without the unchanged lines folded, and how many of them stay around a change. |
| `agent` | `agent <JSON of an event>`, `agent <name>` | What an assistant would send, as if it had arrived; or follow the one with that name. |
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
| `drag` | `drag <x> <y> <x> <y>` | A drag of the left button on the canvas, in physical pixels: it selects code with the pencil over a file, and moves the canvas otherwise. |
| `find` | `find <text>`, `find next\|previous\|close`, `find replace <text>`, `find replace-all <text>` | Find in the file being edited or selected, walk the matches, replace the current one or all of them (the pencil must be on). |
| `touch` | `touch <path> <text>` | Append a line to a file behind the app's back (`\n` in the text is a line break): what another program changing it looks like. |
| `page` | `page <javascript>` | Run a line of script in the webview on screen: a click on something of an extension's page without the mouse. No `;` in it: that ends the step. |
| `git` | `git stage [path]`, `git unstage [path]`, `git discard [path]`, `git commit <message>` | Source control. Discarding asks first. |
| `install` | `install <publisher.name>` | Install an extension from Open VSX. |
| `ext-search` | `ext-search <text>` | Search the extension registry. |
| `format` | `format <path>` | Format a file with the installed formatter. |
| `command` | `command <id>` | Run an extension's command. |
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
  Only `agent` is needed; `line` counts from 1.
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
- Faces: sixteen pictures drawn for the app (`assets/agents`), names and colours
  in the same order. An assistant takes the first face nobody has.
- Following one sets `App::following`, the same travel a change on disk causes,
  each time it moves (`Agent::moved`) or its line is found.

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
  someone's caret. Nothing in the editor knows it is shared. `share_look_over`
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
  (`Watcher::expect`), and the editor counts as saved. Files not saved of a
  shared folder do not hold the guest's app from closing (`App::unsaved`).
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
- The wire has a version (3 now: `Make` and `Drop` came with it): an app of
  another version is told so, instead of failing on a message it does not know.
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
  - The canvas is drawn on the whole window and the interface over it, so its
    round corners are the ground painted over its square ones (`enclose`,
    `corner`: a fan of triangles from the corner to the arc), with the border
    drawn last.
  - There is no status bar. What it said is a small card at the bottom right of
    the canvas (`status_pill`): what is under the pointer, what extensions
    report, problems, what is not saved, the branch, an update. The version is
    in Settings; what a frame costs is the hint of the zoom number.
  - A card's cross is drawn over its top right corner after its content, so
    `panel_title` leaves room for it when it is told one is coming (`CARD_X`).
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
