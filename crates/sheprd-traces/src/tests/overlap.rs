use super::*;
use crate::OverlapPoint;
use rusqlite::params;

#[test]
fn daily_peaks_recent_counts_and_membership_share_a_half_open_window() {
    let store = Store::in_memory().unwrap();
    let db = &store.connection;
    let midnight = 86_400_000;
    let minute = 60_000;
    for (id, agent) in [("one", "codex"), ("two", "claude")] {
        db.execute(
            "INSERT INTO hosts(host_id,label,os,kind) VALUES (?1,'box','test','local')",
            [id],
        )
        .unwrap();
        db.execute("INSERT INTO sessions(session_id,host_id,native_id,user,agent) VALUES (?1,?1,?1,'me',?2)", [id,agent]).unwrap();
    }
    for (seq, (id, kind, offset)) in [
        ("one", "agent", Some(-5)),
        ("one", "tool_call", Some(-4)),
        ("two", "thinking", Some(-1)),
        ("two", "user", Some(10)),
        ("two", "tool_result", Some(12)),
        ("two", "agent", None),
        ("two", "agent", Some(21)),
    ]
    .into_iter()
    .enumerate()
    {
        db.execute(
            "INSERT INTO events(session_id,seq,kind,at,text) VALUES (?1,?2,?3,?4,'')",
            params![id, seq as i64, kind, offset.map(|m| midnight + m * minute)],
        )
        .unwrap();
    }
    let now = midnight + 11 * minute;
    let chart = store.overlap(now, 2, None, None).unwrap();
    assert_eq!(
        chart.daily,
        [
            OverlapPoint {
                at: midnight - minute,
                count: 2
            },
            OverlapPoint {
                at: midnight,
                count: 2
            }
        ]
    );
    assert_eq!(chart.recent.count, 1);
    assert_eq!(
        chart.recent_by_agent,
        [crate::ActiveAgent {
            agent: AgentKind::Claude,
            count: 1
        }]
    );
    let recent = store.active_sessions(now, None, None).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].id, "two");
    assert_eq!(
        store.active_sessions(midnight, None, None).unwrap().len(),
        2
    );
    assert_eq!(
        store.overlap(now, 1, None, None).unwrap().daily[0],
        chart.daily[1]
    );
    assert_eq!(
        store.overlap(now, 1, Some(&[]), None).unwrap().daily[0].count,
        0
    );
    assert_eq!(
        store.overlap(now, 1, Some(&["one"]), None).unwrap().daily[0].count,
        1
    );
    assert!(
        store
            .active_sessions(midnight, Some(&["one"]), Some(AgentKind::Claude))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .overlap(midnight + 15 * minute, 1, None, None)
            .unwrap()
            .recent
            .count,
        0
    );
}
