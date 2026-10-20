//! Local index of agent session traces across harnesses.
//!
//! Sources: Claude Code, Codex, Pi, OMP, DeepSeek Harness (zstd) and Grok's
//! own `chat_history.jsonl`. The index is disposable; source files are the
//! truth and raw lines are re-read from them on demand. traces-ledger-v1

pub mod hook;
mod index;
mod lines;
pub mod normalize;
mod query;
mod receipts;
pub mod roots;

pub use index::{
    Error, MAX_DB_BYTES, MAX_EVENTS_PER_FILE, PARSER_VERSION, Result, RootReport, SCHEMA_VERSION, ScanReport, TraceIndex,
    default_db_path, over_cap_message, session_id,
};
pub use query::{
    ACTIVE_WINDOW_MS, EventFilter, EventRow, Facets, RECEIPT_WINDOW_MS, SessionFilter, SessionRow, Sort, Stats, Status,
    TERMINAL_EVENTS,
};
pub use receipts::Receipt;
pub use roots::{Format, Root, SOURCES, default_roots, root_for, source_label, user_roots};
