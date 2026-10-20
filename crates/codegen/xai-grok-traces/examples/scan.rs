//! `cargo run -p xai-grok-traces --example scan -- [db-path]`: index the
//! current user's roots and print per-source counts (no session content).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(xai_grok_traces::default_db_path);
    let mut idx = xai_grok_traces::TraceIndex::open(&db)?;
    let t = std::time::Instant::now();
    let report = idx.scan(&xai_grok_traces::user_roots())?;
    println!("indexed {} files in {:?}, {} errors", report.indexed, t.elapsed(), report.errors.len());
    for r in &report.roots {
        println!("  {:16} files={:6} accessible={} limited={}", r.id, r.files, r.accessible, r.limited);
    }
    for (source, n) in idx.sources()? {
        println!("  source {source:8} sessions={n}");
    }
    for (path, e) in report.errors.iter().take(5) {
        println!("  error {}: {e}", path.display());
    }
    Ok(())
}
