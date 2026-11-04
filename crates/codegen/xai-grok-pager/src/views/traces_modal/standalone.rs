//! `grok traces <id>`: the one session whose id or native id starts with the prefix.

use super::*;

impl TracesModalState {
    /// The one local session whose id (or native id) starts with `prefix`.
    /// `Err` carries the line to show: not indexed yet, or ambiguous.
    pub fn open_by_prefix(&mut self, prefix: &str) -> Result<SessionRow, String> {
        let filter = SessionFilter { text: Some(prefix.to_string()), limit: 200, ..SessionFilter::default() };
        let rows = self.index.as_ref().and_then(|i| i.list_sessions(&filter).ok()).unwrap_or_default();
        let tail = |id: &str| id.rsplit(':').next().unwrap_or(id).to_string();
        let mut hits = rows.into_iter().filter(|r| {
            r.id.starts_with(prefix) || tail(&r.id).starts_with(prefix) || r.native_id.as_deref().is_some_and(|n| n.starts_with(prefix))
        });
        match (hits.next(), hits.next()) {
            (Some(row), None) => Ok(row),
            (Some(_), Some(_)) => Err(format!("{prefix} matches several sessions")),
            _ => Err(format!("no session matches {prefix}")),
        }
    }
}
