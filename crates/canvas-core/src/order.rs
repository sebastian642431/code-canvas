use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use rayon::prelude::*;

use crate::deps::{DepKind, Deps, Edge};
use crate::scan::{Dir, Repo};

const NONE: u32 = u32::MAX;

fn counts(e: &Edge) -> bool {
    e.kind != DepKind::Module && e.from != e.to
}

/// The order of the things in each folder by their imports: what is imported comes
/// before what imports it. Items are the folders (by their number in a walk of the
/// tree, the root first) and then the files (after the folders, by file number).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Order {
    dirs: HashMap<String, u32>,
    parent: Vec<u32>,
    depth: Vec<u32>,
    file_dir: Vec<u32>,
    rank: Vec<u32>,
    layer: Vec<u32>,
    sig: Vec<u64>,
    pub layers: u32,
    pub tied: usize,
}

impl Order {
    pub fn build(repo: &Repo, deps: &Deps) -> Order {
        let mut o = Order { file_dir: vec![NONE; repo.files.len()], ..Default::default() };
        let mut walk: Vec<&Dir> = Vec::new();
        let mut stack = vec![(&repo.tree, NONE, 0u32)];
        while let Some((dir, parent, depth)) = stack.pop() {
            let id = walk.len() as u32;
            walk.push(dir);
            o.parent.push(parent);
            o.depth.push(depth);
            o.dirs.insert(dir.rel_path.clone(), id);
            for &f in &dir.files {
                o.file_dir[f] = id;
            }
            stack.extend(dir.dirs.iter().rev().map(|d| (d, id, depth + 1)));
        }
        let items = walk.len() + repo.files.len();

        let pair = |e: &Edge| o.siblings(e.from, e.to).map(|(a, b)| ((a as u64) << 32) | b as u64);
        let mut pairs: Vec<u64> = match deps.edges.len() > 100_000 {
            true => deps.edges.par_iter().filter(|e| counts(e)).filter_map(pair).collect(),
            false => deps.edges.iter().filter(|e| counts(e)).filter_map(pair).collect(),
        };
        pairs.par_sort_unstable();
        let mut weight: Vec<i64> = Vec::with_capacity(pairs.len());
        let mut kept = 0;
        for i in 0..pairs.len() {
            if kept > 0 && pairs[kept - 1] == pairs[i] {
                weight[kept - 1] += 1;
                continue;
            }
            pairs[kept] = pairs[i];
            weight.push(1);
            kept += 1;
        }
        pairs.truncate(kept);
        let mut start = vec![0u32; items + 1];
        for &p in &pairs {
            start[(p >> 32) as usize + 1] += 1;
        }
        for i in 0..items {
            start[i + 1] += start[i];
        }
        let next: Vec<u32> = pairs.iter().map(|&p| p as u32).collect();

        let (comp, comp_layer) = components(&start, &next, items);
        o.layer = comp.iter().map(|&c| comp_layer[c as usize]).collect();
        o.layers = comp_layer.iter().copied().max().unwrap_or(0);

        o.rank = vec![0; items];
        o.sig = vec![0; walk.len()];
        let mut first = vec![NONE; comp_layer.len()];
        let mut sizes = vec![0u32; comp_layer.len()];
        for &c in &comp {
            sizes[c as usize] += 1;
        }
        o.tied = sizes.iter().filter(|&&n| n > 1).count();
        let mut score = vec![0i64; items];
        for (&p, &w) in pairs.iter().zip(&weight) {
            let (a, b) = ((p >> 32) as usize, p as u32 as usize);
            if comp[a] == comp[b] {
                score[a] += w;
                score[b] -= w;
            }
        }
        for (id, dir) in walk.iter().enumerate() {
            let mut children: Vec<(u32, &str)> =
                dir.dirs.iter().map(|d| (o.dirs[&d.rel_path], d.name.as_str())).collect();
            let base = walk.len() as u32;
            children.extend(dir.files.iter().map(|&f| (base + f as u32, repo.files[f].name.as_str())));
            for (at, &(item, _)) in children.iter().enumerate() {
                let c = comp[item as usize] as usize;
                if first[c] == NONE {
                    first[c] = at as u32;
                }
            }
            let key = |at: usize| {
                let item = children[at].0 as usize;
                let c = comp[item] as usize;
                (comp_layer[c], first[c], score[item], at)
            };
            let mut sorted: Vec<usize> = (0..children.len()).collect();
            sorted.sort_unstable_by_key(|&at| key(at));
            for (pos, &at) in sorted.iter().enumerate() {
                o.rank[children[at].0 as usize] = pos as u32;
            }
            let mut h = DefaultHasher::new();
            for &(item, name) in &children {
                (name, o.rank[item as usize]).hash(&mut h);
            }
            o.sig[id] = h.finish();
        }
        o
    }

    fn siblings(&self, from: u32, to: u32) -> Option<(u32, u32)> {
        let base = self.parent.len() as u32;
        let (mut a, mut b) = (*self.file_dir.get(from as usize)?, *self.file_dir.get(to as usize)?);
        if a == NONE || b == NONE {
            return None;
        }
        let (mut ca, mut cb) = (base + from, base + to);
        while self.depth[a as usize] > self.depth[b as usize] {
            (ca, a) = (a, self.parent[a as usize]);
        }
        while self.depth[b as usize] > self.depth[a as usize] {
            (cb, b) = (b, self.parent[b as usize]);
        }
        while a != b {
            (ca, a) = (a, self.parent[a as usize]);
            (cb, b) = (b, self.parent[b as usize]);
        }
        Some((ca, cb))
    }

    pub fn dir_rank(&self, rel_path: &str) -> u32 {
        self.dirs.get(rel_path).map_or(0, |&d| self.rank[d as usize])
    }

    pub fn file_rank(&self, file: usize) -> u32 {
        self.rank.get(self.parent.len() + file).copied().unwrap_or(0)
    }

    /// The import goes against the order: what it imports comes after the file (or
    /// the folder holding it) that imports it, in the folder that holds both. Only an
    /// edge inside a cycle of siblings can.
    pub fn against(&self, e: &Edge) -> bool {
        if !counts(e) {
            return false;
        }
        self.siblings(e.from, e.to).is_some_and(|(a, b)| self.rank[b as usize] > self.rank[a as usize])
    }

    /// The folders whose things are in another order in `newer`, by path. A folder
    /// only in `newer` counts as changed.
    pub fn changed(&self, newer: &Order) -> Vec<String> {
        let mut out: Vec<String> = newer
            .dirs
            .iter()
            .filter(|(path, &d)| self.dirs.get(*path).map(|&o| self.sig[o as usize]) != Some(newer.sig[d as usize]))
            .map(|(path, _)| path.clone())
            .collect();
        out.sort();
        out
    }

    /// What a folder holds in its order, with the layer of each: names of folders and
    /// files.
    pub fn arranged(&self, repo: &Repo, rel_path: &str) -> Vec<(String, u32)> {
        let Some(dir) = crate::layout::find_dir(&repo.tree, rel_path) else { return Vec::new() };
        let base = self.parent.len();
        let mut out: Vec<(u32, usize, String, u32)> = Vec::new();
        for d in &dir.dirs {
            let Some(&id) = self.dirs.get(&d.rel_path) else { continue };
            out.push((self.rank[id as usize], out.len(), d.name.clone(), self.layer[id as usize]));
        }
        for &f in &dir.files {
            let Some(&rank) = self.rank.get(base + f) else { continue };
            out.push((rank, out.len(), repo.files[f].name.clone(), self.layer[base + f]));
        }
        out.sort();
        out.into_iter().map(|(_, _, name, layer)| (name, layer)).collect()
    }
}

/// Strongly connected components (Tarjan's, without recursion) and the layer of each:
/// 0 for one that imports nothing, else one more than the deepest it imports.
/// Components are found sinks first, so what one imports is always done before it.
fn components(start: &[u32], next: &[u32], items: usize) -> (Vec<u32>, Vec<u32>) {
    const NEW: u32 = u32::MAX;
    let mut index = vec![NEW; items];
    let mut low = vec![0u32; items];
    let mut on_stack = vec![false; items];
    let mut comp = vec![NEW; items];
    let mut layers: Vec<u32> = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    let mut calls: Vec<(u32, u32)> = Vec::new();
    let mut counter = 0u32;
    for root in 0..items as u32 {
        if index[root as usize] != NEW {
            continue;
        }
        calls.push((root, NEW));
        while let Some(&mut (v, ref mut at)) = calls.last_mut() {
            let vi = v as usize;
            if *at == NEW {
                *at = start[vi];
                (index[vi], low[vi]) = (counter, counter);
                counter += 1;
                stack.push(v);
                on_stack[vi] = true;
            }
            if *at < start[vi + 1] {
                let w = next[*at as usize];
                *at += 1;
                if index[w as usize] == NEW {
                    calls.push((w, NEW));
                } else if on_stack[w as usize] {
                    low[vi] = low[vi].min(index[w as usize]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(u, _)) = calls.last() {
                low[u as usize] = low[u as usize].min(low[vi]);
            }
            if low[vi] != index[vi] {
                continue;
            }
            let from = stack.iter().rposition(|&x| x == v).expect("v is on the stack");
            let members = stack.split_off(from);
            let mut layer = 0;
            for &m in &members {
                on_stack[m as usize] = false;
                for &w in &next[start[m as usize] as usize..start[m as usize + 1] as usize] {
                    if comp[w as usize] != NEW {
                        layer = layer.max(layers[comp[w as usize] as usize] + 1);
                    }
                }
            }
            for &m in &members {
                comp[m as usize] = layers.len() as u32;
            }
            layers.push(layer);
        }
    }
    (comp, layers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::analyze;
    use crate::layout::{Layout, LayoutFilter};
    use crate::scan::{scan, ScanOptions};
    use std::sync::Arc;

    fn repo(tag: &str, files: &[(&str, &str)]) -> Repo {
        let dir = std::env::temp_dir().join(format!("code-canvas-order-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let repo = scan(&dir, &ScanOptions::default(), None);
        std::fs::remove_dir_all(&dir).ok();
        repo
    }

    fn names(order: &Order, repo: &Repo, path: &str) -> Vec<String> {
        order.arranged(repo, path).into_iter().map(|(name, layer)| format!("{name} {layer}")).collect()
    }

    #[test]
    fn what_is_imported_comes_first_layer_by_layer_and_ties_by_name() {
        let repo = repo(
            "layers",
            &[
                ("c.ts", "import { b } from './b';\n"),
                ("b.ts", "import { a } from './a';\n"),
                ("z.ts", "import { a } from './a';\n"),
                ("a.ts", "export const a = 1;\n"),
                ("m.ts", "export const m = 1;\n"),
            ],
        );
        let order = Order::build(&repo, &analyze(&repo));
        assert_eq!(names(&order, &repo, ""), ["a.ts 0", "m.ts 0", "b.ts 1", "z.ts 1", "c.ts 2"]);
        assert_eq!(order.layers, 2);
        assert_eq!(order.tied, 0);
    }

    #[test]
    fn a_cycle_stays_together_and_its_edges_go_against_the_order() {
        let repo = repo(
            "cycle",
            &[
                ("a.ts", "import { w } from './w';\n"),
                ("x.ts", "import { y } from './y';\n"),
                ("y.ts", "import { x } from './x';\nimport { b } from './b';\n"),
                ("w.ts", "import { x } from './x';\n"),
                ("b.ts", "export const b = 1;\n"),
            ],
        );
        let deps = analyze(&repo);
        let order = Order::build(&repo, &deps);
        assert_eq!(names(&order, &repo, ""), ["b.ts 0", "x.ts 1", "y.ts 1", "w.ts 2", "a.ts 3"]);
        let file = |name: &str| repo.file_index(name).unwrap();
        assert_eq!(order.file_rank(file("x.ts")) + 1, order.file_rank(file("y.ts")), "side by side");
        assert!(order.file_rank(file("b.ts")) < order.file_rank(file("x.ts")));
        assert_eq!(order.tied, 1);
        for e in &deps.edges {
            let (from, to) = (&repo.files[e.from as usize].name, &repo.files[e.to as usize].name);
            let back = (from.as_str(), to.as_str()) == ("x.ts", "y.ts");
            assert_eq!(order.against(e), back, "{from} -> {to}");
        }
    }

    #[test]
    fn in_a_cycle_what_is_imported_more_than_it_imports_comes_first() {
        let repo = repo(
            "weights",
            &[
                ("a/one.ts", "import { z } from '../z/one';\n"),
                ("a/two.ts", "import { z } from '../z/two';\n"),
                ("a/three.ts", "export const t = 1;\n"),
                ("z/one.ts", "import { t } from '../a/three';\n"),
                ("z/two.ts", "export const two = 1;\n"),
            ],
        );
        let deps = analyze(&repo);
        let order = Order::build(&repo, &deps);
        assert_eq!(names(&order, &repo, ""), ["z 0", "a 0"], "a imports z twice, z imports a once");
        assert_eq!(deps.edges.iter().filter(|e| order.against(e)).count(), 1);
    }

    #[test]
    fn folders_are_ordered_among_their_siblings_and_modules_do_not_count() {
        let repo = repo(
            "folders",
            &[
                ("Cargo.toml", "[package]\nname = \"t\"\n"),
                ("src/main.rs", "mod app;\nmod lib;\n\nfn main() {}\n"),
                ("src/app/mod.rs", "pub mod main;\n"),
                ("src/app/main.rs", "use crate::lib::util::f;\n"),
                ("src/lib/mod.rs", "pub mod util;\n"),
                ("src/lib/util.rs", "use crate::app::main;\npub fn f() {}\n"),
                ("web/a.ts", "import { z } from '../lib2/z';\n"),
                ("lib2/z.ts", "export const z = 1;\n"),
            ],
        );
        let deps = analyze(&repo);
        let order = Order::build(&repo, &deps);
        assert_eq!(names(&order, &repo, ""), ["lib2 0", "src 0", "Cargo.toml 0", "web 1"]);
        assert_eq!(names(&order, &repo, "src"), ["app 0", "lib 0", "main.rs 0"], "app and lib import each other");
        assert!(deps.edges.iter().any(|e| e.kind == DepKind::Module));
        assert!(deps.edges.iter().filter(|e| e.kind == DepKind::Module).all(|e| !order.against(e)));

        let filter = LayoutFilter { order: Some(Arc::new(order)), ..Default::default() };
        let layout = Layout::build_filtered(&repo, &filter);
        let node = |path: &str| &layout.nodes[layout.dir_node[path] as usize];
        let (lib2, web) = (node("lib2"), node("web"));
        assert!((lib2.rect.y, lib2.rect.x) < (web.rect.y, web.rect.x), "what is imported is placed first");
        let plain = Layout::build(&repo);
        assert_eq!(plain.nodes.len(), layout.nodes.len());
    }

    #[test]
    fn an_import_changed_reorders_only_the_folders_above_the_file() {
        let files = [
            ("ring/p.ts", "export const p = 1;\n"),
            ("ring/q.ts", "import { p } from './p';\n"),
            ("ring/r.ts", "export const r = 1;\n"),
            ("other/s.ts", "import { t } from './t';\n"),
            ("other/t.ts", "export const t = 1;\n"),
        ];
        let before = repo("changed-a", &files);
        let mut changed = files;
        changed[0].1 = "import { r } from './r';\n";
        let after = repo("changed-b", &changed);
        let (old, new) = (Order::build(&before, &analyze(&before)), Order::build(&after, &analyze(&after)));
        assert_eq!(old.changed(&new), ["ring"]);
        assert!(new.changed(&new).is_empty());
        assert_eq!(names(&new, &after, "ring"), ["r.ts 0", "p.ts 1", "q.ts 2"]);
    }
}
