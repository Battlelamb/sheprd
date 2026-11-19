//! Removals passed between daemons inside the relay tunnel
//! (`docs/relay-plan.md`, Removals between daemons), so a removal reaches
//! every member while every Sheprd window is shut.
//!
//! A fleet member's tunnel session starts with one purpose byte: SSH to
//! splice to the host's SSH server, or a removal exchange. Then
//! [`PROOF_LEN`] bytes: the member's fleet key signing the tunnel's
//! handshake hash ([`prove`]). The handshake proves only that the dialer
//! holds a tunnel key, which is public and any member's bundle may name;
//! the proof shows the device in this very session holds the member's
//! fleet key too, so a member naming another device's tunnel key never
//! speaks for it. For a removal exchange, the dialer then
//! sends [`Message::Removal`]; the listener answers [`Message::Want`] for
//! parents it lacks, at most [`MAX_ROUNDS`] times, then
//! [`Message::Held`] with the removals it now holds. Each message is one
//! frame: its length as a big-endian `u32`, then its bytes.

use super::{DOMAIN, Error, MAX_OP, MemberId, OpId, Result, State};
use p256::ecdsa::{Signature, SigningKey, signature::Signer as _, signature::Verifier as _};

/// The session carries SSH to the host's SSH server.
pub const PURPOSE_SSH: u8 = 1;
/// The session carries one removal exchange.
pub const PURPOSE_REMOVAL: u8 = 2;
/// Operations one message carries.
pub const MAX_OPS: usize = 64;
/// The largest frame: 64 operations and their lengths.
pub const MAX_FRAME: usize = MAX_OPS * (MAX_OP + 2) + 3;
/// `Want` rounds before the listener answers with what it holds.
pub const MAX_ROUNDS: usize = 4;

/// A session proof's length: a P-256 signature, `r` then `s`.
pub const PROOF_LEN: usize = 64;

const SESSION_PROOF: &[u8] = b"session";
const REMOVAL: u8 = 1;
const WANT: u8 = 2;
const HELD: u8 = 3;

/// One message of a removal exchange.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Message {
    /// Removals, and any ancestors asked for, as encoded operations.
    Removal(Vec<Vec<u8>>),
    /// Parents the listener lacks.
    Want(Vec<OpId>),
    /// The removals the listener holds now: the dialer owes it no more.
    Held(Vec<OpId>),
}

impl Message {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let (kind, count) = match self {
            Self::Removal(ops) => (REMOVAL, ops.len()),
            Self::Want(ids) => (WANT, ids.len()),
            Self::Held(ids) => (HELD, ids.len()),
        };
        if count > MAX_OPS {
            return Err(Error::Bounds);
        }
        let mut bytes = vec![kind];
        bytes.extend_from_slice(
            &u16::try_from(count)
                .map_err(|_| Error::Bounds)?
                .to_be_bytes(),
        );
        match self {
            Self::Removal(ops) => {
                for op in ops {
                    if op.len() > MAX_OP {
                        return Err(Error::Bounds);
                    }
                    bytes.extend_from_slice(
                        &u16::try_from(op.len())
                            .map_err(|_| Error::Bounds)?
                            .to_be_bytes(),
                    );
                    bytes.extend_from_slice(op);
                }
            }
            Self::Want(ids) | Self::Held(ids) => {
                ids.iter().for_each(|id| bytes.extend_from_slice(&id.0))
            }
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_FRAME {
            return Err(Error::Bounds);
        }
        let (&kind, rest) = bytes.split_first().ok_or(Error::Encoding)?;
        let (count, mut rest) = rest.split_at_checked(2).ok_or(Error::Encoding)?;
        let count = usize::from(u16::from_be_bytes([count[0], count[1]]));
        if count > MAX_OPS {
            return Err(Error::Bounds);
        }
        let message = match kind {
            REMOVAL => {
                let mut ops = Vec::with_capacity(count);
                for _ in 0..count {
                    let (length, tail) = rest.split_at_checked(2).ok_or(Error::Encoding)?;
                    let length = usize::from(u16::from_be_bytes([length[0], length[1]]));
                    if length > MAX_OP {
                        return Err(Error::Bounds);
                    }
                    let (op, tail) = tail.split_at_checked(length).ok_or(Error::Encoding)?;
                    ops.push(op.to_vec());
                    rest = tail;
                }
                Self::Removal(ops)
            }
            WANT | HELD => {
                let mut ids = Vec::with_capacity(count);
                for _ in 0..count {
                    let (id, tail) = rest.split_at_checked(32).ok_or(Error::Encoding)?;
                    ids.push(OpId(id.try_into().map_err(|_| Error::Encoding)?));
                    rest = tail;
                }
                if kind == WANT {
                    Self::Want(ids)
                } else {
                    Self::Held(ids)
                }
            }
            _ => return Err(Error::Encoding),
        };
        if !rest.is_empty() {
            return Err(Error::Encoding);
        }
        Ok(message)
    }
}

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
