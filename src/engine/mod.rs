//! Sans-io MIDI-CI state machines.
//!
//! Feed received SysEx bytes and a millisecond timestamp in; drain outbound
//! frames (and, for the Initiator, events) out. No clocks, no I/O, no
//! threads: the caller owns the transport and the time source.
//!
//! * [`responder::Responder`]: the device side. `no_std`, no allocator,
//!   capacities as const generics. What firmware embeds.
//! * [`initiator::Initiator`] (feature `alloc`): the host side: discovery,
//!   Property Exchange with request IDs, chunk reassembly, timeouts,
//!   subscriptions, and [`initiator::Foundation`], which runs the whole
//!   foundational sequence.

#[cfg(feature = "alloc")]
pub mod initiator;
pub mod responder;

use crate::error::Error;

/// What an engine did with an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handled {
    /// Not a MIDI-CI message (e.g. another SysEx protocol on the port).
    NotCi,
    /// A CI message for someone else, or one this role ignores.
    Ignored,
    /// Processed (replies may be queued).
    Processed,
    /// Malformed; a NAK was queued if the message was addressed to us.
    Malformed(Error),
}

/// A queue of whole frames in one fixed byte buffer: `[len_lo, len_hi,
/// frame…]` records. No allocator.
#[derive(Clone)]
pub(crate) struct FrameQueue<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FrameQueue<N> {
    pub(crate) const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) const fn clear(&mut self) {
        self.len = 0;
    }

    /// Appends a frame produced by `f` into at most `max` bytes.
    pub(crate) fn push_with(
        &mut self,
        max: usize,
        f: impl FnOnce(&mut [u8]) -> Result<usize, Error>,
    ) -> Result<(), Error> {
        let start = self.len + 2;
        let end = start.saturating_add(max).min(N);
        let slot = self.buf.get_mut(start..end).ok_or(Error::BufferTooSmall)?;
        let n = f(slot)?;
        let len = u16::try_from(n).map_err(|_| Error::BufferTooSmall)?;
        let [lo, hi] = len.to_le_bytes();
        if let Some(h) = self.buf.get_mut(self.len..self.len + 2) {
            h.copy_from_slice(&[lo, hi]);
        }
        self.len = start + n;
        Ok(())
    }

    /// Pops the oldest frame into `out`, returning its length. A frame that
    /// does not fit `out` is dropped and reported as an error.
    pub(crate) fn pop(&mut self, out: &mut [u8]) -> Option<Result<usize, Error>> {
        if self.len < 2 {
            return None;
        }
        let n = usize::from(u16::from_le_bytes([
            self.buf.first().copied().unwrap_or(0),
            self.buf.get(1).copied().unwrap_or(0),
        ]));
        let frame_end = (2 + n).min(self.len);
        let result = match (self.buf.get(2..frame_end), out.get_mut(..n)) {
            (Some(src), Some(dst)) if src.len() == n => {
                dst.copy_from_slice(src);
                Ok(n)
            }
            _ => Err(Error::BufferTooSmall),
        };
        self.buf.copy_within(frame_end..self.len, 0);
        self.len -= frame_end;
        Some(result)
    }
}
