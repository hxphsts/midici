//! Mcoded7: 8-bit data as 7-bit SysEx bytes (M2-103 §4.3.1).
//!
//! Each group of up to seven input bytes becomes one byte of their high
//! bits followed by their low seven bits. The high bit of input byte `i`
//! goes to bit `6 - i` of the first byte, so a short final group of `n`
//! bytes keeps its high bits in the top `n` positions (`0ABC0000`).
//! (This is the reverse of the bit order some manufacturer formats use.)

use crate::error::Error;
use crate::pe::sink::Sink;

/// Encoded length of `n` input bytes.
#[must_use]
pub const fn encoded_len(n: usize) -> usize {
    let r = n % 7;
    n / 7 * 8 + if r == 0 { 0 } else { r + 1 }
}

/// Decoded length of `n` encoded bytes, or `None` if `n` cannot be a valid
/// encoding (a trailing group of one byte).
#[must_use]
pub const fn decoded_len(n: usize) -> Option<usize> {
    let r = n % 8;
    if r == 1 {
        None
    } else {
        Some(n / 8 * 7 + if r == 0 { 0 } else { r - 1 })
    }
}

fn encode_group(group: &[u8], out: &mut impl Sink) {
    let mut high = 0u8;
    for (i, b) in group.iter().enumerate() {
        high |= (b >> 7) << (6 - i);
    }
    out.put(&[high]);
    for b in group {
        out.put(&[b & 0x7F]);
    }
}

/// Encodes `src` into `out`.
pub fn encode_to(src: &[u8], out: &mut impl Sink) {
    for group in src.chunks(7) {
        encode_group(group, out);
    }
}

/// Encodes `src` into `dst`, returning the length.
///
/// # Errors
///
/// [`Error::BufferTooSmall`] if `dst` is shorter than [`encoded_len`].
pub fn encode(src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    let mut sink = crate::pe::sink::SliceSink::new(dst);
    encode_to(src, &mut sink);
    sink.finish()
}

/// Decodes `src` into `dst`, returning the length.
///
/// # Errors
///
/// [`Error::InvalidField`] for bytes with the high bit set or a dangling
/// one-byte group; [`Error::BufferTooSmall`] if `dst` is too short.
pub fn decode(src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    let mut sink = crate::pe::sink::SliceSink::new(dst);
    let mut d = Decoder::new();
    d.feed(src, &mut sink)?;
    d.finish()?;
    sink.finish()
}

/// Decodes `buf` in place (the output is never longer than the input),
/// returning the decoded length.
///
/// # Errors
///
/// As [`decode`].
pub fn decode_in_place(buf: &mut [u8]) -> Result<usize, Error> {
    let mut out = 0;
    let mut i = 0;
    while i < buf.len() {
        let end = (i + 8).min(buf.len());
        let high = buf.get(i).copied().unwrap_or(0);
        if end - i == 1 {
            return Err(Error::InvalidField("Mcoded7 trailing group"));
        }
        for j in i..end {
            let b = buf.get(j).copied().unwrap_or(0);
            if b & 0x80 != 0 {
                return Err(Error::InvalidField("Mcoded7 byte"));
            }
            if j > i {
                let bit = (high >> (7 - (j - i))) & 1;
                if let Some(slot) = buf.get_mut(out) {
                    *slot = b | (bit << 7);
                }
                out += 1;
            }
        }
        i = end;
    }
    Ok(out)
}

/// Encodes into a `Vec`.
#[cfg(feature = "alloc")]
#[must_use]
pub fn encode_vec(src: &[u8]) -> alloc::vec::Vec<u8> {
    let mut v = alloc::vec::Vec::with_capacity(encoded_len(src.len()));
    encode_to(src, &mut v);
    v
}

/// Decodes into a `Vec`.
///
/// # Errors
///
/// As [`decode`].
#[cfg(feature = "alloc")]
pub fn decode_vec(src: &[u8]) -> Result<alloc::vec::Vec<u8>, Error> {
    let mut v = alloc::vec::Vec::with_capacity(src.len());
    let mut d = Decoder::new();
    d.feed(src, &mut v)?;
    d.finish()?;
    Ok(v)
}

/// A streaming encoder in front of a sink: feed any number of byte runs,
/// then [`Encoder::finish`].
#[derive(Debug)]
pub struct Encoder<S: Sink> {
    out: S,
    group: [u8; 7],
    len: usize,
}

impl<S: Sink> Encoder<S> {
    /// An encoder writing to `out`.
    pub const fn new(out: S) -> Self {
        Self {
            out,
            group: [0; 7],
            len: 0,
        }
    }

    /// Flushes a partial group and returns the sink.
    pub fn finish(mut self) -> S {
        if let Some(g) = self.group.get(..self.len)
            && !g.is_empty()
        {
            encode_group(g, &mut self.out);
        }
        self.out
    }
}

impl<S: Sink> Sink for Encoder<S> {
    fn put(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if let Some(slot) = self.group.get_mut(self.len) {
                *slot = b;
                self.len += 1;
            }
            if self.len == 7 {
                encode_group(&self.group, &mut self.out);
                self.len = 0;
            }
        }
    }
}

/// A streaming decoder: feed encoded bytes as they arrive (e.g. chunk by
/// chunk), then [`Decoder::finish`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Decoder {
    high: u8,
    /// Position within the current 8-byte group (0 = expecting high bits).
    pos: u8,
}

impl Decoder {
    /// A fresh decoder.
    #[must_use]
    pub const fn new() -> Self {
        Self { high: 0, pos: 0 }
    }

    /// Decodes `src` into `out`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for a byte with the high bit set.
    pub fn feed(&mut self, src: &[u8], out: &mut impl Sink) -> Result<(), Error> {
        for &b in src {
            if b & 0x80 != 0 {
                return Err(Error::InvalidField("Mcoded7 byte"));
            }
            if self.pos == 0 {
                self.high = b;
                self.pos = 1;
            } else {
                let bit = (self.high >> (7 - self.pos)) & 1;
                out.put(&[b | (bit << 7)]);
                self.pos = if self.pos == 7 { 0 } else { self.pos + 1 };
            }
        }
        Ok(())
    }

    /// Checks that the input did not end on a lone high-bits byte.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] if it did.
    pub const fn finish(self) -> Result<(), Error> {
        if self.pos == 1 {
            Err(Error::InvalidField("Mcoded7 trailing group"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M2-103 §4.3.1: `AAAAaaaa BBBBbbbb CCCCcccc` → `0ABC0000 0AAAaaaa
    /// 0BBBbbbb 0CCCcccc`.
    #[test]
    fn short_group_layout() {
        let mut out = [0u8; 4];
        let n = encode(&[0x81, 0x02, 0xFF], &mut out).unwrap();
        assert_eq!(&out[..n], &[0b0101_0000, 0x01, 0x02, 0x7F]);
    }

    #[test]
    fn full_group_layout() {
        let src = [0x80, 0, 0, 0, 0, 0, 0xFF];
        let mut out = [0u8; 8];
        encode(&src, &mut out).unwrap();
        assert_eq!(out, [0b0100_0001, 0, 0, 0, 0, 0, 0, 0x7F]);
        let mut back = [0u8; 7];
        assert_eq!(decode(&out, &mut back), Ok(7));
        assert_eq!(back, src);
    }

    #[test]
    fn in_place_matches_buffered() {
        let src: [u8; 20] = core::array::from_fn(|i| (i * 51 + 3) as u8);
        let mut enc = [0u8; 24];
        let n = encode(&src, &mut enc).unwrap();
        let m = decode_in_place(&mut enc[..n]).unwrap();
        assert_eq!(&enc[..m], &src);
    }

    #[test]
    fn lengths() {
        assert_eq!(encoded_len(0), 0);
        assert_eq!(encoded_len(1), 2);
        assert_eq!(encoded_len(7), 8);
        assert_eq!(encoded_len(8), 10);
        assert_eq!(decoded_len(10), Some(8));
        assert_eq!(decoded_len(9), None);
    }

    #[test]
    fn streaming_matches_buffered_across_any_split() {
        let src: [u8; 23] = core::array::from_fn(|i| (i * 37 + 200) as u8);
        let mut whole = [0u8; 32];
        let n = encode(&src, &mut whole).unwrap();
        for split in 0..=n {
            let mut out = [0u8; 23];
            let mut sink = crate::pe::sink::SliceSink::new(&mut out);
            let mut d = Decoder::new();
            d.feed(&whole[..split], &mut sink).unwrap();
            d.feed(&whole[split..n], &mut sink).unwrap();
            d.finish().unwrap();
            assert_eq!(sink.finish(), Ok(23));
            assert_eq!(out, src);
        }
        let mut buf = [0u8; 32];
        let mut enc = Encoder::new(crate::pe::sink::SliceSink::new(&mut buf));
        enc.put(&src[..5]);
        enc.put(&src[5..]);
        assert_eq!(enc.finish().finish(), Ok(n));
        assert_eq!(buf[..n], whole[..n]);
    }

    #[test]
    fn rejects_bad_input() {
        let mut out = [0u8; 8];
        assert!(decode(&[0x80], &mut out).is_err());
        assert!(decode(&[0x00], &mut out).is_err());
        assert!(decode(&[0, 1, 2, 3, 4, 5, 6, 7, 0], &mut out).is_err());
    }
}
