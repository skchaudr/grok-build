//! Local index of agent session traces across harnesses.
//!
//! Sources: Claude Code, Codex, Pi, OMP, Cursor, DeepSeek Harness (zstd), Grok's
//! own `chat_history.jsonl`, and bb threads read in place from its SQLite db. The index is disposable; source files are the
//! truth and raw lines are re-read from them on demand. traces-ledger-v1

pub mod bb;
mod classify;
mod cursor;
pub mod follow;
pub mod hook;
mod index;
mod lines;
mod links;
pub mod live;
pub mod normalize;
mod query;
pub mod remote;
mod receipts;
pub mod roots;
pub mod wake;

pub use index::{
    Error, GROWTH_LIMIT, GROWTH_WINDOW_MS, HARD_CEILING, Limits, MAX_CAPPED_FILES, MAX_EVENTS_PER_FILE, PARSER_VERSION, Result,
    RootReport, SCHEMA_VERSION, SIZE_BUDGET, ScanReport, TraceIndex, default_db_path, session_id,
};
pub use query::{
    ACTIVE_WINDOW_MS, EventFilter, EventRow, Facets, RECEIPT_WINDOW_MS, SessionFilter, SessionRow, Sort, Stats, Status,
    TERMINAL_EVENTS,
};
pub use receipts::Receipt;
pub use roots::{Format, Root, SOURCES, default_roots, root_for, source_label, user_roots};
