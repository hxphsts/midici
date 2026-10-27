//! MIDI-CI Unique Identifiers (M2-101 §3.3).
//!
//! A MUID is a 28-bit number naming one MIDI-CI Device (one Function Block)
//! for as long as it is powered on. On the wire it is four 7-bit bytes, least
//! significant first (M2-101 §5.2.1).
//!
//! The core has no random number generator: [`Muid::from_random`] maps a
//! caller-supplied `u32` (from a hardware RNG, `getrandom`, a seeded PRNG, …)
//! onto the valid range.

use core::fmt;

use crate::error::Error;

/// A 28-bit MIDI-CI Unique Identifier.
///
/// Holds any value in `0x0000_0000..=0x0FFF_FFFF`, including the reserved
/// range and [`Muid::BROADCAST`]; use [`Muid::is_assignable`] to check that a
/// value may name a device.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "u32", into = "u32"))]
pub struct Muid(u32);

impl Muid {
    /// Largest 28-bit value (`0x0FFF_FFFF`).
    pub const MAX: u32 = 0x0FFF_FFFF;
    /// The Broadcast MUID, `0x0FFF_FFFF`, sent as `7F 7F 7F 7F`
    /// (M2-101 §3.3.3).
    pub const BROADCAST: Self = Self(0x0FFF_FFFF);
    /// First reserved value (M2-101 §3.3).
    pub const RESERVED_START: u32 = 0x0FFF_FF00;
    /// Last reserved value (M2-101 §3.3).
    pub const RESERVED_END: u32 = 0x0FFF_FFFE;

    /// A MUID from a 28-bit value.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] if `value` exceeds 28 bits.
    pub const fn new(value: u32) -> Result<Self, Error> {
        if value > Self::MAX {
            Err(Error::InvalidField("MUID"))
        } else {
            Ok(Self(value))
        }
    }

    /// Maps 32 random bits onto an assignable MUID (never reserved, never
    /// broadcast). Uniform except that the 255 reserved/broadcast values fold
    /// onto `0x0FFF_FExx`.
    ///
    /// Call it with fresh randomness every power-up (M2-101 §3.3.1).
    #[must_use]
    pub const fn from_random(random: u32) -> Self {
        let v = random & Self::MAX;
        if v >= Self::RESERVED_START {
            Self(v & !0x100)
        } else {
            Self(v)
        }
    }

    /// The 28-bit value.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// Whether this is the Broadcast MUID.
    #[must_use]
    pub const fn is_broadcast(self) -> bool {
        self.0 == Self::BROADCAST.0
    }

    /// Whether this is in the reserved range `0x0FFF_FF00..=0x0FFF_FFFE`.
    #[must_use]
    pub const fn is_reserved(self) -> bool {
        self.0 >= Self::RESERVED_START && self.0 <= Self::RESERVED_END
    }

    /// Whether a device may use this as its own MUID (neither reserved nor
    /// broadcast).
    #[must_use]
    pub const fn is_assignable(self) -> bool {
        self.0 < Self::RESERVED_START
    }

    /// The four wire bytes, LSB first.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 4] {
        // Each byte is masked to 7 bits.
        #[allow(clippy::cast_possible_truncation)]
        [
            (self.0 & 0x7F) as u8,
            ((self.0 >> 7) & 0x7F) as u8,
            ((self.0 >> 14) & 0x7F) as u8,
            ((self.0 >> 21) & 0x7F) as u8,
        ]
    }

    /// Reads the four wire bytes, LSB first.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] if any byte has its high bit set.
    pub const fn from_bytes(bytes: [u8; 4]) -> Result<Self, Error> {
        let [a, b, c, d] = bytes;
        if (a | b | c | d) & 0x80 != 0 {
            return Err(Error::InvalidField("MUID"));
        }
        Ok(Self(
            a as u32 | (b as u32) << 7 | (c as u32) << 14 | (d as u32) << 21,
        ))
    }
}

impl fmt::Debug for Muid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Muid(0x{:07X})", self.0)
    }
}

impl fmt::Display for Muid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:07X}", self.0)
    }
}

impl TryFrom<u32> for Muid {
    type Error = Error;
    fn try_from(value: u32) -> Result<Self, Error> {
        Self::new(value)
    }
}

impl From<Muid> for u32 {
    fn from(m: Muid) -> Self {
        m.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_is_four_7f() {
        assert_eq!(Muid::BROADCAST.to_bytes(), [0x7F; 4]);
        assert_eq!(Muid::from_bytes([0x7F; 4]), Ok(Muid::BROADCAST));
        assert!(Muid::BROADCAST.is_broadcast());
        assert!(!Muid::BROADCAST.is_reserved());
        assert!(!Muid::BROADCAST.is_assignable());
    }

    #[test]
    fn range_checks() {
        assert!(Muid::new(0x1000_0000).is_err());
        let first = Muid::new(Muid::RESERVED_START).unwrap();
        let last = Muid::new(Muid::RESERVED_END).unwrap();
        assert!(first.is_reserved() && last.is_reserved());
        assert!(Muid::new(0x0FFF_FEFF).unwrap().is_assignable());
        assert!(Muid::new(0).unwrap().is_assignable());
    }

    #[test]
    fn from_random_is_always_assignable() {
        for r in [
            0,
            u32::MAX,
            0x0FFF_FF00,
            0x0FFF_FFFE,
            0xFFFF_FF80,
            0x1234_5678,
        ] {
            assert!(Muid::from_random(r).is_assignable(), "{r:#x}");
        }
        assert_eq!(Muid::from_random(0x1234_5678).value(), 0x0234_5678);
    }

    #[test]
    fn bytes_round_trip_lsb_first() {
        let m = Muid::new(0x0123_4567).unwrap();
        // 0x0123_4567 = 0b0000100_1000110_1000101_1100111
        assert_eq!(m.to_bytes(), [0x67, 0x0A, 0x0D, 0x09]);
        assert_eq!(Muid::from_bytes(m.to_bytes()), Ok(m));
        assert!(Muid::from_bytes([0x80, 0, 0, 0]).is_err());
    }
}
