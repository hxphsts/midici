//! Whole messages: envelope plus typed body.
//!
//! [`Message::decode`] dispatches on Sub-ID#2 to a typed [`Body`] for every
//! message defined by MIDI-CI 1.2 and 1.1. Only undefined Sub-ID#2 values
//! come back as [`Body::Raw`] (so a Responder can NAK them with status
//! `0x01`).

use crate::codec::{Reader, Writer};
use crate::error::Error;
use crate::legacy::{ProtocolEstablished, ProtocolNegotiation, SetNewProtocol, TestNewProtocol};
use crate::mgmt::{
    Ack, Codec, Discovery, DiscoveryReply, EndpointInquiry, EndpointReply, InvalidateMuid, Nak,
};
use crate::pe::{PeCapabilities, PeChunk};
use crate::process::{
    EndOfMessageReport, MessageReportInquiry, MessageReportReply, ProcessInquiry,
    ProcessInquiryReply,
};
use crate::profile::{
    ProfileChannels, ProfileDetailsInquiry, ProfileDetailsReply, ProfileInquiry,
    ProfileInquiryReply, ProfileReport, ProfileSpecificData, SetProfileOff,
};
use crate::wire::{self, Envelope, Frame, Framing, Header, MessageKind};

macro_rules! bodies {
    ($( $(#[$doc:meta])* $variant:ident ( $ty:ty ) = $kind:ident, )*) => {
        /// A typed message body.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum Body<'a> {
            $( $(#[$doc])* $variant($ty), )*
            /// An undefined Sub-ID#2, with its data field verbatim.
            Raw {
                /// Sub-ID#2.
                kind: MessageKind,
                /// Everything after the destination MUID.
                data: &'a [u8],
            },
        }

        impl<'a> Body<'a> {
            /// The message kind (Sub-ID#2).
            #[must_use]
            pub const fn kind(&self) -> MessageKind {
                match self {
                    $( Self::$variant(_) => MessageKind::$kind, )*
                    Self::Raw { kind, .. } => *kind,
                }
            }

            fn decode(frame: &Frame<'a>) -> Result<Self, Error> {
                let mut r = Reader::new(frame.body);
                let v = frame.header.envelope.version;
                // Trailing bytes are fields from a newer minor version
                // (M2-101 §5.4) and are ignored.
                Ok(match frame.header.kind {
                    $( MessageKind::$kind => Self::$variant(<$ty>::read(&mut r, v)?), )*
                    kind => Self::Raw { kind, data: frame.body },
                })
            }

            fn encode(&self, w: &mut Writer<'_>, v: crate::wire::CiVersion) -> Result<(), Error> {
                match self {
                    $( Self::$variant(b) => Codec::write(b, w, v), )*
                    Self::Raw { data, .. } => w.bytes(data, "message data"),
                }
            }
        }
    };
}

bodies! {
    /// Initiate Protocol Negotiation (`0x10`, 1.1, deprecated).
    InitiateProtocolNegotiation(ProtocolNegotiation<'a>) = InitiateProtocolNegotiation,
    /// Reply to Initiate Protocol Negotiation (`0x11`, 1.1, deprecated).
    InitiateProtocolNegotiationReply(ProtocolNegotiation<'a>) = InitiateProtocolNegotiationReply,
    /// Set New Selected Protocol (`0x12`, 1.1, deprecated).
    SetNewProtocol(SetNewProtocol) = SetNewSelectedProtocol,
    /// Test New Protocol Initiator→Responder (`0x13`, 1.1, deprecated).
    TestNewProtocolInitiator(TestNewProtocol<'a>) = TestNewProtocolInitiatorToResponder,
    /// Test New Protocol Responder→Initiator (`0x14`, 1.1, deprecated).
    TestNewProtocolResponder(TestNewProtocol<'a>) = TestNewProtocolResponderToInitiator,
    /// Confirmation New Protocol Established (`0x15`, 1.1, deprecated).
    ProtocolEstablished(ProtocolEstablished) = ConfirmationProtocolEstablished,
    /// Profile Inquiry (`0x20`).
    ProfileInquiry(ProfileInquiry) = ProfileInquiry,
    /// Reply to Profile Inquiry (`0x21`).
    ProfileInquiryReply(ProfileInquiryReply<'a>) = ProfileInquiryReply,
    /// Set Profile On (`0x22`).
    SetProfileOn(ProfileChannels) = SetProfileOn,
    /// Set Profile Off (`0x23`).
    SetProfileOff(SetProfileOff) = SetProfileOff,
    /// Profile Enabled Report (`0x24`).
    ProfileEnabled(ProfileChannels) = ProfileEnabledReport,
    /// Profile Disabled Report (`0x25`).
    ProfileDisabled(ProfileChannels) = ProfileDisabledReport,
    /// Profile Added Report (`0x26`).
    ProfileAdded(ProfileReport) = ProfileAddedReport,
    /// Profile Removed Report (`0x27`).
    ProfileRemoved(ProfileReport) = ProfileRemovedReport,
    /// Profile Details Inquiry (`0x28`).
    ProfileDetailsInquiry(ProfileDetailsInquiry) = ProfileDetailsInquiry,
    /// Reply to Profile Details Inquiry (`0x29`).
    ProfileDetailsReply(ProfileDetailsReply<'a>) = ProfileDetailsReply,
    /// Profile Specific Data (`0x2F`).
    ProfileSpecificData(ProfileSpecificData<'a>) = ProfileSpecificData,
    /// Inquiry: PE Capabilities (`0x30`).
    PeCapabilitiesInquiry(PeCapabilities) = PeCapabilitiesInquiry,
    /// Reply to PE Capabilities (`0x31`).
    PeCapabilitiesReply(PeCapabilities) = PeCapabilitiesReply,
    /// Inquiry: Has Property Data (`0x32`, 1.1; reserved in 1.2).
    HasPropertyData(PeChunk<'a>) = HasPropertyDataInquiry,
    /// Reply to Has Property Data (`0x33`, 1.1; reserved in 1.2).
    HasPropertyDataReply(PeChunk<'a>) = HasPropertyDataReply,
    /// Inquiry: Get Property Data (`0x34`).
    GetPropertyData(PeChunk<'a>) = GetPropertyData,
    /// Reply to Get Property Data (`0x35`).
    GetPropertyDataReply(PeChunk<'a>) = GetPropertyDataReply,
    /// Inquiry: Set Property Data (`0x36`).
    SetPropertyData(PeChunk<'a>) = SetPropertyData,
    /// Reply to Set Property Data (`0x37`).
    SetPropertyDataReply(PeChunk<'a>) = SetPropertyDataReply,
    /// Subscription (`0x38`).
    Subscription(PeChunk<'a>) = Subscription,
    /// Reply to Subscription (`0x39`).
    SubscriptionReply(PeChunk<'a>) = SubscriptionReply,
    /// Notify (`0x3F`).
    Notify(PeChunk<'a>) = Notify,
    /// Inquiry: Process Inquiry Capabilities (`0x40`).
    ProcessInquiry(ProcessInquiry) = ProcessInquiryCapabilities,
    /// Reply to Process Inquiry Capabilities (`0x41`).
    ProcessInquiryReply(ProcessInquiryReply) = ProcessInquiryCapabilitiesReply,
    /// Inquiry: MIDI Message Report (`0x42`).
    MessageReportInquiry(MessageReportInquiry) = MidiMessageReportInquiry,
    /// Reply to MIDI Message Report (`0x43`).
    MessageReportReply(MessageReportReply) = MidiMessageReportReply,
    /// End of MIDI Message Report (`0x44`).
    EndOfMessageReport(EndOfMessageReport) = EndOfMidiMessageReport,
    /// Discovery (`0x70`).
    Discovery(Discovery) = Discovery,
    /// Reply to Discovery (`0x71`).
    DiscoveryReply(DiscoveryReply) = DiscoveryReply,
    /// Inquiry: Endpoint Information (`0x72`).
    EndpointInquiry(EndpointInquiry) = EndpointInquiry,
    /// Reply to Endpoint Information (`0x73`).
    EndpointReply(EndpointReply<'a>) = EndpointReply,
    /// MIDI-CI ACK (`0x7D`).
    Ack(Ack<'a>) = Ack,
    /// Invalidate MUID (`0x7E`).
    InvalidateMuid(InvalidateMuid) = InvalidateMuid,
    /// MIDI-CI NAK (`0x7F`).
    Nak(Nak<'a>) = Nak,
}

impl Body<'_> {
    /// The PE chunk, for any chunked Property Exchange body.
    #[must_use]
    pub const fn pe_chunk(&self) -> Option<&PeChunk<'_>> {
        match self {
            Self::HasPropertyData(c)
            | Self::HasPropertyDataReply(c)
            | Self::GetPropertyData(c)
            | Self::GetPropertyDataReply(c)
            | Self::SetPropertyData(c)
            | Self::SetPropertyDataReply(c)
            | Self::Subscription(c)
            | Self::SubscriptionReply(c)
            | Self::Notify(c) => Some(c),
            _ => None,
        }
    }
}

/// A complete MIDI-CI message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message<'a> {
    /// Addressing and version.
    pub envelope: Envelope,
    /// Typed content.
    pub body: Body<'a>,
}

impl<'a> Message<'a> {
    /// A message.
    #[must_use]
    pub const fn new(envelope: Envelope, body: Body<'a>) -> Self {
        Self { envelope, body }
    }

    /// Decodes a message, with or without `F0`/`F7`.
    ///
    /// Bytes after the last field this crate knows are ignored, as M2-101
    /// §5.3/§5.4 require for newer minor versions.
    ///
    /// # Errors
    ///
    /// Everything [`Frame::parse`] reports, plus
    /// [`Error::UnsupportedVersion`] when reserved version bits are set
    /// (answer with NAK [`NakStatus::VERSION_NOT_SUPPORTED`]), and
    /// [`Error::Truncated`]/[`Error::InvalidField`] for malformed bodies.
    ///
    /// [`NakStatus::VERSION_NOT_SUPPORTED`]: crate::mgmt::NakStatus::VERSION_NOT_SUPPORTED
    pub fn decode(bytes: &'a [u8]) -> Result<Self, Error> {
        let frame = Frame::parse(bytes)?;
        let envelope = frame.header.envelope;
        if envelope.version.has_reserved_bits() {
            return Err(Error::UnsupportedVersion(envelope.version.byte()));
        }
        Ok(Self {
            envelope,
            body: Body::decode(&frame)?,
        })
    }

    /// The message kind.
    #[must_use]
    pub const fn kind(&self) -> MessageKind {
        self.body.kind()
    }

    /// Encodes into `out`, returning the number of bytes written.
    ///
    /// Version 2 fields are written only if `envelope.version` declares
    /// them.
    ///
    /// # Errors
    ///
    /// [`Error::BufferTooSmall`] if `out` is too short;
    /// [`Error::OutOfRange`]/[`Error::InvalidField`] if a field cannot be
    /// represented.
    pub fn encode(&self, framing: Framing, out: &mut [u8]) -> Result<usize, Error> {
        let mut w = Writer::new(out);
        let header = Header {
            kind: self.kind(),
            envelope: self.envelope,
        };
        wire::write_header(&mut w, &header, framing)?;
        self.body.encode(&mut w, self.envelope.version)?;
        wire::write_trailer(&mut w, framing)?;
        Ok(w.len())
    }

    /// Encodes into a new `Vec`.
    ///
    /// # Errors
    ///
    /// As [`Message::encode`], except never [`Error::BufferTooSmall`] for
    /// messages up to 1 MiB.
    #[cfg(feature = "alloc")]
    pub fn to_vec(&self, framing: Framing) -> Result<alloc::vec::Vec<u8>, Error> {
        let mut buf = alloc::vec![0u8; 256];
        loop {
            match self.encode(framing, &mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    return Ok(buf);
                }
                Err(Error::BufferTooSmall) if buf.len() < 0x10_0000 => {
                    let len = buf.len() * 2;
                    buf.resize(len, 0);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

macro_rules! typed {
    ($ty:ident $(<$lt:lifetime>)?, $variant:ident) => {
        impl<'a> $ty $(<$lt>)? {
            /// Decodes this message from bytes (with or without `F0`/`F7`),
            /// returning its envelope.
            ///
            /// # Errors
            ///
            /// As [`Message::decode`], plus [`Error::UnexpectedKind`] if the
            /// bytes are a different message.
            pub fn decode(bytes: &'a [u8]) -> Result<(Envelope, Self), Error> {
                let m = Message::decode(bytes)?;
                match m.body {
                    Body::$variant(b) => Ok((m.envelope, b)),
                    other => Err(Error::UnexpectedKind(other.kind())),
                }
            }

            /// Encodes this message into `out`, returning the length.
            ///
            /// # Errors
            ///
            /// As [`Message::encode`].
            pub fn encode(
                &self,
                envelope: Envelope,
                framing: Framing,
                out: &mut [u8],
            ) -> Result<usize, Error> {
                Message {
                    envelope,
                    body: Body::$variant(*self),
                }
                .encode(framing, out)
            }
        }

        impl<'a> From<$ty $(<$lt>)?> for Body<'a> {
            fn from(b: $ty $(<$lt>)?) -> Self {
                Self::$variant(b)
            }
        }
    };
}

typed!(Discovery, Discovery);
typed!(DiscoveryReply, DiscoveryReply);
typed!(EndpointInquiry, EndpointInquiry);
typed!(EndpointReply<'a>, EndpointReply);
typed!(Ack<'a>, Ack);
typed!(InvalidateMuid, InvalidateMuid);
typed!(Nak<'a>, Nak);
typed!(ProfileInquiry, ProfileInquiry);
typed!(ProfileInquiryReply<'a>, ProfileInquiryReply);
typed!(SetProfileOff, SetProfileOff);
typed!(ProfileDetailsInquiry, ProfileDetailsInquiry);
typed!(ProfileDetailsReply<'a>, ProfileDetailsReply);
typed!(ProfileSpecificData<'a>, ProfileSpecificData);
typed!(ProcessInquiry, ProcessInquiry);
typed!(ProcessInquiryReply, ProcessInquiryReply);
typed!(MessageReportInquiry, MessageReportInquiry);
typed!(MessageReportReply, MessageReportReply);
typed!(EndOfMessageReport, EndOfMessageReport);
typed!(SetNewProtocol, SetNewProtocol);
typed!(ProtocolEstablished, ProtocolEstablished);
