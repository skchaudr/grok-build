//! `cargo run --release -p xai-grok-traces --example remote_smoke -- user@host`: latency and row count only.
use std::time::Instant;
use xai_grok_traces::remote::{RemoteFilter, fetch_sessions};

fn main() {
    let host = std::env::args().nth(1).expect("host");
    let t = Instant::now();
    match fetch_sessions(&host, &RemoteFilter { active_only: true, limit: 20, ..Default::default() }) {
        Ok(f) => println!("ok rows={} note={:?} in {:?}", f.rows.len(), f.note, t.elapsed()),
        Err(e) => println!("err {e} in {:?}", t.elapsed()),
    }
}
