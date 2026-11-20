# Contributing to Hawk Canvas

Thanks for helping! Issues, ideas and pull requests are all welcome — in English or Spanish.

## Getting started

```sh
git clone https://github.com/sebastian642431/hawk-canvas.git
cd hawk-canvas
cargo run --release -- .            # open the project on itself
cargo test --all
```

You need a recent stable Rust toolchain and a GPU supported by wgpu (Vulkan, Metal,
DX12 or GL).

## Before opening a pull request

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all
```

CI runs the same three commands on Linux, Windows and macOS.

## Where things live

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first. In short:

- **`canvas-core`** — scanning, tokenizer, layout, token cache. Pure Rust, no GPU:
  the easiest place to start, and everything there should have unit tests.
- **`canvas-render`** — wgpu pipelines, levels of detail, culling, text.
- **`code-canvas`** — window, tabs, camera, input.

## Performance rules

Hawk Canvas must stay fast on huge repositories. When you touch a hot path:

1. **Measure before and after.** Use the status bar (frame time, instances, draw calls)
   and `cargo run --release -p canvas-core --example scan_bench -- <big folder>`.
   A good stress test is your own `~/.cargo/registry/src` folder.
2. **Nothing per frame may scale with the size of the repository.** Per-frame work
   must be bounded by what is visible on screen.
3. **No blocking work on the UI thread.** Scanning, parsing and layout run in the
   background.
4. **Keep per-item data small.** Multiply by 10 million before adding a field.

Put the numbers in your PR description.

## Trying a change in the real app

Unit tests cover `canvas-core` and the pure parts of the app. To check behaviour in
the running app without clicking around (and in a repeatable way), script it — see
"Testing the real app without touching the mouse" in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Design

The look follows the concept: dark chrome, panels as rounded cards, Lucide icons, one
accent colour. New UI goes in `crates/code-canvas/src/ui.rs` and should reuse the
helpers there (`section`, `well`, `check`, `icon_button`) rather than raw egui
widgets, so everything stays consistent.

Colours are never written where they are used: they come from the theme
(`crates/canvas-render/src/theme.rs`), which the user can change in the settings
panel. In the interface take them with `pal()` (`pal().text`, `pal().card`); on the
canvas with `theme::get()`. A colour that has no name yet gets one line in the table
of that file, with its value in each built-in theme.

## Good first issues

- More languages or definition patterns in `canvas-core/src/symbols.rs` (with a test).
- More keywords / languages in `canvas-core/src/syntax.rs` (with a test).
- Line numbers next to the code.
- Configurable skip lists (`ScanOptions`) from a config file.
- More settings in the settings panel (font size, keyboard shortcuts).
