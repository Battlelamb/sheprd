//! `relay-connect`: the `ProxyCommand` `ssh` runs for a machine added over
//! Bluetooth (`sheprd_client::relay_host`). It dials the machine's route
//! on the first of its relays that lets this computer's key through, runs
//! the tunnel against the identity pinned when pairing, and carries bytes
//! between its stdio and the tunnel; SSH's own encryption and host key
//! check run inside, unchanged.

use super::{Error, Keys, Result, record::Record};
use sheprd_relay::{Endpoint, RouteId, client};
use std::{path::Path, time::Duration};
use tokio::io::join;

/// How long reaching one relay and dialing through it may take before the
/// next is tried.
const ATTEMPT: Duration = Duration::from_secs(10);

/// How long the machine's last bytes may take once `ssh` closed its side.
/// `ssh` closes it only as it exits, and the machine may never close its
/// own, so waiting for both would keep this process, and the relay session
/// it holds, alive for good: a dial key holds only a few at once.
const LINGER: Duration = Duration::from_secs(3);

/// Carries `ssh`'s connection to `host`, `r-ID.sheprd.invalid`, until
/// either side closes.
pub(crate) async fn connect(state_dir: &Path, host: &str) -> Result<()> {
    let (connection, tunnel) = open(state_dir, host).await?;
    let spliced = splice(tokio::io::stdin(), tokio::io::stdout(), tunnel.io, LINGER).await;
    drop(connection);
    spliced.map_err(Error::Splice)
}

/// Copies `input` to `tunnel` and `tunnel` to `output` until the tunnel
/// closes, or until `linger` after `input` ends.
async fn splice<I, O, T>(
    mut input: I,
    mut output: O,
    tunnel: T,
    linger: Duration,
) -> std::io::Result<()>
where
    I: tokio::io::AsyncRead + Unpin,
    O: tokio::io::AsyncWrite + Unpin,
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite,
{
    use tokio::io::AsyncWriteExt as _;
    let (mut from_tunnel, mut to_tunnel) = tokio::io::split(tunnel);
    let upload = async {
        let copied = tokio::io::copy(&mut input, &mut to_tunnel).await;
        // Tells the machine nothing more comes, as a half-close does.
        let _ = to_tunnel.shutdown().await;
        copied.map(drop)
    };
    let download = async {
        tokio::io::copy(&mut from_tunnel, &mut output).await?;
        output.flush().await
    };
    tokio::pin!(upload, download);
    tokio::select! {
        downloaded = &mut download => downloaded,
        uploaded = &mut upload => {
            uploaded?;
            match tokio::time::timeout(linger, &mut download).await {
                Ok(downloaded) => downloaded,
                // `ssh` is gone: nothing reads what else arrives.
                Err(_) => Ok(()),
            }
        }
    }
}

/// The tunnel to `host`, through the first relay that lets this computer
/// through, and the relay connection carrying it. A machine the fleet
/// shares is told first that the session carries SSH.
pub(crate) async fn open(
    state_dir: &Path,
    host: &str,
) -> Result<(quinn::Connection, sheprd_tunnel::stream::Tunnel)> {
    let id = sheprd_client::relay_host::host_id(host)
        .ok_or_else(|| Error::Record(host.chars().take(80).collect()))?
        .to_owned();
    let state = state_dir.to_owned();
    let (record, keys) = tokio::task::spawn_blocking(move || {
        Ok::<_, Error>((Record::load(&state, &id)?, Keys::load_or_create(&state)?))
    })
    .await
    .map_err(|_| Error::Cancelled)??;
    let (connection, mut tunnel) = open_record(&record, &keys).await?;
    if record.opening {
        use tokio::io::AsyncWriteExt as _;
        let (state, hash) = (state_dir.to_owned(), tunnel.hash);
        let opening = tokio::task::spawn_blocking(move || {
            crate::fleet::opening(&state, crate::control::fleet::removal::PURPOSE_SSH, &hash)
        })
        .await
        .map_err(|_| Error::Cancelled)??;
        tunnel.io.write_all(&opening).await.map_err(Error::Splice)?;
    }
    Ok((connection, tunnel))
}

/// The tunnel to the machine `record` describes, dialed with `keys`.
pub(crate) async fn open_record(
    record: &Record,
    keys: &Keys,
) -> Result<(quinn::Connection, sheprd_tunnel::stream::Tunnel)> {
    let identity = record.tunnel_identity()?;
    let route = RouteId::parse(&record.route).ok_or(Error::RelayInfo)?;
    let mut last = None;
    for relay in &record.relays {
        let Ok(endpoint) = Endpoint::parse(relay) else {
            continue;
        };
        let attempt = async {
            let connection = client::reach(&endpoint).await?;
            let halves = client::dial(&connection, route, &keys.dial).await?;
            Ok::<_, client::Error>((connection, halves))
        };
        let (connection, (send, recv)) = match tokio::time::timeout(ATTEMPT, attempt).await {
            Ok(Ok(dialed)) => dialed,
            Ok(Err(error)) => {
                last = Some(error);
                continue;
            }
            Err(_) => continue,
        };
        let (initiator, first) =
            sheprd_tunnel::Initiator::session(&keys.tunnel, &identity, route.as_bytes())?;
        let tunnel = tokio::time::timeout(
            ATTEMPT,
            sheprd_tunnel::stream::connect(join(recv, send), initiator, &first, identity.x25519()),
        )
        .await
        .map_err(|_| Error::Tunnel(sheprd_tunnel::Error::Closed))??;
        return Ok((connection, tunnel));
    }
    Err(last.map_or(Error::NoRelays, Error::Relay))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::splice;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[tokio::test(start_paused = true)]
    async fn ssh_leaving_ends_the_splice_though_the_machine_stays_open() {
        let (ours, mut machine) = tokio::io::duplex(64);
        let (output, _kept) = tokio::io::duplex(64);
        // `ssh` sends a little, then closes its side, as it does on exit.
        let input: &[u8] = b"bye";
        let spliced = tokio::spawn(splice(input, output, ours, Duration::from_secs(3)));
        let mut seen = [0_u8; 3];
        machine.read_exact(&mut seen).await.unwrap();
        assert_eq!(&seen, b"bye");
        // The machine never closes: the splice ends on its own.
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(spliced.is_finished());
        spliced.await.unwrap().unwrap();
        drop(machine);
    }

    #[tokio::test(start_paused = true)]
    async fn the_machines_last_bytes_still_reach_ssh() {
        let (ours, mut machine) = tokio::io::duplex(64);
        let (output, mut read) = tokio::io::duplex(64);
        let input: &[u8] = b"";
        let spliced = tokio::spawn(splice(input, output, ours, Duration::from_secs(3)));
        machine.write_all(b"last").await.unwrap();
        drop(machine);
        spliced.await.unwrap().unwrap();
        let mut seen = Vec::new();
        read.read_to_end(&mut seen).await.unwrap();
        assert_eq!(seen, b"last");
    }
}
