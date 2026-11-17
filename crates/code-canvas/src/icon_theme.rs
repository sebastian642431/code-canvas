//! File icon themes: the `iconThemes` extensions contribute (Symbols,
//! Material Icon Theme…), in VS Code's own format. The explorer draws its
//! rows with the one in use.
//!
//! A theme is a JSON file: `iconDefinitions` names image files, and tables
//! say which definition a file or folder gets, by its whole name
//! (`fileNames`, `folderNames`) or by its extension (`fileExtensions`).
//! Icons are SVG files, drawn with `resvg`. Font icons and the per-language
//! table (`languageIds`) are not read: a file without a match gets the
//! theme's plain file icon.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug, Default)]
pub struct IconTheme {
    /// Definition name → image file.
    icons: HashMap<String, PathBuf>,
    /// Lower case → definition name.
    extensions: HashMap<String, String>,
    names: HashMap<String, String>,
    folders: HashMap<String, String>,
    folders_open: HashMap<String, String>,
    file: Option<String>,
    folder: Option<String>,
    folder_open: Option<String>,
}

impl IconTheme {
    pub fn load(file: &Path) -> Result<IconTheme, String> {
        let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
        let json: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
        Ok(IconTheme::from_json(&json, file.parent().unwrap_or(Path::new("."))))
    }

    /// `dir` is the folder of the theme file: image paths are relative to it.
    pub fn from_json(json: &Value, dir: &Path) -> IconTheme {
        let table = |key: &str| -> HashMap<String, String> {
            let Some(Value::Object(map)) = json.get(key) else { return HashMap::new() };
            map.iter().filter_map(|(k, v)| Some((k.to_lowercase(), v.as_str()?.to_string()))).collect()
        };
        let name = |key: &str| json.get(key).and_then(Value::as_str).map(str::to_string);
        let mut icons = HashMap::new();
        if let Some(Value::Object(definitions)) = json.get("iconDefinitions") {
            for (id, definition) in definitions {
                if let Some(path) = definition.get("iconPath").and_then(Value::as_str) {
                    icons.insert(id.clone(), dir.join(path));
                }
            }
        }
        IconTheme {
            icons,
            extensions: table("fileExtensions"),
            names: table("fileNames"),
            folders: table("folderNames"),
            folders_open: table("folderNamesExpanded"),
            file: name("file"),
            folder: name("folder"),
            folder_open: name("folderExpanded"),
        }
    }

    /// The first of `definitions` that has an image (a theme may name one
    /// it does not define, or one that is a font character).
    fn image<'a>(&'a self, definitions: impl IntoIterator<Item = Option<&'a String>>) -> Option<&'a Path> {
        definitions.into_iter().flatten().find_map(|d| self.icons.get(d)).map(PathBuf::as_path)
    }

    /// The image for a file called `name` (no folders in it). Its whole name
    /// wins over its extension, and a longer extension over a shorter one
    /// (`x.test.ts` is `test.ts` before `ts`); last comes the plain file.
    pub fn file(&self, name: &str) -> Option<&Path> {
        let name = name.to_lowercase();
        let by_extension = name.match_indices('.').find_map(|(dot, _)| self.extensions.get(&name[dot + 1..]));
        self.image([self.names.get(&name), by_extension, self.file.as_ref()])
    }

    /// The image for a folder called `name`, open or closed. A theme without
    /// open variants shows the closed one.
    pub fn folder(&self, name: &str, open: bool) -> Option<&Path> {
        let name = name.to_lowercase();
        self.image([
            self.folders_open.get(&name).filter(|_| open),
            self.folders.get(&name),
            self.folder_open.as_ref().filter(|_| open),
            self.folder.as_ref(),
        ])
    }
}

/// Draw an SVG file into a square of `size` pixels, centred and whole:
/// RGBA with premultiplied alpha, row by row. `None` when it cannot be read.
pub fn rasterize(svg: &Path, size: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let data = std::fs::read(svg).ok()?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let (w, h) = (tree.size().width(), tree.size().height());
    let scale = (size as f32 / w).min(size as f32 / h);
    let (dx, dy) = ((size as f32 - w * scale) * 0.5, (size as f32 - h * scale) * 0.5);
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(pixmap.take())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn theme() -> IconTheme {
        IconTheme::from_json(
            &json!({
                "iconDefinitions": {
                    "doc": { "iconPath": "./icons/doc.svg" },
                    "ts": { "iconPath": "./icons/ts.svg" },
                    "ts-test": { "iconPath": "./icons/ts-test.svg" },
                    "cargo": { "iconPath": "./icons/cargo.svg" },
                    "dir": { "iconPath": "./icons/dir.svg" },
                    "dir-src": { "iconPath": "./icons/dir-src.svg" },
                    "dir-src-open": { "iconPath": "./icons/dir-src-open.svg" },
                    "font-icon": { "fontCharacter": "\\E001" }
                },
                "fileExtensions": { "ts": "ts", "test.ts": "ts-test", "weird": "font-icon", "gone": "not-defined" },
                "fileNames": { "Cargo.toml": "cargo" },
                "folderNames": { "src": "dir-src" },
                "folderNamesExpanded": { "src": "dir-src-open" },
                "file": "doc",
                "folder": "dir"
            }),
            Path::new("/theme"),
        )
    }

    fn icon(path: Option<&Path>) -> String {
        path.map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).unwrap_or_default()
    }

    #[test]
    fn files_by_name_then_by_the_longest_extension() {
        let t = theme();
        assert_eq!(icon(t.file("main.ts")), "ts.svg");
        assert_eq!(icon(t.file("app.test.ts")), "ts-test.svg");
        assert_eq!(icon(t.file("Main.TS")), "ts.svg");
        assert_eq!(icon(t.file("cargo.toml")), "cargo.svg");
        // Nothing says what these are: the plain file icon.
        assert_eq!(icon(t.file("notes.txt")), "doc.svg");
        assert_eq!(icon(t.file("Makefile")), "doc.svg");
        assert_eq!(t.file("main.ts").unwrap(), Path::new("/theme").join("./icons/ts.svg"));
    }

    #[test]
    fn what_has_no_image_falls_back() {
        let t = theme();
        // A font icon, and a name without a definition: not drawable here,
        // so the plain file icon is used.
        assert_eq!(icon(t.file("x.weird")), "doc.svg");
        assert_eq!(icon(t.file("x.gone")), "doc.svg");
        assert_eq!(IconTheme::default().file("main.ts"), None);
        assert_eq!(IconTheme::default().folder("src", false), None);
    }

    #[test]
    fn folders_open_and_closed() {
        let t = theme();
        assert_eq!(icon(t.folder("src", false)), "dir-src.svg");
        assert_eq!(icon(t.folder("SRC", true)), "dir-src-open.svg");
        assert_eq!(icon(t.folder("docs", false)), "dir.svg");
        // No open variant in the theme: the closed one stands in.
        assert_eq!(icon(t.folder("docs", true)), "dir.svg");
    }

    /// Draws what a real theme gives a few names, side by side, to look at:
    /// `ICON_THEME_TEST=<theme.json> ICON_THEME_SHEET=<out.png> cargo test -p code-canvas real_icon_theme -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sheet_of_a_real_icon_theme() {
        let file = PathBuf::from(std::env::var("ICON_THEME_TEST").expect("set ICON_THEME_TEST to a theme's JSON file"));
        let theme = IconTheme::load(&file).expect("a readable theme");
        let files = [
            "main.rs",
            "Cargo.toml",
            "Cargo.lock",
            "README.md",
            "host.js",
            "app.test.ts",
            "types.d.ts",
            "App.tsx",
            "package.json",
            ".gitignore",
            "icon.png",
            "LICENSE-MIT",
            "style.css",
            "index.html",
            "script.lua",
            "notes.txt",
        ];
        let folders = ["src", "docs", "assets", "scripts", "node_modules", "target", ".github", "whatever"];
        const SIZE: u32 = 64;
        let total = (files.len() + folders.len()) as u32;
        let mut sheet = image::RgbaImage::from_pixel(SIZE * total, SIZE, image::Rgba([0x23, 0x27, 0x2e, 255]));
        let mut drawn = 0;
        let names = files.iter().map(|n| (*n, false)).chain(folders.iter().map(|n| (*n, true)));
        for (i, (name, is_dir)) in names.enumerate() {
            let svg = if is_dir { theme.folder(name, false) } else { theme.file(name) };
            println!("{:14} {}", name, svg.map(|p| icon(Some(p))).unwrap_or_else(|| "(none)".into()));
            let Some(pixels) = svg.and_then(|p| rasterize(p, SIZE)) else { continue };
            drawn += 1;
            for (at, px) in pixels.chunks_exact(4).enumerate() {
                let (x, y) = (at as u32 % SIZE + i as u32 * SIZE, at as u32 / SIZE);
                // Premultiplied over the panel colour, as the app blends it.
                let under = sheet.get_pixel(x, y).0;
                let over = |c: usize| px[c] as u32 + under[c] as u32 * (255 - px[3] as u32) / 255;
                sheet.put_pixel(x, y, image::Rgba([over(0) as u8, over(1) as u8, over(2) as u8, 255]));
            }
        }
        println!(
            "{drawn} of {total} drawn; every image of the theme: {} of {} readable",
            { theme.icons.values().filter(|p| rasterize(p, 16).is_some()).count() },
            theme.icons.len()
        );
        if let Ok(out) = std::env::var("ICON_THEME_SHEET") {
            sheet.save(&out).expect("the sheet saved");
        }
    }

    #[test]
    fn an_svg_becomes_pixels() {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-svg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("square.svg");
        // A red square in the middle of a wider, transparent drawing.
        std::fs::write(
            &file,
            r##"<svg width="48" height="24" viewBox="0 0 48 24" xmlns="http://www.w3.org/2000/svg"><rect x="12" y="0" width="24" height="24" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
        let pixels = rasterize(&file, 32).unwrap();
        assert_eq!(pixels.len(), 32 * 32 * 4);
        let at = |x: usize, y: usize| &pixels[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        // Scaled to fit the width (48 → 32), so 16 tall and centred.
        assert_eq!(at(16, 16), [255, 0, 0, 255]);
        assert_eq!(at(16, 2), [0, 0, 0, 0]);
        assert_eq!(at(2, 16), [0, 0, 0, 0]);
        assert!(rasterize(&dir.join("missing.svg"), 32).is_none());
        std::fs::write(dir.join("broken.svg"), "not an image").unwrap();
        assert!(rasterize(&dir.join("broken.svg"), 32).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
