//! bb threads in the index. One `trace_files`/`trace_sessions` row per thread,
//! keyed `<bb.db path>#<thread id>`; the pointer behind an event is
//! `(thread id, sequence)` with `line_number = sequence`. `bb.db` is opened
//! read-only for the length of one scan and never kept. Threads are scanned newest
//! first alongside files (see `scan_with_limit`).

use super::*;

/// Events read from bb.db per query.
const PAGE: usize = 1000;

impl TraceIndex {
    /// Lists a bb root's threads into `work` (by `updated_at`). A missing or unreadable
    /// db leaves cached rows alone.
    pub(super) fn plan_bb_root(&self, root: &Root, i: usize, work: &mut Vec<(i64, usize, Work)>) -> RootPlan {
        let accessible = root.path.is_file();
        let mut plan = RootPlan {
            files: 0,
            accessible,
            limited: false,
            error: (!accessible).then(|| "Root is missing or not readable".to_string()),
            seen: None,
            bb: None,
        };
        if !accessible {
            return plan;
        }
        match Bb::open(&root.path).and_then(|bb| bb.threads().map(|t| (bb, t))) {
            Err(e) => plan.error = Some(format!("bb.db: {e}")),
            Ok((bb, threads)) => {
                let mut keys = HashSet::new();
                for t in threads.into_iter().filter(|t| t.last_sequence.is_some()) {
                    keys.insert(thread_key(&root.path, &t.id));
                    work.push((t.updated_at, i, Work::Bb(t)));
                }
                plan.files = keys.len();
                plan.seen = Some(keys);
                plan.bb = Some(bb);
            }
        }
        plan
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

    pub(super) fn index_bb_thread(&mut self, root: &Root, bb: &Bb, t: &BbThread) -> Result<bool> {
        self.check_writable()?;
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
        if existing.is_none() && t.updated_at <= self.evict_floor() {
            return Ok(false);
        }
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
        let sk = upsert_session(&tx, &sid, &root.source, &t.id, &key, last_seq)?;
        write_links(
            &tx,
            &sid,
            &Links {
                native_id: Some(t.id.clone()),
                parent_native: t.parent_thread_id.clone(),
                agent_type: Some(t.provider_id.clone()),
                ..Default::default()
            },
        )?;
        if reset {
            delete_events(&tx, sk)?;
        }
        let mut names = Names::default();
        let mut seq = after;
        let mut capped = false;
        let max_events = self.limits.max_events;
        {
            let mut insert = tx.prepare_cached(INSERT_EVENT)?;
            let mut usage = tx.prepare_cached(INSERT_USAGE)?;
            'pages: loop {
                let page = bb.events(&t.id, seq, PAGE)?;
                let short = page.len() < PAGE;
                for ev in page {
                    if count >= max_events as i64 {
                        capped = true;
                        break 'pages;
                    }
                    agg.add(&ev.event);
                    insert_event(&mut insert, &mut usage, &mut names, &tx, sk, ev.sequence, 0, &ev.event)?;
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
            (last_seq, Some(format!("event cap {max_events} reached; rest of thread not indexed")))
        } else {
            (seq.max(last_seq), None)
        };
        if capped {
            self.capped_files += 1;
        }
        if t.status == "error" {
            agg.failed = true;
        }
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
