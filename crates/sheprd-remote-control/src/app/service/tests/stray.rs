//! A daemon the service manager did not start, such as an older release's
//! or one started by hand, holds the port until it is asked to exit: a
//! restart or an update stops whatever answers before starting the service,
//! or the new daemon fails to bind on every retry while the old one serves.

use super::*;

fn on() -> RemoteConfig {
    RemoteConfig {
        enabled: true,
        ..RemoteConfig::default()
    }
}

fn launchd(dir: &Path) -> Host {
    Host {
        manager: Manager::LaunchAgent { uid: 501 },
        file: dir.join("LaunchAgents/so.pen.herdr-gpui.remote.plist"),
        release: Some("20261120.1".into()),
        ..host(dir)
    }
}

/// What happened, in order: requests to the daemon and manager commands.
#[derive(Default)]
struct Log(RefCell<Vec<String>>);

impl Log {
    fn push(&self, entry: impl Into<String>) {
        self.0.borrow_mut().push(entry.into());
    }

    fn entries(self) -> Vec<String> {
        self.0.into_inner()
    }
}

/// A daemon of `version` that exits `answers_after` status checks after it
/// takes a shutdown.
fn daemon<'a>(
    log: &'a Log,
    version: &'a str,
    answers_after: usize,
) -> impl Fn(&Request) -> Result<Response, ControlError> + 'a {
    let left = RefCell::new(None::<usize>);
    move |request| {
        let mut left = left.borrow_mut();
        match (request, *left) {
            (Request::Shutdown, None) => {
                log.push("shutdown");
                *left = Some(answers_after);
                Ok(Response::Done)
            }
            (_, Some(0)) => unreachable(),
            (Request::Status, Some(n)) => {
                *left = Some(n - 1);
                Ok(status(version))
            }
            (Request::Status, None) => Ok(status(version)),
            _ => unreachable(),
        }
    }
}

#[test]
fn an_update_stops_the_old_daemon_before_restarting_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let host = launchd(dir.path());
    let no_core = || Err(sheprd_core_client::Error::NoHome);
    reconcile_with_core(&on(), &host, |_| unreachable(), |_| Ok(()), || {}, no_core).unwrap();
    let log = Log::default();
    let waits = RefCell::new(0);
    let outcome = reconcile_with_core(
        &on(),
        &host,
        daemon(&log, "20261106.1", 2),
        |step| {
            log.push(command(step));
            Ok(())
        },
        || *waits.borrow_mut() += 1,
        no_core,
    )
    .unwrap();
    assert_eq!(outcome, Outcome::Started);
    let entries = log.entries();
    assert_eq!(entries.first().map(String::as_str), Some("shutdown"));
    assert_eq!(
        entries.last().map(String::as_str),
        Some("launchctl kickstart -k gui/501/so.pen.herdr-gpui.remote")
    );
    assert_eq!(waits.into_inner(), 2, "waited until it stopped answering");
}

#[test]
fn a_restart_stops_whatever_daemon_answers_first() {
    let dir = tempfile::tempdir().unwrap();
    let host = launchd(dir.path());
    reconcile(&on(), &host, |_| unreachable(), |_| Ok(())).unwrap();
    let log = Log::default();
    restart_with_wait(
        &on(),
        &host,
        daemon(&log, "20261120.1", 0),
        |step| {
            log.push(command(step));
            Ok(())
        },
        || panic!("a daemon gone at once is not waited for"),
    )
    .unwrap();
    let entries = log.entries();
    assert_eq!(entries.first().map(String::as_str), Some("shutdown"));
    assert!(
        entries.iter().any(|entry| entry.contains("kickstart -k")),
        "{entries:?}"
    );
}

#[test]
fn a_daemon_that_keeps_answering_is_waited_for_a_bounded_time() {
    let dir = tempfile::tempdir().unwrap();
    let host = launchd(dir.path());
    reconcile(&on(), &host, |_| unreachable(), |_| Ok(())).unwrap();
    let waits = RefCell::new(0);
    let ran = RefCell::new(Vec::new());
    restart_with_wait(
        &on(),
        &host,
        |request| match request {
            Request::Shutdown => Ok(Response::Done),
            _ => Ok(status("20261120.1")),
        },
        |step| {
            ran.borrow_mut().push(command(step));
            Ok(())
        },
        || *waits.borrow_mut() += 1,
    )
    .unwrap();
    assert_eq!(waits.into_inner(), SETTLE_ATTEMPTS);
    assert!(!ran.into_inner().is_empty(), "the service still restarts");
}

#[test]
fn nothing_answering_restarts_without_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let host = launchd(dir.path());
    let ran = RefCell::new(Vec::new());
    restart_with_wait(
        &on(),
        &host,
        |_| unreachable(),
        |step| {
            ran.borrow_mut().push(command(step));
            Ok(())
        },
        || panic!("nothing to wait for"),
    )
    .unwrap();
    assert!(
        ran.into_inner()
            .iter()
            .any(|entry| entry.contains("bootstrap")),
        "a missing definition is written and loaded"
    );
}
