//! Property Exchange Header Data (M2-103 §5): one flat JSON object.
//!
//! [`Header`] covers every common property: request (§5.2), reply (§5.3),
//! pagination (§6.6.2), partial set (§8), subscription (§9.1), media type
//! (§5.5), and the State Resource extras (M2-111 §3). Strings borrow from
//! the input. Unknown properties are skipped; nested values are skipped too
//! (headers may only hold numbers, booleans and strings, §5.1.1).

use crate::error::Error;
use crate::pe::json::{Event, JsonStr, JsonWriter, Parser};
use crate::pe::sink::{Sink, SliceSink};
use crate::pe::status::PeStatus;

/// Longest Header Data this crate accepts or writes. A header is never
/// chunked (M2-101 §8.3.2), so it must fit one message; 14-bit length.
pub const MAX_HEADER: usize = 0x3FFF;

/// `mutualEncoding` values (M2-103 §4.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Encoding {
    /// `"ASCII"`: 7-bit data as is.
    #[default]
    Ascii,
    /// `"Mcoded7"`.
    Mcoded7,
    /// `"zlib+Mcoded7"`.
    ZlibMcoded7,
}

impl Encoding {
    /// The header value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ascii => "ASCII",
            Self::Mcoded7 => "Mcoded7",
            Self::ZlibMcoded7 => "zlib+Mcoded7",
        }
    }

    /// Parses a header value, ignoring ASCII case (M2-103 §12.2's example
    /// writes `"MCoded7"`).
    #[must_use]
    pub fn parse(s: JsonStr<'_>) -> Option<Self> {
        [Self::Ascii, Self::Mcoded7, Self::ZlibMcoded7]
            .into_iter()
            .find(|e| s.eq_ignore_ascii_case(e.as_str()))
    }
}

/// Subscription `command` values (M2-103 §9.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Command {
    /// Initiator starts a subscription.
    Start,
    /// Responder sends a partial update.
    Partial,
    /// Responder sends the full data.
    Full,
    /// Responder asks the Initiator to re-Get.
    Notify,
    /// Either side ends the subscription.
    End,
}

impl Command {
    /// The header value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Partial => "partial",
            Self::Full => "full",
            Self::Notify => "notify",
            Self::End => "end",
        }
    }

    fn parse(s: JsonStr<'_>) -> Option<Self> {
        [
            Self::Start,
            Self::Partial,
            Self::Full,
            Self::Notify,
            Self::End,
        ]
        .into_iter()
        .find(|c| s.eq_str(c.as_str()))
    }
}

/// Typed Header Data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Header<'a> {
    /// `resource` (requests; max 36 chars).
    pub resource: Option<JsonStr<'a>>,
    /// `resId` (max 36 chars).
    pub res_id: Option<JsonStr<'a>>,
    /// `command` (subscriptions).
    pub command: Option<Command>,
    /// `subscribeId` (max 8 chars).
    pub subscribe_id: Option<JsonStr<'a>>,
    /// `status` (replies, Notify).
    pub status: Option<PeStatus>,
    /// `message` (replies; max 512 bytes escaped).
    pub message: Option<JsonStr<'a>>,
    /// `mutualEncoding`.
    pub mutual_encoding: Option<Encoding>,
    /// `mediaType` (max 75 chars).
    pub media_type: Option<JsonStr<'a>>,
    /// `offset` (pagination).
    pub offset: Option<u32>,
    /// `limit` (pagination).
    pub limit: Option<u32>,
    /// `totalCount` (paginated replies).
    pub total_count: Option<u32>,
    /// `setPartial` (partial Set, §8).
    pub set_partial: bool,
    /// `cacheTime` seconds (replies).
    pub cache_time: Option<u32>,
    /// `stateRev` (State Resource, M2-111 §3.1.1).
    pub state_rev: Option<JsonStr<'a>>,
    /// `timestamp` (State Resource, Unix seconds).
    pub timestamp: Option<u64>,
}

fn number(ev: Event<'_>) -> Option<u64> {
    match ev {
        Event::Number(n) => n.as_u64(),
        _ => None,
    }
}

const fn string(ev: Event<'_>) -> Option<JsonStr<'_>> {
    match ev {
        Event::Str(s) => Some(s),
        _ => None,
    }
}

impl<'a> Header<'a> {
    /// A request header for `resource`.
    #[must_use]
    pub const fn request(resource: &'a str) -> Self {
        Self {
            resource: Some(JsonStr::plain(resource)),
            ..Self::EMPTY
        }
    }

    /// A reply header with `status`.
    #[must_use]
    pub const fn reply(status: PeStatus) -> Self {
        Self {
            status: Some(status),
            ..Self::EMPTY
        }
    }

    /// No properties.
    pub const EMPTY: Self = Self {
        resource: None,
        res_id: None,
        command: None,
        subscribe_id: None,
        status: None,
        message: None,
        mutual_encoding: None,
        media_type: None,
        offset: None,
        limit: None,
        total_count: None,
        set_partial: false,
        cache_time: None,
        state_rev: None,
        timestamp: None,
    };

    /// Parses Header Data. An empty header (length 0) parses as
    /// [`Header::EMPTY`].
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] if it is not one JSON object, or is longer
    /// than [`MAX_HEADER`].
    ///
    /// # Example
    ///
    /// ```
    /// use midici::pe::header::{Encoding, Header};
    ///
    /// let h = Header::parse(br#"{"resource":"State","resId":"buffer","mutualEncoding":"Mcoded7"}"#)?;
    /// assert!(h.is_resource("State"));
    /// assert!(h.res_id.is_some_and(|r| r.eq_str("buffer")));
    /// assert_eq!(h.encoding(), Encoding::Mcoded7);
    ///
    /// // Writing is compact and puts `resource` first (M2-103 §5.1.1).
    /// let mut buf = [0u8; 64];
    /// let n = Header::request("DeviceInfo").write_to(&mut buf)?;
    /// assert_eq!(&buf[..n], br#"{"resource":"DeviceInfo"}"#);
    /// # Ok::<(), midici::Error>(())
    /// ```
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let mut h = Self::EMPTY;
        if bytes.is_empty() {
            return Ok(h);
        }
        if bytes.len() > MAX_HEADER {
            return Err(Error::InvalidField("header too long"));
        }
        let mut p = Parser::new(bytes);
        if p.expect_event()? != Event::ObjectStart {
            return Err(Error::InvalidField("header is not an object"));
        }
        loop {
            let key = match p.expect_event()? {
                Event::ObjectEnd => break,
                Event::Key(k) => k,
                _ => return Err(Error::InvalidField("header")),
            };
            let v = p.expect_event()?;
            let k = |name: &str| key.eq_str(name);
            if k("resource") {
                h.resource = string(v);
            } else if k("resId") {
                h.res_id = string(v);
            } else if k("command") {
                h.command = string(v).and_then(Command::parse);
            } else if k("subscribeId") {
                h.subscribe_id = string(v);
            } else if k("status") {
                h.status = number(v)
                    .and_then(|n| u16::try_from(n).ok())
                    .map(PeStatus::new);
            } else if k("message") {
                h.message = string(v);
            } else if k("mutualEncoding") {
                h.mutual_encoding = string(v).and_then(Encoding::parse);
            } else if k("mediaType") {
                h.media_type = string(v);
            } else if k("offset") {
                h.offset = number(v).and_then(|n| u32::try_from(n).ok());
            } else if k("limit") {
                h.limit = number(v).and_then(|n| u32::try_from(n).ok());
            } else if k("totalCount") {
                h.total_count = number(v).and_then(|n| u32::try_from(n).ok());
            } else if k("setPartial") {
                h.set_partial = v == Event::Bool(true);
            } else if k("cacheTime") {
                h.cache_time = number(v).and_then(|n| u32::try_from(n).ok());
            } else if k("stateRev") {
                h.state_rev = string(v);
            } else if k("timestamp") {
                h.timestamp = number(v);
            }
            p.skip(v)?;
        }
        if p.next()?.is_some() {
            return Err(Error::InvalidField("header trailing data"));
        }
        Ok(h)
    }

    /// Writes the header as compact JSON (no whitespace, §5.1.1), with
    /// `command`, else `status`, else `resource` first (§5.1.1 ordering).
    pub fn write<S: Sink>(&self, w: &mut JsonWriter<S>) {
        w.begin_object();
        if let Some(c) = self.command {
            w.field_str("command", c.as_str());
        }
        if let Some(s) = self.status {
            w.field_uint("status", u64::from(s.value()));
        }
        let mut s = |k: &str, v: Option<JsonStr<'_>>| {
            if let Some(v) = v {
                w.key(k).json_str(v);
            }
        };
        s("resource", self.resource);
        s("resId", self.res_id);
        s("subscribeId", self.subscribe_id);
        s("message", self.message);
        s("mediaType", self.media_type);
        s("stateRev", self.state_rev);
        if let Some(e) = self.mutual_encoding {
            w.field_str("mutualEncoding", e.as_str());
        }
        for (k, v) in [
            ("offset", self.offset),
            ("limit", self.limit),
            ("totalCount", self.total_count),
            ("cacheTime", self.cache_time),
        ] {
            if let Some(v) = v {
                w.field_uint(k, u64::from(v));
            }
        }
        if let Some(t) = self.timestamp {
            w.field_uint("timestamp", t);
        }
        if self.set_partial {
            w.field_bool("setPartial", true);
        }
        w.end_object();
    }

    /// Writes the header into `buf`, returning the length.
    ///
    /// # Errors
    ///
    /// [`Error::BufferTooSmall`] if it does not fit.
    pub fn write_to(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut w = JsonWriter::new(SliceSink::new(buf));
        self.write(&mut w);
        w.finish()?.finish()
    }

    /// Sets `resource`.
    #[must_use]
    pub const fn with_resource(mut self, resource: &'a str) -> Self {
        self.resource = Some(JsonStr::plain(resource));
        self
    }

    /// Sets `resId`.
    #[must_use]
    pub const fn with_res_id(mut self, res_id: &'a str) -> Self {
        self.res_id = Some(JsonStr::plain(res_id));
        self
    }

    /// Sets `command` (subscriptions).
    #[must_use]
    pub const fn with_command(mut self, command: Command) -> Self {
        self.command = Some(command);
        self
    }

    /// Sets `subscribeId`.
    #[must_use]
    pub const fn with_subscribe_id(mut self, subscribe_id: &'a str) -> Self {
        self.subscribe_id = Some(JsonStr::plain(subscribe_id));
        self
    }

    /// Sets `status`.
    #[must_use]
    pub const fn with_status(mut self, status: PeStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Sets `message`.
    #[must_use]
    pub const fn with_message(mut self, message: &'a str) -> Self {
        self.message = Some(JsonStr::plain(message));
        self
    }

    /// Sets `mutualEncoding`.
    #[must_use]
    pub const fn with_encoding(mut self, encoding: Encoding) -> Self {
        self.mutual_encoding = Some(encoding);
        self
    }

    /// Sets `mediaType`.
    #[must_use]
    pub const fn with_media_type(mut self, media_type: &'a str) -> Self {
        self.media_type = Some(JsonStr::plain(media_type));
        self
    }

    /// Sets `offset` and `limit` (pagination, M2-103 §6.6.2).
    #[must_use]
    pub const fn with_page(mut self, offset: u32, limit: u32) -> Self {
        self.offset = Some(offset);
        self.limit = Some(limit);
        self
    }

    /// Sets `totalCount`.
    #[must_use]
    pub const fn with_total_count(mut self, total_count: u32) -> Self {
        self.total_count = Some(total_count);
        self
    }

    /// Marks a partial Set (`"setPartial":true`, M2-103 §8).
    #[must_use]
    pub const fn with_set_partial(mut self) -> Self {
        self.set_partial = true;
        self
    }

    /// Sets `cacheTime` seconds.
    #[must_use]
    pub const fn with_cache_time(mut self, seconds: u32) -> Self {
        self.cache_time = Some(seconds);
        self
    }

    /// Sets `stateRev` (M2-111).
    #[must_use]
    pub const fn with_state_rev(mut self, state_rev: &'a str) -> Self {
        self.state_rev = Some(JsonStr::plain(state_rev));
        self
    }

    /// Sets `timestamp` (Unix seconds, M2-111).
    #[must_use]
    pub const fn with_timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// Whether `resource` equals `name`.
    #[must_use]
    pub fn is_resource(&self, name: &str) -> bool {
        self.resource.is_some_and(|r| r.eq_str(name))
    }

    /// The encoding in use (`ASCII` when absent).
    #[must_use]
    pub fn encoding(&self) -> Encoding {
        self.mutual_encoding.unwrap_or_default()
    }

    /// The header as a `Vec`.
    #[cfg(feature = "alloc")]
    #[must_use]
    pub fn to_vec(&self) -> alloc::vec::Vec<u8> {
        let mut w = JsonWriter::new(alloc::vec::Vec::new());
        self.write(&mut w);
        w.finish().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spec_examples() {
        // M2-103 §6.6.2, §9.3.1, M2-111 §3.2.
        let h = Header::parse(br#"{"resource":"ExampleList","offset":10,"limit":5}"#).unwrap();
        assert!(h.is_resource("ExampleList"));
        assert_eq!((h.offset, h.limit), (Some(10), Some(5)));
        let h = Header::parse(br#"{"status":200,"subscribeId":"sub32847623"}"#).unwrap();
        assert_eq!(h.status, Some(PeStatus::OK));
        assert!(h.subscribe_id.unwrap().eq_str("sub32847623"));
        let h = Header::parse(
            br#"{"status":200,"mutualEncoding":"Mcoded7","mediaType":"application/octet-stream","stateRev":"buui890adj","timestamp":1586786400}"#,
        )
        .unwrap();
        assert_eq!(h.encoding(), Encoding::Mcoded7);
        assert_eq!(h.timestamp, Some(1_586_786_400));
        let h = Header::parse(br#"{"command":"start","resource":"CurrentMode","x":{"a":[1]}}"#)
            .unwrap();
        assert_eq!(h.command, Some(Command::Start));
        assert!(Header::parse(b"[1]").is_err());
        assert!(Header::parse(br#"{"a":1} "#).is_ok());
        assert!(Header::parse(br#"{"a":1}{"#).is_err());
    }

    #[test]
    fn writes_compact_in_spec_order() {
        let mut buf = [0u8; 128];
        let h = Header {
            res_id: Some(JsonStr::plain("abcd")),
            set_partial: true,
            ..Header::request("X-ProgramEdit")
        };
        let n = h.write_to(&mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            br#"{"resource":"X-ProgramEdit","resId":"abcd","setPartial":true}"#
        );
        let n = Header::reply(PeStatus::OK).write_to(&mut buf).unwrap();
        assert_eq!(&buf[..n], br#"{"status":200}"#);
        assert_eq!(
            Header::parse(&buf[..n]).unwrap(),
            Header::reply(PeStatus::OK)
        );
    }
}
