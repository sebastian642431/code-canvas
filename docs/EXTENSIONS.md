# Extensions: what is planned, and what "VS Code extensions" means here

Status: **design notes — nothing in this document is implemented yet.**

People ask whether Code Canvas can use VS Code extensions. The honest answer depends
on what kind of extension it is, because "an extension" covers three very different
things.

## 1. Data contributions — feasible, planned first

Colour themes, TextMate grammars, file icon themes and snippets are plain data files
inside a `.vsix` (which is a zip with a `package.json`). Nothing has to be executed.

Plan:
- Read `.vsix` files and the `contributes.themes` / `contributes.grammars` entries.
- Map a VS Code colour theme to our token kinds and UI colours.
- Use TextMate grammars (or tree-sitter) behind the existing `Tokens` interface, so
  highlighting stops depending on the built-in tokenizer.

## 2. Language features — feasible through the same servers

Most language extensions are thin wrappers around a **language server** speaking LSP
(rust-analyzer, typescript-language-server, pyright, gopls, clangd …). We do not need
the extension, only the server.

Plan:
- A `RelationProvider` interface with two implementations: today's name-based index
  (`canvas-core::refs`) and an LSP client.
- Per-language start / stop with progress, like the concept's "Language servers" list.
- LSP results fill the same `Relation` type, so the canvas and panels do not change.

This gives exact definitions, references, call hierarchies, type hierarchies and
diagnostics for dozens of languages.

## 3. Arbitrary extensions with their own UI — not planned for now

Running any extension from the marketplace (GitLens, Docker, test explorers, anything
with a webview) requires the **VS Code extension host**: a Node.js process plus the
`vscode` API — hundreds of functions for editors, documents, tree views, webviews,
terminals, debugging, tasks. Extensions assume a text editor made of tabs and
documents; Code Canvas is a canvas. That is why editors that want this compatibility
(Cursor, Windsurf, VSCodium) are forks of VS Code rather than reimplementations.

What would be possible later, as a separate research project: hosting the open-source
extension host out of process and implementing a *subset* of the API (commands,
tree views as panels, status bar items). It is a large effort with an open-ended
tail, so it should start as a spike with a clear list of target extensions.

Also note the Visual Studio Marketplace's terms only allow its use from Microsoft's
own products; a third-party app has to use [Open VSX](https://open-vsx.org).

## Our own extension model

For things specific to a canvas (new overlays, analyses, panels), the plan is the
model that already works well for tools like this one:

- An extension is an **external process** that speaks JSON-RPC over stdio (as LSP and
  MCP servers do), described by a small manifest. Any language can be used.
- It can contribute: relations and symbols, canvas overlays (rectangles, curves,
  labels in world coordinates), panel content, and commands.
- It never runs inside the app's process, so a broken extension cannot crash it or
  slow down the frame.

The Claude Code panel already follows this shape: an external process, a line-based
JSON protocol, results turned into canvas actions.
