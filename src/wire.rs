//! The MIDI-CI envelope: Universal SysEx framing common to every message
//! (M2-101 §5.2.1, Table 5).
//!
//! ```text
//! [F0] 7E <device> 0D <sub-id#2> <version> <source MUID ×4> <dest MUID ×4> <data…> [F7]
//! ```
//!
//! `F0`/`F7` are present on MIDI 1.0 byte streams and absent inside UMP
//! SysEx7 packets (M2-101 §5.2). [`Frame::parse`] accepts both;
//! [`Framing`] selects which one encoders emit.
//!
//! CI frames are Universal *Non-Real-Time* SysEx (`7E`), so they never collide
//! with a manufacturer protocol on the same port (e.g. `F0 7D …`):
//! [`Frame::parse`] answers [`Error::Foreign`] for those, which callers
//! should treat as "not mine" rather than as a fault.

use crate::codec::{Reader, Writer};
use crate::error::Error;
use crate::muid::Muid;

/// `F0`, System Exclusive start.
pub const SYSEX_START: u8 = 0xF0;
/// `F7`, End of Exclusive.
pub const SYSEX_END: u8 = 0xF7;
/// `7E`, Universal Non-Real-Time SysEx ID.
pub const UNIVERSAL_NON_REAL_TIME: u8 = 0x7E;
/// `0D`, Universal SysEx Sub-ID#1 for MIDI-CI (M2-101 §5.2.1).
pub const SUB_ID1_MIDI_CI: u8 = 0x0D;
/// Length of the envelope without `F0`/`F7`: `7E dev 0D sub2 ver` + 2 MUIDs.
pub const HEADER_LEN: usize = 13;
/// Bytes `F0` and `F7` add on a MIDI 1.0 byte stream.
pub const FRAMING_OVERHEAD: usize = 2;
/// Smallest Receivable Maximum SysEx Message Size any CI device may declare
/// (M2-101 §5.5.3).
pub const MIN_RECEIVABLE_SYSEX: u32 = 128;
/// Smallest Receivable Maximum SysEx Message Size for devices that initiate
/// Profile Configuration or Property Exchange (M2-101 §5.5.3).
pub const MIN_RECEIVABLE_SYSEX_INITIATOR: u32 = 512;

/// Whether encoders wrap the message in `F0 … F7`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Framing {
    /// `F0 … F7`, for MIDI 1.0 byte streams.
    #[default]
    SysEx,
    /// No brackets, for UMP SysEx7 payloads.
    Bare,
}

/// A MIDI channel `0..=15` (shown to users as 1–16).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "u8", into = "u8"))]
pub struct Channel(u8);

impl Channel {
    /// Channel `index` (`0..=15`), or `None` if out of range.
    #[must_use]
    pub const fn new(index: u8) -> Option<Self> {
        if index < 16 { Some(Self(index)) } else { None }
    }

    /// The zero-based index.
    #[must_use]
    pub const fn index(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Channel {
    type Error = Error;
    fn try_from(v: u8) -> Result<Self, Error> {
        Self::new(v).ok_or(Error::InvalidField("channel"))
    }
}

impl From<Channel> for u8 {
    fn from(c: Channel) -> Self {
        c.0
    }
}

/// The Universal SysEx Device ID byte, as MIDI-CI uses it
/// (M2-101 §3.2, §5.2.1).
///
/// Whether it names the source or the destination depends on the message:
/// inquiries carry the destination, replies the source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DeviceId {
    /// `0x00..=0x0F`: one MIDI channel (of the UMP Group, on UMP).
    Channel(Channel),
    /// `0x7E`: the whole Group (on MIDI 1.0 streams, the whole port).
    Group,
    /// `0x7F`: the whole Function Block. The default, and the only value
    /// allowed for Management and Property Exchange messages.
    #[default]
    FunctionBlock,
}

impl DeviceId {
    /// Parses the wire byte.
    ///
    /// # Errors
    ///
    /// [`Error::ReservedDeviceId`] for `0x10..=0x7D` and
    /// [`Error::NonDataByte`] for bytes `>= 0x80`.
    pub const fn from_byte(b: u8) -> Result<Self, Error> {
        match b {
            0x00..=0x0F => Ok(Self::Channel(Channel(b))),
            0x7E => Ok(Self::Group),
            0x7F => Ok(Self::FunctionBlock),
            0x10..=0x7D => Err(Error::ReservedDeviceId(b)),
            _ => Err(Error::NonDataByte { offset: 0 }),
        }
    }

    /// The wire byte.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Channel(c) => c.0,
            Self::Group => 0x7E,
            Self::FunctionBlock => 0x7F,
        }
    }
}

/// The MIDI-CI Message Version/Format byte (M2-101 §5.2.1, Figure 3, §5.3).
///
/// Layout `0 rrr mmmm`: bit 7 is the SysEx data bit, bits 6–4 are reserved
/// for future major revisions, bits 3–0 are the minor version. (The prose in
/// §5.2.1 numbers these "bits 7..5" and "bits 4..1", counting from one;
/// Figure 3 shows three reserved bits and four minor bits.)
///
/// Holds any 7-bit value so that frames with reserved bits set still parse
/// and can be answered with NAK status `0x02`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CiVersion(u8);

impl CiVersion {
    /// Version 0x01, MIDI-CI 1.1.
    pub const V1_1: Self = Self(0x01);
    /// Version 0x02, MIDI-CI 1.2. What this crate emits by default.
    pub const V1_2: Self = Self(0x02);
    /// The version this crate implements.
    pub const CURRENT: Self = Self::V1_2;
    /// Reserved major-revision bits.
    pub const RESERVED_MASK: u8 = 0x70;

    /// From the wire byte; `None` if bit 7 is set.
    #[must_use]
    pub const fn from_byte(b: u8) -> Option<Self> {
        if b & 0x80 == 0 { Some(Self(b)) } else { None }
    }

    /// The wire byte.
    #[must_use]
    pub const fn byte(self) -> u8 {
        self.0
    }

    /// The minor version (bits 3–0).
    #[must_use]
    pub const fn minor(self) -> u8 {
        self.0 & 0x0F
    }

    /// Whether reserved bits are set, i.e. a future major revision this crate
    /// cannot read (M2-101 §5.3).
    #[must_use]
    pub const fn has_reserved_bits(self) -> bool {
        self.0 & Self::RESERVED_MASK != 0
    }

    /// Whether fields added in Message Version 2 (MIDI-CI 1.2) are present.
    #[must_use]
    pub const fn has_v2_fields(self) -> bool {
        self.minor() >= 2
    }
}

impl Default for CiVersion {
    fn default() -> Self {
        Self::CURRENT
    }
}

/// The Category of a MIDI-CI message: the high nibble of Sub-ID#2
/// (M2-101 §5.1, Table 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Category {
    /// `0x00..=0x0F`, `0x50..=0x6F`: no messages defined.
    Reserved,
    /// `0x10..=0x1F`: deprecated in MIDI-CI 1.2.
    ProtocolNegotiation,
    /// `0x20..=0x2F`.
    ProfileConfiguration,
    /// `0x30..=0x3F`.
    PropertyExchange,
    /// `0x40..=0x4F`, new in MIDI-CI 1.2.
    ProcessInquiry,
    /// `0x70..=0x7F`.
    Management,
}

/// A Sub-ID#2 value the specification does not define.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UndefinedSubId(u8);

impl UndefinedSubId {
    /// The raw Sub-ID#2.
    #[must_use]
    pub const fn sub_id2(self) -> u8 {
        self.0
    }
}

macro_rules! message_kinds {
    ($( $(#[$doc:meta])* $name:ident = $val:literal, )*) => {
        /// Every MIDI-CI message type, by Universal SysEx Sub-ID#2
        /// (M2-101 Appendix D, plus the MIDI-CI 1.1 messages it deprecates).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[non_exhaustive]
        pub enum MessageKind {
            $( $(#[$doc])* $name, )*
            /// A reserved or undefined Sub-ID#2. Answer with NAK status
            /// `0x01` ("not supported", M2-101 §5.11.2).
            Undefined(UndefinedSubId),
        }

        impl MessageKind {
            /// Every defined kind, in Sub-ID#2 order.
            pub const ALL: &'static [Self] = &[$(Self::$name,)*];

            /// The kind for a Sub-ID#2 byte. Total: undefined values map to
            /// [`MessageKind::Undefined`].
            #[must_use]
            pub const fn from_sub_id2(b: u8) -> Self {
                match b {
                    $( $val => Self::$name, )*
                    _ => Self::Undefined(UndefinedSubId(b)),
                }
            }

            /// The Sub-ID#2 byte.
            #[must_use]
            pub const fn sub_id2(self) -> u8 {
                match self {
                    $( Self::$name => $val, )*
                    Self::Undefined(u) => u.0,
                }
            }
        }
    };
}

message_kinds! {
    /// Initiate Protocol Negotiation (MIDI-CI 1.1 §6; deprecated in 1.2).
    InitiateProtocolNegotiation = 0x10,
    /// Reply to Initiate Protocol Negotiation (MIDI-CI 1.1; deprecated).
    InitiateProtocolNegotiationReply = 0x11,
    /// Set New Selected Protocol (MIDI-CI 1.1; deprecated).
    SetNewSelectedProtocol = 0x12,
    /// Test New Protocol, Initiator to Responder (MIDI-CI 1.1; deprecated).
    TestNewProtocolInitiatorToResponder = 0x13,
    /// Test New Protocol, Responder to Initiator (MIDI-CI 1.1; deprecated).
    TestNewProtocolResponderToInitiator = 0x14,
    /// Confirmation New Protocol Established (MIDI-CI 1.1; deprecated).
    ConfirmationProtocolEstablished = 0x15,
    /// Profile Inquiry (M2-101 §7.2).
    ProfileInquiry = 0x20,
    /// Reply to Profile Inquiry (M2-101 §7.3).
    ProfileInquiryReply = 0x21,
    /// Set Profile On (M2-101 §7.8).
    SetProfileOn = 0x22,
    /// Set Profile Off (M2-101 §7.9).
    SetProfileOff = 0x23,
    /// Profile Enabled Report (M2-101 §7.10).
    ProfileEnabledReport = 0x24,
    /// Profile Disabled Report (M2-101 §7.11).
    ProfileDisabledReport = 0x25,
    /// Profile Added Report (M2-101 §7.4; new in 1.2).
    ProfileAddedReport = 0x26,
    /// Profile Removed Report (M2-101 §7.5; new in 1.2).
    ProfileRemovedReport = 0x27,
    /// Profile Details Inquiry (M2-101 §7.6; new in 1.2).
    ProfileDetailsInquiry = 0x28,
    /// Reply to Profile Details Inquiry (M2-101 §7.7; new in 1.2).
    ProfileDetailsReply = 0x29,
    /// Profile Specific Data (M2-101 §7.12).
    ProfileSpecificData = 0x2F,
    /// Inquiry: Property Exchange Capabilities (M2-101 §8.5).
    PeCapabilitiesInquiry = 0x30,
    /// Reply to Property Exchange Capabilities (M2-101 §8.6).
    PeCapabilitiesReply = 0x31,
    /// Inquiry: Has Property Data (MIDI-CI 1.1; reserved in 1.2).
    HasPropertyDataInquiry = 0x32,
    /// Reply to Has Property Data (MIDI-CI 1.1; reserved in 1.2).
    HasPropertyDataReply = 0x33,
    /// Inquiry: Get Property Data (M2-101 §8.7).
    GetPropertyData = 0x34,
    /// Reply to Get Property Data (M2-101 §8.8).
    GetPropertyDataReply = 0x35,
    /// Inquiry: Set Property Data (M2-101 §8.9).
    SetPropertyData = 0x36,
    /// Reply to Set Property Data (M2-101 §8.10).
    SetPropertyDataReply = 0x37,
    /// Subscription (M2-101 §8.11).
    Subscription = 0x38,
    /// Reply to Subscription (M2-101 §8.12).
    SubscriptionReply = 0x39,
    /// Notify (M2-101 §8.13).
    Notify = 0x3F,
    /// Inquiry: Process Inquiry Capabilities (M2-101 §9.2; new in 1.2).
    ProcessInquiryCapabilities = 0x40,
    /// Reply to Process Inquiry Capabilities (M2-101 §9.3; new in 1.2).
    ProcessInquiryCapabilitiesReply = 0x41,
    /// Inquiry: MIDI Message Report (M2-101 §9.5; new in 1.2).
    MidiMessageReportInquiry = 0x42,
    /// Reply to MIDI Message Report (M2-101 §9.6; new in 1.2).
    MidiMessageReportReply = 0x43,
    /// End of MIDI Message Report (M2-101 §9.8; new in 1.2).
    EndOfMidiMessageReport = 0x44,
    /// Discovery (M2-101 §5.5).
    Discovery = 0x70,
    /// Reply to Discovery (M2-101 §5.6).
    DiscoveryReply = 0x71,
    /// Inquiry: Endpoint Information (M2-101 §5.7; new in 1.2).
    EndpointInquiry = 0x72,
    /// Reply to Endpoint Information (M2-101 §5.8; new in 1.2).
    EndpointReply = 0x73,
    /// MIDI-CI ACK (M2-101 §5.10; new in 1.2).
    Ack = 0x7D,
    /// Invalidate MUID (M2-101 §5.9).
    InvalidateMuid = 0x7E,
    /// MIDI-CI NAK (M2-101 §5.11).
    Nak = 0x7F,
}

impl MessageKind {
    /// The Category (high nibble of Sub-ID#2).
    #[must_use]
    pub const fn category(self) -> Category {
        match self.sub_id2() >> 4 {
            1 => Category::ProtocolNegotiation,
            2 => Category::ProfileConfiguration,
            3 => Category::PropertyExchange,
            4 => Category::ProcessInquiry,
            7 => Category::Management,
            _ => Category::Reserved,
        }
    }

    /// Whether MIDI-CI 1.2 deprecates or reserves this message. Such
    /// messages are still recognised so that 1.1 peers can be NAKed
    /// precisely (M2-101 §5.4: receivers parse all previous versions).
    #[must_use]
    pub const fn is_deprecated(self) -> bool {
        matches!(self.category(), Category::ProtocolNegotiation)
            || matches!(
                self,
                Self::HasPropertyDataInquiry | Self::HasPropertyDataReply
            )
    }

    /// Whether the Device ID must be `0x7F` (Function Block) for this
    /// message (M2-101 Appendix D, "Addresses" column).
    #[must_use]
    pub const fn function_block_only(self) -> bool {
        match self.category() {
            Category::PropertyExchange => true,
            Category::Management => !matches!(self, Self::Ack | Self::Nak),
            Category::ProcessInquiry => {
                matches!(
                    self,
                    Self::ProcessInquiryCapabilities | Self::ProcessInquiryCapabilitiesReply
                )
            }
            _ => false,
        }
    }

    /// Whether the specification allows the Broadcast MUID as destination
    /// (M2-101 §3.3.3). Profile Specific Data allows it only when the active
    /// Profile says so; this returns `true` for it.
    #[must_use]
    pub const fn allows_broadcast(self) -> bool {
        matches!(
            self,
            Self::Discovery
                | Self::InvalidateMuid
                | Self::ProfileAddedReport
                | Self::ProfileRemovedReport
                | Self::ProfileEnabledReport
                | Self::ProfileDisabledReport
                | Self::ProfileSpecificData
        )
    }
}

impl From<u8> for MessageKind {
    fn from(b: u8) -> Self {
        Self::from_sub_id2(b)
    }
}

/// The addressing part of a CI envelope; everything but the message kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Envelope {
    /// Device ID byte.
    pub device: DeviceId,
    /// Message Version/Format.
    pub version: CiVersion,
    /// Sender's MUID.
    pub source: Muid,
    /// Receiver's MUID.
    pub destination: Muid,
}

impl Envelope {
    /// A Function Block-addressed envelope at [`CiVersion::CURRENT`].
    #[must_use]
    pub const fn new(source: Muid, destination: Muid) -> Self {
        Self {
            device: DeviceId::FunctionBlock,
            version: CiVersion::CURRENT,
            source,
            destination,
        }
    }

    /// A Function Block-addressed envelope to the Broadcast MUID.
    #[must_use]
    pub const fn broadcast(source: Muid) -> Self {
        Self::new(source, Muid::BROADCAST)
    }

    /// The reply envelope: source and destination swapped, same Device ID,
    /// this crate's version (M2-101 §5.3: always send one's own version).
    #[must_use]
    pub const fn reply(self) -> Self {
        Self {
            device: self.device,
            version: CiVersion::CURRENT,
            source: self.destination,
            destination: self.source,
        }
    }
}

/// The full CI header: envelope plus message kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Header {
    /// Message type (Sub-ID#2).
    pub kind: MessageKind,
    /// Addressing.
    pub envelope: Envelope,
}

/// A parsed CI envelope with its undecoded data field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    /// The header.
    pub header: Header,
    /// The message data after the destination MUID (without `F7`).
    pub body: &'a [u8],
}

/// Strips optional `F0`/`F7` and checks that everything left is 7-bit data.
fn unframe(bytes: &[u8]) -> Result<&[u8], Error> {
    let (inner, offset) = match bytes {
        [SYSEX_START, inner @ .., SYSEX_END] => (inner, 1),
        [SYSEX_START, ..] => return Err(Error::Unterminated),
        _ => (bytes, 0),
    };
    if let Some(i) = inner.iter().position(|b| b & 0x80 != 0) {
        return Err(Error::NonDataByte { offset: i + offset });
    }
    Ok(inner)
}

/// Whether `bytes` look like a MIDI-CI message (framed or bare), judged by
/// the first bytes only. Cheap routing test for ports that carry several
/// SysEx protocols.
#[must_use]
pub const fn is_midi_ci(bytes: &[u8]) -> bool {
    let inner = match bytes {
        [SYSEX_START, rest @ ..] => rest,
        _ => bytes,
    };
    matches!(inner, [UNIVERSAL_NON_REAL_TIME, _, SUB_ID1_MIDI_CI, ..])
}

impl<'a> Frame<'a> {
    /// Parses the envelope of a CI message, with or without `F0`/`F7`.
    ///
    /// Lenient about semantics (any Sub-ID#2, any version, any MUIDs) so the
    /// caller can always answer with a NAK; strict about structure.
    ///
    /// # Errors
    ///
    /// [`Error::Foreign`] if this is not a Universal Non-Real-Time MIDI-CI
    /// message; [`Error::Unterminated`], [`Error::NonDataByte`],
    /// [`Error::Truncated`], [`Error::ReservedDeviceId`] for malformed ones.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if !is_midi_ci(bytes) {
            return Err(Error::Foreign);
        }
        let inner = unframe(bytes)?;
        let mut r = Reader::new(inner);
        let [_, device, _, sub_id2, version] = r.array::<5>()?;
        let device = DeviceId::from_byte(device)?;
        let kind = MessageKind::from_sub_id2(sub_id2);
        let version = CiVersion::from_byte(version).ok_or(Error::Truncated)?;
        let source = r.muid()?;
        let destination = r.muid()?;
        Ok(Self {
            header: Header {
                kind,
                envelope: Envelope {
                    device,
                    version,
                    source,
                    destination,
                },
            },
            body: r.rest(),
        })
    }
}

/// Writes `[F0] 7E dev 0D sub2 ver src dst`.
pub(crate) fn write_header(
    w: &mut Writer<'_>,
    header: &Header,
    framing: Framing,
) -> Result<(), Error> {
    if framing == Framing::SysEx {
        w.raw(SYSEX_START)?;
    }
    let e = &header.envelope;
    w.raw(UNIVERSAL_NON_REAL_TIME)?;
    w.raw(e.device.to_byte())?;
    w.raw(SUB_ID1_MIDI_CI)?;
    w.u7(header.kind.sub_id2(), "Sub-ID#2")?;
    w.u7(e.version.byte(), "message version")?;
    w.muid(e.source)?;
    w.muid(e.destination)
}

/// Writes `F7` if framed.
pub(crate) fn write_trailer(w: &mut Writer<'_>, framing: Framing) -> Result<(), Error> {
    if framing == Framing::SysEx {
        w.raw(SYSEX_END)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_sub_id() {
        for b in 0..=0x7F {
            assert_eq!(MessageKind::from_sub_id2(b).sub_id2(), b);
        }
        for &k in MessageKind::ALL {
            assert!(!matches!(k, MessageKind::Undefined(_)));
            assert_eq!(MessageKind::from_sub_id2(k.sub_id2()), k);
        }
        assert_eq!(MessageKind::ALL.len(), 40);
    }

    #[test]
    fn appendix_d_reserved_values_are_undefined() {
        // M2-101 Appendix D: 0x00-0x0F, 0x16-0x1F, 0x2A-0x2E, 0x3A-0x3E,
        // 0x45-0x4F (0x45 is not listed at all), 0x50-0x6F, 0x74-0x7C.
        for b in (0x00..=0x0F)
            .chain(0x16..=0x1F)
            .chain(0x2A..=0x2E)
            .chain(0x3A..=0x3E)
            .chain(0x45..=0x4F)
            .chain(0x50..=0x6F)
            .chain(0x74..=0x7C)
        {
            assert!(
                matches!(MessageKind::from_sub_id2(b), MessageKind::Undefined(_)),
                "{b:#04x}"
            );
        }
    }

    #[test]
    fn categories_follow_high_nibble() {
        assert_eq!(MessageKind::Discovery.category(), Category::Management);
        assert_eq!(MessageKind::Notify.category(), Category::PropertyExchange);
        assert_eq!(
            MessageKind::SetNewSelectedProtocol.category(),
            Category::ProtocolNegotiation
        );
        assert!(MessageKind::SetNewSelectedProtocol.is_deprecated());
        assert!(MessageKind::HasPropertyDataReply.is_deprecated());
        assert!(!MessageKind::GetPropertyData.is_deprecated());
    }

    #[test]
    fn addressing_matches_appendix_d() {
        assert!(MessageKind::GetPropertyData.function_block_only());
        assert!(MessageKind::Discovery.function_block_only());
        assert!(!MessageKind::Nak.function_block_only());
        assert!(!MessageKind::ProfileInquiry.function_block_only());
        assert!(MessageKind::ProcessInquiryCapabilities.function_block_only());
        assert!(!MessageKind::MidiMessageReportInquiry.function_block_only());
    }

    #[test]
    fn device_ids() {
        assert_eq!(DeviceId::from_byte(0x7F), Ok(DeviceId::FunctionBlock));
        assert_eq!(DeviceId::from_byte(0x7E), Ok(DeviceId::Group));
        assert_eq!(
            DeviceId::from_byte(0x0F),
            Ok(DeviceId::Channel(Channel::new(15).unwrap()))
        );
        assert_eq!(
            DeviceId::from_byte(0x10),
            Err(Error::ReservedDeviceId(0x10))
        );
    }

    #[test]
    fn version_bits() {
        assert!(!CiVersion::V1_1.has_v2_fields());
        assert!(CiVersion::V1_2.has_v2_fields());
        let future = CiVersion::from_byte(0x12).unwrap();
        assert!(future.has_reserved_bits());
        assert_eq!(future.minor(), 2);
    }

    #[test]
    fn foreign_sysex_is_foreign() {
        // An FMX control frame (F0 7D 'F' 'X' …).
        assert_eq!(
            Frame::parse(&[0xF0, 0x7D, b'F', b'X', 1, 1, 0xF7]),
            Err(Error::Foreign)
        );
        // Identity Request is Universal but not CI.
        assert_eq!(
            Frame::parse(&[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]),
            Err(Error::Foreign)
        );
        assert_eq!(Frame::parse(&[]), Err(Error::Foreign));
    }
}
