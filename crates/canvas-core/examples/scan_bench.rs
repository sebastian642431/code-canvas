use std::path::PathBuf;
use std::time::Instant;

use canvas_core::{scan, Layout, ScanOptions};

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("usage: scan_bench <folder>"));
    for run in 0..2 {
        let t = Instant::now();
        let repo = scan::scan(&root, &ScanOptions::default(), None);
        let scan_ms = t.elapsed().as_millis();
        let t = Instant::now();
        let layout = Layout::build(&repo);
        let layout_ms = t.elapsed().as_millis();
        let heap: usize = repo.files.iter().map(|f| f.heap_bytes()).sum();
        let text: usize = repo.files.iter().map(|f| f.text.len()).sum();
        let t = Instant::now();
        let tokens: usize = repo
            .files
            .iter()
            .take(2000)
            .map(|f| (0..f.chunk_count()).map(|c| f.tokenize_chunk(c).tokens.len()).sum::<usize>())
            .sum();
        let tok_ms = t.elapsed().as_millis();
        println!(
            "run {run}: {} files, {} lines, {:.1} MB text, {:.1} MB heap | {} tokens in first 2000 files tokenized in {tok_ms} ms | scan {scan_ms} ms (walk {} ms, load {} ms, index {} ms, {} symbols) | layout {layout_ms} ms, {} nodes",
            repo.files.len(),
            repo.total_lines,
            text as f64 / 1e6,
            heap as f64 / 1e6,
            tokens,
            repo.timings.walk_ms,
            repo.timings.load_ms,
            repo.timings.index_ms,
            repo.symbols.count,
            layout.nodes.len()
        );
        let secs = |i: usize| scan::LOAD_PROFILE[i].swap(0, std::sync::atomic::Ordering::Relaxed) as f64 / 1e9;
        println!(
            "       load, thread-seconds: stat {:.1}, read {:.1}, normalise {:.1}, analyse {:.1}",
            secs(0),
            secs(1),
            secs(2),
            secs(3)
        );
    }
}
