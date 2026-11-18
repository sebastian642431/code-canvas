use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use canvas_core::deps::Deps;
use canvas_core::{deps, scan, store, Layout, LayoutFilter, Repo, ScanOptions, SymbolIndex};
use canvas_render::SceneData;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn full_scan(root: &Path, cache: &Path) -> (f64, Repo) {
    let t = Instant::now();
    let opts = ScanOptions { cache: Some(cache.to_path_buf()), ..Default::default() };
    let mut repo = scan::scan(root, &opts, None);
    let (found, per_file) = deps::analyze_keeping(&repo);
    let _ = store::keep(&mut repo, &per_file, &opts);
    let layout = Layout::build_filtered(&repo, &LayoutFilter::default());
    let scene = SceneData::build(&repo, &layout);
    std::hint::black_box((found, scene));
    (ms(t), repo)
}

fn rebuild(repo: &Repo) -> (f64, f64) {
    let t = Instant::now();
    let layout = Layout::build_filtered(repo, &LayoutFilter::default());
    let layout_ms = ms(t);
    let t = Instant::now();
    std::hint::black_box(SceneData::build(repo, &layout));
    (layout_ms, ms(t))
}

fn same(repo: &Repo, layout: &Layout, found: &Deps, check: bool) -> &'static str {
    if !check {
        return "";
    }
    let whole = Layout::build_filtered(repo, &LayoutFilter::default());
    let wrong = layout.differences(&whole);
    assert!(wrong.is_empty(), "the layout differs: {:?}", &wrong[..wrong.len().min(5)]);
    assert!(repo.symbols.tidied() == SymbolIndex::build(&repo.files), "the symbol index differs");
    let (mut mine, mut built) = (found.clone(), deps::analyze(repo));
    (mine.ms, built.ms) = (0, 0);
    assert!(mine == built, "the dependencies differ");
    " (same as built again)"
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("usage: update_bench <folder> [--write] [--check]"));
    let write = std::env::args().any(|a| a == "--write");
    let check = std::env::args().any(|a| a == "--check");
    let cache = std::env::temp_dir().join(format!("update-bench-{}.bin", std::process::id()));
    let opts = ScanOptions::default();
    let (scan_ms, mut repo) = full_scan(&root, &cache);
    let (layout_ms, scene_ms) = rebuild(&repo);
    let t = Instant::now();
    let mut found = deps::analyze(&repo);
    let deps_ms = ms(t);
    let t = Instant::now();
    let index = SymbolIndex::build(&repo.files);
    let index_ms = ms(t);
    std::hint::black_box(index);
    println!(
        "{} files, {} lines, {} symbols, {} edges | scan {scan_ms:.0} ms | full rebuilds: layout {layout_ms:.1} ms, scene {scene_ms:.1} ms, symbol index {index_ms:.1} ms, dependencies {deps_ms:.1} ms",
        repo.files.len(),
        repo.total_lines,
        repo.symbols.count,
        found.edges.len()
    );

    let pick = (0..repo.files.len())
        .map(|i| (i + repo.files.len() / 2) % repo.files.len())
        .find(|&i| repo.files[i].rel_path.ends_with(".rs") || repo.files[i].rel_path.ends_with(".ts"))
        .expect("a source file");
    let rel = repo.files[pick].rel_path.clone();
    let text = repo.files[pick].text().to_string();
    println!("the file: {rel}");
    let filter = LayoutFilter::default();
    let mut layout = Layout::build_filtered(&repo, &filter);
    let one: BTreeSet<u32> = [pick as u32].into();

    let typed = format!("{text}\nfn typed_by_the_bench() {{}}\n");
    let t = Instant::now();
    repo.replace_source(pick, &typed, &opts);
    let replace_ms = ms(t);
    let t = Instant::now();
    let moved = layout.resize_file(&repo, &filter, pick).expect("in place");
    let laid_ms = ms(t);
    let t = Instant::now();
    repo.reindex_symbols();
    found.update_files(&repo, &one);
    let index_ms = ms(t);
    let placed: u32 = moved.iter().map(|r| r.end - r.start).sum();
    println!(
        "typing a line: text {replace_ms:.2} ms + layout {laid_ms:.3} ms ({placed} of {} boxes placed again), then symbols and imports {index_ms:.3} ms{}",
        layout.nodes.len(),
        same(&repo, &layout, &found, check)
    );

    let t = Instant::now();
    repo.reload_file(pick, &opts);
    let reload_ms = ms(t);
    let t = Instant::now();
    found.update_files(&repo, &one);
    let deps_ms = ms(t);
    let t = Instant::now();
    let moved = layout.resize_file(&repo, &filter, pick).expect("in place");
    let laid_ms = ms(t);
    let placed: u32 = moved.iter().map(|r| r.end - r.start).sum();
    println!(
        "a file read again (save or another program): read and index {reload_ms:.2} ms + imports {deps_ms:.3} ms + layout {laid_ms:.3} ms ({placed} boxes){}",
        same(&repo, &layout, &found, check)
    );

    if !write {
        return;
    }
    let dir = Path::new(&rel).parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
    let fresh = format!("{dir}/made_by_the_bench.ts");
    std::fs::write(repo.root.join(&fresh), "export function made(): number {\n    return 1;\n}\n").unwrap();
    let t = Instant::now();
    let at = repo.add_file(&fresh, &opts).expect("a new file");
    let add_ms = ms(t);
    let t = Instant::now();
    let resolved = found.insert_file(&repo, at as u32).expect("one by one");
    let deps_ms = ms(t);
    let t = Instant::now();
    layout.insert_file(&repo, &filter, at).expect("laid out around it");
    let laid_ms = ms(t);
    println!(
        "a file created: read and index {add_ms:.2} ms + imports of {resolved} files {deps_ms:.2} ms + layout {laid_ms:.2} ms{}",
        same(&repo, &layout, &found, check)
    );

    std::fs::remove_file(repo.root.join(&fresh)).unwrap();
    let at = repo.file_index(&fresh).unwrap();
    let t = Instant::now();
    let old = repo.remove_file(at);
    let remove_ms = ms(t);
    let t = Instant::now();
    let resolved = found.remove_file(&repo, at as u32, &old).expect("one by one");
    let deps_ms = ms(t);
    let t = Instant::now();
    layout.remove_file(&repo, &filter, at, &fresh).expect("laid out without it");
    let laid_ms = ms(t);
    println!(
        "a file deleted: taken out {remove_ms:.2} ms + imports of {resolved} files {deps_ms:.2} ms + layout {laid_ms:.2} ms{}",
        same(&repo, &layout, &found, check)
    );

    std::fs::write(repo.root.join(&fresh), "export function made(): number {\n    return 1;\n}\n").unwrap();
    let (created_ms, _) = full_scan(&root, &cache);
    println!("a file created (before: scanned again with what the last scan kept): {created_ms:.0} ms");
    std::fs::remove_file(repo.root.join(&fresh)).unwrap();
    let (deleted_ms, _) = full_scan(&root, &cache);
    println!("a file deleted (before: scanned again with what the last scan kept): {deleted_ms:.0} ms");
    let _ = std::fs::remove_file(&cache);
}
