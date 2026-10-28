//! Device identity, and the MIDI 1.0 Universal Identity Request/Reply
//! ("Device Inquiry") that hosts send before anything else.
//!
//! ```text
//! Request: F0 7E <dev> 06 01 F7
//! Reply:   F0 7E <dev> 06 02 <manufacturer: 1 or 3> <family ×2> <model ×2> <revision ×4> F7
//! ```
//!
//! These are MIDI 1.0 messages, not MIDI-CI, but MIDI-CI Discovery carries
//! the same four identity fields (M2-101 §5.5.1), so they share
//! [`DeviceIdentity`]. The one difference: Identity Reply sends the
//! manufacturer ID in its native length (1 or 3 bytes), while MIDI-CI always
//! uses 3 bytes, padding a 1-byte ID with two zeros (M2-101 §5.5.1).
//!
//! Layout of the Identity Reply follows the MIDI 1.0 Detailed Specification
//! (General Information, Device Inquiry); that document is not freely
//! downloadable and was not consulted for this crate (see
//! `tests/conformance/README.md`).

use crate::codec::{Reader, Writer};
use crate::error::Error;
use crate::wire::{Framing, SYSEX_END, SYSEX_START, UNIVERSAL_NON_REAL_TIME};

/// Universal Non-Real-Time Sub-ID#1 "General Information".
pub const SUB_ID1_GENERAL_INFORMATION: u8 = 0x06;
/// Sub-ID#2 "Identity Request".
pub const SUB_ID2_IDENTITY_REQUEST: u8 = 0x01;
/// Sub-ID#2 "Identity Reply".
pub const SUB_ID2_IDENTITY_REPLY: u8 = 0x02;
/// The "all call" Device ID: every device answers.
pub const ALL_CALL: u8 = 0x7F;

/// A SysEx manufacturer ID, stored in MIDI-CI's normalised 3-byte form.
///
/// One-byte IDs (`0x01..=0x7D`) are `[id, 0, 0]`; three-byte IDs start with
/// `0x00` and are `[0, a, b]` (M2-101 §5.5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ManufacturerId([u8; 3]);

impl ManufacturerId {
    /// `0x7D`, the non-commercial / educational ID.
    pub const NON_COMMERCIAL: Self = Self([0x7D, 0, 0]);

    /// A one-byte ID (`0x01..=0x7D`).
    #[must_use]
    pub const fn short(id: u8) -> Option<Self> {
        if id >= 0x01 && id <= 0x7D {
            Some(Self([id, 0, 0]))
        } else {
            None
        }
    }

    /// A three-byte ID `00 a b`.
    #[must_use]
    pub const fn extended(a: u8, b: u8) -> Option<Self> {
        if (a | b) & 0x80 == 0 {
            Some(Self([0, a, b]))
        } else {
            None
        }
    }

    /// From the 3-byte MIDI-CI form. Kept verbatim (lossless), so a peer that
    /// pads a short ID with non-zero bytes still round-trips.
    #[must_use]
    pub const fn from_ci_bytes(bytes: [u8; 3]) -> Self {
        Self(bytes)
    }

    /// The 3-byte MIDI-CI form.
    #[must_use]
    pub const fn ci_bytes(self) -> [u8; 3] {
        self.0
    }

    /// Whether this is a three-byte (`00 a b`) ID.
    #[must_use]
    pub const fn is_extended(self) -> bool {
        self.0[0] == 0
    }

    /// The ID in its native SysEx length: 1 byte, or 3 if extended.
    #[must_use]
    pub const fn sysex_bytes(&self) -> &[u8] {
        if self.is_extended() {
            &self.0
        } else {
            self.0.split_at(1).0
        }
    }
}

/// The four identity fields shared by Identity Reply and MIDI-CI Discovery
/// (M2-101 §5.5.1).
///
/// Not `#[non_exhaustive]`: these four fields are fixed by the MIDI 1.0
/// Device Inquiry message, which MIDI-CI reuses unchanged; build it with a
/// struct literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeviceIdentity {
    /// SysEx manufacturer ID.
    pub manufacturer: ManufacturerId,
    /// Device family, 14 bits (two 7-bit bytes, LSB first on the wire).
    pub family: u16,
    /// Family model number, 14 bits.
    pub model: u16,
    /// Software revision level; the format is the manufacturer's, so the
    /// four 7-bit bytes are kept as sent.
    pub revision: [u8; 4],
}

impl DeviceIdentity {
    pub(crate) fn read_tail(
        r: &mut Reader<'_>,
        manufacturer: ManufacturerId,
    ) -> Result<Self, Error> {
        Ok(Self {
            manufacturer,
            family: r.u14()?,
            model: r.u14()?,
            revision: r.array()?,
        })
    }

    pub(crate) fn read_ci(r: &mut Reader<'_>) -> Result<Self, Error> {
        let m = ManufacturerId::from_ci_bytes(r.array()?);
        Self::read_tail(r, m)
    }

    pub(crate) fn write_tail(&self, w: &mut Writer<'_>) -> Result<(), Error> {
        w.u14(self.family, "device family")?;
        w.u14(self.model, "device model")?;
        w.bytes(&self.revision, "software revision")
    }

    pub(crate) fn write_ci(&self, w: &mut Writer<'_>) -> Result<(), Error> {
        w.bytes(&self.manufacturer.ci_bytes(), "manufacturer ID")?;
        self.write_tail(w)
    }
}

/// `F0 7E <dev> 06 01 F7`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IdentityRequest {
    /// Target Device ID (`0x7F` = all call).
    pub device_id: u8,
}

/// `F0 7E <dev> 06 02 …`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IdentityReply {
    /// The replying device's Device ID.
    pub device_id: u8,
    /// Who it is.
    pub identity: DeviceIdentity,
}

/// Strips brackets, checks 7-bit data, and returns `(device_id, sub_id2, rest)`.
fn open(bytes: &[u8]) -> Result<(u8, u8, Reader<'_>), Error> {
    let inner = match bytes {
        [SYSEX_START, inner @ .., SYSEX_END] => inner,
        [SYSEX_START, ..] => return Err(Error::Unterminated),
        _ => bytes,
    };
    match inner {
        [
            UNIVERSAL_NON_REAL_TIME,
            dev,
            SUB_ID1_GENERAL_INFORMATION,
            sub2,
            rest @ ..,
        ] => {
            if let Some(i) = inner.iter().position(|b| b & 0x80 != 0) {
                return Err(Error::NonDataByte { offset: i });
            }
            Ok((*dev, *sub2, Reader::new(rest)))
        }
        _ => Err(Error::Foreign),
    }
}

fn begin(w: &mut Writer<'_>, framing: Framing, device_id: u8, sub2: u8) -> Result<(), Error> {
    if framing == Framing::SysEx {
        w.raw(SYSEX_START)?;
    }
    w.raw(UNIVERSAL_NON_REAL_TIME)?;
    w.u7(device_id, "device ID")?;
    w.raw(SUB_ID1_GENERAL_INFORMATION)?;
    w.raw(sub2)
}

impl IdentityRequest {
    /// An all-call request.
    pub const ALL: Self = Self {
        device_id: ALL_CALL,
    };

    /// Encodes into `out`, returning the length.
    ///
    /// # Errors
    ///
    /// [`Error::BufferTooSmall`], or [`Error::OutOfRange`] if `device_id`
    /// exceeds 7 bits.
    pub fn encode(&self, framing: Framing, out: &mut [u8]) -> Result<usize, Error> {
        let mut w = Writer::new(out);
        begin(&mut w, framing, self.device_id, SUB_ID2_IDENTITY_REQUEST)?;
        if framing == Framing::SysEx {
            w.raw(SYSEX_END)?;
        }
        Ok(w.len())
    }

    /// Decodes a request, with or without `F0`/`F7`. Trailing bytes are
    /// ignored.
    ///
    /// # Errors
    ///
    /// [`Error::Foreign`] if this is not a General Information message,
    /// [`Error::InvalidField`] if it is not an Identity Request.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let (device_id, sub2, _) = open(bytes)?;
        if sub2 != SUB_ID2_IDENTITY_REQUEST {
            return Err(Error::InvalidField("identity Sub-ID#2"));
        }
        Ok(Self { device_id })
    }
}

impl IdentityReply {
    /// Encodes into `out`, returning the length.
    ///
    /// # Errors
    ///
    /// [`Error::BufferTooSmall`], or [`Error::OutOfRange`] if a field does
    /// not fit (e.g. `family > 0x3FFF`).
    pub fn encode(&self, framing: Framing, out: &mut [u8]) -> Result<usize, Error> {
        let mut w = Writer::new(out);
        begin(&mut w, framing, self.device_id, SUB_ID2_IDENTITY_REPLY)?;
        w.bytes(self.identity.manufacturer.sysex_bytes(), "manufacturer ID")?;
        self.identity.write_tail(&mut w)?;
        if framing == Framing::SysEx {
            w.raw(SYSEX_END)?;
        }
        Ok(w.len())
    }

    /// Decodes a reply, with or without `F0`/`F7`. Bytes after the software
    /// revision are ignored (some devices append extra data).
    ///
    /// # Errors
    ///
    /// [`Error::Foreign`] if this is not a General Information message,
    /// [`Error::InvalidField`] if it is not an Identity Reply,
    /// [`Error::Truncated`] if fields are missing.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let (device_id, sub2, mut r) = open(bytes)?;
        if sub2 != SUB_ID2_IDENTITY_REPLY {
            return Err(Error::InvalidField("identity Sub-ID#2"));
        }
        let first = r.u7()?;
        let manufacturer = if first == 0 {
            let [a, b] = r.array()?;
            ManufacturerId::from_ci_bytes([0, a, b])
        } else {
            ManufacturerId::from_ci_bytes([first, 0, 0])
        };
        Ok(Self {
            device_id,
            identity: DeviceIdentity::read_tail(&mut r, manufacturer)?,
        })
    }
}
