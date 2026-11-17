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
- Several folders open at once on the same canvas (e.g. a frontend and a backend).

**Symbols — "where is this used?"**
- Click a name in the code: curves go to its definition, references, incoming and
  outgoing calls and implementations, each kind in its own colour.
- Name chips mark the functions at the other end; function bodies are highlighted.
- Filters per kind with opacity; **show only connected files** re-lays the canvas out
  with just the files involved.
- Works instantly with a built-in name-based index (Rust, TypeScript/JavaScript,
  Python, Go, C-family), approximate by design. With the language's extension
  installed (rust-analyzer, TypeScript…) the uses of the selected name become the
  exact ones a moment later: the guesses that were another thing with the same
  name go, the uses the names missed come.

**Dependencies**
- Which file imports which, read from the import statements (JS/TS, C and C++, Rust,
  Python, Java and Kotlin). Select a file or a folder: curves go to what it depends on
  and come from what uses it, and a panel lists both.
- Counts per kind of relationship with switches, and an option to draw every edge.
  On Chromium: 1.35 million edges found in half a second.

**Finding things**
- Files panel: tree with checkboxes, filter, click to fly to a file. Right click for
  new file or folder, rename, delete (to the recycle bin), copy path, reveal in the
  file manager, open a terminal there.
- Drop a file on the window to copy it into the selected folder; a video, image or
  PDF opens in the viewer when it has arrived.
- What git ignores stays off the canvas but is listed, dimmed, in the file tree.
- **Updates**: a newer release is offered in a notice when the app starts and in the
  status bar, next to the version; *Update and restart* downloads it and restarts into
  it. Settings shows the version and has *Check for updates*.
- **Themes of VS Code**: install a colour theme as an extension (Dracula, GitHub
  Theme, Tokyo Night…) and pick it in Settings, Appearance: the window, the canvas
  and the code take its colours, light ones as well. Any colour can still be
  changed on top.
- **Settings** (the gear, or `Ctrl+,`) open over the app: version and updates, theme,
  file icons, every colour, every shortcut and who writes commit messages, with a
  search box across all of it. The window opens where and as big as it was left,
  maximized if it was, and never larger than the screen.
- **Images, video, audio and PDF**: click one (on the canvas, in the file tree, or a
  link an assistant gives) and it opens in a viewer panel. Windows only for now.
- **Markdown as it reads**: *Preview*, next to the name of a `.md` file on the canvas
  (or in the file tree's menu), shows it rendered as GitHub does, in the same viewer,
  and follows what you type. Links to other files open them; web links go to your
  browser.
- An assistant's panel (Claude Code) stays scrolled to what it is writing until you
  scroll up.
- Several folders open share **one canvas**, next to each other, with one zoom for
  all; click one to work in it. `Ctrl+E` (or the grid icon) goes back to one at a time.
- `Ctrl+P` go to a file or folder; `Ctrl+Shift+F` search text in every file, with the
  hits marked on the canvas.

**Editing**
- The toolbar has two tools. With the **hand** (`H`) the canvas is only looked at:
  drags travel, clicks select, nothing can be changed by accident. With the
  **pencil** (`E`) readable code is an editor: click to place the caret and type.
  Selection, undo and redo, copy and paste, `Ctrl+S` to save. Each file keeps its
  own tabs and line endings.
- What is typed shows at once, before saving: a dot on the file's name (on its
  card, in the explorer, in the status bar), its changed lines washed in colour,
  and the file listed in Source Control. Staging or committing it saves it.
- **See what changed inside the files** (Source Control): each changed file shows
  its removed lines in red above what replaced them, or what it was and what it is
  **side by side** in two columns of the same card. The lines that did not change
  fold away, so a long file is only its changes. With *Show only changed files*
  the canvas becomes the change itself.
- With the pencil, dragging over code selects it; the hand, or the right button,
  moves the canvas.
- `Ctrl+F` finds in the file being edited or selected: every match is marked, Enter
  and Shift+Enter walk them, and with the pencil one or all can be replaced.
- **Pictures and videos are on the canvas too**: a picture is drawn inside its
  card, and a video, a sound or a PDF plays inside its card when it is clicked
  (`Esc`, or a click somewhere else, stops it).
- **Any path opens on the canvas**: a file an assistant names in its panel, a
  definition in another file, a link in a Markdown file. One that is not in an open
  folder gets its own card next to them, and the canvas goes to the line asked for.
- **Back and forward**: the arrows of the toolbar, `Ctrl+Z` / `Ctrl+Y` (while no
  file is being typed in) or `Alt+←` / `Alt+→` go back to where the canvas was
  before it last jumped, and forward again.
- Saving formats the file (with Prettier where it is installed; turn it off with
  `editor.formatOnSave`), and problems update while you type. Rust is checked by
  rust-analyzer and TypeScript by VS Code's own extension: the app offers to
  install them the first time such a file is edited.
- Files changed by another program (an editor, `git checkout`, an AI assistant)
  are reloaded by themselves, and the canvas **travels to the lines that changed**
  and lights them, so what an assistant is touching is seen as it happens. Moving
  the canvas yourself holds it; it can be switched off in the settings.
- **Code is offered while it is typed** when the language has its extension
  installed (rust-analyzer, TypeScript and the like): the members after a dot, the
  names that fit what is typed, each with its kind and its type. Enter or Tab takes
  one, arrows choose, Esc closes; what it needs imported is imported.
- While a path is typed inside quotes (`"./`, `"../`, `"src/`), the files and folders
  that are there are offered under the caret: arrows, then Enter or Tab.
- In `package.json` and `Cargo.toml`, each dependency says whether a newer version
  exists (`✓ latest`, `↑ 2.1.0`), and a notice sums it up. Only package names are
  sent, to npm and crates.io; it can be switched off in the settings.
- `Ctrl+Shift+P` opens the command palette: the app's commands and every extension's.

**VS Code extensions**
- Installs the same `.vsix` extensions VS Code uses, from [Open VSX](https://open-vsx.org)
  or a file, and runs them unmodified in an extension host.
- Linters and language servers: problems underline their code, and files with problems
  stay marked from any zoom level, so they show on the map of the whole repository.
- Formatters: `Shift+Alt+F` formats the file under the pointer with the project's own
  configuration. Each extension gets its icon in the rail, with its commands.
- Webviews: an extension's own interface opens in the right panel — the official
  Claude Code extension renders there (Windows only for now).
- Language extensions answer about the code under the pointer (hover) and `F12` goes
  to a definition. VS Code's own TypeScript extension works: real compiler errors.
- Checked so far with ESLint, Prettier, Error Lens, Claude Code and TypeScript. Tree views are
  not shown yet — the state of each piece is in [docs/EXTENSIONS.md](docs/EXTENSIONS.md).

**Working together**
- **Share the session** (the icon at the top right): the app gives an invitation to
  copy; whoever pastes it in their Code Canvas gets your open folders on their own
  canvas. Nothing is installed or signed into: the two apps talk to each other
  directly, sealed with the key that is in the invitation.
- What anyone types shows for everyone as it is typed, two people in the same
  file included. What another program changes on your disk (an assistant, a
  formatter, git) reaches them as well.
- **To look or to edit**: each one who joins only looks until you let them edit,
  one by one or everyone by default. A reader's keys change nothing. `Ctrl+S` of
  someone who edits saves the file where it lives, on your computer.
- **Where everyone is**: each person's pointer and caret with their name, in
  their colour, and the outline of what their window shows. Click their circle
  at the top to **follow** them: your canvas goes where theirs goes until you
  move it or press Esc. *Bring everyone to what I see* makes the others follow
  you.
- It ends when you say so, and by itself when the app closes. Whoever joined is
  left with nothing of yours on their disk.
- On the same network it works as it is. From outside, both computers need a way
  to reach each other: a VPN both are in (Tailscale, ZeroTier) or the port of
  the invitation forwarded on the router.

**Source control**
- A panel laid out like VS Code's: the repository and its branch, a message box and
  Commit, *Staged Changes* and *Changes* with stage / unstage, sync, and the latest
  commits. On the canvas a strip marks each changed file and a bar each changed line.
- **Click a commit** (in the Graph, or in the Timeline of a file) and the canvas
  becomes that commit: only the files it changed, each showing what it changed in
  them. The bar at the top says which commit it is; its cross, or the same click
  again, brings the files back as they are, where the canvas was.
- **Right click on a changed line** stages, unstages or discards that change alone;
  the same menu stages the file, copies its path or reveals it.
- The Commit button's arrow: commit and push, commit and sync, amend. With nothing to
  commit it becomes *Sync Changes* or *Publish Branch*. The "…" menu has pull, push,
  fetch, discard all and stash.
- The sparkle in the message box has an assistant write the message from the change:
  Claude Code, Codex, or a command of your own (set in Settings).
- **Show only changed files** re-lays the canvas out with just your work in progress.
- Timeline: the commits that touched the selected file, under the file tree.

**Claude Code**
- Install the official Claude Code extension from the Extensions panel: its own
  panel opens in the right side of the window (Windows only for now).

**An app, not a demo**
- Own title bar and panels, icon rails on both sides, remembers your folders, cameras
  and layout between launches.
- Every icon of the rails can be dragged to the other side, or reordered (or right
  click it): the explorer on the right, an assistant's panel on the left, as you like.

## Install and run

**Download**: the [releases page](https://github.com/sebastian642431/code-canvas/releases)
has `code-canvas.exe` for Windows. It is a single file: run it, no installer. Windows
may warn that it comes from an unknown publisher (the file is not signed yet): "More
info" → "Run anyway". Extensions also need [Node.js](https://nodejs.org) installed.

From the source, on Windows (per user, no admin rights):

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
| Edit a file | Click in its code (zoomed in to text) and type; `Ctrl+S` saves |
| See where a name is used | `Ctrl`+click it (zoomed in to text); click inside a function when zoomed out |
| Deselect | Click it again, `Esc` for everything |
| Go to a file or folder | `Ctrl+P` |
| Search in files | `Ctrl+Shift+F` |
| Open a folder in a new canvas | `Ctrl+O` |
| Next / previous canvas | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| Close the canvas / read everything from disk again | `Ctrl+W` / `F5` |

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

And on **Chromium** (a shallow clone: **514,507 files / 63.7 M lines**), same machine:

| | |
|---|---|
| Open, files already in the disk cache | **~19 s** (walk 3 s, read + analyse 12 s, layout 0.7 s) |
| Open, first time after cloning | 7.6 min — almost all of it the disk reading 5 GB of small files |
| CPU time per frame, whole repository on screen | **~5 ms** |
| Memory | ~4 GB, when every file's text was kept in memory |

Two thirds of that warm time was the operating system opening 470,000 files, which
more threads do not speed up. That is what the next two things are for; Chromium has
not been measured again with them yet.

**A folder opened again reads only what changed.** What a scan finds out about each
file is kept on disk, and a file whose size and date are the same is not opened: its
text is read when it is looked at. **Above 768 MB of text** the text is not kept in
memory at all, and a small filter per file says which ones a search, or the relations
of a name, have to read. On the cargo registry as it is today (51,600 files, 18.8 M
lines, 782 MB of text), same machine:

| | first time | opened again |
|---|---|---|
| Open | 3.3 s | **0.8 s**, no file read |
| Memory once open | 1,320 MB | **620 MB** |
| Relations of a name (996 of them) | 10 ms | 30 ms |
| A phrase searched in every file | 12 ms | 6 ms |

How (details in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)): parallel walk and load;
tokens produced on demand per visible chunk instead of stored; static geometry uploaded
once and culled by contiguous folder ranges; near views built only from visible lines;
text shaped within a per-frame budget; frames drawn only when something changes.

Measure it yourself: `CODE_CANVAS_BENCH=1` logs frame statistics while the camera
sweeps the canvas; `cargo run --release -p canvas-core --example scan_bench -- <folder>`
times each loading phase.

## Roadmap

Contributions are very welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

- [x] **Editing**: click into a file and type, with undo and save; format on save;
      reload when files change on disk; command palette.
- [x] A hand to look and a pencil to edit; what is typed shown before it is saved;
      drag to select; find and replace in a file.
- [x] The canvas travels to what an assistant or another program changes; any path
      opens on the canvas, also outside the open folders; back and forward.
- [x] Completions from language servers while typing, with the imports they need.
- [ ] Editing: multiple cursors.
- [x] **VS Code extensions**: an extension host that runs real `.vsix` extensions;
      diagnostics, formatting, decorations and commands. See
      [docs/EXTENSIONS.md](docs/EXTENSIONS.md).
- [x] **Webviews** for extensions (the official Claude Code panel renders).
- [x] Language servers that come as extensions (rust-analyzer, VS Code's TypeScript):
      problems while typing, hover, go to definition. Tabs for what an extension
      opens as an editor (the sessions of Claude Code).
- [ ] Tree views and code lenses on the canvas; webviews on Linux and macOS.
- [x] Colour themes from extensions (Dracula, GitHub Theme, Tokyo Night…): the
      canvas, the panels and the code take their colours. File icon themes too.
- [ ] Grammars from extensions (highlighting is the app's own).
- [ ] **Soft wrapping** for lines longer than the card width (160 columns today).
- [x] **Language servers** give the exact uses of the selected name (references and
      incoming calls) in place of the ones guessed by name.
- [ ] The same for outgoing calls and implementations.
- [x] Dependency edges between files (imports), for the selected file or folder.
- [ ] Dependencies: edges bundled per folder so that all of them can be drawn on a
      huge repository, arrowheads, backwards edges, arranging by namespace.
- [x] Git: changed files and lines on the canvas, file timeline.
- [x] Git: staging, committing and syncing from the source control panel.
- [x] Git: the diff inside each file (removed lines too), together or with what it
      was and what it is side by side, with the lines that did not change folded;
      discarding changes, switching branches, stashes.
- [x] Git: what a commit changed, on the canvas; staging, unstaging or discarding
      one change of a file (right click on it).
- [x] Text read on demand: above a budget (768 MB) the text of the files is let go
      after the scan and read again when it is looked at, searched or related.
- [x] What a scan finds is kept on disk: a folder opened again reads only the files
      that changed (`F5` reads everything again).
- [ ] Cheaper single-file updates on huge folders (today they rebuild the whole layout).
- [x] The symbol index is made in parallel, with no string kept per name.
- [ ] Memory-mapped sources for 1M+ file monorepos.
- [x] A settings window: theme, every colour, every shortcut, commit messages.
- [x] Line numbers (the file's own, also where a card shows a diff).
- [x] The folders and files never read can be chosen (Settings, "Never read").
- [x] Sharing a session: others see and edit your folders live from their own app,
      as readers or writers, with everyone's pointer, caret and view, and following.
- [ ] Sharing across networks without a VPN or a forwarded port (a relay); files
      made, renamed and deleted by those who join; pictures and other files that
      are not text.
- [ ] Linux / macOS packaging, a web build (WebGPU).

## Project layout

```
crates/
  canvas-core/    scanning, tokenizer, symbols, relations, search, layout   (no GPU, unit-tested)
  canvas-render/  wgpu renderer: rects, code bars, curves, text (glyphon), LOD, culling
  code-canvas/    the app: window, UI (egui), pages, camera, extensions, git, Claude Code
    exthost/      the extension host (Node): the `vscode` API for real VS Code extensions
docs/             ARCHITECTURE.md, EXTENSIONS.md
scripts/          install / uninstall, icon generation
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Contributions are accepted under the same terms.

Bundled third-party assets: [Codicons](https://github.com/microsoft/vscode-codicons), VS Code's
icon set (CC BY 4.0, see `crates/code-canvas/assets/CODICON-LICENSE.txt`), and the
[Lucide](https://lucide.dev) icon font (ISC, see
`crates/code-canvas/assets/LUCIDE-LICENSE.txt`) and egui's default fonts (Ubuntu font
licence / OFL, via the `epaint_default_fonts` crate).
