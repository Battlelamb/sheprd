//! Daily peaks over distinct sessions with recent model activity.
use super::{COLUMNS, MAX_LIMIT, Store, session};
use crate::{ActiveAgent, Result, Session};
use rusqlite::params;
use serde::Serialize;
use sheprd_client::herdr::AgentKind;
use std::collections::{BTreeSet, HashMap};

const DAY: i64 = 86_400_000;
const WINDOW: i64 = 15 * 60_000;
const ACTIVITY: &str = "FROM events e JOIN sessions s USING (session_id)
    WHERE e.at > ?1 AND e.at <= ?2 AND e.kind IN ('agent','thinking','tool_call')
    AND (?3 IS NULL OR s.host_id IN (SELECT value FROM json_each(?3)))
    AND (?4 IS NULL OR s.agent = ?4)";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct OverlapPoint {
    /// Unix milliseconds. Ties use the earliest instant in the UTC day.
    pub at: i64,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Overlap {
    pub recent: OverlapPoint,
    pub recent_by_agent: Vec<ActiveAgent>,
    /// One point per UTC day including empty days, oldest first.
    pub daily: Vec<OverlapPoint>,
}

#[derive(Default)]
struct Active {
    last: HashMap<String, (i64, AgentKind)>,
    ordered: BTreeSet<(i64, String)>,
}

impl Active {
    fn record(&mut self, id: String, at: i64, agent: AgentKind) {
        if let Some((old, _)) = self.last.insert(id.clone(), (at, agent)) {
            self.ordered.remove(&(old, id.clone()));
        }
        self.ordered.insert((at, id));
    }

    fn count(&mut self, at: i64) -> usize {
        while self
            .ordered
            .first()
            .is_some_and(|(last, _)| *last <= at - WINDOW)
        {
            if let Some((_, id)) = self.ordered.pop_first() {
                self.last.remove(&id);
            }
        }
        self.last.len()
    }
}

impl Store {
    /// Count sessions in the trailing 15 minutes. Stream events by timestamp,
    /// retaining only active sessions and at most a year of daily peaks.
    pub fn overlap(
        &self,
        now: i64,
        days: u32,
        hosts: Option<&[&str]>,
        agent: Option<AgentKind>,
    ) -> Result<Overlap> {
        let days = i64::from(days.clamp(1, 365));
        let first = (now.div_euclid(DAY) - days + 1) * DAY;
        let hosts = hosts.map(|ids| serde_json::json!(ids).to_string());
        let mut stmt = self.connection.prepare_cached(&format!(
            "SELECT e.at, e.session_id, s.agent {ACTIVITY} ORDER BY e.at"
        ))?;
        let mut rows = stmt.query(params![
            first - WINDOW,
            now,
            hosts,
            agent.map(AgentKind::name)
        ])?;
        let mut row = rows.next()?;
        let mut active = Active::default();
        let mut daily = Vec::new();
        for i in 0..days {
            let start = first + i * DAY;
            let mut peak = OverlapPoint {
                at: start,
                count: active.count(start),
            };
            while let Some(event) = row {
                let at: i64 = event.get(0)?;
                if at >= start + DAY {
                    break;
                }
                let agent: String = event.get(2)?;
                if let Some(agent) = AgentKind::parse(&agent) {
                    active.record(event.get(1)?, at, agent);
                }
                // Include activity carried in from before the first midnight.
                let at = at.max(start);
                let count = active.count(at);
                if count > peak.count {
                    peak = OverlapPoint { at, count };
                }
                row = rows.next()?;
            }
            daily.push(peak);
        }
        Ok(Overlap {
            recent: OverlapPoint {
                at: now,
                count: active.count(now),
            },
            recent_by_agent: ActiveAgent::counts(active.last.values().map(|(_, agent)| *agent)),
            daily,
        })
    }

    /// Session metadata for a chart point, independent of the recent-session listing.
    pub fn active_sessions(
        &self,
        at: i64,
        hosts: Option<&[&str]>,
        agent: Option<AgentKind>,
    ) -> Result<Vec<Session>> {
        let hosts = hosts.map(|ids| serde_json::json!(ids).to_string());
        let mut stmt = self.connection.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM sessions s JOIN hosts h USING (host_id)
             WHERE s.session_id IN (SELECT e.session_id {ACTIVITY})
             ORDER BY s.updated_at DESC, s.session_id LIMIT {MAX_LIMIT}"
        ))?;
        let rows = stmt.query_map(
            params![
                at.saturating_sub(WINDOW),
                at,
                hosts,
                agent.map(AgentKind::name)
            ],
            session,
        )?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.extend(row?);
        }
        Ok(sessions)
    }
}
