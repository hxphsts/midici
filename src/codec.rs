//! Bounds-checked 7-bit field reader and writer shared by every message.
//!
//! Multi-byte numeric fields in MIDI-CI are sequences of 7-bit bytes, least
//! significant first (M2-101 §5.2.1, note under Table 5).

use crate::error::Error;
use crate::muid::Muid;

/// Reads fields from a message body. Every read is bounds-checked.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub const fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    /// Bytes not yet read.
    pub const fn rest(&self) -> &'a [u8] {
        self.buf
    }

    #[cfg(test)]
    pub const fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub const fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if n > self.buf.len() {
            return Err(Error::Truncated);
        }
        let (head, tail) = self.buf.split_at(n);
        self.buf = tail;
        Ok(head)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    /// One 7-bit byte.
    pub fn u7(&mut self) -> Result<u8, Error> {
        let [b] = self.array::<1>()?;
        if b & 0x80 != 0 {
            return Err(Error::InvalidField("7-bit data byte"));
        }
        Ok(b)
    }

    /// `n` 7-bit bytes, LSB first, as one integer (`n <= 4`).
    fn uint(&mut self, n: usize) -> Result<u32, Error> {
        let mut v = 0u32;
        for i in 0..n {
            v |= u32::from(self.u7()?) << (7 * i);
        }
        Ok(v)
    }

    /// Two 7-bit bytes, LSB first.
    pub fn u14(&mut self) -> Result<u16, Error> {
        let v = self.uint(2)?;
        u16::try_from(v).map_err(|_| Error::InvalidField("14-bit field"))
    }

    /// Four 7-bit bytes, LSB first.
    pub fn u28(&mut self) -> Result<u32, Error> {
        self.uint(4)
    }

    pub fn muid(&mut self) -> Result<Muid, Error> {
        Muid::from_bytes(self.array()?)
    }
}

/// Writes a message into a caller-provided buffer.
#[derive(Debug)]
pub struct Writer<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> Writer<'a> {
    pub const fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0 }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    /// Any byte, including status bytes (`F0`, `F7`).
    pub fn raw(&mut self, b: u8) -> Result<(), Error> {
        let slot = self.buf.get_mut(self.len).ok_or(Error::BufferTooSmall)?;
        *slot = b;
        self.len += 1;
        Ok(())
    }

    /// One 7-bit data byte.
    pub fn u7(&mut self, b: u8, what: &'static str) -> Result<(), Error> {
        if b & 0x80 != 0 {
            return Err(Error::OutOfRange(what));
        }
        self.raw(b)
    }

    /// 7-bit data bytes.
    pub fn bytes(&mut self, bytes: &[u8], what: &'static str) -> Result<(), Error> {
        for &b in bytes {
            self.u7(b, what)?;
        }
        Ok(())
    }

    fn uint(&mut self, v: u32, n: u32, what: &'static str) -> Result<(), Error> {
        if v >> (7 * n) != 0 {
            return Err(Error::OutOfRange(what));
        }
        for i in 0..n {
            // Masked to 7 bits, so the cast cannot truncate.
            #[allow(clippy::cast_possible_truncation)]
            self.raw(((v >> (7 * i)) & 0x7F) as u8)?;
        }
        Ok(())
    }

    /// Two 7-bit bytes, LSB first (`v <= 0x3FFF`).
    pub fn u14(&mut self, v: u16, what: &'static str) -> Result<(), Error> {
        self.uint(u32::from(v), 2, what)
    }

    /// Four 7-bit bytes, LSB first (`v <= 0x0FFF_FFFF`).
    pub fn u28(&mut self, v: u32, what: &'static str) -> Result<(), Error> {
        self.uint(v, 4, what)
    }

    pub fn muid(&mut self, m: Muid) -> Result<(), Error> {
        for b in m.to_bytes() {
            self.raw(b)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u14_and_u28_are_lsb_first() {
        let mut buf = [0u8; 6];
        let mut w = Writer::new(&mut buf);
        w.u14(0x3FFF, "x").unwrap();
        w.u28(0x0000_0200, "y").unwrap();
        assert_eq!(buf, [0x7F, 0x7F, 0x00, 0x04, 0x00, 0x00]);
        let mut r = Reader::new(&buf);
        assert_eq!(r.u14(), Ok(0x3FFF));
        assert_eq!(r.u28(), Ok(512));
        assert!(r.is_empty());
    }

    #[test]
    fn writer_rejects_oversize_values_and_full_buffers() {
        let mut buf = [0u8; 1];
        let mut w = Writer::new(&mut buf);
        assert_eq!(w.u14(0x4000, "x"), Err(Error::OutOfRange("x")));
        assert_eq!(w.u7(0x80, "x"), Err(Error::OutOfRange("x")));
        w.u7(1, "x").unwrap();
        assert_eq!(w.u7(1, "x"), Err(Error::BufferTooSmall));
    }

    #[test]
    fn reader_reports_truncation() {
        let mut r = Reader::new(&[1]);
        assert_eq!(r.u14(), Err(Error::Truncated));
    }
}
