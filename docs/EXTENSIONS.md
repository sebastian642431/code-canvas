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
- **Problems** (circle icon): what linters and language servers report. On the canvas
  each problem underlines its code, and the file's card gets a coloured edge and a dot
  that stay visible from far away, so problems show on the map of the whole repository.
- **Format a file**: `Shift+Alt+F` over it, or the wand in the toolbar. The formatter
  comes from an installed extension and uses the project's own configuration.
- Settings are read like VS Code reads them: the extension's defaults, then
  `settings.json` in the app's data folder, then the folder's `.vscode/settings.json`
  (including `"[language]"` sections and `editor.defaultFormatter`).

Extensions with code need **Node.js** on the `PATH`. Extensions run with your
permissions, exactly as in VS Code: install the ones you trust.

A new installation gets four by itself, once (`ext::DEFAULTS`): Claude Code,
Prettier, ESLint and the Symbols file icons. Removing one is respected: it does
not come back.

**Colour themes** work: every theme an extension declares (`contributes.themes`)
is listed in the settings next to the built-in ones. The app has far fewer
colours than VS Code, so a theme is read as a few anchors (the editor's and the
side bar's background and foreground, the accent, the selection) from which the
rest is made, and its `tokenColors` give the colours of code by scope. The
grammar is still the app's own: what a theme says about scopes the app does not
tell apart is not used.

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
that only has tree views gets no icon yet: there would be nothing to show.

Implemented: colour themes, file icon themes, webview views and panels, documents and their events, configuration, diagnostics, every
`languages.register…Provider` (kept by kind; the app uses formatting, hovers,
definitions and completions, and references can be requested), commands, workspace edits,
`workspace.fs`, output channels, status bar items, messages, decorations, extension
storage, activation events, and the classes and enums extensions construct.

Not yet — these are the next pieces, in order:

1. **Tree views**: the lists extensions add to the side bar (their containers are
   read already, and get an icon once they have something that can be shown).
2. **Code lenses and hovers** drawn on the canvas (the providers are already kept).
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
| rust-analyzer (`rust-lang.rust-analyzer`) | works: syntax errors while typing, `cargo check` problems on save, hover, go to definition, completions with their imports. Its tree view, inlay hints and code lenses are not drawn |
| ESLint (`dbaeumer.vscode-eslint`) | works: problems on the canvas, "Fix all auto-fixable problems" |
| Prettier (`esbenp.prettier-vscode`) | works: formats with the project's `.prettierrc` |
| Error Lens (`usernamehw.errorlens`) | works: the message of each problem after its line |
| Version Lens (`pflannery.vscode-versionlens`) | starts; shows nothing until code lenses are drawn |
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
`format <path>`, `command <id>`, `left extensions|problems|<publisher.name>`.

## A model of our own, later

For things specific to a canvas (new overlays, analyses, layouts) a small native
extension model is still planned: an external process speaking JSON-RPC, contributing
relations, overlays in world coordinates and panels. The host protocol above is the
first half of it.
