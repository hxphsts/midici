//! Management messages (Sub-ID#2 `0x70..=0x7F`, M2-101 §5.5–§5.11):
//! Discovery, Endpoint Information, Invalidate MUID, ACK and NAK.
//!
//! Every type decodes from borrowed bytes and encodes into a caller buffer;
//! none allocate. Fields added in Message Version 2 (MIDI-CI 1.2) are read
//! only when the received version declares them, and written only when the
//! envelope's version does, so 1.1 peers interoperate (M2-101 §5.3, §5.4).

use core::fmt;

use crate::codec::{Reader, Writer};
use crate::error::Error;
use crate::identity::DeviceIdentity;
use crate::muid::Muid;
use crate::wire::{CiVersion, MessageKind};

/// The "Capability Inquiry Category Supported" bitmap
/// (M2-101 §5.5.2, Table 7). Bit `n` is Category `n` (Sub-ID#2 `0xn0..=0xnF`).
///
/// Reserved bits are preserved, so a reply from a newer device round-trips.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Categories(u8);

impl Categories {
    /// Nothing beyond Management (which every CI device supports).
    pub const NONE: Self = Self(0);
    /// D1: Protocol Negotiation (deprecated in 1.2; do not set).
    pub const PROTOCOL_NEGOTIATION: Self = Self(1 << 1);
    /// D2: Profile Configuration.
    pub const PROFILE_CONFIGURATION: Self = Self(1 << 2);
    /// D3: Property Exchange.
    pub const PROPERTY_EXCHANGE: Self = Self(1 << 3);
    /// D4: Process Inquiry.
    pub const PROCESS_INQUIRY: Self = Self(1 << 4);

    /// From the wire byte (bit 7 is dropped).
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & 0x7F)
    }

    /// The wire byte.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every flag in `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Both sets of flags.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl core::ops::BitOr for Categories {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl fmt::Debug for Categories {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut l = f.debug_set();
        for (flag, name) in [
            (Self::PROTOCOL_NEGOTIATION, "ProtocolNegotiation"),
            (Self::PROFILE_CONFIGURATION, "ProfileConfiguration"),
            (Self::PROPERTY_EXCHANGE, "PropertyExchange"),
            (Self::PROCESS_INQUIRY, "ProcessInquiry"),
        ] {
            if self.contains(flag) {
                l.entry(&format_args!("{name}"));
            }
        }
        let reserved = self.0 & !0x1E;
        if reserved != 0 {
            l.entry(&format_args!("Reserved(0x{reserved:02X})"));
        }
        l.finish()
    }
}

/// Wire encoding of a message body. Implemented by every typed message.
pub(crate) trait Codec<'a>: Sized {
    fn read(r: &mut Reader<'a>, v: CiVersion) -> Result<Self, Error>;
    fn write(&self, w: &mut Writer<'_>, v: CiVersion) -> Result<(), Error>;
}

/// Discovery (`0x70`, M2-101 §5.5, Table 6). Sent by an Initiator to the
/// Broadcast MUID with Device ID `0x7F`.
///
/// ```text
/// manufacturer ×3, family ×2, model ×2, revision ×4, categories ×1,
/// max SysEx ×4, [v2: output path ×1]
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Discovery {
    /// Who the Initiator is.
    pub identity: DeviceIdentity,
    /// Categories the Initiator supports.
    pub categories: Categories,
    /// Receivable Maximum SysEx Message Size, 28 bits (M2-101 §5.5.3:
    /// at least 128; at least 512 for PE/Profile Initiators). Not enforced
    /// when decoding.
    pub max_sysex: u32,
    /// Initiator's Output Path Id (M2-101 §5.5.4; v2). `0` when decoded from
    /// a version 1 message.
    pub output_path: u8,
}

impl Codec<'_> for Discovery {
    fn read(r: &mut Reader<'_>, v: CiVersion) -> Result<Self, Error> {
        let identity = DeviceIdentity::read_ci(r)?;
        let categories = Categories::from_bits(r.u7()?);
        let max_sysex = r.u28()?;
        let output_path = if v.has_v2_fields() { r.u7()? } else { 0 };
        Ok(Self {
            identity,
            categories,
            max_sysex,
            output_path,
        })
    }

    fn write(&self, w: &mut Writer<'_>, v: CiVersion) -> Result<(), Error> {
        self.identity.write_ci(w)?;
        w.u7(self.categories.bits(), "categories")?;
        w.u28(self.max_sysex, "max SysEx size")?;
        if v.has_v2_fields() {
            w.u7(self.output_path, "output path ID")?;
        }
        Ok(())
    }
}

/// Reply to Discovery (`0x71`, M2-101 §5.6, Table 8).
///
/// ```text
/// manufacturer ×3, family ×2, model ×2, revision ×4, categories ×1,
/// max SysEx ×4, [v2: output path ×1, function block ×1]
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct DiscoveryReply {
    /// Who the Responder is.
    pub identity: DeviceIdentity,
    /// Categories the Responder supports.
    pub categories: Categories,
    /// Receivable Maximum SysEx Message Size, 28 bits.
    pub max_sysex: u32,
    /// The Output Path Id from the Discovery being answered (M2-101 §5.6.1;
    /// v2). `0` from a version 1 message.
    pub output_path: u8,
    /// The Function Block this MUID represents; `None` is sent as `0x7F`
    /// (M2-101 §5.6.2; v2). `None` from a version 1 message.
    pub function_block: Option<u8>,
}

/// The Function Block byte meaning "no Function Block" (M2-101 §5.6.2).
pub const NO_FUNCTION_BLOCK: u8 = 0x7F;

impl Codec<'_> for DiscoveryReply {
    fn read(r: &mut Reader<'_>, v: CiVersion) -> Result<Self, Error> {
        let identity = DeviceIdentity::read_ci(r)?;
        let categories = Categories::from_bits(r.u7()?);
        let max_sysex = r.u28()?;
        let (output_path, function_block) = if v.has_v2_fields() {
            let path = r.u7()?;
            let fb = r.u7()?;
            (path, (fb != NO_FUNCTION_BLOCK).then_some(fb))
        } else {
            (0, None)
        };
        Ok(Self {
            identity,
            categories,
            max_sysex,
            output_path,
            function_block,
        })
    }

    fn write(&self, w: &mut Writer<'_>, v: CiVersion) -> Result<(), Error> {
        self.identity.write_ci(w)?;
        w.u7(self.categories.bits(), "categories")?;
        w.u28(self.max_sysex, "max SysEx size")?;
        if v.has_v2_fields() {
            w.u7(self.output_path, "output path ID")?;
            match self.function_block {
                Some(NO_FUNCTION_BLOCK) => return Err(Error::OutOfRange("function block")),
                Some(fb) => w.u7(fb, "function block")?,
                None => w.raw(NO_FUNCTION_BLOCK)?,
            }
        }
        Ok(())
    }
}

/// Inquiry: Endpoint Information (`0x72`, M2-101 §5.7, Table 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct EndpointInquiry {
    /// Which information to return (M2-101 Table 10). Only
    /// [`EndpointInquiry::PRODUCT_INSTANCE_ID`] is defined.
    pub status: u8,
}

impl EndpointInquiry {
    /// Status `0x00`: the UMP Endpoint's Product Instance Id.
    pub const PRODUCT_INSTANCE_ID: u8 = 0x00;
}

impl Codec<'_> for EndpointInquiry {
    fn read(r: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self { status: r.u7()? })
    }

    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        w.u7(self.status, "endpoint status")
    }
}

/// Longest Product Instance Id (M2-101 §5.8.3.1).
pub const MAX_PRODUCT_INSTANCE_ID: usize = 16;

/// Reply to Endpoint Information (`0x73`, M2-101 §5.8, Table 11).
///
/// ```text
/// status ×1, length ×2 (LSB first), data ×length
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct EndpointReply<'a> {
    /// Echo of the inquiry's status.
    pub status: u8,
    /// Information Data, borrowed from the input.
    pub data: &'a [u8],
}

impl<'a> EndpointReply<'a> {
    /// A status-0 reply carrying a Product Instance Id.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] unless `id` is 1–16 bytes of ASCII `32..=126`
    /// (M2-101 §5.8.3.1).
    pub fn product_instance_id(id: &'a str) -> Result<Self, Error> {
        validate_product_instance_id(id.as_bytes())?;
        Ok(Self {
            status: EndpointInquiry::PRODUCT_INSTANCE_ID,
            data: id.as_bytes(),
        })
    }

    /// The Product Instance Id, if this is a status-0 reply.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] if the status is not `0x00` or the data is not
    /// a valid Product Instance Id.
    pub fn as_product_instance_id(&self) -> Result<&'a str, Error> {
        if self.status != EndpointInquiry::PRODUCT_INSTANCE_ID {
            return Err(Error::InvalidField("endpoint status"));
        }
        validate_product_instance_id(self.data)?;
        core::str::from_utf8(self.data).map_err(|_| Error::InvalidField("product instance ID"))
    }
}

fn validate_product_instance_id(id: &[u8]) -> Result<(), Error> {
    if id.is_empty()
        || id.len() > MAX_PRODUCT_INSTANCE_ID
        || !id.iter().all(|b| (32..=126).contains(b))
    {
        return Err(Error::InvalidField("product instance ID"));
    }
    Ok(())
}

impl<'a> Codec<'a> for EndpointReply<'a> {
    fn read(r: &mut Reader<'a>, _: CiVersion) -> Result<Self, Error> {
        let status = r.u7()?;
        let len = r.u14()?;
        Ok(Self {
            status,
            data: r.bytes(usize::from(len))?,
        })
    }

    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        w.u7(self.status, "endpoint status")?;
        let len = u16::try_from(self.data.len()).map_err(|_| Error::OutOfRange("endpoint data"))?;
        w.u14(len, "endpoint data length")?;
        w.bytes(self.data, "endpoint data")
    }
}

/// Invalidate MUID (`0x7E`, M2-101 §5.9, Table 12). Sent to the Broadcast
/// MUID; no reply is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct InvalidateMuid {
    /// The MUID to invalidate.
    pub target: Muid,
}

impl Codec<'_> for InvalidateMuid {
    fn read(r: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self { target: r.muid()? })
    }

    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        w.muid(self.target)
    }
}

/// Human-readable Message Text in ACK and NAK (M2-101 §5.10.4).
///
/// Always 7-bit ASCII; characters outside ASCII travel as JSON `\uXXXX`
/// escapes, which this type does not interpret. Length is at most 16383
/// bytes (a 14-bit field); stay within the peer's Receivable Maximum SysEx
/// Message Size (103 bytes is safe at the 128-byte minimum).
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Text<'a>(&'a str);

impl<'a> Text<'a> {
    /// No text.
    pub const EMPTY: Self = Self("");
    /// Longest text the 14-bit length field can describe.
    pub const MAX_LEN: usize = 0x3FFF;

    /// Wraps ASCII text.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] if `s` is not ASCII or is longer than
    /// [`Text::MAX_LEN`].
    pub const fn new(s: &'a str) -> Result<Self, Error> {
        if !s.is_ascii() || s.len() > Self::MAX_LEN {
            return Err(Error::InvalidField("message text"));
        }
        Ok(Self(s))
    }

    /// The text, still JSON-escaped.
    #[must_use]
    pub const fn as_str(&self) -> &'a str {
        self.0
    }

    fn read(r: &mut Reader<'a>) -> Result<Self, Error> {
        let len = r.u14()?;
        let bytes = r.bytes(usize::from(len))?;
        let s = core::str::from_utf8(bytes).map_err(|_| Error::InvalidField("message text"))?;
        Self::new(s)
    }

    fn write(&self, w: &mut Writer<'_>) -> Result<(), Error> {
        let len = u16::try_from(self.0.len()).map_err(|_| Error::OutOfRange("message text"))?;
        w.u14(len, "message text length")?;
        w.bytes(self.0.as_bytes(), "message text")
    }
}

impl fmt::Debug for Text<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0, f)
    }
}

impl fmt::Display for Text<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// An ACK Status Code (M2-101 §5.10.2, Table 14). Open-ended: unknown codes
/// are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AckStatus(u8);

impl AckStatus {
    /// An ACK Status Code.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] if `code` exceeds 7 bits.
    pub const fn new(code: u8) -> Result<Self, Error> {
        if code > 0x7F {
            Err(Error::OutOfRange("ACK status code"))
        } else {
            Ok(Self(code))
        }
    }

    /// The code.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }

    /// `0x00`: ACK.
    pub const ACK: Self = Self(0x00);
    /// `0x10`: Timeout Wait; Status Data is the wait in 100 ms units.
    pub const TIMEOUT_WAIT: Self = Self(0x10);

    /// `0x00..=0x0F`: success, do not retry.
    #[must_use]
    pub const fn is_success(self) -> bool {
        self.0 <= 0x0F
    }

    /// `0x10..=0x1F`: notification requiring further action.
    #[must_use]
    pub const fn is_notification(self) -> bool {
        self.0 >= 0x10 && self.0 <= 0x1F
    }
}

/// A NAK Status Code (M2-101 §5.11.2, Table 16). Open-ended: unknown codes
/// are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NakStatus(u8);

impl NakStatus {
    /// A NAK Status Code.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] if `code` exceeds 7 bits.
    pub const fn new(code: u8) -> Result<Self, Error> {
        if code > 0x7F {
            Err(Error::OutOfRange("NAK status code"))
        } else {
            Ok(Self(code))
        }
    }

    /// The code.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }

    /// `0x00`: NAK.
    pub const NAK: Self = Self(0x00);
    /// `0x01`: MIDI-CI message not supported.
    pub const NOT_SUPPORTED: Self = Self(0x01);
    /// `0x02`: MIDI-CI version not supported.
    pub const VERSION_NOT_SUPPORTED: Self = Self(0x02);
    /// `0x03`: Channel/Group/Function Block not in use.
    pub const TARGET_NOT_IN_USE: Self = Self(0x03);
    /// `0x04`: Profile not supported on the requested target.
    pub const PROFILE_NOT_SUPPORTED: Self = Self(0x04);
    /// `0x20`: Terminate Inquiry (replaces PE Notify status 144).
    pub const TERMINATE_INQUIRY: Self = Self(0x20);
    /// `0x21`: Property Exchange chunks out of sequence.
    pub const PE_CHUNKS_OUT_OF_SEQUENCE: Self = Self(0x21);
    /// `0x40`: Error occurred, please retry.
    pub const RETRY: Self = Self(0x40);
    /// `0x41`: Message was malformed.
    pub const MALFORMED: Self = Self(0x41);
    /// `0x42`: Timeout has occurred.
    pub const TIMEOUT: Self = Self(0x42);
    /// `0x43`: Busy; Status Data is the retry delay in 100 ms units.
    pub const BUSY: Self = Self(0x43);

    /// `0x00..=0x1F`: failure, do not retry.
    #[must_use]
    pub const fn is_fatal(self) -> bool {
        self.0 <= 0x1F
    }

    /// `0x20..=0x3F`: notification, further action recommended.
    #[must_use]
    pub const fn is_notification(self) -> bool {
        self.0 >= 0x20 && self.0 <= 0x3F
    }

    /// `0x40..=0x5F`: failure, retry recommended.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        self.0 >= 0x40 && self.0 <= 0x5F
    }
}

/// The five per-category detail bytes of ACK and NAK (M2-101 §5.10.3,
/// §5.11.3).
///
/// A Profile ID for Profile Configuration; the PE stream (Request) ID and
/// chunk number for Property Exchange; reserved otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Details([u8; 5]);

impl Details {
    /// All zero (no details).
    pub const NONE: Self = Self([0; 5]);

    /// Details from five 7-bit bytes (e.g. a Profile ID).
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] if a byte exceeds 7 bits.
    pub const fn new(bytes: [u8; 5]) -> Result<Self, Error> {
        let [b0, b1, b2, b3, b4] = bytes;
        if (b0 | b1 | b2 | b3 | b4) & 0x80 != 0 {
            Err(Error::OutOfRange("details"))
        } else {
            Ok(Self(bytes))
        }
    }

    /// Profile Configuration details: the Profile ID (§5.11.3).
    #[must_use]
    pub const fn from_profile(profile: crate::profile::ProfileId) -> Self {
        Self(profile.bytes())
    }

    /// The five bytes.
    #[must_use]
    pub const fn bytes(self) -> [u8; 5] {
        self.0
    }

    /// Property Exchange details: byte 1 the stream ID, bytes 2–3 the chunk
    /// number (LSB first), the rest zero.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] if `request_id > 0x7F` or `chunk > 0x3FFF`.
    pub const fn property_exchange(request_id: u8, chunk: u16) -> Result<Self, Error> {
        if request_id > 0x7F || chunk > 0x3FFF {
            return Err(Error::OutOfRange("PE details"));
        }
        // Masked to 7 bits.
        #[allow(clippy::cast_possible_truncation)]
        Ok(Self([
            request_id,
            (chunk & 0x7F) as u8,
            (chunk >> 7) as u8,
            0,
            0,
        ]))
    }

    /// Read as Property Exchange details: `(request_id, chunk)`.
    #[must_use]
    pub const fn as_property_exchange(self) -> (u8, u16) {
        let [id, lo, hi, _, _] = self.0;
        (id, lo as u16 | (hi as u16) << 7)
    }
}

/// The fields shared by ACK and the version 2 NAK (M2-101 Tables 13, 15).
///
/// ```text
/// original Sub-ID#2 ×1, status ×1, status data ×1, details ×5,
/// text length ×2 (LSB first), text ×length
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Report<'a, S> {
    /// The message being acknowledged or refused.
    pub original: MessageKind,
    /// Status Code.
    pub status: S,
    /// Status Data (meaning depends on the code; `0` when unused).
    pub status_data: u8,
    /// Per-category details.
    pub details: Details,
    /// Message Text.
    pub text: Text<'a>,
}

trait ReportCodec<'a>: Sized {
    fn read_report(r: &mut Reader<'a>) -> Result<Self, Error>;
    fn write_report(&self, w: &mut Writer<'_>) -> Result<(), Error>;
}

impl<'a, S: StatusCode> ReportCodec<'a> for Report<'a, S> {
    fn read_report(r: &mut Reader<'a>) -> Result<Self, Error> {
        Ok(Self {
            original: MessageKind::from_sub_id2(r.u7()?),
            status: S::from_wire(r.u7()?),
            status_data: r.u7()?,
            details: Details(r.array()?),
            text: Text::read(r)?,
        })
    }

    fn write_report(&self, w: &mut Writer<'_>) -> Result<(), Error> {
        w.u7(self.original.sub_id2(), "original Sub-ID#2")?;
        w.u7(self.status.wire(), "status code")?;
        w.u7(self.status_data, "status data")?;
        w.bytes(&self.details.0, "details")?;
        self.text.write(w)
    }
}

/// Wire conversion for status codes (crate-private; values read from the
/// wire are already 7-bit).
pub(crate) trait StatusCode: Copy {
    fn from_wire(b: u8) -> Self;
    fn wire(self) -> u8;
}

impl StatusCode for AckStatus {
    fn from_wire(b: u8) -> Self {
        Self(b & 0x7F)
    }
    fn wire(self) -> u8 {
        self.0
    }
}

impl StatusCode for NakStatus {
    fn from_wire(b: u8) -> Self {
        Self(b & 0x7F)
    }
    fn wire(self) -> u8 {
        self.0
    }
}

/// MIDI-CI ACK (`0x7D`, M2-101 §5.10, Table 13). New in MIDI-CI 1.2; its
/// fields are required whatever the message version.
pub type Ack<'a> = Report<'a, AckStatus>;

impl Ack<'_> {
    /// A plain ACK of `original`.
    #[must_use]
    pub const fn ack(original: MessageKind) -> Self {
        Self {
            original,
            status: AckStatus::ACK,
            status_data: 0,
            details: Details([0; 5]),
            text: Text::EMPTY,
        }
    }
}

impl<'a> Codec<'a> for Ack<'a> {
    fn read(r: &mut Reader<'a>, _: CiVersion) -> Result<Self, Error> {
        Self::read_report(r)
    }

    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        self.write_report(w)
    }
}

/// MIDI-CI NAK (`0x7F`, M2-101 §5.11, Table 15).
///
/// A version 1 NAK (MIDI-CI 1.1) has no data after the MUIDs; version 2
/// adds a [`Report`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Nak<'a> {
    /// `None` for a version 1 NAK.
    pub report: Option<Report<'a, NakStatus>>,
}

impl<'a> Nak<'a> {
    /// A version 2 NAK of `original` with `status`, no data, no text.
    #[must_use]
    pub const fn new(original: MessageKind, status: NakStatus) -> Self {
        Self {
            report: Some(Report {
                original,
                status,
                status_data: 0,
                details: Details([0; 5]),
                text: Text::EMPTY,
            }),
        }
    }

    /// The same NAK with Message Text.
    #[must_use]
    pub const fn with_text(mut self, text: Text<'a>) -> Self {
        if let Some(r) = &mut self.report {
            r.text = text;
        }
        self
    }
}

impl<'a> Codec<'a> for Nak<'a> {
    fn read(r: &mut Reader<'a>, v: CiVersion) -> Result<Self, Error> {
        let report = if v.has_v2_fields() {
            Some(Report::read_report(r)?)
        } else {
            None
        };
        Ok(Self { report })
    }

    fn write(&self, w: &mut Writer<'_>, v: CiVersion) -> Result<(), Error> {
        match (&self.report, v.has_v2_fields()) {
            (Some(report), true) => report.write_report(w),
            (_, false) => Ok(()),
            (None, true) => Err(Error::InvalidField("version 2 NAK needs a report")),
        }
    }
}

impl Discovery {
    /// A Discovery with Output Path Id 0.
    #[must_use]
    pub const fn new(identity: DeviceIdentity, categories: Categories, max_sysex: u32) -> Self {
        Self {
            identity,
            categories,
            max_sysex,
            output_path: 0,
        }
    }

    /// Sets the Output Path Id (M2-101 §5.5.4).
    #[must_use]
    pub const fn with_output_path(mut self, output_path: u8) -> Self {
        self.output_path = output_path;
        self
    }
}

impl DiscoveryReply {
    /// A Reply to Discovery with Output Path Id 0 and no Function Block.
    #[must_use]
    pub const fn new(identity: DeviceIdentity, categories: Categories, max_sysex: u32) -> Self {
        Self {
            identity,
            categories,
            max_sysex,
            output_path: 0,
            function_block: None,
        }
    }

    /// Sets the echoed Output Path Id (M2-101 §5.6.1).
    #[must_use]
    pub const fn with_output_path(mut self, output_path: u8) -> Self {
        self.output_path = output_path;
        self
    }

    /// Sets the Function Block (`None` is sent as `0x7F`, §5.6.2).
    #[must_use]
    pub const fn with_function_block(mut self, function_block: Option<u8>) -> Self {
        self.function_block = function_block;
        self
    }
}

impl EndpointInquiry {
    /// An inquiry for `status` (see [`EndpointInquiry::PRODUCT_INSTANCE_ID`]).
    #[must_use]
    pub const fn new(status: u8) -> Self {
        Self { status }
    }
}

impl<'a> EndpointReply<'a> {
    /// A reply carrying `data` for `status`.
    #[must_use]
    pub const fn new(status: u8, data: &'a [u8]) -> Self {
        Self { status, data }
    }
}

impl InvalidateMuid {
    /// Invalidate `target`.
    #[must_use]
    pub const fn new(target: Muid) -> Self {
        Self { target }
    }
}

impl<'a, S> Report<'a, S> {
    /// A report about `original` with `status`, no data, no details, no
    /// text.
    #[must_use]
    pub const fn new(original: MessageKind, status: S) -> Self {
        Self {
            original,
            status,
            status_data: 0,
            details: Details::NONE,
            text: Text::EMPTY,
        }
    }

    /// Sets the Status Data byte.
    #[must_use]
    pub const fn with_status_data(mut self, status_data: u8) -> Self {
        self.status_data = status_data;
        self
    }

    /// Sets the per-category details.
    #[must_use]
    pub const fn with_details(mut self, details: Details) -> Self {
        self.details = details;
        self
    }

    /// Sets the Message Text.
    #[must_use]
    pub const fn with_text(mut self, text: Text<'a>) -> Self {
        self.text = text;
        self
    }
}

impl<'a> Nak<'a> {
    /// A MIDI-CI 1.1 NAK: no data after the MUIDs. Only valid when encoded
    /// with a version 1 envelope.
    #[must_use]
    pub const fn v1() -> Self {
        Self { report: None }
    }

    /// A version 2 NAK carrying `report`.
    #[must_use]
    pub const fn from_report(report: Report<'a, NakStatus>) -> Self {
        Self {
            report: Some(report),
        }
    }

    /// The same NAK with these details (no effect on a version 1 NAK).
    #[must_use]
    pub const fn with_details(mut self, details: Details) -> Self {
        if let Some(r) = &mut self.report {
            r.details = details;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_bits() {
        let c = Categories::PROFILE_CONFIGURATION | Categories::PROPERTY_EXCHANGE;
        assert_eq!(c.bits(), 0b0000_1100);
        assert!(c.contains(Categories::PROPERTY_EXCHANGE));
        assert!(!c.contains(Categories::PROCESS_INQUIRY));
        assert_eq!(Categories::from_bits(0xFF).bits(), 0x7F);
    }

    #[test]
    fn pe_details() {
        let d = Details::property_exchange(3, 200).unwrap();
        assert_eq!(d.0, [3, 0x48, 1, 0, 0]);
        assert_eq!(d.as_property_exchange(), (3, 200));
        assert!(Details::property_exchange(0x80, 0).is_err());
    }

    #[test]
    fn product_instance_id_rules() {
        assert!(EndpointReply::product_instance_id("SN-0001").is_ok());
        assert!(EndpointReply::product_instance_id("").is_err());
        assert!(EndpointReply::product_instance_id("0123456789ABCDEFG").is_err());
        assert!(EndpointReply::product_instance_id("tab\there").is_err());
    }

    #[test]
    fn status_classes() {
        assert!(NakStatus::VERSION_NOT_SUPPORTED.is_fatal());
        assert!(NakStatus::TERMINATE_INQUIRY.is_notification());
        assert!(NakStatus::BUSY.is_retryable());
        assert!(AckStatus::TIMEOUT_WAIT.is_notification());
    }
}
