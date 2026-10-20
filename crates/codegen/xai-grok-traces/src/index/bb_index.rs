//! bb threads in the index. One `trace_files`/`trace_sessions` row per thread,
//! keyed `<bb.db path>#<thread id>`; the pointer behind an event is
//! `(thread id, sequence)` with `line_number = sequence`. `bb.db` is opened
//! read-only for the length of one scan and never kept.

use super::*;
use crate::bb::{Bb, BbThread, thread_key};

/// Events read from bb.db per query.
const PAGE: usize = 1000;

impl TraceIndex {
    /// Indexes every thread of a bb root, pruning threads that are gone. A missing
    /// or unreadable db leaves cached rows alone.
    pub(super) fn scan_bb_root(&mut self, root: &Root, report: &mut ScanReport) -> Result<RootReport> {
        let accessible = root.path.is_file();
        let mut rr = RootReport { id: root.id.clone(), accessible, ..Default::default() };
        let mut error = (!accessible).then(|| "Root is missing or not readable".to_string());
        let mut seen: Option<HashSet<String>> = None;
        if accessible {
            match Bb::open(&root.path) {
                Err(e) => error = Some(format!("bb.db: {e}")),
                Ok(bb) => match bb.threads() {
                    Err(e) => error = Some(format!("bb.db: {e}")),
                    Ok(threads) => {
                        let mut keys = HashSet::new();
                        for t in threads.iter().filter(|t| t.last_sequence.is_some()) {
                            let key = thread_key(&root.path, &t.id);
                            match self.index_bb_thread(root, &bb, t) {
                                Ok(true) => report.indexed += 1,
                                Ok(false) => {}
                                Err(e @ Error::OverCap { .. }) => return Err(e),
                                Err(e) => report.errors.push((PathBuf::from(&key), e.to_string())),
                            }
                            keys.insert(key);
                        }
                        rr.files = keys.len();
                        seen = Some(keys);
                    }
                },
            }
        }
        self.conn.execute(
            "INSERT INTO trace_roots (id, source_id, path, exists_flag, file_count, last_scan_at, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET source_id=?2, path=?3, exists_flag=?4, file_count=?5, last_scan_at=?6, error=?7",
            params![root.id, root.source, root.path.to_string_lossy(), accessible, rr.files, now_ms(), error],
        )?;
        if let Some(keys) = seen {
            self.prune_files(&root.id, |p| !keys.contains(p))?;
        }
        Ok(rr)
    }

    /// Indexes one thread of a bb root (a hook's `thread_id`). Returns whether anything was parsed.
    pub fn index_bb_thread_id(&mut self, root: &Root, thread_id: &str) -> Result<bool> {
        if root.format != Format::Sqlite {
            return Err(Error::Rejected(root.path.clone()));
        }
        let bb = Bb::open(&root.path)?;
        match bb.thread(thread_id)? {
            Some(t) if t.last_sequence.is_some() => self.index_bb_thread(root, &bb, &t),
            _ => Ok(false),
        }
    }

    fn index_bb_thread(&mut self, root: &Root, bb: &Bb, t: &BbThread) -> Result<bool> {
        self.check_cap()?;
        let key = thread_key(&root.path, &t.id);
        let sid = session_id(&root.source, &key);
        let last_seq = t.last_sequence.unwrap_or(0);
        // (mtime, last indexed sequence, events indexed, parser version)
        let existing: Option<(i64, i64, i64, i64)> = self
            .conn
            .query_row(
                "SELECT mtime_ms, indexed_bytes, indexed_lines, parser_version FROM trace_files WHERE path = ?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let reset = existing.is_none_or(|(_, done, _, pv)| pv != PARSER_VERSION || last_seq < done);
        let (after, mut count) = match existing {
            Some((mtime, done, lines, _)) if !reset => {
                if done == last_seq && mtime == t.updated_at {
                    return Ok(false);
                }
                (done, lines)
            }
            _ => (0, 0),
        };

        let mut agg = if reset { Aggregate { title: t.id.clone(), ..Default::default() } } else { self.load_aggregate(&sid, &t.id)? };
        if let Some(title) = &t.title {
            agg.user_title = Some(clip(title.lines().next().unwrap_or(title), TITLE_CHARS));
        }
        if t.model.is_some() {
            agg.model = t.model.clone();
        }
        if t.cwd.is_some() {
            agg.cwd = t.cwd.clone();
        }

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO trace_sessions (id, source_id, title, file_path, file_size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET file_path = ?4, file_size_bytes = ?5",
            params![sid, root.source, t.id, key, last_seq],
        )?;
        if reset {
            tx.execute("DELETE FROM trace_events WHERE session_id = ?1", [&sid])?;
        }
        let mut seq = after;
        let mut capped = false;
        {
            let mut insert = tx.prepare_cached(INSERT_EVENT)?;
            'pages: loop {
                let page = bb.events(&t.id, seq, PAGE)?;
                let short = page.len() < PAGE;
                for ev in page {
                    if count >= MAX_EVENTS_PER_FILE as i64 {
                        capped = true;
                        break 'pages;
                    }
                    agg.add(&ev.event);
                    insert_event(&mut insert, &sid, ev.sequence, 0, &ev.event)?;
                    seq = ev.sequence;
                    count += 1;
                }
                if short {
                    break;
                }
            }
        }
        // Skipped event types trail the last indexed one; everything up to the listing's
        // last sequence was examined.
        let (done, parse_error) = if capped {
            (last_seq, Some(format!("event cap {MAX_EVENTS_PER_FILE} reached; rest of thread not indexed")))
        } else {
            (seq.max(last_seq), None)
        };
        Self::write_aggregate(&tx, &sid, &agg, t.updated_at)?;
        tx.execute(
            "INSERT INTO trace_files (path, root_id, source_id, format, size_bytes, mtime_ms, indexed_bytes, indexed_lines,
             parser_version, content_hash, session_id, indexed_at, parse_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12)
             ON CONFLICT(path) DO UPDATE SET root_id=?2, source_id=?3, format=?4, size_bytes=?5, mtime_ms=?6, indexed_bytes=?7,
             indexed_lines=?8, parser_version=?9, content_hash=NULL, session_id=?10, indexed_at=?11, parse_error=?12",
            params![
                key, root.id, root.source, root.format.as_str(), last_seq, t.updated_at, done, count, PARSER_VERSION, sid,
                now_ms(), parse_error
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }
}
