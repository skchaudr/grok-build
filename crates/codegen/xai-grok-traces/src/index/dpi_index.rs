//! dpi agents in the index. One `trace_files`/`trace_sessions` row per conversation, keyed
//! `<agent.sqlite path>#<conversation id>`; the pointer behind an event is `(conversation id, entry id)`
//! with `line_number = entry id`. Each `agent.sqlite` is opened read-only for the length of one scan and
//! never kept. Conversations are scanned newest first alongside files (see `scan_with_limit`).

use super::*;

use crate::dpi::{self, DpiConversation, DpiDb};

/// Entries read from an agent.sqlite per query.
const PAGE: usize = 1000;

impl TraceIndex {
    /// Lists a dpi root's conversations into `work` (by `updated_at`). A missing root leaves cached rows alone;
    /// so does a session whose db could not be read.
    pub(super) fn plan_dpi_root(&self, root: &Root, i: usize, work: &mut Vec<(i64, usize, Work)>) -> RootPlan {
        let accessible = root.path.is_dir();
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
        let listing = dpi::list(&root.path);
        let mut keys = HashSet::new();
        for c in listing.conversations.into_iter().filter(|c| c.last_sequence.is_some()) {
            keys.insert(c.key());
            work.push((c.updated_at, i, Work::Dpi(c)));
        }
        plan.files = keys.len();
        if let Some((session, e)) = listing.unreadable.first() {
            plan.error = Some(format!("{session}: {e}"));
        } else {
            plan.seen = Some(keys);
        }
        plan
    }

    /// Indexes one conversation of a dpi root (a hook's `session:conversation`). Returns whether anything was parsed.
    pub fn index_dpi_conversation_id(&mut self, root: &Root, session: &str, conversation: i64) -> Result<bool> {
        if root.format != Format::Sqlite {
            return Err(Error::Rejected(root.path.clone()));
        }
        match dpi::find(&root.path, session, conversation)? {
            Some(c) if c.last_sequence.is_some() => self.index_dpi_conversation(root, &c),
            _ => Ok(false),
        }
    }

    pub(super) fn index_dpi_conversation(&mut self, root: &Root, c: &DpiConversation) -> Result<bool> {
        self.check_writable()?;
        let key = c.key();
        let sid = session_id(&root.source, &key);
        let last_seq = c.last_sequence.unwrap_or(0);
        // (mtime, last indexed entry id, events indexed, parser version)
        let existing: Option<(i64, i64, i64, i64)> = self
            .conn
            .query_row(
                "SELECT mtime_ms, indexed_bytes, indexed_lines, parser_version FROM trace_files WHERE path = ?1",
                [&key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if existing.is_none() && c.updated_at <= self.evict_floor() {
            return Ok(false);
        }
        let reset = existing.is_none_or(|(_, done, _, pv)| pv != PARSER_VERSION || last_seq < done);
        let (after, mut count) = match existing {
            Some((mtime, done, lines, _)) if !reset => {
                if done == last_seq && mtime == c.updated_at {
                    return Ok(false);
                }
                (done, lines)
            }
            _ => (0, 0),
        };

        let db = DpiDb::open(&c.db)?;
        let native = c.native_id();
        let mut agg = if reset { Aggregate::default() } else { self.load_aggregate(&sid, &native)? };
        if let Some(name) = &c.name {
            agg.user_title = Some(clip(name.lines().next().unwrap_or(name), TITLE_CHARS));
        }
        if c.cwd.is_some() {
            agg.cwd = c.cwd.clone();
        }

        let tx = self.conn.transaction()?;
        let sk = upsert_session(&tx, &sid, &root.source, &native, &key, last_seq)?;
        write_links(
            &tx,
            &sid,
            &Links {
                native_id: Some(native),
                parent_native: c.parent_id.map(|p| dpi::native_id(&c.session, p)),
                label: c.parent_id.and(c.name.clone()),
                agent_type: c.provider.clone(),
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
            let mut find_call = tx.prepare_cached(FIND_CALL)?;
            let mut set_duration = tx.prepare_cached(SET_DURATION)?;
            let mut spans = ToolSpans::default();
            'pages: loop {
                let page = db.events(c.id, seq, PAGE)?;
                let short = page.len() < PAGE;
                for ev in page {
                    if count >= max_events as i64 {
                        capped = true;
                        break 'pages;
                    }
                    agg.add(&ev.event);
                    insert_event(&mut insert, &mut usage, &mut names, &tx, sk, ev.entry_id, 0, &ev.event)?;
                    spans.observe(&mut find_call, &mut set_duration, sk, ev.entry_id, &ev.event)?;
                    seq = ev.entry_id;
                    count += 1;
                }
                if short {
                    break;
                }
            }
        }
        // Skipped entry kinds trail the last indexed one; everything up to the listing's last id was examined.
        let (done, parse_error) = if capped {
            (last_seq, Some(format!("event cap {max_events} reached; rest of conversation not indexed")))
        } else {
            (seq.max(last_seq), None)
        };
        if capped {
            self.capped_files += 1;
        }
        // The catalog's model is the session default; the model the messages name wins.
        if agg.model.is_none() {
            agg.model = c.model.clone();
        }
        Self::write_aggregate(&tx, &sid, &agg, c.updated_at)?;
        tx.execute(
            "INSERT INTO trace_files (path, root_id, source_id, format, size_bytes, mtime_ms, indexed_bytes, indexed_lines,
             parser_version, content_hash, session_id, indexed_at, parse_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12)
             ON CONFLICT(path) DO UPDATE SET root_id=?2, source_id=?3, format=?4, size_bytes=?5, mtime_ms=?6, indexed_bytes=?7,
             indexed_lines=?8, parser_version=?9, content_hash=NULL, session_id=?10, indexed_at=?11, parse_error=?12",
            params![
                key, root.id, root.source, root.format.as_str(), last_seq, c.updated_at, done, count, PARSER_VERSION, sid,
                now_ms(), parse_error
            ],
        )?;
        tx.commit()?;
        Ok(true)
    }
}
