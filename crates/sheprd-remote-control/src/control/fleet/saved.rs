//! A log as a member keeps it on disk: the checkpoint it was pruned to,
//! with that checkpoint's state, then every operation it holds. Reading it
//! checks every operation again, as if it had just arrived, so a file
//! changed on disk computes nothing a signature does not back.

use super::{Body, Error, Log, MAX_OPS, MAX_REMOVALS, MAX_STATE, Op, Result};

const MAGIC: &[u8] = b"sheprd-fleet-log-v1";
/// The largest file read: a full log, a full state, and their lengths.
pub const MAX_SAVED: usize = MAX_STATE + (MAX_OPS + MAX_REMOVALS) * (super::MAX_OP + 4) + 1024;

/// The bytes that keep `log`.
pub fn encode(log: &Log) -> Result<Vec<u8>> {
    let mut bytes = MAGIC.to_vec();
    match log.base() {
        None => bytes.push(0),
        Some((root, state)) => {
            bytes.push(1);
            push(&mut bytes, &root.encode())?;
            push(&mut bytes, &state.encode()?)?;
        }
    }
    let ops: Vec<Vec<u8>> = log.ops().map(Op::encode).collect();
    bytes.extend_from_slice(
        &u32::try_from(ops.len())
            .map_err(|_| Error::Bounds)?
            .to_be_bytes(),
    );
    for op in &ops {
        push(&mut bytes, op)?;
    }
    Ok(bytes)
}

/// The log `bytes` keep, read at `now_ms`. Operations that no longer
/// check out are dropped; a file without the operation that founded the
/// fleet, or the checkpoint it was pruned to, is refused.
pub fn decode(bytes: &[u8], now_ms: u64) -> Result<Log> {
    if bytes.len() > MAX_SAVED {
        return Err(Error::Bounds);
    }
    let mut rest = bytes.strip_prefix(MAGIC).ok_or(Error::Encoding)?;
    let (&flag, after) = rest.split_first().ok_or(Error::Encoding)?;
    rest = after;
    let root = match flag {
        0 => None,
        1 => {
            let root = Op::decode(take(&mut rest, super::MAX_OP)?)?;
            let state = take(&mut rest, MAX_STATE)?;
            Some((root, state.to_vec()))
        }
        _ => return Err(Error::Encoding),
    };
    let count =
        usize::try_from(u32::from_be_bytes(array(&mut rest)?)).map_err(|_| Error::Bounds)?;
    if count > MAX_OPS + MAX_REMOVALS {
        return Err(Error::Bounds);
    }
    let mut ops = (0..count)
        .map(|_| Op::decode(take(&mut rest, super::MAX_OP)?))
        .collect::<Result<Vec<_>>>()?;
    if !rest.is_empty() {
        return Err(Error::Encoding);
    }
    let mut log = match root {
        Some((root, state)) => Log::from_checkpoint(root, &state)?,
        None => {
            let found = ops
                .iter()
                .position(|op| matches!(op.body(), Body::Found(_)))
                .ok_or(Error::Invalid)?;
            Log::found(ops.swap_remove(found))?
        }
    };
    // What no longer checks out is dropped with its reason; the rest
    // stands.
    let _ = log.insert_all(ops, now_ms);
    Ok(log)
}

fn push(bytes: &mut Vec<u8>, item: &[u8]) -> Result<()> {
    bytes.extend_from_slice(
        &u32::try_from(item.len())
            .map_err(|_| Error::Bounds)?
            .to_be_bytes(),
    );
    bytes.extend_from_slice(item);
    Ok(())
}

fn array(rest: &mut &[u8]) -> Result<[u8; 4]> {
    let (head, tail) = rest.split_at_checked(4).ok_or(Error::Encoding)?;
    *rest = tail;
    head.try_into().map_err(|_| Error::Encoding)
}

fn take<'a>(rest: &mut &'a [u8], max: usize) -> Result<&'a [u8]> {
    let length = usize::try_from(u32::from_be_bytes(array(rest)?)).map_err(|_| Error::Bounds)?;
    if length > max {
        return Err(Error::Bounds);
    }
    let (head, tail) = rest.split_at_checked(length).ok_or(Error::Encoding)?;
    *rest = tail;
    Ok(head)
}
