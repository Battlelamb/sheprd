//! Listening on relays, so a paired phone reaches this computer from any
//! network (`docs/relay-plan.md`, Clients). For each relay in `[remote]
//! relays`, the daemon keeps one QUIC connection and listens on its route,
//! naming the dial keys of its paired devices. Each dial runs the
//! end-to-end tunnel (`sheprd-tunnel`), admitting only a paired device's
//! tunnel key, then serves the same API over it, over the same TLS as on
//! the network with the certificate this computer presents for
//! `127.0.0.1`: the phone reaches it through a loopback proxy at
//! `https://127.0.0.1:PORT`, so its pinning, sign-in, and bearer tokens
//! work unchanged, and TLS adds a layer inside the tunnel.
//!
//! A relay sees only the tunnel's ciphertext, and a device that is not
//! paired hears `offline` from the relay or, past it, nothing at all.
//!
//! Relays share nothing, so a phone always dials the relays this computer
//! listens on, never its own: two machines set up with different relays
//! still meet. To use the nearest, the daemon measures each relay's round
//! trip and `GET /api/relay` lists them nearest first, with those times,
//! and the phone adds its own leg to each before choosing (`nearest`).

mod error;
pub(crate) mod keys;
mod listen;
mod nearest;
mod serve;

pub use error::Error;
pub(crate) use error::Result;

pub(crate) use crate::control::RelayState as Status;
use crate::control::RelayStatus;
use crate::relay_label::RelayLabel;
use crate::server::Relay;
use serde::Serialize;
use sheprd_relay::Endpoint;
use sheprd_relay::wire::Key;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
use tokio::sync::watch;

/// How long a pairing dial stays open on the relays: the nearby token's
/// claim window, with room for the phone to find a relay.
const PAIRING: Duration = Duration::from_secs(120);

/// A pairing dial the relays let through once: the key a nearby phone
/// derives from the token sealed to it, and that token, the tunnel's
/// pairing secret.
#[derive(Clone, Debug)]
struct Pairing {
    key: Key,
    secret: [u8; 32],
    until: Instant,
}

/// Lets the phone a nearby offer was just sealed to pair through the
/// relays from another network, when this computer listens on any.
pub(crate) fn open_pairing(state: &crate::web::State) {
    let Some(relay) = &state.relay else {
        return;
    };
    let secret = state.auth().take_relay_pairing();
    if let Some(secret) = secret {
        relay.offer_pairing(secret);
    }
}

/// Has nearby offers say what the relays measure now, nearest first, as
/// `GET /api/relay` does, rather than what they said at start, before any
/// listened: called before each offer is sealed.
pub(crate) fn refresh_offer(relays: Option<&Relays>, auth: &mut crate::auth::Auth) {
    if let Some(relays) = relays {
        auth.set_relay_offer(relays.offer());
    }
}

/// The relays this computer listens on, and what it listens with.
#[derive(Debug)]
pub(crate) struct Relays {
    /// Bumped whenever a device's relay keys change, so each listener
    /// names them again.
    changed: watch::Sender<u64>,
    endpoints: Vec<Endpoint>,
    keys: keys::Keys,
    /// What `[remote] relays` calls each relay, by endpoint; unlabelled
    /// relays are left out.
    labels: BTreeMap<String, RelayLabel>,
    /// The open pairing dial, if any, and a revision listeners follow.
    pairing: Mutex<Option<Pairing>>,
    pairing_changed: watch::Sender<u64>,
    /// Each listening relay's round-trip time, keyed by endpoint.
    rtt: Mutex<BTreeMap<String, Duration>>,
    /// Each relay's state, for `sheprd remote status`.
    status: Mutex<BTreeMap<String, Status>>,
}

/// What a paired phone needs to reach this computer through its relays.
#[derive(Debug, Serialize)]
pub(crate) struct Info {
    /// The tunnel identity to pin: X25519 then ML-KEM-768, base64url.
    pub(crate) identity: String,
    /// What the user calls each relay, by endpoint as `relays` writes it;
    /// unlabelled relays are left out, and the field with them.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) labels: BTreeMap<String, RelayLabel>,
    /// The relays this computer listens on, nearest first; those not
    /// listening now keep their configured order, after the rest.
    pub(crate) relays: Vec<String>,
    /// This computer's route, base64url.
    pub(crate) route: String,
    /// Each listening relay's round trip from this computer, in
    /// milliseconds, so the phone can add its own leg. Older phones ignore
    /// it.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) rtt_ms: BTreeMap<String, u32>,
}

impl Info {
    /// The info without labels: see [`Relays::offer`].
    fn without_labels(self) -> Self {
        Self {
            labels: BTreeMap::new(),
            ..self
        }
    }
}

impl Relays {
    /// Loads or makes the relay keys in `state_dir`. Blocking.
    pub(crate) fn load(state_dir: &Path, relays: Vec<Relay>) -> Result<Self> {
        let labels = relays
            .iter()
            .filter(|relay| !relay.label.is_empty())
            .map(|relay| (relay.endpoint.to_string(), relay.label.clone()))
            .collect();
        Ok(Self {
            changed: watch::Sender::new(0),
            endpoints: relays.into_iter().map(|relay| relay.endpoint).collect(),
            keys: keys::Keys::load_or_create(state_dir)?,
            labels,
            pairing: Mutex::default(),
            pairing_changed: watch::Sender::new(0),
            rtt: Mutex::default(),
            status: Mutex::default(),
        })
    }

    /// What nearby offers carry: [`Self::info`] without labels, as their
    /// sealed bytes cross Bluetooth and match shared vectors; the phone
    /// learns the labels from `GET /api/relay` once paired.
    pub(crate) fn offer(&self) -> Info {
        self.info().without_labels()
    }

    /// What `GET /api/relay` answers.
    pub(crate) fn info(&self) -> Info {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        let rtt = self.rtt.lock().unwrap_or_else(PoisonError::into_inner);
        Info {
            identity: URL_SAFE_NO_PAD.encode(self.keys.tunnel.identity().to_bytes()),
            labels: self.labels.clone(),
            relays: nearest::first(&self.endpoints, &rtt),
            route: self.keys.route().to_string(),
            rtt_ms: nearest::millis(&rtt),
        }
    }

    /// Opens a pairing dial for the phone holding `secret`, on every relay.
    pub(crate) fn offer_pairing(&self, secret: [u8; 32]) {
        let key = sheprd_relay::client::public(&sheprd_relay::route::pairing_dial_key(&secret));
        *self.pairing.lock().unwrap_or_else(PoisonError::into_inner) = Some(Pairing {
            key,
            secret,
            until: Instant::now() + PAIRING,
        });
        self.pairing_changed.send_modify(|revision| *revision += 1);
    }

    /// The open pairing dial's key, and how many seconds it has left.
    fn pairing_key(&self) -> Option<(Key, u16)> {
        let pairing = self.pairing.lock().unwrap_or_else(PoisonError::into_inner);
        let pairing = pairing.as_ref()?;
        let left = pairing.until.checked_duration_since(Instant::now())?;
        Some((
            pairing.key,
            u16::try_from(left.as_secs()).unwrap_or(u16::MAX).max(1),
        ))
    }

    /// The pairing secret, when `key` is the open pairing dial's.
    fn pairing_secret(&self, key: &Key) -> Option<[u8; 32]> {
        let pairing = self.pairing.lock().unwrap_or_else(PoisonError::into_inner);
        pairing
            .as_ref()
            .filter(|pairing| pairing.key == *key && pairing.until > Instant::now())
            .map(|pairing| pairing.secret)
    }

    /// A device's relay keys changed: every listener names them again.
    pub(crate) fn devices_changed(&self) {
        self.changed.send_modify(|revision| *revision += 1);
    }

    /// Each relay's state, with the latest round trip of those listening,
    /// and its label.
    pub(crate) fn status(&self) -> BTreeMap<String, RelayStatus> {
        let rtt = nearest::millis(&self.rtt.lock().unwrap_or_else(PoisonError::into_inner));
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(endpoint, state)| {
                let mut state = state.clone();
                if let Status::Listening { rtt_ms } = &mut state {
                    *rtt_ms = rtt.get(endpoint).copied();
                }
                let status = RelayStatus {
                    label: self.labels.get(endpoint).cloned().unwrap_or_default(),
                    state,
                };
                (endpoint.clone(), status)
            })
            .collect()
    }

    /// Records `endpoint`'s round trip while it listens; `None` forgets it,
    /// so a relay that dropped this computer is not offered as nearest.
    fn set_rtt(&self, endpoint: &Endpoint, rtt: Option<Duration>) {
        let mut times = self.rtt.lock().unwrap_or_else(PoisonError::into_inner);
        match rtt {
            Some(rtt) => times.insert(endpoint.to_string(), rtt),
            None => times.remove(&endpoint.to_string()),
        };
    }

    fn set_status(&self, endpoint: &Endpoint, status: Status) {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(endpoint.to_string(), status);
    }

    /// Starts a listener for each relay, serving `router` over each
    /// session, until `stopped`.
    pub(crate) fn start(self: &Arc<Self>, served: Served, stopped: watch::Receiver<bool>) {
        for endpoint in &self.endpoints {
            tokio::spawn(listen::run(
                Arc::clone(self),
                endpoint.clone(),
                served.clone(),
                stopped.clone(),
            ));
        }
    }
}

/// What each relay session serves: the daemon's state and router, behind
/// the TLS the phone pins.
#[derive(Clone)]
pub(crate) struct Served {
    pub(crate) router: axum::Router,
    pub(crate) state: crate::web::Shared,
    pub(crate) tls: Arc<rustls::ServerConfig>,
}

impl std::fmt::Debug for Served {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Served").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
