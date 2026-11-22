use crate::control::server::tabs::machine_claims::*;

fn caller(id: &str) -> Caller {
    Caller {
        terminal_id: Some(id.into()),
        ..Caller::default()
    }
}
fn busy(response: Response) {
    assert!(matches!(
        response,
        Response::Error {
            code: ErrorCode::Busy,
            ..
        }
    ));
}
fn done(response: Response) {
    assert!(matches!(response, Response::Done { .. }));
}

#[test]
fn reservations_exclude_other_panes_and_release_only_the_owner() {
    let mut reservations = Reservations::default();
    let now = Instant::now();
    done(reservations.change(&caller("a"), "one", false, now));
    busy(reservations.change(&caller("b"), "one", false, now));
    busy(reservations.change(&caller("b"), "one", true, now));
    assert!(matches!(
        reservations.check(&caller("b"), "one", now),
        Some(Response::Error {
            code: ErrorCode::Busy,
            ..
        })
    ));
    done(reservations.change(&caller("b"), "two", false, now));
    done(reservations.change(&caller("a"), "one", true, now));
    done(reservations.change(&caller("b"), "one", false, now));
}

#[test]
fn use_renews_and_expiry_allows_recovery_without_stale_release() {
    let mut reservations = Reservations::default();
    let now = Instant::now();
    done(reservations.change(&caller("a"), "one", false, now));
    assert!(
        reservations
            .check(&caller("a"), "one", now + IDLE / 2)
            .is_none()
    );
    busy(reservations.change(&caller("b"), "one", false, now + IDLE));
    done(reservations.change(&caller("b"), "one", false, now + IDLE * 2));
    busy(reservations.change(&caller("a"), "one", true, now + IDLE * 2));
}

#[test]
fn anonymous_callers_cannot_claim_and_daemons_disambiguate_panes() {
    let mut reservations = Reservations::default();
    let now = Instant::now();
    assert!(matches!(
        reservations.change(&Caller::default(), "one", false, now),
        Response::Error {
            code: ErrorCode::InvalidRequest,
            ..
        }
    ));
    let a = caller("same");
    let b = Caller {
        daemon_socket: Some("other".into()),
        ..a.clone()
    };
    done(reservations.change(&a, "one", false, now));
    busy(reservations.change(&b, "one", false, now));
}
