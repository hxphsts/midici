//! MIDI-CI 1.1 Protocol Negotiation (Sub-ID#2 `0x10..=0x15`, M2-101 v1.1
//! §6), deprecated by MIDI-CI 1.2 (M2-101 §6).
//!
//! Decoded so that 1.1 peers can be recognised and NAKed precisely
//! (M2-101 §5.4 requires receivers to parse earlier versions). The engines
//! never send these; encoders exist for tests and tools.
//!
//! These structs are not `#[non_exhaustive]`: MIDI-CI 1.2 deprecated
//! Protocol Negotiation, so its layouts are frozen.

use crate::codec::{Reader, Writer};
use crate::error::Error;
use crate::mgmt::Codec;
use crate::wire::CiVersion;

/// A 5-byte Protocol Type (M2-101 v1.1 §6): type (`0x01` MIDI 1.0, `0x02`
/// MIDI 2.0), version, extensions, two reserved bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProtocolType([u8; 5]);

impl ProtocolType {
    /// A Protocol Type from its five wire bytes.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] if a byte exceeds 7 bits.
    pub const fn new(bytes: [u8; 5]) -> Result<Self, Error> {
        let [b0, b1, b2, b3, b4] = bytes;
        if (b0 | b1 | b2 | b3 | b4) & 0x80 != 0 {
            Err(Error::OutOfRange("protocol type"))
        } else {
            Ok(Self(bytes))
        }
    }

    /// The five bytes.
    #[must_use]
    pub const fn bytes(self) -> [u8; 5] {
        self.0
    }
}

/// Length of the Test New Protocol test data (M2-101 v1.1 §6.7, §6.8).
pub const TEST_DATA_LEN: usize = 48;

/// Initiate Protocol Negotiation (`0x10`) and its Reply (`0x11`):
/// authority level, count, `5 × count` protocol bytes (preferred first).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProtocolNegotiation<'a> {
    /// Authority Level.
    pub authority: u8,
    /// `5 × n` bytes of Protocol Types.
    pub protocols: &'a [u8],
}

impl<'a> ProtocolNegotiation<'a> {
    /// The protocols, preferred first.
    pub fn iter(&self) -> impl Iterator<Item = ProtocolType> + 'a {
        self.protocols
            .as_chunks::<5>()
            .0
            .iter()
            .map(|c| ProtocolType(*c))
    }
}

impl<'a> Codec<'a> for ProtocolNegotiation<'a> {
    fn read(r: &mut Reader<'a>, _: CiVersion) -> Result<Self, Error> {
        let authority = r.u7()?;
        let n = usize::from(r.u7()?);
        Ok(Self {
            authority,
            protocols: r.bytes(n * 5)?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        if !self.protocols.len().is_multiple_of(5) {
            return Err(Error::InvalidField("protocol list"));
        }
        let n = u8::try_from(self.protocols.len() / 5)
            .map_err(|_| Error::OutOfRange("protocol count"))?;
        w.u7(self.authority, "authority level")?;
        w.u7(n, "protocol count")?;
        w.bytes(self.protocols, "protocols")
    }
}

/// Set New Selected Protocol (`0x12`): authority level, protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SetNewProtocol {
    /// Authority Level.
    pub authority: u8,
    /// The new protocol.
    pub protocol: ProtocolType,
}

impl Codec<'_> for SetNewProtocol {
    fn read(r: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self {
            authority: r.u7()?,
            protocol: ProtocolType(r.array()?),
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        w.u7(self.authority, "authority level")?;
        w.bytes(&self.protocol.0, "protocol")
    }
}

/// Test New Protocol (`0x13` Initiator→Responder, `0x14`
/// Responder→Initiator): authority level and 48 test bytes `0x00..=0x2F`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TestNewProtocol<'a> {
    /// Authority Level.
    pub authority: u8,
    /// The 48 test bytes.
    pub data: &'a [u8],
}

impl<'a> Codec<'a> for TestNewProtocol<'a> {
    fn read(r: &mut Reader<'a>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self {
            authority: r.u7()?,
            data: r.bytes(TEST_DATA_LEN)?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        if self.data.len() != TEST_DATA_LEN {
            return Err(Error::InvalidField("test data"));
        }
        w.u7(self.authority, "authority level")?;
        w.bytes(self.data, "test data")
    }
}

/// Confirmation New Protocol Established (`0x15`): authority level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProtocolEstablished {
    /// Authority Level.
    pub authority: u8,
}

impl Codec<'_> for ProtocolEstablished {
    fn read(r: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self { authority: r.u7()? })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        w.u7(self.authority, "authority level")
    }
}
