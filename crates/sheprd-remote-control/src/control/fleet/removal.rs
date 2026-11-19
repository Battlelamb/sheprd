//! Removals passed between daemons, and to agents, inside the relay
//! tunnel (`docs/relay-plan.md`, Removals between daemons), so a removal
//! reaches every member while every Sheprd window is shut.
//!
//! The wire is `sheprd_relay_agent::removal`'s, which plain hosts' agents
//! speak too: a fleet member's tunnel session starts with one purpose
//! byte, SSH or a removal exchange. The dialer then sends
//! [`Message::Removal`]; the listener answers [`Message::Want`] for
//! parents it lacks, at most [`MAX_ROUNDS`] times, then
//! [`Message::Held`] with the removals it now holds. Each message is one
//! frame: its length as a big-endian `u32`, then its bytes.

use super::{DOMAIN, MemberId, OpId, State};
use p256::ecdsa::{Signature, SigningKey, signature::Signer as _, signature::Verifier as _};

pub use sheprd_relay_agent::removal::{
    CONCURRENT, MAX_FRAME, MAX_OPS, MAX_ROUNDS, PURPOSE_REMOVAL, PURPOSE_SSH, SESSION,
};

/// A session proof's length: a P-256 signature, `r` then `s`.
pub const PROOF_LEN: usize = 64;

const SESSION_PROOF: &[u8] = b"session";

/// One message of a removal exchange.
pub type Message = sheprd_relay_agent::removal::Message<OpId>;

// The agent reads operations with the log's own bound.
const _: () = assert!(sheprd_relay_agent::removal::MAX_OP == super::MAX_OP);

fn proof_message(hash: &[u8; 32]) -> Vec<u8> {
    [DOMAIN, SESSION_PROOF, hash].concat()
}

/// The proof a member sends after the purpose byte: its fleet `key`
/// signing the session's handshake `hash`.
#[must_use]
pub fn prove(key: &SigningKey, hash: &[u8; 32]) -> [u8; PROOF_LEN] {
    let signature: Signature = key.sign(&proof_message(hash));
    signature.to_bytes().into()
}

/// Whether `proof` is `member`'s fleet key, live in `state`, signing the
/// session's handshake `hash`.
#[must_use]
pub fn proves(state: &State, member: &MemberId, hash: &[u8; 32], proof: &[u8; PROOF_LEN]) -> bool {
    let Some(key) = state
        .members
        .get(member)
        .and_then(|member| member.bundle.verifying_key().ok())
    else {
        return false;
    };
    Signature::from_slice(proof)
        .is_ok_and(|signature| key.verify(&proof_message(hash), &signature).is_ok())
}
