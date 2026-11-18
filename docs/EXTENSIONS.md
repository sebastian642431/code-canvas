# Extensions

Code Canvas runs **real VS Code extensions**: the same `.vsix` packages, unmodified,
installed from [Open VSX](https://open-vsx.org) or from a file. This document says how
it works, what works today, and what does not yet.

## Using them

- **Extensions panel** (puzzle icon in the left rail): search Open VSX, press *Install*.
  The archive icon installs a `.vsix` file from disk.
- From a terminal, like `code` does:
  `code-canvas --install-extension dbaeumer.vscode-eslint esbenp.prettier-vscode`
- Each extension that offers something gets **its own icon in the left rail**. It opens
  the extension's panel: what it is, and its commands, which run with a click.
- **Webviews**: an extension's own interface (the official Claude Code panel, for
  one) opens in the right panel; the extension's icon appears in the right rail.
  Windows only for now.
- **Tree views**: the lists an extension adds to VS Code's side bar. Each container of
  them (`contributes.viewsContainers`) gets an icon in the right rail, and the trees an
  extension adds to VS Code's own places (the explorer, source control, testing) share
  one more icon of the extension. The icon opens a card with one list per view: rows
  that open and close (their children are asked when opened), codicons, descriptions,
  a click runs the row's command, the view's title buttons (`view/title`, group
  `navigation`) sit in the card's title and a row's own buttons (`view/item/context`,
  group `inline`) appear on the row under the pointer.
- **Code lenses**: what extensions put above a line ("3 references", "▶ Run") is drawn
  faint above it on the card, once code is readable, with `|` between several. A click
  runs it. "N references" selects the name on the canvas, which draws its uses.
  *Settings → General → Show code lenses* turns them off.
- **Problems** (circle icon): what linters and language servers report. On the canvas
  each problem underlines its code, and the file's card gets a coloured edge and a dot
  that stay visible from far away, so problems show on the map of the whole repository.
- **Format a file**: `Shift+Alt+F` over it, or the wand in the toolbar. The formatter
  comes from an installed extension and uses the project's own configuration.
- Settings are read like VS Code reads them: the extension's defaults, then
  `settings.json` in the app's data folder, then the folder's `.vscode/settings.json`
  (including `"[language]"` sections and `editor.defaultFormatter`).

Extensions with code need **Node.js** on the `PATH` (or where its installer puts it). They run in a process of their own that gets only part of your environment (no `NODE_OPTIONS`, no tokens of other programs; see `ext/host.rs` in NOTES). Extensions run with your
permissions, exactly as in VS Code: install the ones you trust.

**Workspace trust.** A folder opened for the first time is not trusted, and a
folder received from someone sharing a session never is until you say so. While a
folder open here is not trusted, extensions run in *restricted mode*, as in
VS Code: those whose manifest says they cannot run in an untrusted folder
(`capabilities.untrustedWorkspaces.supported: false`, or nothing said, as for most)
stay off; those that say `limited` run, but the settings they list in
`restrictedConfigurations` (and a few the app always holds back, such as
`typescript.tsdk`, `eslint.*`, `prettier.prettierPath`) are not read from the
folder's `.vscode/settings.json`; and `workspace.isTrusted` is false. So ESLint stays
off, TypeScript uses the tsserver it brings and Prettier the prettier it brings,
ignoring the folder's plugins and configuration files. Opening such a folder asks
whether to trust it (only when an extension would be held back); the answer is
kept per folder, and *Restricted mode* in the status bar asks again. Folders that
were open before this existed count as trusted. A shared folder is not given to
the extensions at all until trusted, and that trust lasts for the session.

A new installation offers four, once (`ext::DEFAULTS`): Claude Code, Prettier,
ESLint and the Symbols file icons, each at an exact version whose file must have
the checksum the app knows. Nothing is installed until you say yes; "Not now"
is respected, and so is removing one: it does not come back. Whatever is
installed from the registry must be the extension asked for (same publisher and
name in its package.json) and stay within a size once unpacked.

**Colour themes** work: every theme an extension declares (`contributes.themes`)
is listed in the settings next to the built-in ones. The app has far fewer
colours than VS Code, so a theme is read as a few anchors (the editor's and the
side bar's background and foreground, the accent, the selection) from which the
rest is made, and its `tokenColors` give the colours of code by scope. Code has
eleven colours (keywords, types, functions, macros, strings, numbers, comments,
punctuation, names, headings, prose): what a theme says about finer scopes than
those is not used.

**Grammars** work: a language the app does not know (Elm, Terraform, Ink, a
language of your company's own…) is coloured by the TextMate grammar its
extension brings (`contributes.languages` and `contributes.grammars`,
`.tmLanguage.json` or the older property-list `.tmLanguage`), embedded languages
and injections included.
They need no Node: grammars are read and run inside the app, on a thread of their
own, with the same engine VS Code uses ported to Rust. Each token's scopes become
one of the eleven kinds above, coloured by the theme. Languages the app colours
itself (Rust, the C family, Python, the web's, shell, TOML, YAML, JSON, Markdown,
SQL, Lua…) keep its own colours, unless *Settings → Appearance → Highlighting →
Colour every language with its extension's grammar* is on. The status bar names
the grammar's language when it is the one colouring the file. Files over 200,000
lines keep the app's own colours; lines over 4,000 characters are left plain.

**File icon themes** work: install one (Symbols, Material Icon Theme) and the
explorer and the go-to-file list use it. With several, pick one in the settings.
Themes made of SVG files are read; font icons are not.

## How it works

```
code-canvas (Rust)  ── JSON lines over stdio ──►  node exthost/host.js
   ext/mod.rs                                        │  provides the `vscode` module
   ext/host.rs                                       ▼
   ext/market.rs (Open VSX, .vsix)              the extensions, unmodified
```

- `ext/market.rs` downloads and unpacks extensions into `<data dir>/extensions`.
- `exthost/host.js` is the **extension host**: one Node process that loads every
  enabled extension and gives it a `vscode` module, the API extensions are written
  against. It is embedded in the app and written to the data folder at start.
- The host and the app exchange one JSON object per line. The app sends the open
  folders, which files are readable on screen (those are the extensions' *open
  documents*) and requests (format this file, run this command). The host sends back
  diagnostics, decorations, messages, status items and file contents to write.
- The app never blocks on the host: reader threads feed a channel and wake the UI.

### What the canvas means to an extension

VS Code extensions assume an editor made of tabs. Code Canvas maps its own ideas onto
theirs:

| VS Code | Code Canvas |
|---|---|
| workspace folders | the folders open as canvases |
| open text documents, tabs | files whose code is readable on screen right now |
| active editor | the file under the pointer |
| diagnostics | underlines, card markers and the Problems panel |
| text editor decorations | washes over ranges and text after a line, drawn on the card |
| `workspace.applyEdit`, formatting edits | the app writes the file and reloads its card |
| status bar items | text in the status bar |
| webview views and panels | a browser view (WebView2) over the right panel |
| tree views | a card of lists, opened from the extension's icon in the right rail |
| code lenses | faint links on the card above their line |
| TextMate grammars | the colours of a language the app does not know (or of every one, if asked) |
| `editor.action.showReferences` | the name is selected: its uses are drawn |
| `showInformationMessage` & co. | notices in the corner of the canvas |

### Coverage of the `vscode` API

The API is large; the host implements a growing part of it. Anything an extension
asks for that is missing gets a harmless stand-in and is reported once, so gaps are
visible (the Extensions panel says "N VS Code features it asked for are not available
yet", and the log lists them) instead of crashing the extension.

### Webviews

`webview.rs` shows a webview's page in a child browser view (`wry`, WebView2 on
Windows) placed over the right panel. The page and its files are served from
`https://ccres.localhost` by a protocol handler: `/view/<handle>` is the HTML the
extension set, `/file/<path>` a file under the extensions folder or an open folder
(anything else is refused). A script added to every page provides
`acquireVsCodeApi()` and the `--vscode-*` colours pages are styled with: the common
ones are set exactly, the rest are guessed from their name. Messages travel as JSON:
page → app → host → extension, and back.

Which icons the right rail has comes from the manifests, as in VS Code: each view
container an extension declares (`contributes.viewsContainers`) gets one as soon as
the extension is installed, if it has a webview view whose `when` holds. Opening it
is what starts the extension (`onView:<id>`). `when` clauses are evaluated in
`ext/when.rs` against the context keys extensions set with `setContext`. A container
whose webview views are hidden but has trees gets an icon that opens its trees
(`tree:<container>`); one with both shows its webview.

Implemented: colour themes, file icon themes, webview views and panels, tree views
(`createTreeView`, `registerTreeDataProvider`: `getChildren`, `getTreeItem`,
`resolveTreeItem` for a tooltip, `onDidChangeTreeData`; `TreeItem` label, description,
codicon `iconPath` and its colour, `resourceUri` with the file icon theme,
collapsible state, command, `contextValue`, tooltip; the view's `title`,
`description` and `message`, its expand, collapse, selection and visibility events),
code lenses (`registerCodeLensProvider` with `resolveCodeLens` and
`onDidChangeCodeLenses`), documents and their events, configuration, diagnostics, every
`languages.register…Provider` (kept by kind; the app uses formatting, hovers,
definitions and completions, code lenses, and references, call hierarchies and
implementations for the relations of a selected name), commands, workspace edits,
`workspace.fs`, output channels, status bar items, messages, decorations, extension
storage, activation events, and the classes and enums extensions construct.

Not yet — these are the next pieces, in order:

1. **Trees, the rest**: the menus of a row's right click and the title's "…" (only
   the buttons are shown), checkboxes, drag and drop, `reveal`, badges, icons that
   are image files, several rows selected, `when` clauses with `=~` (regular
   expressions, which many row buttons use) and the trees of a container that also
   has a webview.
2. **Inlay hints** and code actions (the light bulb) drawn on the canvas (the
   providers are already kept).
3. Signature help, snippets with more than one stop, and the commands a completion
   asks to run after it is taken; showing proposed diffs.
4. Webviews on Linux and macOS; `when` clauses for menus and commands (views have
   them); the containers' own icons when they are image files; webview state kept
   across restarts.
5. Terminals, debugging, notebooks, source control providers, authentication.

### Checked with real extensions

| Extension | State |
|---|---|
| TypeScript (`vscode.typescript-language-features`, VS Code's own) | works: compiler errors, hover, go to definition (`F12`), completions |
| rust-analyzer (`rust-lang.rust-analyzer`) | works: syntax errors while typing, `cargo check` problems on save, hover, go to definition, completions with their imports. Its code lenses and its dependency tree should show now, not checked with it yet; inlay hints are not drawn |
| ESLint (`dbaeumer.vscode-eslint`) | works: problems on the canvas, "Fix all auto-fixable problems" |
| Prettier (`esbenp.prettier-vscode`) | works: formats with the project's `.prettierrc` |
| Error Lens (`usernamehw.errorlens`) | works: the message of each problem after its line |
| Version Lens (`pflannery.vscode-versionlens`) | not needed: Code Canvas shows the versions of dependencies itself, clickable, with bulk updates, prereleases, private npm scopes and the project's install (see `versions` in [NOTES.md](NOTES.md)) |
| Pretty TypeScript Errors (`yoavbls.pretty-ts-errors`) | starts; needs hovers and a TypeScript language server |
| Claude Code (`Anthropic.claude-code`) | its panel and its list of sessions open and render, signed in; a session opened from the list, or a new one, is a tab that can be closed; sending a message has not been tested yet |
| Git Graph (`mhutchie.git-graph`) | starts; its graph is a webview panel — not tried since webviews landed |
| GitHub Pull Requests | starts; its interface is tree views and webviews |

Other extensions have not been tried yet. Language servers shipped as extensions
use the same client library as ESLint and rust-analyzer, which work, but each one
should be checked before it is listed here.

The first time a Rust, TypeScript or JavaScript file is edited and nothing
installed checks that language, the app offers the extension that does
(`CHECKERS` in `app.rs`): rust-analyzer, or VS Code's TypeScript extension.

**Tabs.** What VS Code opens as an editor tab (a webview panel) opens in the side
panel under a strip of tabs, with one more icon of the extension in the bar and
the number of tabs on it. Closing a tab closes the panel for the extension too.

## Testing the host

The host is plain Node and can be driven without the app: send it an `init` message
with a folder and extension folders, then `open` a file, and read what comes back.
In the app, script steps cover the same ground (see "Testing the real app" in
[ARCHITECTURE.md](ARCHITECTURE.md)): `install <publisher.name>`, `ext-search <text>`,
`format <path>`, `command <id>`, `left extensions|problems|<publisher.name>`,
`right <publisher.name>` (its first icon in the right rail, a tree card too),
`lens report <path>`, `lens <path> <line> [n]`, `tree report`, `tree <row>`,
`tree button <command> <row>`, `tree title <command>`, `colours <path> <line>` (the
kind of each token of a line, as drawn). `qa/host/views.test.js` drives the host's
lenses and trees with `qa/fake/views-extension`; `qa/fake/grammar-extension` brings
grammars only (the scenarios `canvas/grammars-*.scn`).

## A model of our own, later

For things specific to a canvas (new overlays, analyses, layouts) a small native
extension model is still planned: an external process speaking JSON-RPC, contributing
relations, overlays in world coordinates and panels. The host protocol above is the
first half of it.
