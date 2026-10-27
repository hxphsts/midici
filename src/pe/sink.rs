//! Byte sinks: where JSON writers and encoders put their output.
//!
//! A [`Sink`] never fails mid-write. Fixed-size sinks record overflow and
//! report it when finished, so writers stay simple and panic-free.

use crate::error::Error;

/// Something that accepts bytes.
pub trait Sink {
    /// Appends `bytes`.
    fn put(&mut self, bytes: &[u8]);
}

impl<S: Sink + ?Sized> Sink for &mut S {
    fn put(&mut self, bytes: &[u8]) {
        (**self).put(bytes);
    }
}

/// Counts bytes without storing them (first pass of two-pass rendering).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counter(pub usize);

impl Sink for Counter {
    fn put(&mut self, bytes: &[u8]) {
        self.0 = self.0.saturating_add(bytes.len());
    }
}

/// Writes into a caller buffer; remembers overflow.
#[derive(Debug)]
pub struct SliceSink<'a> {
    buf: &'a mut [u8],
    len: usize,
    overflow: bool,
}

impl<'a> SliceSink<'a> {
    /// An empty sink over `buf`.
    pub const fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            overflow: false,
        }
    }

    /// Bytes written so far.
    #[must_use]
    pub fn written(&self) -> &[u8] {
        self.buf.get(..self.len).unwrap_or_default()
    }

    /// The number of bytes written, or [`Error::BufferTooSmall`] if any
    /// write did not fit.
    ///
    /// # Errors
    ///
    /// [`Error::BufferTooSmall`] on overflow.
    pub const fn finish(&self) -> Result<usize, Error> {
        if self.overflow {
            Err(Error::BufferTooSmall)
        } else {
            Ok(self.len)
        }
    }
}

impl Sink for SliceSink<'_> {
    fn put(&mut self, bytes: &[u8]) {
        let end = self.len.saturating_add(bytes.len());
        match self.buf.get_mut(self.len..end) {
            Some(dst) => {
                dst.copy_from_slice(bytes);
                self.len = end;
            }
            None => self.overflow = true,
        }
    }
}

/// Keeps only bytes `start..start + buf.len()` of a stream: renders one
/// chunk of a document without buffering the whole document.
#[derive(Debug)]
pub struct Window<'a> {
    buf: &'a mut [u8],
    start: usize,
    pos: usize,
    len: usize,
}

impl<'a> Window<'a> {
    /// A window over stream offsets `start..start + buf.len()`.
    pub const fn new(start: usize, buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            start,
            pos: 0,
            len: 0,
        }
    }

    /// Bytes captured.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing was captured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Total stream length seen.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.pos
    }
}

impl Sink for Window<'_> {
    fn put(&mut self, bytes: &[u8]) {
        let begin = self.pos;
        self.pos = self.pos.saturating_add(bytes.len());
        let end_window = self.start.saturating_add(self.buf.len());
        let lo = begin.max(self.start);
        let hi = self.pos.min(end_window);
        if lo < hi
            && let (Some(src), Some(dst)) = (
                bytes.get(lo - begin..hi - begin),
                self.buf.get_mut(lo - self.start..hi - self.start),
            )
        {
            dst.copy_from_slice(src);
            self.len = self.len.max(hi - self.start);
        }
    }
}

#[cfg(feature = "alloc")]
impl Sink for alloc::vec::Vec<u8> {
    fn put(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

/// A fixed-capacity byte buffer with a length: the no-alloc storage used by
/// reassemblers and engines.
#[derive(Clone)]
pub struct ArrayBuf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Default for ArrayBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> ArrayBuf<N> {
    /// Empty.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    /// The contents.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or_default()
    }

    /// The contents, mutably.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.bytes.get_mut(..self.len).unwrap_or_default()
    }

    /// Shortens to `len` bytes (no-op if already shorter).
    pub const fn truncate(&mut self, len: usize) {
        if len < self.len {
            self.len = len;
        }
    }

    /// Clears the contents.
    pub const fn clear(&mut self) {
        self.len = 0;
    }

    /// Appends `bytes`; `false` (and no change) if they do not fit.
    pub fn try_extend(&mut self, bytes: &[u8]) -> bool {
        let end = self.len.saturating_add(bytes.len());
        match self.bytes.get_mut(self.len..end) {
            Some(dst) => {
                dst.copy_from_slice(bytes);
                self.len = end;
                true
            }
            None => false,
        }
    }

    /// Length.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Capacity.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        N
    }
}

impl<const N: usize> core::fmt::Debug for ArrayBuf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ArrayBuf({:02X?})", self.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_captures_a_slice_of_the_stream() {
        let mut buf = [0u8; 4];
        let mut w = Window::new(3, &mut buf);
        w.put(b"ab");
        w.put(b"cdef");
        w.put(b"gh");
        assert_eq!(w.len(), 4);
        assert_eq!(w.total(), 8);
        assert_eq!(&buf, b"defg");
    }

    #[test]
    fn slice_sink_overflow() {
        let mut buf = [0u8; 3];
        let mut s = SliceSink::new(&mut buf);
        s.put(b"ab");
        s.put(b"cd");
        assert_eq!(s.finish(), Err(Error::BufferTooSmall));
    }
}
