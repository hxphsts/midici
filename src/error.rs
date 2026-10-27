//! The one error type every parser and encoder returns.

use core::fmt;

use crate::wire::MessageKind;

/// Why bytes could not be decoded, or a value could not be encoded.
///
/// Decoding never panics: any input, however short or hostile, maps to one
/// of these variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Error {
    /// Not a message this decoder handles: not SysEx, not Universal
    /// Non-Real-Time (`7E`), or the wrong Sub-ID#1. Expected when a port
    /// carries other traffic (e.g. a manufacturer protocol on `F0 7D …`), so
    /// callers usually ignore it rather than NAK.
    Foreign,
    /// Starts with `F0` but does not end with `F7`.
    Unterminated,
    /// A byte with the high bit set where only 7-bit data may appear.
    NonDataByte {
        /// Offset of the byte within the input.
        offset: usize,
    },
    /// The input ends before a field the message requires.
    Truncated,
    /// The Device ID byte is in the reserved range `0x10..=0x7D`
    /// (M2-101 §5.2.1, Table 5).
    ReservedDeviceId(u8),
    /// The Message Version/Format byte has reserved bits set
    /// (M2-101 §5.2.1, Figure 3). A Responder answers this with NAK status
    /// `0x02` (M2-101 §5.3); parse the envelope with
    /// [`Frame::parse`](crate::wire::Frame::parse) to build that NAK.
    UnsupportedVersion(u8),
    /// The bytes are a well-formed CI frame, but of a different message than
    /// the caller asked to decode.
    UnexpectedKind(MessageKind),
    /// A field holds a value the specification does not allow (e.g. a MUID
    /// above 28 bits, a Product Instance Id with non-printable characters).
    InvalidField(&'static str),
    /// A value is too large for its wire field when encoding.
    OutOfRange(&'static str),
    /// The output buffer is too small for the encoded message.
    BufferTooSmall,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Foreign => f.write_str("not a MIDI-CI message"),
            Self::Unterminated => f.write_str("SysEx frame is missing its F7 terminator"),
            Self::NonDataByte { offset } => {
                write!(f, "byte at offset {offset} has its high bit set")
            }
            Self::Truncated => f.write_str("message ends before a required field"),
            Self::ReservedDeviceId(b) => write!(f, "reserved Device ID 0x{b:02X}"),
            Self::UnsupportedVersion(b) => {
                write!(f, "MIDI-CI message version 0x{b:02X} has reserved bits set")
            }
            Self::UnexpectedKind(k) => {
                write!(
                    f,
                    "unexpected MIDI-CI message (Sub-ID#2 0x{:02X})",
                    k.sub_id2()
                )
            }
            Self::InvalidField(what) => write!(f, "invalid {what}"),
            Self::OutOfRange(what) => write!(f, "{what} does not fit its wire field"),
            Self::BufferTooSmall => f.write_str("output buffer too small"),
        }
    }
}

impl core::error::Error for Error {}
