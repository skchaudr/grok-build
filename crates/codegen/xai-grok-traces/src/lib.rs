//! Local index of agent session traces across harnesses.
//!
//! Sources: Claude Code, Codex, Pi, OMP, Cursor, DeepSeek Harness (zstd) and Grok's
//! own `chat_history.jsonl`. The index is disposable; source files are the
//! truth and raw lines are re-read from them on demand. traces-ledger-v1

pub mod bb;
mod cursor;
mod index;
mod lines;
pub mod normalize;
mod query;
pub mod roots;

pub use index::{Error, Result, RootReport, ScanReport, TraceIndex, default_db_path, session_id};
pub use query::{ACTIVE_WINDOW_MS, EventFilter, EventRow, Facets, SessionFilter, SessionRow, Sort, Status};
pub use roots::{Format, Root, SOURCES, default_roots, source_label, user_roots};
