//! Typed Property Exchange resources.
//!
//! Every type is generic over its string type `S`: a Responder uses
//! `&'static str` (or any `AsRef<str>`) and writes JSON with no allocator;
//! an Initiator parses replies into `String`s (feature `alloc`).
//!
//! | Resource | Spec | Type |
//! |---|---|---|
//! | `ResourceList` | M2-103 §12 | [`ResourceDecl`] / [`ResourceInfo`] |
//! | `DeviceInfo` | M2-105 §3 | [`DeviceInfo`] |
//! | `ChannelList` | M2-105 §4 | [`ChannelEntry`] |
//! | `JSONSchema` | M2-105 §5 | raw JSON |
//! | `AllCtrlList`, `ChCtrlList` | M2-117 §2, §3 | [`Controller`] |
//! | `CtrlMapList` | M2-117 §4 | [`CtrlMapEntry`] |
//! | `ProgramList` | M2-107 (not public; minimal) | [`ProgramEntry`] |
//! | `StateList` / `State` | M2-111 §2, §3 | [`StateEntry`] / raw bytes |

use crate::pe::json::JsonWriter;
use crate::pe::sink::Sink;

/// Resource names defined by AMEI/MMA specifications.
pub mod names {
    /// M2-103 §12.
    pub const RESOURCE_LIST: &str = "ResourceList";
    /// M2-105 §3.
    pub const DEVICE_INFO: &str = "DeviceInfo";
    /// M2-105 §4.
    pub const CHANNEL_LIST: &str = "ChannelList";
    /// M2-105 §5.
    pub const JSON_SCHEMA: &str = "JSONSchema";
    /// M2-117 §2.
    pub const ALL_CTRL_LIST: &str = "AllCtrlList";
    /// M2-117 §3.
    pub const CH_CTRL_LIST: &str = "ChCtrlList";
    /// M2-117 §4.
    pub const CTRL_MAP_LIST: &str = "CtrlMapList";
    /// M2-107.
    pub const PROGRAM_LIST: &str = "ProgramList";
    /// M2-111 §2.
    pub const STATE_LIST: &str = "StateList";
    /// M2-111 §3.
    pub const STATE: &str = "State";
}

/// `canSet` (M2-103 §12.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CanSet {
    /// `"none"` (default).
    #[default]
    None,
    /// `"full"`.
    Full,
    /// `"partial"` (implies full).
    Partial,
}

impl CanSet {
    /// The JSON value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Full => "full",
            Self::Partial => "partial",
        }
    }
}

/// A ResourceList entry for a Responder to declare (M2-103 §12.2,
/// §12.2.1). Written minimised: properties equal to their defaults are
/// omitted (§12.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // mirrors the spec's properties
pub struct ResourceDecl<'a> {
    /// Resource name.
    pub resource: &'a str,
    /// `canGet` (default true).
    pub can_get: bool,
    /// `canSet` (default none).
    pub can_set: CanSet,
    /// `canSubscribe` (default false).
    pub can_subscribe: bool,
    /// `requireResId` (default false).
    pub require_res_id: bool,
    /// `canPaginate` (default false; list resources).
    pub can_paginate: bool,
    /// `mediaTypes` (default `["application/json"]`; empty = default).
    pub media_types: &'a [&'a str],
    /// `encodings` (default `["ASCII"]`; empty = default).
    pub encodings: &'a [&'a str],
    /// `schema`: a raw JSON value, written verbatim.
    pub schema: Option<&'a str>,
}

impl<'a> ResourceDecl<'a> {
    /// A Get-only resource with every default.
    #[must_use]
    pub const fn new(resource: &'a str) -> Self {
        Self {
            resource,
            can_get: true,
            can_set: CanSet::None,
            can_subscribe: false,
            require_res_id: false,
            can_paginate: false,
            media_types: &[],
            encodings: &[],
            schema: None,
        }
    }

    /// Writes the entry object.
    pub fn write<W: Sink>(&self, w: &mut JsonWriter<W>) {
        w.begin_object().field_str("resource", self.resource);
        if !self.can_get {
            w.field_bool("canGet", false);
        }
        if self.can_set != CanSet::None {
            w.field_str("canSet", self.can_set.as_str());
        }
        if self.can_subscribe {
            w.field_bool("canSubscribe", true);
        }
        if self.require_res_id {
            w.field_bool("requireResId", true);
        }
        if self.can_paginate {
            w.field_bool("canPaginate", true);
        }
        for (k, list) in [
            ("mediaTypes", self.media_types),
            ("encodings", self.encodings),
        ] {
            if !list.is_empty() {
                w.key(k).begin_array();
                for s in list {
                    w.string(s);
                }
                w.end_array();
            }
        }
        if let Some(schema) = self.schema {
            w.key("schema").raw_value(schema.as_bytes());
        }
        w.end_object();
    }
}

/// Writes a whole ResourceList reply body (never listing ResourceList
/// itself, M2-103 §7.1).
pub fn write_resource_list<W: Sink>(decls: &[ResourceDecl<'_>], w: &mut JsonWriter<W>) {
    w.begin_array();
    for d in decls.iter().filter(|d| d.resource != names::RESOURCE_LIST) {
        d.write(w);
    }
    w.end_array();
}

/// DeviceInfo (M2-105 §3.2, Table 5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct DeviceInfo<S> {
    /// Same 3 bytes as Discovery.
    pub manufacturer_id: [u8; 3],
    /// Same 2 bytes as Discovery (LSB first).
    pub family_id: [u8; 2],
    /// Same 2 bytes as Discovery (LSB first).
    pub model_id: [u8; 2],
    /// Same 4 bytes as Discovery.
    pub version_id: [u8; 4],
    /// Manufacturer name.
    pub manufacturer: S,
    /// Family name.
    pub family: S,
    /// Model name.
    pub model: S,
    /// Version string.
    pub version: S,
    /// Optional serial number.
    pub serial_number: Option<S>,
}

impl<S: AsRef<str>> DeviceInfo<S> {
    /// Writes the object.
    pub fn write<W: Sink>(&self, w: &mut JsonWriter<W>) {
        w.begin_object()
            .field_uints("manufacturerId", &self.manufacturer_id)
            .field_str("manufacturer", self.manufacturer.as_ref())
            .field_uints("familyId", &self.family_id)
            .field_str("family", self.family.as_ref())
            .field_uints("modelId", &self.model_id)
            .field_str("model", self.model.as_ref())
            .field_uints("versionId", &self.version_id)
            .field_str("version", self.version.as_ref());
        if let Some(s) = &self.serial_number {
            w.field_str("serialNumber", s.as_ref());
        }
        w.end_object();
    }
}

/// One ChannelList entry (M2-105 §4.3, Table 10).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct ChannelEntry<S> {
    /// Name of the channel.
    pub title: S,
    /// Channel 1–256 (across Groups of the Function Block).
    pub channel: u16,
    /// Current program name.
    pub program_title: Option<S>,
    /// Bank MSB, Bank LSB, Program.
    #[cfg_attr(feature = "serde", serde(rename = "bankPC"))]
    pub bank_pc: Option<[u8; 3]>,
    /// First channel of the cluster.
    pub cluster_channel_start: Option<u16>,
    /// Cluster length.
    pub cluster_length: Option<u16>,
    /// MIDI Mode 1–4.
    pub cluster_midi_mode: Option<u8>,
    /// `"other"`, `"profile"`, `"mpe1"`.
    pub cluster_type: Option<S>,
}

impl<S: AsRef<str>> ChannelEntry<S> {
    /// Writes the object.
    pub fn write<W: Sink>(&self, w: &mut JsonWriter<W>) {
        w.begin_object()
            .field_str("title", self.title.as_ref())
            .field_uint("channel", u64::from(self.channel));
        if let Some(p) = &self.program_title {
            w.field_str("programTitle", p.as_ref());
        }
        if let Some(b) = &self.bank_pc {
            w.field_uints("bankPC", b);
        }
        for (k, v) in [
            ("clusterChannelStart", self.cluster_channel_start),
            ("clusterLength", self.cluster_length),
            ("clusterMidiMode", self.cluster_midi_mode.map(u16::from)),
        ] {
            if let Some(v) = v {
                w.field_uint(k, u64::from(v));
            }
        }
        if let Some(t) = &self.cluster_type {
            w.field_str("clusterType", t.as_ref());
        }
        w.end_object();
    }
}

/// `ctrlType` (M2-117 Table 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CtrlType {
    /// `"cc"`.
    #[default]
    Cc,
    /// `"chPress"`.
    ChPress,
    /// `"pPress"`.
    PPress,
    /// `"nrpn"`.
    Nrpn,
    /// `"rpn"`.
    Rpn,
    /// `"pBend"`.
    PBend,
    /// `"pnrc"` (MIDI 2.0).
    Pnrc,
    /// `"pnac"` (MIDI 2.0).
    Pnac,
    /// `"pnp"` (MIDI 2.0).
    Pnp,
}

impl CtrlType {
    const ALL: [Self; 9] = [
        Self::Cc,
        Self::ChPress,
        Self::PPress,
        Self::Nrpn,
        Self::Rpn,
        Self::PBend,
        Self::Pnrc,
        Self::Pnac,
        Self::Pnp,
    ];

    /// The JSON value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cc => "cc",
            Self::ChPress => "chPress",
            Self::PPress => "pPress",
            Self::Nrpn => "nrpn",
            Self::Rpn => "rpn",
            Self::PBend => "pBend",
            Self::Pnrc => "pnrc",
            Self::Pnac => "pnac",
            Self::Pnp => "pnp",
        }
    }

    /// Parses the JSON value.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// An Active Controller Message entry of AllCtrlList / ChCtrlList
/// (M2-117 Tables 5 and 11).
///
/// Note: M2-117's published JSON Schema omits `paramPath`, which Table 5
/// defines; this type follows the table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Controller<S> {
    /// Human-readable name.
    pub title: S,
    /// CommonMark description.
    pub description: Option<S>,
    /// Channel 1–256 (required in AllCtrlList).
    pub channel: Option<u16>,
    /// Message type.
    pub ctrl_type: CtrlType,
    /// Index numbers: CC number (or MSB, LSB), (N)RPN bank and index.
    /// Not required for chPress/pPress/pBend/pnp.
    pub ctrl_index: Option<[u16; 2]>,
    /// How many of `ctrl_index` are used (1 or 2).
    pub ctrl_index_len: u8,
    /// Priority 1 (essential) – 5 (hidden).
    pub priority: Option<u8>,
    /// Default value (32-bit).
    pub default: Option<u32>,
    /// `"absolute"`, `"relative"`, `"both"`, `"none"`.
    pub transmit: Option<S>,
    /// `"absolute"`, `"relative"`, `"both"`, `"none"`.
    pub recognize: Option<S>,
    /// Significant bits (default 32).
    pub num_sig_bits: Option<u8>,
    /// JSON Pointer grouping, e.g. `/Oscillator/A/Envelope/Attack`.
    pub param_path: Option<S>,
    /// `"continuous"`, `"momentary"`, `"toggle"`, `"relative"`,
    /// `"valueSelect"`.
    pub type_hint: Option<S>,
    /// `resId` for CtrlMapList.
    pub ctrl_map_id: Option<S>,
    /// Number of discrete steps.
    pub step_count: Option<u32>,
    /// Recognised range (32-bit).
    pub min_max: Option<[u32; 2]>,
}

impl<S: AsRef<str>> Controller<S> {
    /// Writes the object.
    pub fn write<W: Sink>(&self, w: &mut JsonWriter<W>) {
        w.begin_object().field_str("title", self.title.as_ref());
        let opt_str = |w: &mut JsonWriter<W>, k: &str, v: &Option<S>| {
            if let Some(v) = v {
                w.field_str(k, v.as_ref());
            }
        };
        opt_str(w, "description", &self.description);
        if let Some(c) = self.channel {
            w.field_uint("channel", u64::from(c));
        }
        w.field_str("ctrlType", self.ctrl_type.as_str());
        if let Some(idx) = self.ctrl_index {
            w.key("ctrlIndex").begin_array();
            for v in idx
                .iter()
                .take(usize::from(self.ctrl_index_len.clamp(1, 2)))
            {
                w.uint(u64::from(*v));
            }
            w.end_array();
        }
        if let Some(p) = self.priority {
            w.field_uint("priority", u64::from(p));
        }
        if let Some(d) = self.default {
            w.field_uint("default", u64::from(d));
        }
        opt_str(w, "transmit", &self.transmit);
        opt_str(w, "recognize", &self.recognize);
        if let Some(n) = self.num_sig_bits {
            w.field_uint("numSigBits", u64::from(n));
        }
        opt_str(w, "paramPath", &self.param_path);
        opt_str(w, "typeHint", &self.type_hint);
        opt_str(w, "ctrlMapId", &self.ctrl_map_id);
        if let Some(n) = self.step_count {
            w.field_uint("stepCount", u64::from(n));
        }
        if let Some([lo, hi]) = self.min_max {
            w.key("minMax")
                .begin_array()
                .uint(u64::from(lo))
                .uint(u64::from(hi))
                .end_array();
        }
        w.end_object();
    }
}

/// A CtrlMapList entry (M2-117 §4.1.1, Table 19).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CtrlMapEntry<S> {
    /// A 32-bit controller value.
    pub value: u32,
    /// Its name.
    pub title: S,
}

impl<S: AsRef<str>> CtrlMapEntry<S> {
    /// Writes the object.
    pub fn write<W: Sink>(&self, w: &mut JsonWriter<W>) {
        w.begin_object()
            .field_uint("value", u64::from(self.value))
            .field_str("title", self.title.as_ref())
            .end_object();
    }
}

/// A ProgramList entry. **Minimal:** M2-107 (ProgramList) is not publicly
/// available; these two properties are the ones M2-103/M2-105 examples use
/// (`title`, `bankPC`). Other properties are skipped when parsing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct ProgramEntry<S> {
    /// Program name.
    pub title: S,
    /// Bank MSB, Bank LSB, Program.
    #[cfg_attr(feature = "serde", serde(rename = "bankPC"))]
    pub bank_pc: [u8; 3],
}

impl<S: AsRef<str>> ProgramEntry<S> {
    /// Writes the object.
    pub fn write<W: Sink>(&self, w: &mut JsonWriter<W>) {
        w.begin_object()
            .field_str("title", self.title.as_ref())
            .field_uints("bankPC", &self.bank_pc)
            .end_object();
    }
}

/// A StateList entry (M2-111 §2.3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct StateEntry<S> {
    /// Name.
    pub title: S,
    /// `resId` for the State resource (max 36, `a-z0-9_`).
    pub state_id: S,
    /// Device-specific revision.
    pub state_rev: Option<S>,
    /// Unix seconds of the last update.
    pub timestamp: Option<u64>,
    /// CommonMark description.
    pub description: Option<S>,
    /// Expected size in bytes.
    pub size: Option<u64>,
}

impl<S: AsRef<str>> StateEntry<S> {
    /// Writes the object.
    pub fn write<W: Sink>(&self, w: &mut JsonWriter<W>) {
        w.begin_object()
            .field_str("title", self.title.as_ref())
            .field_str("stateId", self.state_id.as_ref());
        if let Some(r) = &self.state_rev {
            w.field_str("stateRev", r.as_ref());
        }
        if let Some(t) = self.timestamp {
            w.field_uint("timestamp", t);
        }
        if let Some(d) = &self.description {
            w.field_str("description", d.as_ref());
        }
        if let Some(s) = self.size {
            w.field_uint("size", s);
        }
        w.end_object();
    }
}

/// Writes `[a, b, …]` with `write_item` for each element.
pub fn write_list<T, W: Sink>(
    items: &[T],
    w: &mut JsonWriter<W>,
    mut write_item: impl FnMut(&T, &mut JsonWriter<W>),
) {
    w.begin_array();
    for item in items {
        write_item(item, w);
    }
    w.end_array();
}

#[cfg(feature = "alloc")]
pub use owned::*;

#[cfg(feature = "alloc")]
mod owned {
    //! Parsing replies into owned types.

    use alloc::string::{String, ToString};
    use alloc::vec::Vec;

    use super::{
        CanSet, ChannelEntry, Controller, CtrlMapEntry, CtrlType, DeviceInfo, ProgramEntry,
        StateEntry,
    };
    use crate::error::Error;
    use crate::pe::json::{Event, JsonStr, Parser};

    const fn bad(what: &'static str) -> Error {
        Error::InvalidField(what)
    }

    fn text(v: Event<'_>) -> Result<String, Error> {
        match v {
            Event::Str(s) => Ok(s.to_string()),
            _ => Err(bad("expected a string")),
        }
    }

    fn uint<T: TryFrom<u64>>(v: Event<'_>) -> Result<T, Error> {
        match v {
            Event::Number(n) => n
                .as_u64()
                .and_then(|n| T::try_from(n).ok())
                .ok_or(bad("number out of range")),
            _ => Err(bad("expected a number")),
        }
    }

    const fn boolean(v: Event<'_>) -> Result<bool, Error> {
        match v {
            Event::Bool(b) => Ok(b),
            _ => Err(bad("expected a boolean")),
        }
    }

    fn uints<T: TryFrom<u64> + Copy + Default, const N: usize>(
        p: &mut Parser<'_>,
        v: Event<'_>,
    ) -> Result<([T; N], usize), Error> {
        if v != Event::ArrayStart {
            return Err(bad("expected an array"));
        }
        let mut out = [T::default(); N];
        let mut n = 0;
        loop {
            match p.expect_event()? {
                Event::ArrayEnd => return Ok((out, n)),
                e => {
                    let slot = out.get_mut(n).ok_or(bad("array too long"))?;
                    *slot = uint(e)?;
                    n += 1;
                }
            }
        }
    }

    fn exact<T: TryFrom<u64> + Copy + Default, const N: usize>(
        p: &mut Parser<'_>,
        v: Event<'_>,
    ) -> Result<[T; N], Error> {
        match uints::<T, N>(p, v)? {
            (a, n) if n == N => Ok(a),
            _ => Err(bad("array length")),
        }
    }

    fn strings(p: &mut Parser<'_>, v: Event<'_>) -> Result<Vec<String>, Error> {
        if v != Event::ArrayStart {
            return Err(bad("expected an array"));
        }
        let mut out = Vec::new();
        loop {
            match p.expect_event()? {
                Event::ArrayEnd => return Ok(out),
                e => out.push(text(e)?),
            }
        }
    }

    /// Walks one object: calls `f(key, value, parser)` for each property.
    /// `f` returns `true` if it consumed the value (containers included);
    /// otherwise the value is skipped.
    fn object<'a>(
        p: &mut Parser<'a>,
        first: Event<'a>,
        mut f: impl FnMut(JsonStr<'a>, Event<'a>, &mut Parser<'a>) -> Result<bool, Error>,
    ) -> Result<(), Error> {
        if first != Event::ObjectStart {
            return Err(bad("expected an object"));
        }
        loop {
            match p.expect_event()? {
                Event::ObjectEnd => return Ok(()),
                Event::Key(k) => {
                    let v = p.expect_event()?;
                    if !f(k, v, p)? {
                        p.skip(v)?;
                    }
                }
                _ => return Err(bad("object")),
            }
        }
    }

    fn list<'a, T>(
        bytes: &'a [u8],
        mut item: impl FnMut(&mut Parser<'a>, Event<'a>) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        let mut p = Parser::new(bytes);
        if p.expect_event()? != Event::ArrayStart {
            return Err(bad("expected a list"));
        }
        let mut out = Vec::new();
        loop {
            match p.expect_event()? {
                Event::ArrayEnd => break,
                e => out.push(item(&mut p, e)?),
            }
        }
        if p.next()?.is_some() {
            return Err(bad("trailing data"));
        }
        Ok(out)
    }

    /// A parsed ResourceList entry (M2-103 §12.2), defaults applied.
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
    #[allow(clippy::struct_excessive_bools)] // mirrors the spec's properties
    pub struct ResourceInfo {
        /// Resource name.
        pub resource: String,
        /// `canGet`.
        pub can_get: bool,
        /// `canSet`.
        pub can_set: CanSet,
        /// `canSubscribe`.
        pub can_subscribe: bool,
        /// `requireResId` (also accepts M2-117 Table 26's `requireId`).
        pub require_res_id: bool,
        /// `canPaginate`.
        pub can_paginate: bool,
        /// `mediaTypes`.
        pub media_types: Vec<String>,
        /// `encodings`.
        pub encodings: Vec<String>,
    }

    impl ResourceInfo {
        /// Whether `encodings` lists `enc` (case-insensitive).
        #[must_use]
        pub fn supports_encoding(&self, enc: &str) -> bool {
            self.encodings.iter().any(|e| e.eq_ignore_ascii_case(enc))
        }
    }

    /// Parses a ResourceList reply.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for malformed data.
    pub fn parse_resource_list(bytes: &[u8]) -> Result<Vec<ResourceInfo>, Error> {
        list(bytes, |p, e| {
            let mut r = ResourceInfo {
                resource: String::new(),
                can_get: true,
                can_set: CanSet::None,
                can_subscribe: false,
                require_res_id: false,
                can_paginate: false,
                media_types: alloc::vec![String::from("application/json")],
                encodings: alloc::vec![String::from("ASCII")],
            };
            object(p, e, |k, v, p| {
                let key = |n: &str| k.eq_str(n);
                if key("resource") {
                    r.resource = text(v)?;
                } else if key("canGet") {
                    r.can_get = boolean(v)?;
                } else if key("canSet") {
                    r.can_set = match v {
                        Event::Str(s) if s.eq_str("full") => CanSet::Full,
                        Event::Str(s) if s.eq_str("partial") => CanSet::Partial,
                        _ => CanSet::None,
                    };
                } else if key("canSubscribe") {
                    r.can_subscribe = boolean(v)?;
                } else if key("requireResId") || key("requireId") {
                    r.require_res_id = boolean(v)?;
                } else if key("canPaginate") {
                    r.can_paginate = boolean(v)?;
                } else if key("mediaTypes") {
                    r.media_types = strings(p, v)?;
                    return Ok(true);
                } else if key("encodings") {
                    r.encodings = strings(p, v)?;
                    return Ok(true);
                }
                Ok(false)
            })?;
            if r.resource.is_empty() {
                return Err(bad("resource name missing"));
            }
            Ok(r)
        })
    }

    /// Parses a DeviceInfo reply.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for malformed data or a missing ID array.
    pub fn parse_device_info(bytes: &[u8]) -> Result<DeviceInfo<String>, Error> {
        let mut p = Parser::new(bytes);
        let first = p.expect_event()?;
        let mut d = DeviceInfo::<String>::default();
        let mut ids = 0u8;
        object(&mut p, first, |k, v, p| {
            let key = |n: &str| k.eq_str(n);
            if key("manufacturerId") {
                d.manufacturer_id = exact(p, v)?;
                ids |= 1;
            } else if key("familyId") {
                d.family_id = exact(p, v)?;
                ids |= 2;
            } else if key("modelId") {
                d.model_id = exact(p, v)?;
                ids |= 4;
            } else if key("versionId") {
                d.version_id = exact(p, v)?;
                ids |= 8;
            } else {
                if key("manufacturer") {
                    d.manufacturer = text(v)?;
                } else if key("family") {
                    d.family = text(v)?;
                } else if key("model") {
                    d.model = text(v)?;
                } else if key("version") {
                    d.version = text(v)?;
                } else if key("serialNumber") {
                    d.serial_number = Some(text(v)?);
                }
                return Ok(false);
            }
            Ok(true)
        })?;
        if ids != 0xF {
            return Err(bad("DeviceInfo ID arrays missing"));
        }
        Ok(d)
    }

    /// Parses a ChannelList reply.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for malformed data.
    pub fn parse_channel_list(bytes: &[u8]) -> Result<Vec<ChannelEntry<String>>, Error> {
        list(bytes, |p, e| {
            let mut c = ChannelEntry::<String>::default();
            object(p, e, |k, v, p| {
                let key = |n: &str| k.eq_str(n);
                if key("bankPC") {
                    c.bank_pc = Some(exact(p, v)?);
                    return Ok(true);
                }
                if key("title") {
                    c.title = text(v)?;
                } else if key("channel") {
                    c.channel = uint(v)?;
                } else if key("programTitle") {
                    c.program_title = Some(text(v)?);
                } else if key("clusterChannelStart") {
                    c.cluster_channel_start = Some(uint(v)?);
                } else if key("clusterLength") {
                    c.cluster_length = Some(uint(v)?);
                } else if key("clusterMidiMode") {
                    c.cluster_midi_mode = Some(uint(v)?);
                } else if key("clusterType") {
                    c.cluster_type = Some(text(v)?);
                }
                Ok(false)
            })?;
            Ok(c)
        })
    }

    /// Parses an AllCtrlList or ChCtrlList reply.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for malformed data or an unknown `ctrlType`.
    pub fn parse_controllers(bytes: &[u8]) -> Result<Vec<Controller<String>>, Error> {
        list(bytes, |p, e| {
            let mut c = Controller::<String>::default();
            object(p, e, |k, v, p| {
                let key = |n: &str| k.eq_str(n);
                if key("ctrlIndex") {
                    let (idx, n) = uints::<u16, 2>(p, v)?;
                    c.ctrl_index = Some(idx);
                    c.ctrl_index_len = u8::try_from(n).unwrap_or(2);
                    return Ok(true);
                }
                if key("minMax") {
                    c.min_max = Some(exact(p, v)?);
                    return Ok(true);
                }
                // M2-117 Table 11 capitalises "Title".
                if key("title") || key("Title") {
                    c.title = text(v)?;
                } else if key("description") {
                    c.description = Some(text(v)?);
                } else if key("channel") {
                    c.channel = Some(uint(v)?);
                } else if key("ctrlType") {
                    c.ctrl_type = CtrlType::parse(&text(v)?).ok_or(bad("ctrlType"))?;
                } else if key("priority") {
                    c.priority = Some(uint(v)?);
                } else if key("default") {
                    c.default = Some(uint(v)?);
                } else if key("transmit") {
                    c.transmit = Some(text(v)?);
                } else if key("recognize") {
                    c.recognize = Some(text(v)?);
                } else if key("numSigBits") {
                    c.num_sig_bits = Some(uint(v)?);
                } else if key("paramPath") {
                    c.param_path = Some(text(v)?);
                } else if key("typeHint") {
                    c.type_hint = Some(text(v)?);
                } else if key("ctrlMapId") {
                    c.ctrl_map_id = Some(text(v)?);
                } else if key("stepCount") {
                    c.step_count = Some(uint(v)?);
                }
                Ok(false)
            })?;
            Ok(c)
        })
    }

    /// Parses a CtrlMapList reply.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for malformed data.
    pub fn parse_ctrl_map_list(bytes: &[u8]) -> Result<Vec<CtrlMapEntry<String>>, Error> {
        list(bytes, |p, e| {
            let mut c = CtrlMapEntry::<String>::default();
            object(p, e, |k, v, _| {
                if k.eq_str("value") {
                    c.value = uint(v)?;
                } else if k.eq_str("title") {
                    c.title = text(v)?;
                }
                Ok(false)
            })?;
            Ok(c)
        })
    }

    /// Parses a ProgramList reply (minimal, see [`ProgramEntry`]).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for malformed data.
    pub fn parse_program_list(bytes: &[u8]) -> Result<Vec<ProgramEntry<String>>, Error> {
        list(bytes, |p, e| {
            let mut c = ProgramEntry::<String>::default();
            object(p, e, |k, v, p| {
                if k.eq_str("bankPC") {
                    c.bank_pc = exact(p, v)?;
                    return Ok(true);
                }
                if k.eq_str("title") {
                    c.title = text(v)?;
                }
                Ok(false)
            })?;
            Ok(c)
        })
    }

    /// Parses a StateList reply.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for malformed data.
    pub fn parse_state_list(bytes: &[u8]) -> Result<Vec<StateEntry<String>>, Error> {
        list(bytes, |p, e| {
            let mut s = StateEntry::<String>::default();
            object(p, e, |k, v, _| {
                let key = |n: &str| k.eq_str(n);
                if key("title") {
                    s.title = text(v)?;
                } else if key("stateId") {
                    s.state_id = text(v)?;
                } else if key("stateRev") {
                    s.state_rev = Some(text(v)?);
                } else if key("timestamp") {
                    s.timestamp = Some(uint(v)?);
                } else if key("description") {
                    s.description = Some(text(v)?);
                } else if key("size") {
                    s.size = Some(uint(v)?);
                }
                Ok(false)
            })?;
            Ok(s)
        })
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn render(f: impl FnOnce(&mut JsonWriter<Vec<u8>>)) -> Vec<u8> {
        let mut w = JsonWriter::new(Vec::new());
        f(&mut w);
        w.finish().unwrap()
    }

    #[test]
    fn device_info_round_trip_and_spec_example() {
        let d = DeviceInfo {
            manufacturer_id: [0x7D, 0, 0],
            family_id: [0, 0],
            model_id: [48, 0],
            version_id: [0, 0, 1, 0],
            manufacturer: "Educational Use",
            family: "Example Range",
            model: "Example Pedal",
            version: "1.0",
            serial_number: None,
        };
        let json = render(|w| d.write(w));
        let back = parse_device_info(&json).unwrap();
        assert_eq!(back.model, "Example Pedal");
        assert_eq!(back.model_id, [48, 0]);
        // M2-103 §4.1.4 example, including an "x-" property.
        let spec = br#"{"manufacturerId":[125,0,0],"manufacturer":"Educational Use","familyId":[0,0],"family":"Example Range","modelId":[48,0],"model":"Example Pedal","versionId":[0,0,1,0],"version":"1.0","x-uniqueKey":"myuniquevalue"}"#;
        assert_eq!(parse_device_info(spec).unwrap(), back);
    }

    #[test]
    fn resource_list_defaults() {
        // M2-103 §12.2 example (note "MCoded7" casing).
        let spec = br#"[{"resource": "DeviceInfo"},{"resource": "ChannelList","canSubscribe":true},{"resource": "BufferState","canSet": "full","mediaTypes": ["application/octet-stream"],"encodings": ["MCoded7"]}]"#;
        let l = parse_resource_list(spec).unwrap();
        assert_eq!(l.len(), 3);
        assert!(l[0].can_get && !l[0].can_subscribe);
        assert!(l[1].can_subscribe);
        assert_eq!(l[2].can_set, CanSet::Full);
        assert!(l[2].supports_encoding("Mcoded7"));
        let decls = [
            ResourceDecl::new("DeviceInfo"),
            ResourceDecl {
                can_subscribe: true,
                ..ResourceDecl::new("ChannelList")
            },
            ResourceDecl::new("ResourceList"),
        ];
        let json = render(|w| write_resource_list(&decls, w));
        assert_eq!(
            json,
            br#"[{"resource":"DeviceInfo"},{"resource":"ChannelList","canSubscribe":true}]"#
        );
    }

    #[test]
    fn controllers_and_maps() {
        let c = Controller {
            title: "Cutoff",
            channel: Some(1),
            ctrl_type: CtrlType::Cc,
            ctrl_index: Some([74, 0]),
            ctrl_index_len: 1,
            priority: Some(1),
            param_path: Some("/Filter/Cutoff"),
            min_max: Some([0, u32::MAX]),
            ..Controller::default()
        };
        let json = render(|w| write_list(core::slice::from_ref(&c), w, Controller::write));
        let back = parse_controllers(&json).unwrap();
        assert_eq!(back[0].param_path.as_deref(), Some("/Filter/Cutoff"));
        assert_eq!(back[0].ctrl_index, Some([74, 0]));
        assert_eq!(back[0].min_max, Some([0, u32::MAX]));
        let m = parse_ctrl_map_list(
            br#"[{"value": 0, "title": "Sine"},{"value": 3238268993, "title": "Square"}]"#,
        )
        .unwrap();
        assert_eq!(m[1].value, 3_238_268_993);
    }

    #[test]
    fn channel_program_state_lists() {
        // M2-105 Table 11.
        let ch = parse_channel_list(br#"[{"title": "Lead","channel": 1,"programTitle": "Piano + Strings","bankPC": [1,0,76]},{"title": "Drums","channel": 10,"programTitle": "GM2 Jazz Drum Set","bankPC": [0,0,33]}]"#).unwrap();
        assert_eq!(ch[1].channel, 10);
        assert_eq!(ch[0].bank_pc, Some([1, 0, 76]));
        let json = render(|w| write_list(&ch, w, ChannelEntry::write));
        assert_eq!(parse_channel_list(&json).unwrap(), ch);
        let p = parse_program_list(br#"[{"title":"Piano","bankPC":[0,0,1],"category":["keys"]}]"#)
            .unwrap();
        assert_eq!(p[0].bank_pc, [0, 0, 1]);
        let s = parse_state_list(br#"[{"title": "Buffer","stateId": "buffer","stateRev": "adoi234dvd","timestamp": 1580652000,"size": 4456953}]"#).unwrap();
        assert_eq!(s[0].size, Some(4_456_953));
    }
}
