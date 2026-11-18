use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::deps::{analyze, Deps};
use crate::layout::{Layout, LayoutFilter, NodeKind, NONE};
use crate::refs::SymbolIndex;
use crate::scan::{scan, Dir, Repo, ScanOptions};

struct World {
    dir: PathBuf,
    repo: Repo,
    layout: Layout,
    deps: Deps,
    filter: LayoutFilter,
}

fn key(layout: &Layout, n: u32) -> String {
    match layout.nodes.get(n as usize) {
        None => "none".into(),
        Some(node) if node.is_dir() => format!("dir {}", node.rel_path),
        Some(node) => format!("card {}", node.rel_path),
    }
}

fn shape(layout: &Layout, repo: &Repo) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (i, n) in layout.nodes.iter().enumerate() {
        let parent = n.parent.map_or("none".into(), |p| key(layout, p));
        let inside: Vec<String> = (i + 1..n.subtree_end as usize).map(|c| key(layout, c as u32)).collect();
        let kind = match n.kind {
            NodeKind::File(fi) => repo.files[fi].rel_path.clone(),
            NodeKind::Dir => "dir".into(),
        };
        let rows = n.wrap.as_ref().map(|w| w.per());
        let what = (&n.name, n.rect, n.depth, parent, n.files, n.lines, n.collapsed, n.col_stride, n.fold, n.gutter);
        let more = (n.offset, n.slot, kind, rows, inside.into_iter().collect::<BTreeSet<_>>());
        assert!(out.insert(key(layout, i as u32), format!("{what:?} {more:?}")).is_none(), "one node per path");
    }
    for (fi, f) in repo.files.iter().enumerate() {
        out.insert(format!("anchor of {}", f.rel_path), key(layout, layout.file_anchor[fi]));
        out.insert(format!("card of {}", f.rel_path), key(layout, layout.file_node[fi]));
    }
    for (path, &n) in &layout.dir_node {
        out.insert(format!("folder {path}"), key(layout, n));
    }
    out.insert("bounds".into(), format!("{:?}", layout.bounds));
    out
}

fn folders(dir: &Dir, repo: &Repo, out: &mut Vec<(String, Vec<String>)>) {
    out.push((dir.rel_path.clone(), dir.files.iter().map(|&f| repo.files[f].rel_path.clone()).collect()));
    for d in &dir.dirs {
        folders(d, repo, out);
    }
}

impl World {
    fn new(tag: &str, files: &[(&str, String)], filter: LayoutFilter) -> World {
        let dir = std::env::temp_dir().join(format!("code-canvas-updates-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let repo = scan(&dir, &ScanOptions::default(), None);
        let layout = Layout::build_filtered(&repo, &filter);
        let deps = analyze(&repo);
        let world = World { dir, repo, layout, deps, filter };
        world.check("at the start");
        world
    }

    fn check(&self, after: &str) {
        let fresh = scan(&self.dir, &ScanOptions::default(), None);
        let paths = |repo: &Repo| repo.files.iter().map(|f| f.rel_path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&self.repo), paths(&fresh), "{after}: the files");
        let (mut mine, mut theirs) = (Vec::new(), Vec::new());
        folders(&self.repo.tree, &self.repo, &mut mine);
        folders(&fresh.tree, &fresh, &mut theirs);
        assert_eq!(mine, theirs, "{after}: the folders");
        assert_eq!(self.repo.total_lines, fresh.total_lines, "{after}: the lines");
        let whole = Layout::build_filtered(&fresh, &self.filter);
        let (mine, theirs) = (shape(&self.layout, &self.repo), shape(&whole, &fresh));
        for (k, v) in &theirs {
            assert_eq!(mine.get(k), Some(v), "{after}: {k}");
        }
        assert_eq!(mine.len(), theirs.len(), "{after}: as many things laid out");
        assert_eq!(self.repo.symbols.tidied(), SymbolIndex::build(&fresh.files), "{after}: the symbol index");
        assert_eq!(self.repo.symbols.count, fresh.symbols.count, "{after}: symbols counted");
        let (mut mine, mut theirs) = (self.deps.clone(), analyze(&fresh));
        (mine.ms, theirs.ms) = (0, 0);
        assert_eq!(mine, theirs, "{after}: the dependencies");
    }

    fn write(&mut self, rel: &str, text: &str) {
        std::fs::create_dir_all(self.dir.join(rel).parent().unwrap()).unwrap();
        std::fs::write(self.dir.join(rel), text).unwrap();
        let opts = ScanOptions::default();
        match self.repo.file_index(rel) {
            Some(at) => {
                self.repo.reload_file(at, &opts);
                self.layout.resize_file(&self.repo, &self.filter, at).expect("laid out again in place");
                self.deps.update_files(&self.repo, &[at as u32].into());
            }
            None => {
                let at = self.repo.add_file(rel, &opts).expect("a new file");
                self.layout.insert_file(&self.repo, &self.filter, at).expect("laid out around it");
                if self.deps.insert_file(&self.repo, at as u32).is_none() {
                    self.deps = analyze(&self.repo);
                }
            }
        }
        self.check(&format!("writing {rel}"));
    }

    fn typed(&mut self, rel: &str, text: &str) {
        let at = self.repo.file_index(rel).unwrap();
        self.repo.replace_source(at, text, &ScanOptions::default());
        self.layout.resize_file(&self.repo, &self.filter, at).expect("laid out again in place");
        assert_eq!(self.repo.reindex_symbols(), vec![at]);
        self.deps.update_files(&self.repo, &[at as u32].into());
        std::fs::write(self.dir.join(rel), text).unwrap();
        self.check(&format!("typing in {rel}"));
    }

    fn delete(&mut self, rel: &str) {
        std::fs::remove_file(self.dir.join(rel)).unwrap();
        let at = self.repo.file_index(rel).unwrap();
        let old = self.repo.remove_file(at);
        self.layout.remove_file(&self.repo, &self.filter, at, rel).expect("laid out without it");
        if self.deps.remove_file(&self.repo, at as u32, &old).is_none() {
            self.deps = analyze(&self.repo);
        }
        self.check(&format!("deleting {rel}"));
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn lines(prefix: &str, count: usize, wide: usize) -> String {
    (0..count).map(|i| format!("pub fn {prefix}_{i}() {{ let x = \"{}\"; }}\n", "x".repeat(i % wide))).collect()
}

fn start() -> Vec<(&'static str, String)> {
    let mut files = vec![
        ("Cargo.toml", "[package]\nname = \"t\"\n".to_string()),
        ("src/main.rs", "mod a;\nmod b;\nmod gone;\n\nfn main() {\n    a::alpha_0();\n}\n".to_string()),
        ("src/a.rs", lines("alpha", 40, 30)),
        ("src/b/mod.rs", "pub mod c;\n\npub fn beta() {}\n".to_string()),
        ("src/b/c.rs", "use crate::a::alpha_1;\n\npub fn gamma() {\n    alpha_1();\n}\n".to_string()),
        (
            "web/app.ts",
            "import { x } from './util/x';\nimport { m } from './missing';\n\nexport const app = x;\n".into(),
        ),
        ("web/util/x.ts", "export function x() {}\n".into()),
        ("deep/one/two/three.py", "import four\n\ndef three():\n    pass\n".into()),
        ("docs/readme.md", "# hi\n".into()),
    ];
    const LOTS: [&str; 12] = ["l00", "l01", "l02", "l03", "l04", "l05", "l06", "l07", "l08", "l09", "l10", "l11"];
    for (i, name) in LOTS.iter().enumerate() {
        let path: &'static str = Box::leak(format!("lots/{name}.txt").into_boxed_str());
        files.push((path, lines("lot", 3 + i * 7, 5 + i * 11)));
    }
    files
}

fn walk(tag: &str, filter: LayoutFilter) {
    let mut w = World::new(tag, &start(), filter);
    w.write("src/a.rs", &lines("alpha", 300, 200));
    w.write("lots/l03.txt", "short\n");
    w.write("src/a.rs", &lines("alpha", 300, 200));
    w.typed(
        "web/app.ts",
        "import { x } from './util/x';\nimport { y } from './util/y';\nimport { m } from './missing';\n",
    );
    w.write("web/util/y.ts", "export function y() {}\n");
    w.write("web/missing.ts", "export const m = 1;\n");
    w.write("newdir/inner/deeper/z.rs", "pub fn z() {}\n");
    w.write("src/b/d.rs", "pub fn delta() {}\n");
    w.write("src/gone.rs", &lines("gone", 9, 3));
    w.delete("web/util/x.ts");
    w.delete("deep/one/two/three.py");
    w.write("deep/one/x.py", "def x():\n    pass\n");
    w.write("deep/one/two/y.py", "import x\n");
    w.delete("newdir/inner/deeper/z.rs");
    w.delete("src/b/c.rs");
    w.write("src/b/e.rs", "pub fn gamma() {}\n");
    w.write("lots/l99.txt", &lines("new", 500, 90));
    w.delete("lots/l00.txt");
    w.delete("docs/readme.md");
    w.write("docs/again.md", "# again\n");
    w.typed("src/main.rs", "mod a;\nmod b;\n\nfn main() {}\n");
}

#[test]
fn one_file_changed_made_or_deleted_is_the_same_as_everything_built_again() {
    walk("plain", LayoutFilter::default());
}

#[test]
fn the_same_with_folders_collapsed_and_hidden() {
    let mut filter = LayoutFilter::default();
    filter.collapsed.insert("lots".into());
    filter.collapsed.insert("deep/one".into());
    filter.hidden.insert("docs".into());
    filter.hidden.insert("src/b/d.rs".into());
    walk("filtered", filter);
}

#[test]
fn a_card_that_keeps_its_size_moves_nothing() {
    let mut w = World::new("same-size", &start(), LayoutFilter::default());
    let at = w.repo.file_index("src/b/c.rs").unwrap();
    let card = w.layout.file_node[at];
    std::fs::write(w.dir.join("src/b/c.rs"), "use crate::a::alpha_1;\n\npub fn gamma() {\n    alpha_2();\n}\n")
        .unwrap();
    w.repo.reload_file(at, &ScanOptions::default());
    let changed = w.layout.resize_file(&w.repo, &w.filter, at).unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0], card..card + 1);
    assert_ne!(card, NONE);
    w.deps.update_files(&w.repo, &[at as u32].into());
    w.check("a change of one letter");
}
