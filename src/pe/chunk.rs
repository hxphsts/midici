//! Chunking Property Data across SysEx messages (M2-101 §8.3).
//!
//! [`ChunkPlan`] decides how a header plus `total` bytes of data split into
//! frames that fit a peer's Receivable Maximum SysEx Message Size (§5.5.3);
//! the header goes in the first chunk only (§8.3.1, §8.3.2). [`Assembler`]
//! reassembles received chunks into caller-provided storage, enforcing
//! sequence (NAK `0x21` when broken, §5.11.2), a size cap, and the 3-second
//! inter-chunk timeout (M2-103 §10.3).

use core::ops::Range;

use crate::error::Error;
use crate::pe::sink::ArrayBuf;
use crate::pe::{CHUNK_OVERHEAD, PeChunk};
use crate::wire::{FRAMING_OVERHEAD, HEADER_LEN};

/// Bytes of a framed PE message that are not header or data:
/// `F0` + 13-byte envelope + 9 chunk bytes + `F7` = 24.
pub const FRAME_OVERHEAD: usize = FRAMING_OVERHEAD + HEADER_LEN + CHUNK_OVERHEAD;

/// Largest data or header length a chunk can declare (14 bits).
pub const MAX_FIELD: usize = 0x3FFF;

/// Default inter-chunk / reply timeout (M2-103 §10.3): 3 seconds.
pub const TIMEOUT_MS: u64 = 3000;

/// How one message's header and data split into chunks.
///
/// # Example
///
/// ```
/// use midici::pe::chunk::ChunkPlan;
///
/// // A peer that accepts 128-byte SysEx: 104 bytes of header plus data per
/// // frame. With a 14-byte header, 300 bytes of data need 4 chunks.
/// let header = br#"{"status":200}"#;
/// let data = [b'x'; 300];
/// let plan = ChunkPlan::new(128, header.len(), data.len())?;
/// assert_eq!(plan.num_chunks(), 4);
/// let first = plan.chunk(1, header, &data, 1);
/// assert_eq!(first.header, header);
/// assert_eq!(first.data.len(), 104 - header.len());
/// assert!(plan.chunk(1, header, &data, 4).is_last());
/// # Ok::<(), midici::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkPlan {
    capacity: usize,
    header_len: usize,
    total: usize,
    num_chunks: u16,
}

impl ChunkPlan {
    /// Plans `total` data bytes after a `header_len`-byte header, in frames of
    /// at most `max_sysex` bytes (`F0`/`F7` included).
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] if the header does not fit one frame or more
    /// than 16383 chunks would be needed.
    pub fn new(max_sysex: u32, header_len: usize, total: usize) -> Result<Self, Error> {
        let max = usize::try_from(max_sysex).unwrap_or(usize::MAX);
        let capacity = max.saturating_sub(FRAME_OVERHEAD).min(MAX_FIELD);
        if capacity == 0 || header_len > capacity {
            return Err(Error::OutOfRange("PE header does not fit one chunk"));
        }
        let first = capacity - header_len;
        let n = if total <= first {
            1
        } else {
            1 + (total - first).div_ceil(capacity)
        };
        let num_chunks = u16::try_from(n)
            .ok()
            .filter(|&n| usize::from(n) <= MAX_FIELD)
            .ok_or(Error::OutOfRange("too many PE chunks"))?;
        Ok(Self {
            capacity,
            header_len,
            total,
            num_chunks,
        })
    }

    /// Number of chunks (at least 1).
    #[must_use]
    pub const fn num_chunks(&self) -> u16 {
        self.num_chunks
    }

    /// Total data length.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.total
    }

    /// The data range carried by chunk `n` (1-based); empty if out of range.
    #[must_use]
    pub fn range(&self, n: u16) -> Range<usize> {
        if n == 0 || n > self.num_chunks {
            return 0..0;
        }
        let first = self.capacity - self.header_len;
        let start = if n == 1 {
            0
        } else {
            first + (usize::from(n) - 2) * self.capacity
        };
        let end = if n == 1 { first } else { start + self.capacity };
        start.min(self.total)..end.min(self.total)
    }

    /// Chunk `n` of a message whose data is all in memory.
    #[must_use]
    pub fn chunk<'a>(
        &self,
        request_id: u8,
        header: &'a [u8],
        data: &'a [u8],
        n: u16,
    ) -> PeChunk<'a> {
        PeChunk {
            request_id,
            header: if n == 1 { header } else { &[] },
            num_chunks: self.num_chunks,
            this_chunk: n,
            data: data.get(self.range(n)).unwrap_or_default(),
        }
    }
}

/// Where an [`Assembler`] keeps bytes.
pub trait Storage {
    /// The bytes stored.
    fn as_slice(&self) -> &[u8];
    /// Empties the storage.
    fn clear(&mut self);
    /// Appends; `false` if they do not fit.
    fn try_extend(&mut self, bytes: &[u8]) -> bool;
}

impl<const N: usize> Storage for ArrayBuf<N> {
    fn as_slice(&self) -> &[u8] {
        Self::as_slice(self)
    }
    fn clear(&mut self) {
        Self::clear(self);
    }
    fn try_extend(&mut self, bytes: &[u8]) -> bool {
        Self::try_extend(self, bytes)
    }
}

#[cfg(feature = "alloc")]
impl Storage for alloc::vec::Vec<u8> {
    fn as_slice(&self) -> &[u8] {
        self
    }
    fn clear(&mut self) {
        Self::clear(self);
    }
    fn try_extend(&mut self, bytes: &[u8]) -> bool {
        self.extend_from_slice(bytes);
        true
    }
}

/// Result of feeding a chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Progress {
    /// More chunks expected.
    Partial,
    /// The message is complete. `usable` is false when the sender ended it
    /// with chunk number 0 (data may be incomplete, §8.3).
    Complete {
        /// Whether the sender vouched for the data.
        usable: bool,
    },
}

/// Why a chunk was refused. The assembler is reset afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AssembleError {
    /// Wrong chunk number (answer NAK `0x21`).
    OutOfSequence,
    /// Exceeds the configured cap or storage (answer status 413).
    TooLarge,
    /// A different Request ID while another message is in progress.
    Busy,
}

/// Reassembles one chunked message: header (first chunk) and data.
///
/// # Example
///
/// ```
/// use midici::pe::chunk::{Assembler, ChunkPlan, Progress};
/// use midici::pe::sink::ArrayBuf;
///
/// let data = [b'a'; 200];
/// let plan = ChunkPlan::new(128, 2, data.len())?;
/// // Fixed storage, no allocator: at most 512 bytes of header plus data.
/// let mut asm = Assembler::new(ArrayBuf::<512>::new(), 512);
/// let mut done = false;
/// for n in 1..=plan.num_chunks() {
///     let progress = asm.push(&plan.chunk(7, b"{}", &data, n), 0);
///     done = progress == Ok(Progress::Complete { usable: true });
/// }
/// assert!(done);
/// assert_eq!(asm.header(), b"{}");
/// assert_eq!(asm.data(), &data[..]);
/// # Ok::<(), midici::Error>(())
/// ```
#[derive(Debug)]
pub struct Assembler<S: Storage> {
    storage: S,
    limit: usize,
    header_len: usize,
    request_id: Option<u8>,
    next: u16,
    num_chunks: u16,
    last_ms: u64,
}

impl<S: Storage> Assembler<S> {
    /// An idle assembler over `storage`, accepting at most `limit` bytes of
    /// header plus data.
    pub const fn new(storage: S, limit: usize) -> Self {
        Self {
            storage,
            limit,
            header_len: 0,
            request_id: None,
            next: 1,
            num_chunks: 0,
            last_ms: 0,
        }
    }

    /// Drops any message in progress.
    pub fn reset(&mut self) {
        self.storage.clear();
        self.header_len = 0;
        self.request_id = None;
        self.next = 1;
        self.num_chunks = 0;
    }

    /// The Request ID of the message in progress.
    #[must_use]
    pub const fn request_id(&self) -> Option<u8> {
        self.request_id
    }

    /// Whether more than `timeout_ms` passed since the last chunk.
    #[must_use]
    pub const fn expired(&self, now_ms: u64, timeout_ms: u64) -> bool {
        self.request_id.is_some() && now_ms.saturating_sub(self.last_ms) > timeout_ms
    }

    /// The header of the assembled (or in-progress) message.
    #[must_use]
    pub fn header(&self) -> &[u8] {
        self.storage
            .as_slice()
            .get(..self.header_len)
            .unwrap_or_default()
    }

    /// The data assembled so far.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        self.storage
            .as_slice()
            .get(self.header_len..)
            .unwrap_or_default()
    }

    /// Takes the storage back.
    pub fn into_storage(self) -> S {
        self.storage
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), AssembleError> {
        if self.storage.as_slice().len().saturating_add(bytes.len()) > self.limit
            || !self.storage.try_extend(bytes)
        {
            self.reset();
            return Err(AssembleError::TooLarge);
        }
        Ok(())
    }

    /// Feeds one chunk.
    ///
    /// # Errors
    ///
    /// See [`AssembleError`]. After an error other than
    /// [`AssembleError::Busy`] the message in progress is dropped.
    pub fn push(&mut self, chunk: &PeChunk<'_>, now_ms: u64) -> Result<Progress, AssembleError> {
        match self.request_id {
            None => {
                if chunk.this_chunk > 1 {
                    return Err(AssembleError::OutOfSequence);
                }
                self.storage.clear();
                self.request_id = Some(chunk.request_id);
                self.header_len = 0;
                self.append(chunk.header)?;
                self.header_len = chunk.header.len();
            }
            Some(id) if id != chunk.request_id => return Err(AssembleError::Busy),
            Some(_) => {
                if chunk.this_chunk != 0 && chunk.this_chunk != self.next {
                    self.reset();
                    return Err(AssembleError::OutOfSequence);
                }
            }
        }
        self.append(chunk.data)?;
        self.last_ms = now_ms;
        if chunk.num_chunks != 0 {
            self.num_chunks = chunk.num_chunks;
        }
        let done = if chunk.this_chunk == 0 {
            Some(false)
        } else if self.num_chunks != 0 && chunk.this_chunk >= self.num_chunks {
            Some(true)
        } else {
            None
        };
        if let Some(usable) = done {
            self.request_id = None;
            self.next = 1;
            return Ok(Progress::Complete { usable });
        }
        self.next = chunk.this_chunk.saturating_add(1);
        Ok(Progress::Partial)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_at_128_bytes() {
        // 128 − 24 = 104 bytes per frame; a 20-byte header leaves 84 in the
        // first chunk.
        let p = ChunkPlan::new(128, 20, 300).unwrap();
        assert_eq!(p.num_chunks(), 4); // 84 + 104 + 104 + 8
        assert_eq!(p.range(1), 0..84);
        assert_eq!(p.range(2), 84..188);
        assert_eq!(p.range(4), 292..300);
        assert_eq!(p.range(5), 0..0);
        assert_eq!(ChunkPlan::new(128, 20, 0).unwrap().num_chunks(), 1);
        assert!(ChunkPlan::new(128, 105, 0).is_err());
        assert!(ChunkPlan::new(20, 0, 0).is_err());
    }

    #[test]
    fn assembles_in_order_and_rejects_gaps() {
        let data: [u8; 300] = core::array::from_fn(|i| (i % 128) as u8);
        let plan = ChunkPlan::new(128, 2, data.len()).unwrap();
        let mut a = Assembler::new(ArrayBuf::<512>::new(), 512);
        for n in 1..=plan.num_chunks() {
            let c = plan.chunk(7, b"{}", &data, n);
            let p = a.push(&c, 0).unwrap();
            assert_eq!(
                p == (Progress::Complete { usable: true }),
                n == plan.num_chunks()
            );
        }
        assert_eq!(a.header(), b"{}");
        assert_eq!(a.data(), &data[..]);

        a.push(&plan.chunk(8, b"{}", &data, 1), 0).unwrap();
        assert_eq!(
            a.push(&plan.chunk(9, b"", &data, 2), 0),
            Err(AssembleError::Busy)
        );
        assert_eq!(
            a.push(&plan.chunk(8, b"", &data, 3), 0),
            Err(AssembleError::OutOfSequence)
        );
        let mut small = Assembler::new(ArrayBuf::<64>::new(), 64);
        assert_eq!(
            small.push(&plan.chunk(1, b"{}", &data, 1), 0),
            Err(AssembleError::TooLarge)
        );
    }

    #[test]
    fn unknown_count_and_abort() {
        let mut a = Assembler::new(ArrayBuf::<64>::new(), 64);
        let c = |n, total, d: &'static [u8]| PeChunk {
            request_id: 1,
            header: if n == 1 { b"{}" } else { b"" },
            num_chunks: total,
            this_chunk: n,
            data: d,
        };
        assert_eq!(a.push(&c(1, 0, b"ab"), 0), Ok(Progress::Partial));
        assert_eq!(a.push(&c(2, 0, b"cd"), 10), Ok(Progress::Partial));
        assert!(a.expired(4000, TIMEOUT_MS));
        assert_eq!(
            a.push(&c(3, 3, b"e"), 20),
            Ok(Progress::Complete { usable: true })
        );
        assert_eq!(a.data(), b"abcde");
        a.push(&c(1, 5, b"x"), 0).unwrap();
        assert_eq!(
            a.push(&c(0, 5, b""), 0),
            Ok(Progress::Complete { usable: false })
        );
    }
}
