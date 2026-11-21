//! A push whose content is sealed to one phone: the only kind
//! [`crate::RelayClient`] takes, so sending plain text through the push
//! relay is a compile error rather than something a review has to catch
//! (`docs/apns-relay-plan.md`, Live Activity privacy).
//!
//! A [`SealedPush`] is made only by [`SealedPush::build`], which hands the
//! code building the payload a [`Sealer`] for one phone's key and refuses a
//! push that sealed nothing.

use crate::{Error, Push, Result, seal};
use p256::PublicKey;
use std::cell::Cell;

/// Seals to one phone while a [`SealedPush`] is built, and notes that it did.
pub struct Sealer<'a> {
    device: &'a PublicKey,
    info: &'a [u8],
    used: Cell<bool>,
}

impl Sealer<'_> {
    /// `plaintext` sealed to the phone, under the push's `info`.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        self.used.set(true);
        seal::seal(self.device, plaintext, self.info)
    }
}

/// A push built around content sealed to one phone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SealedPush(Push);

impl SealedPush {
    /// The push `build` makes with a [`Sealer`] for `device` under `info`.
    /// A push that sealed nothing is refused with [`Error::NotSealed`].
    pub fn build<E: From<Error>>(
        device: &PublicKey,
        info: &[u8],
        build: impl FnOnce(&Sealer<'_>) -> Result<Push, E>,
    ) -> Result<Self, E> {
        let sealer = Sealer {
            device,
            info,
            used: Cell::new(false),
        };
        let push = build(&sealer)?;
        if !sealer.used.get() {
            return Err(Error::NotSealed.into());
        }
        Ok(Self(push))
    }

    pub fn push(&self) -> &Push {
        &self.0
    }
}
