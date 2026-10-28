//! The Responder: the device side of MIDI-CI, with no allocator.
//!
//! The application implements [`Device`] (profiles, resources, process
//! inquiry). The [`Responder`] handles everything else:
//! * MUID lifecycle and collisions (M2-101 §3.3, §5.9).
//! * Discovery and Endpoint replies (§5.6, §5.8).
//! * Profile Configuration (§7).
//! * Property Exchange: capabilities, Get, Set, Subscribe (§8; M2-103 §5–§10).
//! * Process Inquiry (§9).
//! * NAKs for anything unsupported (§5.11).
//!
//! Large Get replies are never buffered. The Responder calls
//! [`Device::get`] once to count the bytes and plan chunks, then once more
//! for each chunk, keeping only that chunk's window. `get` must therefore
//! produce the same bytes each time during a transfer. If the length
//! changes, the transfer is aborted with Notify 144.
//!
//! Capacities are const generics:
//! * `FRAME`: the largest frame sent. It also bounds each chunk; the
//!   runtime `max_frame` can only lower it.
//! * `OUT`: bytes of queued outbound frames.
//! * `SET`: the largest Set Property Data accepted, header included.
//! * `SUBS`: concurrent subscriptions.
//!
//! # No-alloc design
//!
//! The Responder is built for firmware with no allocator that sends at most
//! 128 bytes per SysEx (the FM-1 SDK's limit).
//!
//! * **Chunk budget.** A framed PE chunk carries 24 bytes of overhead (`F0`,
//!   the 13-byte envelope, 9 chunk bytes, `F7`), so a 128-byte frame leaves
//!   104 bytes for header plus data
//!   ([`FRAME_OVERHEAD`](crate::pe::chunk::FRAME_OVERHEAD)). Reply headers are
//!   never chunked (M2-101 §8.3.2).
//! * **Pull-based output.** [`Responder::handle`] queues short replies in a
//!   fixed `OUT`-byte frame queue. [`Responder::poll_transmit`] drains one
//!   frame at a time, so the caller paces the transport.
//! * **Get replies are never buffered.** [`Device::get`] is rendered once
//!   into a [`Counter`], which gives the length, so
//!   the number of chunks is known up front. It is then rendered once per
//!   chunk into a [`Window`] that keeps only that
//!   chunk's bytes. Mcoded7 is applied by a streaming encoder in the same
//!   pipeline.
//! * **Sets** are reassembled into a fixed `SET`-byte buffer, and Mcoded7 is
//!   decoded in place.
//! * **JSON** goes through the borrowed pull parser and the streaming writer
//!   in [`pe::json`](crate::pe::json): no DOM, nothing beyond `core`.
//! * **The proof.** `cargo build --no-default-features --target
//!   riscv32imc-unknown-none-elf` (or `thumbv7em-none-eabihf`), and the
//!   `examples/no_std_responder` crate for the same targets.

use crate::error::Error;
use crate::identity::DeviceIdentity;
use crate::message::{Body, Message};
use crate::mgmt::{
    Categories, Details, DiscoveryReply, EndpointInquiry, EndpointReply, InvalidateMuid, Nak,
    NakStatus,
};
use crate::muid::Muid;
use crate::pe::chunk::{AssembleError, Assembler, ChunkPlan, Progress, TIMEOUT_MS};
use crate::pe::header::{Command, Encoding, Header};
use crate::pe::json::{JsonStr, JsonWriter};
use crate::pe::mcoded7;
use crate::pe::resource::{CanSet, ResourceDecl, names, write_resource_list};
use crate::pe::sink::{ArrayBuf, Counter, Sink, SliceSink, Window};
use crate::pe::status::PeStatus;
use crate::pe::{PeCapabilities, PeChunk};
use crate::process::{
    EndOfMessageReport, MessageReportInquiry, MessageReportReply, ProcessFeatures,
    ProcessInquiryReply, ReportSelection,
};
use crate::profile::{
    ProfileChannels, ProfileDetailsReply, ProfileId, ProfileInquiryReply, ProfileList,
    ProfileReport, ProfileState,
};
use crate::wire::{CiVersion, DeviceId, Envelope, Frame, Framing, MessageKind, is_midi_ci};

use super::{FrameQueue, Handled};

/// Responder settings.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Config<'a> {
    /// Who we are (Discovery, M2-101 §5.5.1).
    pub identity: DeviceIdentity,
    /// Our Receivable Maximum SysEx Message Size (≥ 128, §5.5.3).
    pub max_sysex: u32,
    /// The largest frame we send (FMX: 128). Clamped to `FRAME`.
    pub max_frame: u32,
    /// Function Block number for Reply to Discovery (`None` = `0x7F`).
    pub function_block: Option<u8>,
    /// Product Instance Id for Endpoint Information (§5.8.3.1).
    pub product_instance_id: Option<&'a str>,
    /// Whether to advertise and answer Profile Configuration.
    pub profiles: bool,
    /// Property Exchange capabilities; `None` disables PE.
    pub property_exchange: Option<PeCapabilities>,
    /// Process Inquiry features; empty disables Process Inquiry.
    pub process: ProcessFeatures,
    /// Framing of emitted frames.
    pub framing: Framing,
    /// Accept Device ID `0x0F` on PE messages (M2-101 §8.2 erratum).
    pub lenient_pe_device_id: bool,
}

impl Config<'_> {
    /// A Management-only Responder sending 128-byte frames.
    #[must_use]
    pub const fn new(identity: DeviceIdentity) -> Self {
        Self {
            identity,
            max_sysex: 512,
            max_frame: 128,
            function_block: None,
            product_instance_id: None,
            profiles: false,
            property_exchange: None,
            process: ProcessFeatures::NONE,
            framing: Framing::SysEx,
            lenient_pe_device_id: false,
        }
    }

    /// The category bitmap advertised in Reply to Discovery.
    #[must_use]
    pub const fn categories(&self) -> Categories {
        let mut c = Categories::NONE;
        if self.profiles {
            c = c.union(Categories::PROFILE_CONFIGURATION);
        }
        if self.property_exchange.is_some() {
            c = c.union(Categories::PROPERTY_EXCHANGE);
        }
        if self.process.bits() != 0 {
            c = c.union(Categories::PROCESS_INQUIRY);
        }
        c
    }
}

/// The Profiles on one target.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ProfileSet<'a> {
    /// Supported and enabled.
    pub enabled: &'a [ProfileId],
    /// Supported and disabled.
    pub disabled: &'a [ProfileId],
}

impl<'a> ProfileSet<'a> {
    /// Profiles supported on a target, split by state.
    #[must_use]
    pub const fn new(enabled: &'a [ProfileId], disabled: &'a [ProfileId]) -> Self {
        Self { enabled, disabled }
    }

    fn contains(&self, p: ProfileId) -> bool {
        self.enabled.contains(&p) || self.disabled.contains(&p)
    }
}

/// What [`Device::set_profile`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileOutcome {
    /// The Profile is now enabled on this many channels (0 for Group or
    /// Function Block targets).
    Enabled(u16),
    /// The Profile is now (or remains) disabled.
    Disabled(u16),
    /// Not supported on that target: NAK `0x04`.
    NotSupported,
}

/// Extra Reply Header properties for a Get (M2-103 §5.3, §6.6.2; M2-111).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct GetMeta<'a> {
    /// `mediaType` for non-JSON data (requires `Mcoded7` if 8-bit).
    pub media_type: Option<&'a str>,
    /// `totalCount` for paginated lists.
    pub total_count: Option<u32>,
    /// `cacheTime` seconds.
    pub cache_time: Option<u32>,
    /// `stateRev` (State Resource).
    pub state_rev: Option<&'a str>,
    /// `timestamp` (State Resource).
    pub timestamp: Option<u64>,
}

impl<'a> GetMeta<'a> {
    /// Sets `mediaType`.
    #[must_use]
    pub const fn with_media_type(mut self, media_type: &'a str) -> Self {
        self.media_type = Some(media_type);
        self
    }

    /// Sets `totalCount`.
    #[must_use]
    pub const fn with_total_count(mut self, total_count: u32) -> Self {
        self.total_count = Some(total_count);
        self
    }

    /// Sets `cacheTime`.
    #[must_use]
    pub const fn with_cache_time(mut self, seconds: u32) -> Self {
        self.cache_time = Some(seconds);
        self
    }

    /// Sets `stateRev`.
    #[must_use]
    pub const fn with_state_rev(mut self, state_rev: &'a str) -> Self {
        self.state_rev = Some(state_rev);
        self
    }

    /// Sets `timestamp`.
    #[must_use]
    pub const fn with_timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = Some(timestamp);
        self
    }
}

/// The application side of a Responder. Every method except
/// [`Device::random`] has a default that declines politely.
pub trait Device {
    /// 32 random bits, for new MUIDs (M2-101 §3.3.1).
    fn random(&mut self) -> u32;

    /// The Profiles on `target`.
    fn profiles(&self, target: DeviceId) -> ProfileSet<'_> {
        let _ = target;
        ProfileSet::default()
    }

    /// Enable or disable a Profile (M2-101 §7.8, §7.9).
    fn set_profile(
        &mut self,
        target: DeviceId,
        profile: ProfileId,
        state: ProfileState,
        channels: u16,
    ) -> ProfileOutcome {
        let _ = (target, profile, state, channels);
        ProfileOutcome::NotSupported
    }

    /// Profile Details data for `inquiry_target` (M2-101 §7.7) written to
    /// `out`; `None` NAKs. Target `0x00` (channel counts, M2-102 §2.5.1)
    /// can use [`crate::profile::ChannelCount::to_bytes`].
    fn profile_details(
        &self,
        target: DeviceId,
        profile: ProfileId,
        inquiry_target: u8,
        out: &mut [u8],
    ) -> Option<usize> {
        let _ = (target, profile, inquiry_target, out);
        None
    }

    /// Profile Specific Data received (M2-101 §7.12).
    fn profile_data(&mut self, from: Muid, target: DeviceId, profile: ProfileId, data: &[u8]) {
        let _ = (from, target, profile, data);
    }

    /// The resources to list in ResourceList (the Responder answers
    /// ResourceList itself). Used also to enforce `canGet`/`canSet`/
    /// `canSubscribe`/`requireResId`.
    fn resources(&self) -> &[ResourceDecl<'_>] {
        &[]
    }

    /// Writes the Property Data for a Get (raw bytes; the Responder applies
    /// Mcoded7 if requested). Must be deterministic during a transfer.
    ///
    /// # Errors
    ///
    /// A [`PeStatus`] to reply with instead (e.g. 404).
    fn get(&self, request: &Header<'_>, out: &mut dyn Sink) -> Result<GetMeta<'_>, PeStatus> {
        let _ = (request, out);
        Err(PeStatus::NOT_FOUND)
    }

    /// Applies a Set (data already Mcoded7-decoded if so declared).
    ///
    /// # Errors
    ///
    /// A [`PeStatus`] to reply with.
    fn set(&mut self, request: &Header<'_>, data: &[u8]) -> Result<(), PeStatus> {
        let _ = (request, data);
        Err(PeStatus::NOT_ALLOWED)
    }

    /// A subscription started (`true`) or ended (`false`).
    fn subscription(
        &mut self,
        peer: Muid,
        request: &Header<'_>,
        subscribe_id: &str,
        change: SubscriptionChange,
    ) {
        let _ = (peer, request, subscribe_id, change);
    }

    /// A MIDI Message Report was requested (M2-101 §9.5). Return what will
    /// be reported (a subset of the request); then send the MIDI messages
    /// and call [`Responder::end_message_report`]. `None` NAKs.
    fn message_report(
        &mut self,
        peer: Muid,
        target: DeviceId,
        inquiry: &MessageReportInquiry,
    ) -> Option<ReportSelection> {
        let _ = (peer, target, inquiry);
        None
    }
}

/// Whether a subscription started or ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SubscriptionChange {
    /// The Initiator subscribed (`"command":"start"`).
    Started,
    /// The subscription ended (`"command":"end"`, either side).
    Ended,
}

/// A subscription update to publish (M2-103 §9.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Update<'a> {
    /// Send the full data (rendered with [`Device::get`]); falls back to
    /// `notify` if it does not fit one chunk (§9.1.1).
    Full,
    /// A partial update: a JSON object of JSON Pointer keys (§8).
    Partial(&'a [u8]),
    /// Ask the Initiator to re-Get.
    Notify,
    /// End the subscription.
    End,
}

const PEERS: usize = 8;
const TRANSFERS: usize = 2;
const REQ_HEADER: usize = 192;
const REPLY_HEADER: usize = 160;
const SUB_RES: usize = 96;
const DEFAULT_PEER_MAX: u32 = 128;

#[derive(Clone, Copy, Debug)]
struct Peer {
    muid: Muid,
    max_sysex: u32,
}

#[derive(Clone, Debug)]
struct Transfer {
    peer: Muid,
    device: DeviceId,
    request_id: u8,
    request: ArrayBuf<REQ_HEADER>,
    reply: ArrayBuf<REPLY_HEADER>,
    plan: ChunkPlan,
    next: u16,
    mcoded7: bool,
}

#[derive(Clone, Debug)]
struct Subscription {
    peer: Muid,
    id: u16,
    /// The `start` request header (to re-render `full` updates).
    request: ArrayBuf<SUB_RES>,
}

#[derive(Clone, Copy, Debug)]
struct SetState {
    peer: Muid,
    device: DeviceId,
}

/// The Responder. See the module docs.
///
/// # Example
///
/// A Management-only device answering Discovery, in 128-byte frames:
///
/// ```
/// use midici::engine::responder::{Config, Device, Responder};
/// use midici::identity::{DeviceIdentity, ManufacturerId};
/// use midici::mgmt::{Categories, Discovery};
/// use midici::wire::{Envelope, Framing};
/// use midici::{Body, Message, Muid};
///
/// struct Synth;
/// impl Device for Synth {
///     fn random(&mut self) -> u32 {
///         0x2468_ACE0 // use a hardware RNG
///     }
/// }
///
/// let identity = DeviceIdentity {
///     manufacturer: ManufacturerId::NON_COMMERCIAL,
///     family: 1,
///     model: 1,
///     revision: [0, 0, 1, 0],
/// };
/// let mut ci: Responder<'_, 128> = Responder::new(Config::new(identity), 0x1357_9BDF);
///
/// // A host broadcasts Discovery.
/// let host = Muid::new(0x0101_0101)?;
/// let discovery = Discovery::new(identity, Categories::NONE, 512);
/// let mut frame = [0u8; 128];
/// let n = discovery.encode(Envelope::broadcast(host), Framing::SysEx, &mut frame)?;
/// ci.handle(&mut Synth, 0, &frame[..n]);
///
/// // The device answers with its own MUID.
/// let mut out = [0u8; 128];
/// let n = ci.poll_transmit(&Synth, &mut out).ok_or(midici::Error::Truncated)?;
/// let reply = Message::decode(&out[..n])?;
/// assert!(matches!(reply.body, Body::DiscoveryReply(_)));
/// assert_eq!(reply.envelope.source, ci.muid());
/// assert_eq!(reply.envelope.destination, host);
/// # Ok::<(), midici::Error>(())
/// ```
pub struct Responder<
    'c,
    const FRAME: usize = 512,
    const OUT: usize = 1024,
    const SET: usize = 1024,
    const SUBS: usize = 4,
> {
    config: Config<'c>,
    muid: Muid,
    used: bool,
    peers: [Option<Peer>; PEERS],
    out: FrameQueue<OUT>,
    transfers: [Option<Transfer>; TRANSFERS],
    set: Assembler<ArrayBuf<SET>>,
    set_state: Option<SetState>,
    subs: [Option<Subscription>; SUBS],
    next_sub: u16,
    scratch: [u8; FRAME],
    dropped: u32,
}

/// Sink adapter that notes any byte with the high bit set.
struct HighBit<S> {
    inner: S,
    high: bool,
}

impl<S: Sink> Sink for HighBit<S> {
    fn put(&mut self, bytes: &[u8]) {
        self.high |= bytes.iter().any(|b| b & 0x80 != 0);
        self.inner.put(bytes);
    }
}

fn copy_into<const N: usize>(bytes: &[u8]) -> Option<ArrayBuf<N>> {
    let mut b = ArrayBuf::new();
    b.try_extend(bytes).then_some(b)
}

fn sub_id_text(id: u16, buf: &mut [u8; 8]) -> &str {
    // "sub" + up to 5 digits: within the 8-character limit (M2-103 §9.1).
    let mut n = id;
    let mut digits = [b'0'; 5];
    let mut len = 0;
    loop {
        if let Some(d) = digits.get_mut(len) {
            // `n % 10 < 10`.
            #[allow(clippy::cast_possible_truncation)]
            {
                *d = b'0' + (n % 10) as u8;
            }
        }
        len += 1;
        n /= 10;
        if n == 0 || len == digits.len() {
            break;
        }
    }
    buf[..3].copy_from_slice(b"sub");
    for i in 0..len {
        if let (Some(dst), Some(&src)) = (buf.get_mut(3 + i), digits.get(len - 1 - i)) {
            *dst = src;
        }
    }
    core::str::from_utf8(buf.get(..3 + len).unwrap_or_default()).unwrap_or("sub")
}

fn parse_sub_id(s: JsonStr<'_>) -> Option<u16> {
    let raw = s.as_plain()?;
    raw.strip_prefix("sub")?.parse().ok()
}

impl<'c, const FRAME: usize, const OUT: usize, const SET: usize, const SUBS: usize>
    Responder<'c, FRAME, OUT, SET, SUBS>
{
    /// A Responder with a MUID derived from `random`.
    #[must_use]
    pub const fn new(config: Config<'c>, random: u32) -> Self {
        Self {
            config,
            muid: Muid::from_random(random),
            used: false,
            peers: [None; PEERS],
            out: FrameQueue::new(),
            transfers: [const { None }; TRANSFERS],
            set: Assembler::new(ArrayBuf::new(), SET),
            set_state: None,
            subs: [const { None }; SUBS],
            next_sub: 1,
            scratch: [0; FRAME],
            dropped: 0,
        }
    }

    /// Our current MUID.
    #[must_use]
    pub const fn muid(&self) -> Muid {
        self.muid
    }

    /// The configuration.
    #[must_use]
    pub const fn config(&self) -> &Config<'c> {
        &self.config
    }

    /// Frames dropped because the outbound queue was full.
    #[must_use]
    pub const fn dropped(&self) -> u32 {
        self.dropped
    }

    /// Number of active subscriptions.
    #[must_use]
    pub fn subscriptions(&self) -> usize {
        self.subs.iter().flatten().count()
    }

    /// Whether frames are waiting (queued replies or a Get in progress).
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.out.is_empty() || self.transfers.iter().any(Option::is_some)
    }

    fn frame_limit(&self, peer: Option<Muid>) -> usize {
        let peer_max = peer
            .and_then(|m| self.peers.iter().flatten().find(|p| p.muid == m))
            .map_or(DEFAULT_PEER_MAX, |p| p.max_sysex.max(DEFAULT_PEER_MAX));
        let limit = self.config.max_frame.min(peer_max);
        usize::try_from(limit).unwrap_or(FRAME).min(FRAME)
    }

    const fn envelope(&self, device: DeviceId, to: Muid) -> Envelope {
        Envelope {
            device,
            version: CiVersion::CURRENT,
            source: self.muid,
            destination: to,
        }
    }

    fn send(&mut self, to: Option<Muid>, envelope: Envelope, body: Body<'_>) {
        let limit = self.frame_limit(to);
        let framing = self.config.framing;
        let msg = Message::new(envelope, body);
        if self
            .out
            .push_with(limit, |buf| msg.encode(framing, buf))
            .is_err()
        {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    fn nak(&mut self, req: &Envelope, original: MessageKind, status: NakStatus, details: Details) {
        let mut nak = Nak::new(original, status);
        if let Some(r) = &mut nak.report {
            r.details = details;
        }
        let env = self.envelope(req.device, req.source);
        self.send(Some(req.source), env, Body::Nak(nak));
    }

    fn remember_peer(&mut self, muid: Muid, max_sysex: u32) {
        if let Some(p) = self.peers.iter_mut().flatten().find(|p| p.muid == muid) {
            p.max_sysex = max_sysex;
            return;
        }
        let i = self.peers.iter().position(Option::is_none).unwrap_or(0);
        if let Some(slot) = self.peers.get_mut(i) {
            *slot = Some(Peer { muid, max_sysex });
        }
    }

    fn forget_peer(&mut self, muid: Muid) {
        for p in &mut self.peers {
            if p.is_some_and(|p| p.muid == muid) {
                *p = None;
            }
        }
        for t in &mut self.transfers {
            if t.as_ref().is_some_and(|t| t.peer == muid) {
                *t = None;
            }
        }
        for s in &mut self.subs {
            if s.as_ref().is_some_and(|s| s.peer == muid) {
                *s = None;
            }
        }
        if self.set_state.is_some_and(|s| s.peer == muid) {
            self.set.reset();
            self.set_state = None;
        }
    }

    /// Takes a fresh MUID and drops all transactions and subscriptions
    /// (M2-101 §3.3.2, §5.9).
    pub fn renew_muid(&mut self, random: u32) {
        let mut m = Muid::from_random(random);
        if m == self.muid {
            m = Muid::from_random(random.wrapping_add(0x9E37_79B9));
        }
        self.muid = m;
        self.used = false;
        self.transfers = [const { None }; TRANSFERS];
        self.subs = [const { None }; SUBS];
        self.set.reset();
        self.set_state = None;
    }

    /// Announces that our MUID is going away (e.g. before shutdown), then
    /// takes a new one: Invalidate MUID to broadcast (M2-101 §5.9).
    pub fn invalidate_self(&mut self, random: u32) {
        let old = self.muid;
        let env = self.envelope(DeviceId::FunctionBlock, Muid::BROADCAST);
        self.send(
            None,
            env,
            Body::InvalidateMuid(InvalidateMuid { target: old }),
        );
        self.renew_muid(random);
    }

    /// Handles one received SysEx message (with or without `F0`/`F7`).
    pub fn handle<D: Device>(&mut self, device: &mut D, now_ms: u64, bytes: &[u8]) -> Handled {
        if !is_midi_ci(bytes) {
            return Handled::NotCi;
        }
        let frame = match Frame::parse(bytes) {
            Ok(f) => f,
            Err(e) => return Handled::Malformed(e),
        };
        let env = frame.header.envelope;
        let kind = frame.header.kind;
        let to_us = env.destination == self.muid;
        if !(to_us || env.destination.is_broadcast()) {
            return Handled::Ignored;
        }
        if env.source == self.muid && kind != MessageKind::Discovery {
            return Handled::Ignored;
        }
        if env.version.has_reserved_bits() {
            if to_us || kind == MessageKind::Discovery {
                self.nak(
                    &env,
                    kind,
                    NakStatus::VERSION_NOT_SUPPORTED,
                    Details::default(),
                );
            }
            return Handled::Malformed(Error::UnsupportedVersion(env.version.byte()));
        }
        let msg = match Message::decode(bytes) {
            Ok(m) => m,
            Err(e) => {
                if to_us {
                    self.nak(&env, kind, NakStatus::MALFORMED, Details::default());
                }
                return Handled::Malformed(e);
            }
        };
        if !to_us && !kind.allows_broadcast() {
            return Handled::Ignored;
        }
        self.dispatch(device, now_ms, &env, msg.body)
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch<D: Device>(
        &mut self,
        device: &mut D,
        now_ms: u64,
        env: &Envelope,
        body: Body<'_>,
    ) -> Handled {
        let kind = body.kind();
        let from = env.source;
        match body {
            Body::Discovery(d) => {
                if env.source == self.muid {
                    // MUID collision (M2-101 §5.9.1).
                    if self.used {
                        let inv = self.envelope(DeviceId::FunctionBlock, Muid::BROADCAST);
                        self.send(
                            None,
                            inv,
                            Body::InvalidateMuid(InvalidateMuid { target: self.muid }),
                        );
                        let r = device.random();
                        self.renew_muid(r);
                        return Handled::Processed;
                    }
                    let r = device.random();
                    self.renew_muid(r);
                }
                self.remember_peer(env.source, d.max_sysex);
                let reply = DiscoveryReply {
                    identity: self.config.identity,
                    categories: self.config.categories(),
                    max_sysex: self.config.max_sysex,
                    output_path: d.output_path,
                    function_block: self.config.function_block,
                };
                let out = self.envelope(DeviceId::FunctionBlock, env.source);
                self.send(Some(env.source), out, Body::DiscoveryReply(reply));
            }
            Body::InvalidateMuid(inv) => {
                if inv.target == self.muid {
                    let r = device.random();
                    self.renew_muid(r);
                } else {
                    self.forget_peer(inv.target);
                }
            }
            Body::EndpointInquiry(q) => {
                self.used = true;
                match self.config.product_instance_id {
                    Some(id) if q.status == EndpointInquiry::PRODUCT_INSTANCE_ID => {
                        if let Ok(reply) = EndpointReply::product_instance_id(id) {
                            let out = self.envelope(env.device, from);
                            self.send(Some(from), out, Body::EndpointReply(reply));
                        }
                    }
                    _ => self.nak(env, kind, NakStatus::NOT_SUPPORTED, Details::default()),
                }
            }
            Body::ProfileInquiry(_)
            | Body::SetProfileOn(_)
            | Body::SetProfileOff(_)
            | Body::ProfileDetailsInquiry(_)
            | Body::ProfileSpecificData(_)
                if self.config.profiles =>
            {
                self.used = true;
                self.profile(device, env, body);
            }
            Body::PeCapabilitiesInquiry(_)
            | Body::GetPropertyData(_)
            | Body::SetPropertyData(_)
            | Body::Subscription(_)
            | Body::Notify(_)
                if self.config.property_exchange.is_some() =>
            {
                self.used = true;
                let fb = env.device == DeviceId::FunctionBlock
                    || (self.config.lenient_pe_device_id && env.device.to_byte() == 0x0F);
                if !fb {
                    self.nak(env, kind, NakStatus::MALFORMED, Details::default());
                    return Handled::Processed;
                }
                self.property_exchange(device, now_ms, env, body);
            }
            Body::ProcessInquiry(_) | Body::MessageReportInquiry(_)
                if self.config.process.bits() != 0 =>
            {
                self.used = true;
                match body {
                    Body::ProcessInquiry(_) => {
                        let out = self.envelope(DeviceId::FunctionBlock, from);
                        let reply = ProcessInquiryReply {
                            features: self.config.process,
                        };
                        self.send(Some(from), out, Body::ProcessInquiryReply(reply));
                    }
                    Body::MessageReportInquiry(q) => {
                        if let Some(sel) = device.message_report(from, env.device, &q) {
                            let out = self.envelope(env.device, from);
                            let reply = MessageReportReply {
                                selection: sel.intersection(q.selection),
                            };
                            self.send(Some(from), out, Body::MessageReportReply(reply));
                        } else {
                            self.nak(env, kind, NakStatus::NOT_SUPPORTED, Details::default());
                        }
                    }
                    _ => {}
                }
            }
            // Replies and reports addressed to us need no answer.
            Body::DiscoveryReply(_)
            | Body::EndpointReply(_)
            | Body::Ack(_)
            | Body::Nak(_)
            | Body::ProfileInquiryReply(_)
            | Body::ProfileEnabled(_)
            | Body::ProfileDisabled(_)
            | Body::ProfileAdded(_)
            | Body::ProfileRemoved(_)
            | Body::ProfileDetailsReply(_)
            | Body::PeCapabilitiesReply(_)
            | Body::GetPropertyDataReply(_)
            | Body::SetPropertyDataReply(_)
            | Body::SubscriptionReply(_)
            | Body::ProcessInquiryReply(_)
            | Body::MessageReportReply(_)
            | Body::EndOfMessageReport(_) => return Handled::Ignored,
            _ => {
                // Unsupported category, deprecated 1.1 message, or undefined
                // Sub-ID#2 (M2-101 §5.11).
                self.nak(env, kind, NakStatus::NOT_SUPPORTED, Details::default());
            }
        }
        Handled::Processed
    }

    fn profile<D: Device>(&mut self, device: &mut D, env: &Envelope, body: Body<'_>) {
        let from = env.source;
        let target = env.device;
        match body {
            Body::ProfileInquiry(_) => {
                let set = device.profiles(target);
                let reply = ProfileInquiryReply {
                    enabled: ProfileList::Ids(set.enabled),
                    disabled: ProfileList::Ids(set.disabled),
                };
                let out = self.envelope(target, from);
                self.send(Some(from), out, Body::ProfileInquiryReply(reply));
            }
            Body::SetProfileOn(p) => {
                self.set_profile(device, env, p.profile, ProfileState::Enabled, p.channels);
            }
            Body::SetProfileOff(p) => {
                self.set_profile(device, env, p.profile, ProfileState::Disabled, 0);
            }
            Body::ProfileDetailsInquiry(q) => {
                let mut buf = [0u8; 64];
                let known = device.profiles(target).contains(q.profile);
                match device
                    .profile_details(target, q.profile, q.target, &mut buf)
                    .filter(|_| known)
                {
                    Some(n) => {
                        let reply = ProfileDetailsReply {
                            profile: q.profile,
                            target: q.target,
                            data: buf.get(..n).unwrap_or_default(),
                        };
                        let out = self.envelope(target, from);
                        self.send(Some(from), out, Body::ProfileDetailsReply(reply));
                    }
                    None => self.nak(
                        env,
                        MessageKind::ProfileDetailsInquiry,
                        NakStatus::PROFILE_NOT_SUPPORTED,
                        Details::from_profile(q.profile),
                    ),
                }
            }
            Body::ProfileSpecificData(d) => device.profile_data(from, target, d.profile, d.data),
            _ => {}
        }
    }

    fn set_profile<D: Device>(
        &mut self,
        device: &mut D,
        env: &Envelope,
        profile: ProfileId,
        state: ProfileState,
        channels: u16,
    ) {
        let original = if state == ProfileState::Enabled {
            MessageKind::SetProfileOn
        } else {
            MessageKind::SetProfileOff
        };
        if !device.profiles(env.device).contains(profile) {
            self.nak(
                env,
                original,
                NakStatus::PROFILE_NOT_SUPPORTED,
                Details::from_profile(profile),
            );
            return;
        }
        // Reports go to the Broadcast MUID (M2-101 §7.10, §7.11).
        let out = self.envelope(env.device, Muid::BROADCAST);
        match device.set_profile(env.device, profile, state, channels) {
            ProfileOutcome::Enabled(n) => self.send(
                Some(env.source),
                out,
                Body::ProfileEnabled(ProfileChannels {
                    profile,
                    channels: n,
                }),
            ),
            ProfileOutcome::Disabled(n) => self.send(
                Some(env.source),
                out,
                Body::ProfileDisabled(ProfileChannels {
                    profile,
                    channels: n,
                }),
            ),
            ProfileOutcome::NotSupported => self.nak(
                env,
                original,
                NakStatus::PROFILE_NOT_SUPPORTED,
                Details::from_profile(profile),
            ),
        }
    }

    /// Announces a Profile newly added (and, if `enabled`, enabled) on
    /// `target` (M2-101 §7.4).
    pub fn profile_added(&mut self, target: DeviceId, profile: ProfileId, enabled: Option<u16>) {
        let out = self.envelope(target, Muid::BROADCAST);
        self.send(None, out, Body::ProfileAdded(ProfileReport { profile }));
        if let Some(channels) = enabled {
            self.send(
                None,
                out,
                Body::ProfileEnabled(ProfileChannels { profile, channels }),
            );
        }
    }

    /// Announces a Profile removed from `target`; if it was enabled, a
    /// Disabled Report goes first (M2-101 §7.5).
    pub fn profile_removed(&mut self, target: DeviceId, profile: ProfileId, was: ProfileState) {
        let out = self.envelope(target, Muid::BROADCAST);
        if was == ProfileState::Enabled {
            self.send(
                None,
                out,
                Body::ProfileDisabled(ProfileChannels {
                    profile,
                    channels: 0,
                }),
            );
        }
        self.send(None, out, Body::ProfileRemoved(ProfileReport { profile }));
    }

    /// Ends a MIDI Message Report after the app sent its MIDI messages
    /// (M2-101 §9.8).
    pub fn end_message_report(&mut self, peer: Muid, target: DeviceId) {
        let out = self.envelope(target, peer);
        self.send(
            Some(peer),
            out,
            Body::EndOfMessageReport(EndOfMessageReport),
        );
    }

    fn decl<'d, D: Device>(device: &'d D, h: &Header<'_>) -> Option<ResourceDecl<'d>> {
        let r = h.resource?;
        device
            .resources()
            .iter()
            .find(|d| r.eq_str(d.resource))
            .copied()
    }

    fn pe_reply(&mut self, env: &Envelope, kind: MessageKind, request_id: u8, header: &Header<'_>) {
        let mut hbuf = [0u8; REPLY_HEADER];
        let Ok(n) = header.write_to(&mut hbuf) else {
            return;
        };
        let chunk = PeChunk::single(request_id, hbuf.get(..n).unwrap_or_default(), &[]);
        let body = match kind {
            MessageKind::SetPropertyDataReply => Body::SetPropertyDataReply(chunk),
            MessageKind::SubscriptionReply => Body::SubscriptionReply(chunk),
            MessageKind::Notify => Body::Notify(chunk),
            _ => Body::GetPropertyDataReply(chunk),
        };
        let out = self.envelope(DeviceId::FunctionBlock, env.source);
        self.send(Some(env.source), out, body);
    }

    fn status_reply(
        &mut self,
        env: &Envelope,
        kind: MessageKind,
        request_id: u8,
        status: PeStatus,
    ) {
        self.pe_reply(env, kind, request_id, &Header::reply(status));
    }

    #[allow(clippy::too_many_lines)]
    fn property_exchange<D: Device>(
        &mut self,
        device: &mut D,
        now_ms: u64,
        env: &Envelope,
        body: Body<'_>,
    ) {
        let from = env.source;
        match body {
            Body::PeCapabilitiesInquiry(_) => {
                let caps = self.config.property_exchange.unwrap_or_default();
                let out = self.envelope(DeviceId::FunctionBlock, from);
                self.send(Some(from), out, Body::PeCapabilitiesReply(caps));
            }
            Body::GetPropertyData(c) => {
                let reply_kind = MessageKind::GetPropertyDataReply;
                let Ok(h) = Header::parse(c.header) else {
                    self.status_reply(env, reply_kind, c.request_id, PeStatus::BAD_REQUEST);
                    return;
                };
                if h.resource.is_none() {
                    self.status_reply(env, reply_kind, c.request_id, PeStatus::BAD_REQUEST);
                    return;
                }
                if !h.is_resource(names::RESOURCE_LIST) {
                    match Self::decl(device, &h) {
                        None if !device.resources().is_empty() => {
                            self.status_reply(env, reply_kind, c.request_id, PeStatus::NOT_FOUND);
                            return;
                        }
                        Some(d) if !d.can_get => {
                            self.status_reply(env, reply_kind, c.request_id, PeStatus::NOT_ALLOWED);
                            return;
                        }
                        Some(d) if d.require_res_id && h.res_id.is_none() => {
                            self.status_reply(env, reply_kind, c.request_id, PeStatus::BAD_REQUEST);
                            return;
                        }
                        _ => {}
                    }
                }
                if h.encoding() == Encoding::ZlibMcoded7 {
                    self.status_reply(
                        env,
                        reply_kind,
                        c.request_id,
                        PeStatus::UNSUPPORTED_MEDIA_TYPE,
                    );
                    return;
                }
                self.start_get(device, env, c.request_id, c.header, &h);
            }
            Body::SetPropertyData(c) => self.set_chunk(device, now_ms, env, &c),
            Body::Subscription(c) => {
                let reply_kind = MessageKind::SubscriptionReply;
                let Ok(h) = Header::parse(c.header) else {
                    self.status_reply(env, reply_kind, c.request_id, PeStatus::BAD_REQUEST);
                    return;
                };
                match h.command {
                    Some(Command::Start) => {
                        let ok = Self::decl(device, &h).is_some_and(|d| d.can_subscribe);
                        let slot = self.subs.iter().position(Option::is_none);
                        let req = copy_into::<SUB_RES>(c.header);
                        match (ok, slot, req) {
                            (false, _, _) => self.status_reply(
                                env,
                                reply_kind,
                                c.request_id,
                                PeStatus::NOT_ALLOWED,
                            ),
                            (true, Some(i), Some(request)) => {
                                let id = self.next_sub;
                                self.next_sub = self.next_sub.wrapping_add(1).max(1);
                                if let Some(s) = self.subs.get_mut(i) {
                                    *s = Some(Subscription {
                                        peer: from,
                                        id,
                                        request,
                                    });
                                }
                                let mut idbuf = [0u8; 8];
                                let text = sub_id_text(id, &mut idbuf);
                                device.subscription(from, &h, text, SubscriptionChange::Started);
                                let reply = Header {
                                    subscribe_id: Some(JsonStr::plain(text)),
                                    ..Header::reply(PeStatus::OK)
                                };
                                self.pe_reply(env, reply_kind, c.request_id, &reply);
                            }
                            _ => self.status_reply(
                                env,
                                reply_kind,
                                c.request_id,
                                PeStatus::TOO_MANY_REQUESTS,
                            ),
                        }
                    }
                    Some(Command::End) => {
                        let id = h.subscribe_id.and_then(parse_sub_id);
                        let pos = self.subs.iter().position(|s| {
                            s.as_ref()
                                .is_some_and(|s| s.peer == from && Some(s.id) == id)
                        });
                        if let Some(i) = pos {
                            if let Some(s) = self.subs.get_mut(i).and_then(Option::take) {
                                let mut idbuf = [0u8; 8];
                                let text = sub_id_text(s.id, &mut idbuf);
                                let req = Header::parse(s.request.as_slice()).unwrap_or_default();
                                device.subscription(from, &req, text, SubscriptionChange::Ended);
                            }
                            self.status_reply(env, reply_kind, c.request_id, PeStatus::OK);
                        } else {
                            self.status_reply(env, reply_kind, c.request_id, PeStatus::NOT_FOUND);
                        }
                    }
                    _ => self.status_reply(env, reply_kind, c.request_id, PeStatus::BAD_REQUEST),
                }
            }
            Body::Notify(c) => {
                // Terminate (144) or timeout (408) of a request in progress.
                for t in &mut self.transfers {
                    if t.as_ref()
                        .is_some_and(|t| t.peer == from && t.request_id == c.request_id)
                    {
                        *t = None;
                    }
                }
                if self.set.request_id() == Some(c.request_id)
                    && self.set_state.is_some_and(|s| s.peer == from)
                {
                    self.set.reset();
                    self.set_state = None;
                }
            }
            _ => {}
        }
    }

    /// Renders the body Mcoded7-encoded into `sink` (8-bit data allowed).
    fn render_mcoded7<D: Device, S: Sink>(
        device: &D,
        list: bool,
        h: &Header<'_>,
        sink: S,
    ) -> Result<(S, Option<ReplyExtras>), PeStatus> {
        let mut enc = mcoded7::Encoder::new(sink);
        let extras = if list {
            let mut w = JsonWriter::new(&mut enc);
            write_resource_list(device.resources(), &mut w);
            w.finish().map_err(|_| PeStatus::INTERNAL_ERROR)?;
            None
        } else {
            let meta = device.get(h, &mut enc)?;
            Some(ReplyExtras::from_meta(&meta))
        };
        Ok((enc.finish(), extras))
    }

    fn start_get<D: Device>(
        &mut self,
        device: &D,
        env: &Envelope,
        request_id: u8,
        raw_header: &[u8],
        h: &Header<'_>,
    ) {
        let reply_kind = MessageKind::GetPropertyDataReply;
        let list = h.is_resource(names::RESOURCE_LIST);
        let mcoded = h.encoding() == Encoding::Mcoded7;
        let counted = if mcoded {
            Self::render_mcoded7(device, list, h, Counter::default())
        } else {
            Self::render_plain(device, list, h, Counter::default())
        };
        let (total, extras) = match counted {
            Ok((c, e)) => (c.count(), e),
            Err(status) => {
                self.status_reply(env, reply_kind, request_id, status);
                return;
            }
        };
        let mut reply = Header::reply(PeStatus::OK);
        let mut media = ArrayBuf::<80>::new();
        let mut rev = ArrayBuf::<40>::new();
        if mcoded {
            reply.mutual_encoding = Some(Encoding::Mcoded7);
        }
        if let Some(e) = &extras {
            reply.total_count = e.total_count;
            reply.cache_time = e.cache_time;
            reply.timestamp = e.timestamp;
            media = e.media_type.clone();
            rev = e.state_rev.clone();
        }
        if let Ok(s) = core::str::from_utf8(media.as_slice())
            && !s.is_empty()
        {
            reply.media_type = Some(JsonStr::plain(s));
        }
        if let Ok(s) = core::str::from_utf8(rev.as_slice())
            && !s.is_empty()
        {
            reply.state_rev = Some(JsonStr::plain(s));
        }
        let mut reply_buf = ArrayBuf::<REPLY_HEADER>::new();
        let mut tmp = [0u8; REPLY_HEADER];
        let Ok(n) = reply.write_to(&mut tmp) else {
            self.status_reply(env, reply_kind, request_id, PeStatus::INTERNAL_ERROR);
            return;
        };
        reply_buf.try_extend(tmp.get(..n).unwrap_or_default());
        let limit = self.frame_limit(Some(env.source));
        let Ok(plan) = ChunkPlan::new(
            u32::try_from(limit).unwrap_or(u32::MAX),
            reply_buf.len(),
            total,
        ) else {
            self.status_reply(env, reply_kind, request_id, PeStatus::PAYLOAD_TOO_LARGE);
            return;
        };
        let Some(request) = copy_into::<REQ_HEADER>(raw_header) else {
            self.status_reply(env, reply_kind, request_id, PeStatus::BAD_REQUEST);
            return;
        };
        let Some(slot) = self.transfers.iter_mut().find(|t| t.is_none()) else {
            self.status_reply(env, reply_kind, request_id, PeStatus::TOO_MANY_REQUESTS);
            return;
        };
        *slot = Some(Transfer {
            peer: env.source,
            device: env.device,
            request_id,
            request,
            reply: reply_buf,
            plan,
            next: 1,
            mcoded7: mcoded,
        });
    }

    fn render_plain<D: Device, S: Sink>(
        device: &D,
        list: bool,
        h: &Header<'_>,
        sink: S,
    ) -> Result<(S, Option<ReplyExtras>), PeStatus> {
        let mut hb = HighBit {
            inner: sink,
            high: false,
        };
        let extras = if list {
            let mut w = JsonWriter::new(&mut hb);
            write_resource_list(device.resources(), &mut w);
            w.finish().map_err(|_| PeStatus::INTERNAL_ERROR)?;
            None
        } else {
            let meta = device.get(h, &mut hb)?;
            Some(ReplyExtras::from_meta(&meta))
        };
        if hb.high {
            return Err(PeStatus::UNSUPPORTED_MEDIA_TYPE);
        }
        Ok((hb.inner, extras))
    }

    fn set_chunk<D: Device>(
        &mut self,
        device: &mut D,
        now_ms: u64,
        env: &Envelope,
        c: &PeChunk<'_>,
    ) {
        let reply_kind = MessageKind::SetPropertyDataReply;
        if let Some(s) = self.set_state
            && s.peer != env.source
            && self.set.request_id().is_some()
        {
            self.status_reply(env, reply_kind, c.request_id, PeStatus::TOO_MANY_REQUESTS);
            return;
        }
        match self.set.push(c, now_ms) {
            Ok(Progress::Partial) => {
                self.set_state = Some(SetState {
                    peer: env.source,
                    device: env.device,
                });
            }
            Ok(Progress::Complete { usable }) => {
                self.set_state = None;
                let status = if usable {
                    self.apply_set(device)
                } else {
                    PeStatus::BAD_DATA
                };
                self.set.reset();
                self.status_reply(env, reply_kind, c.request_id, status);
            }
            Err(AssembleError::Busy) => {
                self.status_reply(env, reply_kind, c.request_id, PeStatus::TOO_MANY_REQUESTS);
            }
            Err(AssembleError::TooLarge) => {
                self.set_state = None;
                self.status_reply(env, reply_kind, c.request_id, PeStatus::PAYLOAD_TOO_LARGE);
            }
            Err(AssembleError::OutOfSequence) => {
                self.set_state = None;
                let details =
                    Details::property_exchange(c.request_id, c.this_chunk).unwrap_or_default();
                self.nak(
                    env,
                    MessageKind::SetPropertyData,
                    NakStatus::PE_CHUNKS_OUT_OF_SEQUENCE,
                    details,
                );
            }
        }
    }

    // A nested match reads better than the suggested map_or chains.
    #[allow(clippy::option_if_let_else)]
    fn apply_set<D: Device>(&mut self, device: &mut D) -> PeStatus {
        let header_len = self.set.header().len();
        let mut storage =
            core::mem::replace(&mut self.set, Assembler::new(ArrayBuf::new(), SET)).into_storage();
        let status = {
            let (head, data) = storage.as_mut_slice().split_at_mut(header_len.min(SET));
            match Header::parse(head) {
                Err(_) => PeStatus::BAD_REQUEST,
                Ok(h) => match Self::decl(device, &h) {
                    None if !device.resources().is_empty() => PeStatus::NOT_FOUND,
                    Some(d) if d.can_set == CanSet::None => PeStatus::NOT_ALLOWED,
                    Some(d) if h.set_partial && d.can_set != CanSet::Partial => {
                        PeStatus::NOT_ALLOWED
                    }
                    _ => match h.encoding() {
                        Encoding::Ascii => {
                            device.set(&h, data).map_or_else(|e| e, |()| PeStatus::OK)
                        }
                        Encoding::Mcoded7 => match mcoded7::decode_in_place(data) {
                            Ok(n) => device
                                .set(&h, data.get(..n).unwrap_or_default())
                                .map_or_else(|e| e, |()| PeStatus::OK),
                            Err(_) => PeStatus::BAD_DATA,
                        },
                        Encoding::ZlibMcoded7 => PeStatus::UNSUPPORTED_MEDIA_TYPE,
                    },
                },
            }
        };
        storage.clear();
        self.set = Assembler::new(storage, SET);
        status
    }

    /// Publishes a subscription update for `resource` to every subscriber
    /// (M2-103 §9.1). Returns how many were sent.
    pub fn publish<D: Device>(&mut self, device: &D, resource: &str, update: Update<'_>) -> usize {
        let mut sent = 0;
        for i in 0..SUBS {
            let Some(sub) = self.subs.get(i).cloned().flatten() else {
                continue;
            };
            let Ok(req) = Header::parse(sub.request.as_slice()) else {
                continue;
            };
            if !req.is_resource(resource) {
                continue;
            }
            self.send_update(device, &sub, &req, update);
            if update == Update::End
                && let Some(s) = self.subs.get_mut(i)
            {
                *s = None;
            }
            sent += 1;
        }
        sent
    }

    fn send_update<D: Device>(
        &mut self,
        device: &D,
        sub: &Subscription,
        req: &Header<'_>,
        update: Update<'_>,
    ) {
        let mut idbuf = [0u8; 8];
        let id = sub_id_text(sub.id, &mut idbuf);
        let mut header = Header {
            subscribe_id: Some(JsonStr::plain(id)),
            ..Header::EMPTY
        };
        let limit = self.frame_limit(Some(sub.peer));
        let mut data_buf = [0u8; FRAME];
        let mut data: &[u8] = &[];
        header.command = Some(match update {
            Update::Partial(d) => {
                data = d;
                Command::Partial
            }
            Update::Full => {
                let mut sink = SliceSink::new(&mut data_buf);
                let ok = device.get(req, &mut sink).is_ok();
                match sink.finish() {
                    Ok(n)
                        if ok
                            && data_buf
                                .get(..n)
                                .is_some_and(|d| d.iter().all(|b| b & 0x80 == 0)) =>
                    {
                        data = data_buf.get(..n).unwrap_or_default();
                        Command::Full
                    }
                    _ => Command::Notify,
                }
            }
            Update::Notify => Command::Notify,
            Update::End => Command::End,
        });
        let mut hbuf = [0u8; REPLY_HEADER];
        let Ok(mut n) = header.write_to(&mut hbuf) else {
            return;
        };
        let fits = ChunkPlan::new(u32::try_from(limit).unwrap_or(u32::MAX), n, data.len())
            .is_ok_and(|p| p.num_chunks() == 1);
        if !fits {
            // Too big for one chunk: ask for a refresh instead (§9.1.1).
            header.command = Some(Command::Notify);
            data = &[];
            n = header.write_to(&mut hbuf).unwrap_or(0);
        }
        let request_id = u8::try_from(sub.id & 0x7F).unwrap_or(0);
        let chunk = PeChunk::single(request_id, hbuf.get(..n).unwrap_or_default(), data);
        let out = self.envelope(DeviceId::FunctionBlock, sub.peer);
        self.send(Some(sub.peer), out, Body::Subscription(chunk));
    }

    /// Times out a stalled incoming Set (3 s, M2-103 §10.3) with Notify
    /// 408. Call regularly.
    pub fn poll(&mut self, now_ms: u64) {
        if self.set.expired(now_ms, TIMEOUT_MS) {
            if let (Some(id), Some(st)) = (self.set.request_id(), self.set_state) {
                let env = Envelope {
                    device: st.device,
                    version: CiVersion::CURRENT,
                    source: st.peer,
                    destination: self.muid,
                };
                self.status_reply(&env, MessageKind::Notify, id, PeStatus::TIMEOUT);
            }
            self.set.reset();
            self.set_state = None;
        }
    }

    /// The next outbound frame, written to `out` (at least `FRAME` bytes
    /// recommended). Queued replies first, then the next Get chunk.
    pub fn poll_transmit<D: Device>(&mut self, device: &D, out: &mut [u8]) -> Option<usize> {
        loop {
            if let Some(r) = self.out.pop(out) {
                match r {
                    Ok(n) => return Some(n),
                    Err(_) => continue,
                }
            }
            let idx = self.transfers.iter().position(Option::is_some)?;
            self.next_chunk(device, idx);
        }
    }

    fn next_chunk<D: Device>(&mut self, device: &D, idx: usize) {
        let Some(t) = self.transfers.get_mut(idx).and_then(Option::take) else {
            return;
        };
        let range = t.plan.range(t.next);
        let len = range.len().min(FRAME);
        let parsed = Header::parse(t.request.as_slice()).unwrap_or_default();
        let list = parsed.is_resource(names::RESOURCE_LIST);
        let window_buf = self.scratch.get_mut(..len).unwrap_or_default();
        let window = Window::new(range.start, window_buf);
        let rendered = if t.mcoded7 {
            Self::render_mcoded7(device, list, &parsed, window).map(|(w, _)| w.total())
        } else {
            Self::render_plain(device, list, &parsed, window).map(|(w, _)| w.total())
        };
        let env = Envelope {
            device: t.device,
            version: CiVersion::CURRENT,
            source: t.peer,
            destination: self.muid,
        };
        if rendered != Ok(t.plan.total()) {
            // The data changed under us (or failed): terminate (M2-103 §10.2).
            self.status_reply(&env, MessageKind::Notify, t.request_id, PeStatus::TERMINATE);
            return;
        }
        let chunk = PeChunk {
            request_id: t.request_id,
            header: if t.next == 1 { t.reply.as_slice() } else { &[] },
            num_chunks: t.plan.num_chunks(),
            this_chunk: t.next,
            data: self.scratch.get(..len).unwrap_or_default(),
        };
        let limit = self.frame_limit(Some(t.peer));
        let framing = self.config.framing;
        let msg = Message::new(
            Envelope {
                device: DeviceId::FunctionBlock,
                version: CiVersion::CURRENT,
                source: self.muid,
                destination: t.peer,
            },
            Body::GetPropertyDataReply(chunk),
        );
        if self
            .out
            .push_with(limit, |b| msg.encode(framing, b))
            .is_err()
        {
            self.dropped = self.dropped.saturating_add(1);
        }
        if t.next < t.plan.num_chunks() {
            let mut t = t;
            t.next += 1;
            if let Some(slot) = self.transfers.get_mut(idx) {
                *slot = Some(t);
            }
        }
    }

    /// Drops all queued frames and transfers (e.g. on transport reset).
    pub const fn reset_output(&mut self) {
        self.out.clear();
        self.transfers = [const { None }; TRANSFERS];
    }
}

impl<const FRAME: usize, const OUT: usize, const SET: usize, const SUBS: usize> core::fmt::Debug
    for Responder<'_, FRAME, OUT, SET, SUBS>
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Responder")
            .field("muid", &self.muid)
            .field("used", &self.used)
            .field("peers", &self.peers.iter().flatten().count())
            .field("queued_bytes", &self.out.len())
            .field("transfers", &self.transfers.iter().flatten().count())
            .field("set_in_progress", &self.set.request_id().is_some())
            .field("subscriptions", &self.subscriptions())
            .field("dropped", &self.dropped)
            .finish_non_exhaustive()
    }
}

/// Reply-header extras captured from [`GetMeta`] (owned, fixed size).
#[derive(Clone, Debug, Default)]
struct ReplyExtras {
    media_type: ArrayBuf<80>,
    state_rev: ArrayBuf<40>,
    total_count: Option<u32>,
    cache_time: Option<u32>,
    timestamp: Option<u64>,
}

impl ReplyExtras {
    fn from_meta(m: &GetMeta<'_>) -> Self {
        let mut e = Self {
            total_count: m.total_count,
            cache_time: m.cache_time,
            timestamp: m.timestamp,
            ..Self::default()
        };
        if let Some(s) = m.media_type {
            e.media_type.try_extend(s.as_bytes());
        }
        if let Some(s) = m.state_rev {
            e.state_rev.try_extend(s.as_bytes());
        }
        e
    }
}
