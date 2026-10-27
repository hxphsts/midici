//! Property Exchange (M2-101 §8; M2-103 Common Rules; M2-105/M2-111/M2-117
//! resources).
//!
//! * [`PeCapabilities`] and [`PeChunk`]: the wire bodies of the PE messages.
//! * [`mcoded7`]: 8-bit ↔ 7-bit encoding (M2-103 §4.3.1).
//! * [`json`]: a no-alloc JSON pull parser and writer.
//! * [`header`]: typed Header Data (M2-103 §5).
//! * [`status`]: reply and Notify status codes (M2-103 §5.4.1, §10.1.1).
//! * [`chunk`]: splitting into chunks and reassembling them (M2-101 §8.3).
//! * [`resource`]: typed foundational resources.

pub mod chunk;
pub mod header;
pub mod json;
pub mod mcoded7;
pub mod resource;
pub mod sink;
pub mod status;
#[cfg(feature = "zlib")]
pub mod zlib;

use crate::codec::{Reader, Writer};
use crate::error::Error;
use crate::mgmt::Codec;
use crate::wire::CiVersion;

/// Fixed bytes in a PE chunk besides header and data: Request ID ×1,
/// header length ×2, number of chunks ×2, chunk number ×2, data length ×2.
pub const CHUNK_OVERHEAD: usize = 9;

/// Inquiry (`0x30`) and Reply (`0x31`) to Property Exchange Capabilities
/// (M2-101 §8.5, §8.6, Tables 30/32).
///
/// ```text
/// simultaneous requests ×1, [v2: PE major ×1, PE minor ×1]
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PeCapabilities {
    /// Number of Simultaneous Property Exchange Requests Supported.
    pub simultaneous: u8,
    /// PE Major Version (v2; `0` for Common Rules 1.0/1.1, Table 31).
    pub major: u8,
    /// PE Minor Version (v2; `0`).
    pub minor: u8,
}

impl Default for PeCapabilities {
    fn default() -> Self {
        Self {
            simultaneous: 1,
            major: 0,
            minor: 0,
        }
    }
}

impl Codec<'_> for PeCapabilities {
    fn read(r: &mut Reader<'_>, v: CiVersion) -> Result<Self, Error> {
        let simultaneous = r.u7()?;
        let (major, minor) = if v.has_v2_fields() {
            (r.u7()?, r.u7()?)
        } else {
            (0, 0)
        };
        Ok(Self {
            simultaneous,
            major,
            minor,
        })
    }
    fn write(&self, w: &mut Writer<'_>, v: CiVersion) -> Result<(), Error> {
        w.u7(self.simultaneous, "simultaneous requests")?;
        if v.has_v2_fields() {
            w.u7(self.major, "PE major version")?;
            w.u7(self.minor, "PE minor version")?;
        }
        Ok(())
    }
}

/// The body every chunked PE message shares (M2-101 Tables 33–39; also the
/// 1.1 Has Property Data messages).
///
/// ```text
/// request ID ×1, header length ×2, header, number of chunks ×2,
/// this chunk ×2, data length ×2, data
/// ```
///
/// Inquiry: Get Property Data and Reply to Set Property Data carry no
/// property data; in v1.2 they still carry the chunk fields with the fixed
/// values `1, 1, 0` (Tables 33, 36). [`PeChunk::single`] builds those.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PeChunk<'a> {
    /// Request ID (M2-101 §8.4), 7 bits.
    pub request_id: u8,
    /// Header Data (JSON), present in the first chunk only (§8.3.1).
    pub header: &'a [u8],
    /// Number of Chunks in Message; `0` = unknown (§8.3).
    pub num_chunks: u16,
    /// Number of This Chunk, from 1; `0` = final chunk of a message whose
    /// data may be unusable (§8.3).
    pub this_chunk: u16,
    /// Property Data in this chunk.
    pub data: &'a [u8],
}

impl<'a> PeChunk<'a> {
    /// A one-chunk message (`1 of 1`).
    #[must_use]
    pub const fn single(request_id: u8, header: &'a [u8], data: &'a [u8]) -> Self {
        Self {
            request_id,
            header,
            num_chunks: 1,
            this_chunk: 1,
            data,
        }
    }

    /// Whether this is the last chunk of its message (§8.3, Table 29).
    #[must_use]
    pub const fn is_last(&self) -> bool {
        self.this_chunk == 0 || self.this_chunk == self.num_chunks
    }

    /// Encoded size of the body (without the 13/15-byte envelope).
    #[must_use]
    pub const fn body_len(&self) -> usize {
        CHUNK_OVERHEAD + self.header.len() + self.data.len()
    }
}

fn len14(n: usize, what: &'static str) -> Result<u16, Error> {
    u16::try_from(n)
        .ok()
        .filter(|&n| n <= 0x3FFF)
        .ok_or(Error::OutOfRange(what))
}

impl<'a> Codec<'a> for PeChunk<'a> {
    fn read(r: &mut Reader<'a>, _: CiVersion) -> Result<Self, Error> {
        let request_id = r.u7()?;
        let hlen = r.u14()?;
        let header = r.bytes(usize::from(hlen))?;
        let num_chunks = r.u14()?;
        let this_chunk = r.u14()?;
        let dlen = r.u14()?;
        Ok(Self {
            request_id,
            header,
            num_chunks,
            this_chunk,
            data: r.bytes(usize::from(dlen))?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        w.u7(self.request_id, "request ID")?;
        w.u14(len14(self.header.len(), "header length")?, "header length")?;
        w.bytes(self.header, "header data")?;
        w.u14(self.num_chunks, "number of chunks")?;
        w.u14(self.this_chunk, "chunk number")?;
        w.u14(
            len14(self.data.len(), "property data length")?,
            "property data length",
        )?;
        w.bytes(self.data, "property data")
    }
}
