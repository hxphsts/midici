//! The Initiator: the host side of MIDI-CI (feature `alloc`).
//!
//! Discovers Responders, then drives Property Exchange: Get/Set with
//! request IDs (M2-101 §8.4), chunk reassembly (§8.3), Mcoded7 (and
//! zlib with the `zlib` feature) bodies, 3-second timeouts with Notify 408
//! (M2-103 §10.3), and subscriptions (§9). It also covers Profiles,
//! Endpoint Information and Process Inquiry. [`Foundation`] runs the whole
//! foundational sequence and returns typed results.
//!
//! Sans-io: call [`Initiator::handle`] for every received SysEx,
//! [`Initiator::poll`] regularly, and drain [`Initiator::pop_transmit`] and
//! [`Initiator::pop_event`].

use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::error::Error;
use crate::identity::DeviceIdentity;
use crate::message::{Body, Message};
use crate::mgmt::{
    Categories, Discovery, DiscoveryReply, EndpointInquiry, InvalidateMuid, Nak, NakStatus,
};
use crate::muid::Muid;
use crate::pe::chunk::{AssembleError, Assembler, ChunkPlan, Progress};
use crate::pe::header::{Command, Encoding, Header};
use crate::pe::json::JsonStr;
use crate::pe::mcoded7;
use crate::pe::resource::{
    ChannelEntry, Controller, CtrlMapEntry, DeviceInfo, ProgramEntry, ResourceInfo, names,
    parse_channel_list, parse_controllers, parse_ctrl_map_list, parse_device_info,
    parse_program_list, parse_resource_list,
};
use crate::pe::status::PeStatus;
use crate::pe::{PeCapabilities, PeChunk};
use crate::process::{MessageReportInquiry, ProcessFeatures, ProcessInquiry, ReportSelection};
use crate::profile::{
    ProfileChannels, ProfileDetailsInquiry, ProfileId, ProfileInquiry, SetProfileOff,
};
use crate::wire::{CiVersion, DeviceId, Envelope, Frame, Framing, MessageKind, is_midi_ci};

use super::Handled;

/// Initiator settings.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Who we are (sent in Discovery).
    pub identity: DeviceIdentity,
    /// Our Receivable Maximum SysEx Message Size (≥ 512 for PE/Profile
    /// Initiators, M2-101 §5.5.3).
    pub max_sysex: u32,
    /// The largest frame we send.
    pub max_frame: u32,
    /// Our Output Path Id (§5.5.4).
    pub output_path: u8,
    /// Framing of emitted frames.
    pub framing: Framing,
    /// Reply / inter-chunk timeout (M2-103 §10.3: 3000 ms).
    pub timeout_ms: u64,
    /// How long to collect Discovery replies (§5.5.5: at least 3000 ms).
    pub discovery_ms: u64,
    /// Largest reply body accepted.
    pub body_limit: usize,
}

impl Config {
    /// Defaults: 512-byte frames both ways, 3 s timeouts, 1 MiB bodies.
    #[must_use]
    pub const fn new(identity: DeviceIdentity) -> Self {
        Self {
            identity,
            max_sysex: 512,
            max_frame: 512,
            output_path: 0,
            framing: Framing::SysEx,
            timeout_ms: 3000,
            discovery_ms: 3000,
            body_limit: 1 << 20,
        }
    }
}

/// Identifies one Property Exchange request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(pub u8);

/// The kind of a tracked request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestKind {
    /// Inquiry: Get Property Data.
    Get,
    /// Inquiry: Set Property Data.
    Set,
    /// Subscription `start`.
    Subscribe,
    /// Subscription `end`.
    Unsubscribe,
}

/// Why a request or inquiry failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// No reply within the timeout (Notify 408 was sent for PE).
    Timeout,
    /// The Responder sent Notify with this status (144, 408).
    Notify(PeStatus),
    /// The Responder sent a NAK.
    Nak(NakStatus),
    /// Reply chunks arrived out of sequence.
    OutOfSequence,
    /// The reply exceeded [`Config::body_limit`].
    TooLarge,
    /// The reply data could not be decoded (Mcoded7/zlib) or was flagged
    /// unusable (chunk number 0).
    BadData,
    /// The peer is unknown or was invalidated.
    NoPeer,
}

/// A discovered Responder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// Its MUID.
    pub muid: Muid,
    /// Message version it used.
    pub version: CiVersion,
    /// Its Reply to Discovery.
    pub info: DiscoveryReply,
    /// PE capabilities, once known.
    pub pe: Option<PeCapabilities>,
}

/// Something that happened.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A Responder answered Discovery.
    Discovered(Peer),
    /// The discovery window closed.
    DiscoveryFinished,
    /// Two Responders answered with one MUID; it was invalidated
    /// (M2-101 §5.9.2).
    Collision(Muid),
    /// A peer's MUID was invalidated.
    Invalidated(Muid),
    /// Our MUID was invalidated or collided and has been replaced.
    MuidChanged {
        /// Old MUID.
        old: Muid,
        /// New MUID.
        new: Muid,
    },
    /// Reply to Endpoint Information.
    Endpoint {
        /// Peer.
        muid: Muid,
        /// Status echoed.
        status: u8,
        /// Information Data (status 0: Product Instance Id).
        data: Vec<u8>,
    },
    /// Reply to PE Capabilities.
    PeCapabilities {
        /// Peer.
        muid: Muid,
        /// Capabilities.
        caps: PeCapabilities,
    },
    /// A complete PE reply.
    Response {
        /// Peer.
        muid: Muid,
        /// The request.
        request: RequestId,
        /// What it answered.
        kind: RequestKind,
        /// Reply `status`.
        status: PeStatus,
        /// Reply Header Data (parse with [`Header::parse`]).
        header: Vec<u8>,
        /// Property Data, decoded per `mutualEncoding`.
        data: Vec<u8>,
    },
    /// A subscription message from the Responder (already acknowledged).
    SubscriptionUpdate {
        /// Peer.
        muid: Muid,
        /// `subscribeId`.
        subscribe_id: String,
        /// `partial`, `full`, `notify` or `end`.
        command: Command,
        /// Property Data.
        data: Vec<u8>,
    },
    /// A request or inquiry failed.
    Failed {
        /// Peer.
        muid: Muid,
        /// The PE request, if it was one.
        request: Option<RequestId>,
        /// The message that failed.
        inquiry: MessageKind,
        /// Why.
        reason: Failure,
    },
    /// Reply to Profile Inquiry.
    Profiles {
        /// Peer.
        muid: Muid,
        /// Target.
        device: DeviceId,
        /// Enabled.
        enabled: Vec<ProfileId>,
        /// Disabled.
        disabled: Vec<ProfileId>,
    },
    /// Profile Enabled/Disabled Report.
    ProfileState {
        /// Peer.
        muid: Muid,
        /// Target.
        device: DeviceId,
        /// Profile.
        profile: ProfileId,
        /// Now enabled?
        enabled: bool,
        /// Channels.
        channels: u16,
    },
    /// Profile Added/Removed Report.
    ProfileListChanged {
        /// Peer.
        muid: Muid,
        /// Target.
        device: DeviceId,
        /// Profile.
        profile: ProfileId,
        /// Added (true) or removed.
        added: bool,
    },
    /// Reply to Profile Details Inquiry.
    ProfileDetails {
        /// Peer.
        muid: Muid,
        /// Profile.
        profile: ProfileId,
        /// Inquiry target.
        target: u8,
        /// Data.
        data: Vec<u8>,
    },
    /// Reply to Process Inquiry Capabilities.
    ProcessCapabilities {
        /// Peer.
        muid: Muid,
        /// Features.
        features: ProcessFeatures,
    },
    /// Reply to MIDI Message Report (begin).
    MessageReportBegin {
        /// Peer.
        muid: Muid,
        /// Target.
        device: DeviceId,
        /// What will follow.
        selection: ReportSelection,
    },
    /// End of MIDI Message Report.
    MessageReportEnd {
        /// Peer.
        muid: Muid,
        /// Target.
        device: DeviceId,
    },
    /// A NAK not tied to a tracked request.
    Nak {
        /// Peer.
        muid: Muid,
        /// The refused message.
        original: MessageKind,
        /// Status.
        status: NakStatus,
        /// Message Text.
        text: String,
    },
}

#[derive(Debug)]
struct Pending {
    peer: Muid,
    id: u8,
    kind: RequestKind,
    deadline: u64,
    asm: Assembler<Vec<u8>>,
    encoding: Encoding,
}

#[derive(Debug)]
struct Inquiry {
    peer: Muid,
    kind: MessageKind,
    deadline: u64,
}

/// The Initiator. See the module docs.
#[derive(Debug)]
pub struct Initiator {
    config: Config,
    muid: Muid,
    rng: u32,
    peers: Vec<Peer>,
    discovery_until: Option<u64>,
    replied_this_round: Vec<Muid>,
    pending: Vec<Pending>,
    inquiries: Vec<Inquiry>,
    inbound: Vec<(Muid, u8, Assembler<Vec<u8>>)>,
    subscriptions: Vec<(Muid, String)>,
    next_id: u8,
    out: VecDeque<Vec<u8>>,
    events: VecDeque<Event>,
}

impl Initiator {
    /// An Initiator seeded with 32 random bits (for MUIDs).
    #[must_use]
    pub const fn new(config: Config, seed: u32) -> Self {
        let rng = seed | 1;
        Self {
            config,
            muid: Muid::from_random(seed),
            rng,
            peers: Vec::new(),
            discovery_until: None,
            replied_this_round: Vec::new(),
            pending: Vec::new(),
            inquiries: Vec::new(),
            inbound: Vec::new(),
            subscriptions: Vec::new(),
            next_id: 0,
            out: VecDeque::new(),
            events: VecDeque::new(),
        }
    }

    /// Our MUID.
    #[must_use]
    pub const fn muid(&self) -> Muid {
        self.muid
    }

    /// Discovered peers.
    #[must_use]
    pub fn peers(&self) -> &[Peer] {
        &self.peers
    }

    /// A discovered peer.
    #[must_use]
    pub fn peer(&self, muid: Muid) -> Option<&Peer> {
        self.peers.iter().find(|p| p.muid == muid)
    }

    /// Active subscriptions as `(peer, subscribeId)`.
    #[must_use]
    pub fn subscriptions(&self) -> &[(Muid, String)] {
        &self.subscriptions
    }

    /// Number of PE requests in flight.
    #[must_use]
    pub const fn in_flight(&self) -> usize {
        self.pending.len()
    }

    /// The next frame to send.
    pub fn pop_transmit(&mut self) -> Option<Vec<u8>> {
        self.out.pop_front()
    }

    /// The next event.
    pub fn pop_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    const fn random(&mut self) -> u32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    const fn envelope(&self, device: DeviceId, to: Muid) -> Envelope {
        Envelope {
            device,
            version: CiVersion::CURRENT,
            source: self.muid,
            destination: to,
        }
    }

    fn frame_limit(&self, peer: Muid) -> u32 {
        let peer_max = self.peer(peer).map_or(128, |p| p.info.max_sysex.max(128));
        self.config.max_frame.min(peer_max)
    }

    fn send(&mut self, envelope: Envelope, body: Body<'_>) -> Result<(), Error> {
        let bytes = Message::new(envelope, body).to_vec(self.config.framing)?;
        self.out.push_back(bytes);
        Ok(())
    }

    fn inquire(
        &mut self,
        now_ms: u64,
        peer: Muid,
        device: DeviceId,
        body: Body<'_>,
    ) -> Result<(), Error> {
        let kind = body.kind();
        let env = self.envelope(device, peer);
        self.send(env, body)?;
        self.inquiries.push(Inquiry {
            peer,
            kind,
            deadline: now_ms + self.config.timeout_ms,
        });
        Ok(())
    }

    /// Broadcasts Discovery and opens the reply window (M2-101 §5.5).
    ///
    /// # Errors
    ///
    /// Encoding errors (out-of-range identity fields).
    pub fn discover(&mut self, now_ms: u64) -> Result<(), Error> {
        let d = Discovery {
            identity: self.config.identity,
            categories: Categories::PROFILE_CONFIGURATION
                | Categories::PROPERTY_EXCHANGE
                | Categories::PROCESS_INQUIRY,
            max_sysex: self.config.max_sysex,
            output_path: self.config.output_path,
        };
        self.replied_this_round.clear();
        self.discovery_until = Some(now_ms + self.config.discovery_ms);
        let env = self.envelope(DeviceId::FunctionBlock, Muid::BROADCAST);
        self.send(env, Body::Discovery(d))
    }

    /// Announces that our MUID is going away (Invalidate MUID, §5.9).
    ///
    /// # Errors
    ///
    /// Encoding errors.
    pub fn invalidate_self(&mut self) -> Result<(), Error> {
        let env = self.envelope(DeviceId::FunctionBlock, Muid::BROADCAST);
        let target = self.muid;
        self.send(env, Body::InvalidateMuid(InvalidateMuid { target }))?;
        self.new_muid();
        Ok(())
    }

    fn new_muid(&mut self) {
        let old = self.muid;
        let mut new = Muid::from_random(self.random());
        while new == old {
            new = Muid::from_random(self.random());
        }
        self.muid = new;
        self.pending.clear();
        self.inquiries.clear();
        self.inbound.clear();
        self.subscriptions.clear();
        self.events.push_back(Event::MuidChanged { old, new });
    }

    /// Inquiry: Endpoint Information, status 0 (Product Instance Id).
    ///
    /// # Errors
    ///
    /// Encoding errors.
    pub fn endpoint_info(&mut self, now_ms: u64, peer: Muid) -> Result<(), Error> {
        self.inquire(
            now_ms,
            peer,
            DeviceId::FunctionBlock,
            Body::EndpointInquiry(EndpointInquiry {
                status: EndpointInquiry::PRODUCT_INSTANCE_ID,
            }),
        )
    }

    /// Inquiry: Property Exchange Capabilities (§8.5).
    ///
    /// # Errors
    ///
    /// Encoding errors.
    pub fn pe_capabilities(
        &mut self,
        now_ms: u64,
        peer: Muid,
        simultaneous: u8,
    ) -> Result<(), Error> {
        self.inquire(
            now_ms,
            peer,
            DeviceId::FunctionBlock,
            Body::PeCapabilitiesInquiry(PeCapabilities {
                simultaneous,
                major: 0,
                minor: 0,
            }),
        )
    }

    /// Profile Inquiry on `device` (§7.2).
    ///
    /// # Errors
    ///
    /// Encoding errors.
    pub fn profile_inquiry(
        &mut self,
        now_ms: u64,
        peer: Muid,
        device: DeviceId,
    ) -> Result<(), Error> {
        self.inquire(now_ms, peer, device, Body::ProfileInquiry(ProfileInquiry))
    }

    /// Set Profile On/Off (§7.8, §7.9). The answer arrives as a
    /// [`Event::ProfileState`] (or a NAK).
    ///
    /// # Errors
    ///
    /// Encoding errors.
    pub fn set_profile(
        &mut self,
        peer: Muid,
        device: DeviceId,
        profile: ProfileId,
        enable: bool,
        channels: u16,
    ) -> Result<(), Error> {
        let env = self.envelope(device, peer);
        let body = if enable {
            Body::SetProfileOn(ProfileChannels { profile, channels })
        } else {
            Body::SetProfileOff(SetProfileOff { profile })
        };
        self.send(env, body)
    }

    /// Profile Details Inquiry (§7.6).
    ///
    /// # Errors
    ///
    /// Encoding errors.
    pub fn profile_details(
        &mut self,
        now_ms: u64,
        peer: Muid,
        device: DeviceId,
        profile: ProfileId,
        target: u8,
    ) -> Result<(), Error> {
        self.inquire(
            now_ms,
            peer,
            device,
            Body::ProfileDetailsInquiry(ProfileDetailsInquiry { profile, target }),
        )
    }

    /// Inquiry: Process Inquiry Capabilities (§9.2).
    ///
    /// # Errors
    ///
    /// Encoding errors.
    pub fn process_capabilities(&mut self, now_ms: u64, peer: Muid) -> Result<(), Error> {
        self.inquire(
            now_ms,
            peer,
            DeviceId::FunctionBlock,
            Body::ProcessInquiry(ProcessInquiry),
        )
    }

    /// Inquiry: MIDI Message Report (§9.5).
    ///
    /// # Errors
    ///
    /// Encoding errors.
    pub fn message_report(
        &mut self,
        now_ms: u64,
        peer: Muid,
        device: DeviceId,
        inquiry: MessageReportInquiry,
    ) -> Result<(), Error> {
        self.inquire(now_ms, peer, device, Body::MessageReportInquiry(inquiry))
    }

    fn alloc_id(&mut self, peer: Muid) -> Option<u8> {
        for _ in 0..128 {
            let id = self.next_id & 0x7F;
            self.next_id = self.next_id.wrapping_add(1) & 0x7F;
            if !self.pending.iter().any(|p| p.peer == peer && p.id == id) {
                return Some(id);
            }
        }
        None
    }

    fn track(&mut self, now_ms: u64, peer: Muid, kind: RequestKind, id: u8, encoding: Encoding) {
        self.pending.push(Pending {
            peer,
            id,
            kind,
            deadline: now_ms + self.config.timeout_ms,
            asm: Assembler::new(Vec::new(), self.config.body_limit),
            encoding,
        });
    }

    /// Inquiry: Get Property Data with `header` (§8.7). Mcoded7 replies
    /// are decoded automatically.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] if the header does not fit one frame or 128
    /// requests are already in flight to `peer`.
    pub fn get(
        &mut self,
        now_ms: u64,
        peer: Muid,
        header: &Header<'_>,
    ) -> Result<RequestId, Error> {
        self.request(now_ms, peer, RequestKind::Get, header, &[])
    }

    /// Inquiry: Set Property Data (§8.9). `data` is raw; it is
    /// Mcoded7-encoded here if `header.mutual_encoding` says so, then split
    /// into chunks for the peer's maximum SysEx size.
    ///
    /// # Errors
    ///
    /// As [`Initiator::get`], plus [`Error::InvalidField`] for 8-bit data
    /// without Mcoded7.
    pub fn set(
        &mut self,
        now_ms: u64,
        peer: Muid,
        header: &Header<'_>,
        data: &[u8],
    ) -> Result<RequestId, Error> {
        self.request(now_ms, peer, RequestKind::Set, header, data)
    }

    /// Subscription `start` to `resource` (M2-103 §9.1).
    ///
    /// # Errors
    ///
    /// As [`Initiator::get`].
    pub fn subscribe(
        &mut self,
        now_ms: u64,
        peer: Muid,
        resource: &str,
        res_id: Option<&str>,
    ) -> Result<RequestId, Error> {
        let h = Header {
            command: Some(Command::Start),
            res_id: res_id.map(JsonStr::plain),
            ..Header::request(resource)
        };
        self.request(now_ms, peer, RequestKind::Subscribe, &h, &[])
    }

    /// Subscription `end` (M2-103 §9.5).
    ///
    /// # Errors
    ///
    /// As [`Initiator::get`].
    pub fn unsubscribe(
        &mut self,
        now_ms: u64,
        peer: Muid,
        subscribe_id: &str,
    ) -> Result<RequestId, Error> {
        let h = Header {
            command: Some(Command::End),
            subscribe_id: Some(JsonStr::plain(subscribe_id)),
            ..Header::EMPTY
        };
        self.subscriptions
            .retain(|(p, s)| !(*p == peer && s == subscribe_id));
        self.request(now_ms, peer, RequestKind::Unsubscribe, &h, &[])
    }

    fn request(
        &mut self,
        now_ms: u64,
        peer: Muid,
        kind: RequestKind,
        header: &Header<'_>,
        data: &[u8],
    ) -> Result<RequestId, Error> {
        let id = self
            .alloc_id(peer)
            .ok_or(Error::OutOfRange("request IDs exhausted"))?;
        let hbytes = header.to_vec();
        let encoded;
        let data = match (kind, header.encoding()) {
            (RequestKind::Set, Encoding::Mcoded7) => {
                encoded = mcoded7::encode_vec(data);
                &encoded[..]
            }
            #[cfg(feature = "zlib")]
            (RequestKind::Set, Encoding::ZlibMcoded7) => {
                encoded = crate::pe::zlib::encode(data);
                &encoded[..]
            }
            _ => {
                if data.iter().any(|b| b & 0x80 != 0) {
                    return Err(Error::InvalidField("8-bit Property Data needs Mcoded7"));
                }
                data
            }
        };
        let plan = ChunkPlan::new(self.frame_limit(peer), hbytes.len(), data.len())?;
        let env = self.envelope(DeviceId::FunctionBlock, peer);
        for n in 1..=plan.num_chunks() {
            let chunk = plan.chunk(id, &hbytes, data, n);
            let body = match kind {
                RequestKind::Get => Body::GetPropertyData(chunk),
                RequestKind::Set => Body::SetPropertyData(chunk),
                RequestKind::Subscribe | RequestKind::Unsubscribe => Body::Subscription(chunk),
            };
            self.send(env, body)?;
        }
        let enc = if kind == RequestKind::Get {
            header.encoding()
        } else {
            Encoding::Ascii
        };
        self.track(now_ms, peer, kind, id, enc);
        Ok(RequestId(id))
    }

    /// Expires timeouts: PE requests (sending Notify 408), inquiries, the
    /// discovery window, stalled inbound subscription messages.
    pub fn poll(&mut self, now_ms: u64) {
        if self.discovery_until.is_some_and(|t| now_ms >= t) {
            self.discovery_until = None;
            self.events.push_back(Event::DiscoveryFinished);
        }
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending.get(i).is_some_and(|p| now_ms > p.deadline) {
                let p = self.pending.remove(i);
                let h = Header::reply(PeStatus::TIMEOUT).to_vec();
                let env = self.envelope(DeviceId::FunctionBlock, p.peer);
                let _ = self.send(env, Body::Notify(PeChunk::single(p.id, &h, &[])));
                self.events.push_back(Event::Failed {
                    muid: p.peer,
                    request: Some(RequestId(p.id)),
                    inquiry: request_message(p.kind),
                    reason: Failure::Timeout,
                });
            } else {
                i += 1;
            }
        }
        let timeout = self.config.timeout_ms;
        let (expired, kept): (Vec<_>, Vec<_>) =
            self.inquiries.drain(..).partition(|q| now_ms > q.deadline);
        self.inquiries = kept;
        for q in expired {
            self.events.push_back(Event::Failed {
                muid: q.peer,
                request: None,
                inquiry: q.kind,
                reason: Failure::Timeout,
            });
        }
        self.inbound.retain(|(_, _, a)| !a.expired(now_ms, timeout));
    }

    fn answered(&mut self, peer: Muid, kind: MessageKind) {
        if let Some(i) = self
            .inquiries
            .iter()
            .position(|q| q.peer == peer && q.kind == kind)
        {
            self.inquiries.remove(i);
        }
    }

    fn fail_peer(&mut self, peer: Muid) {
        let pend: Vec<_> = self
            .pending
            .iter()
            .filter(|p| p.peer == peer)
            .map(|p| (p.id, p.kind))
            .collect();
        for (id, kind) in pend {
            self.events.push_back(Event::Failed {
                muid: peer,
                request: Some(RequestId(id)),
                inquiry: request_message(kind),
                reason: Failure::NoPeer,
            });
        }
        self.pending.retain(|p| p.peer != peer);
        self.inquiries.retain(|q| q.peer != peer);
        self.inbound.retain(|(m, _, _)| *m != peer);
        self.subscriptions.retain(|(m, _)| *m != peer);
        self.peers.retain(|p| p.muid != peer);
    }

    /// Handles one received SysEx message.
    #[allow(clippy::too_many_lines)]
    pub fn handle(&mut self, now_ms: u64, bytes: &[u8]) -> Handled {
        if !is_midi_ci(bytes) {
            return Handled::NotCi;
        }
        let frame = match Frame::parse(bytes) {
            Ok(f) => f,
            Err(e) => return Handled::Malformed(e),
        };
        let env = frame.header.envelope;
        let to_us = env.destination == self.muid;
        if !to_us && !env.destination.is_broadcast() {
            return Handled::Ignored;
        }
        if env.version.has_reserved_bits() {
            if to_us {
                let nak = Nak::new(frame.header.kind, NakStatus::VERSION_NOT_SUPPORTED);
                let _ = self.send(self.envelope(env.device, env.source), Body::Nak(nak));
            }
            return Handled::Malformed(Error::UnsupportedVersion(env.version.byte()));
        }
        let msg = match Message::decode(bytes) {
            Ok(m) => m,
            Err(e) => return Handled::Malformed(e),
        };
        let from = env.source;
        if from == self.muid && msg.kind() != MessageKind::Discovery {
            return Handled::Ignored;
        }
        match msg.body {
            Body::Discovery(d) => {
                if from == self.muid {
                    self.new_muid();
                }
                // Every CI device answers Discovery (M2-101 §4.1).
                let reply = DiscoveryReply {
                    identity: self.config.identity,
                    categories: Categories::NONE,
                    max_sysex: self.config.max_sysex,
                    output_path: d.output_path,
                    function_block: None,
                };
                let _ = self.send(
                    self.envelope(DeviceId::FunctionBlock, from),
                    Body::DiscoveryReply(reply),
                );
            }
            Body::DiscoveryReply(info) if to_us => {
                if self.replied_this_round.contains(&from) {
                    // Two Responders share a MUID (M2-101 §5.9.2).
                    let env = self.envelope(DeviceId::FunctionBlock, Muid::BROADCAST);
                    let _ = self.send(env, Body::InvalidateMuid(InvalidateMuid { target: from }));
                    self.fail_peer(from);
                    self.events.push_back(Event::Collision(from));
                    return Handled::Processed;
                }
                self.replied_this_round.push(from);
                let peer = Peer {
                    muid: from,
                    version: env.version,
                    info,
                    pe: None,
                };
                if let Some(p) = self.peers.iter_mut().find(|p| p.muid == from) {
                    *p = peer.clone();
                } else {
                    self.peers.push(peer.clone());
                }
                self.events.push_back(Event::Discovered(peer));
            }
            Body::InvalidateMuid(inv) => {
                if inv.target == self.muid {
                    self.new_muid();
                } else if self.peer(inv.target).is_some() {
                    self.fail_peer(inv.target);
                    self.events.push_back(Event::Invalidated(inv.target));
                }
            }
            Body::EndpointReply(r) if to_us => {
                self.answered(from, MessageKind::EndpointInquiry);
                self.events.push_back(Event::Endpoint {
                    muid: from,
                    status: r.status,
                    data: r.data.to_vec(),
                });
            }
            Body::PeCapabilitiesReply(caps) if to_us => {
                self.answered(from, MessageKind::PeCapabilitiesInquiry);
                if let Some(p) = self.peers.iter_mut().find(|p| p.muid == from) {
                    p.pe = Some(caps);
                }
                self.events
                    .push_back(Event::PeCapabilities { muid: from, caps });
            }
            Body::GetPropertyDataReply(c)
            | Body::SetPropertyDataReply(c)
            | Body::SubscriptionReply(c)
                if to_us =>
            {
                self.reply_chunk(now_ms, from, &c);
            }
            Body::Subscription(c) if to_us => self.subscription_chunk(now_ms, from, env.device, &c),
            Body::Notify(c) if to_us => {
                let status = Header::parse(c.header).ok().and_then(|h| h.status);
                if let Some(i) = self
                    .pending
                    .iter()
                    .position(|p| p.peer == from && p.id == c.request_id)
                {
                    if status == Some(PeStatus::TIMEOUT_WAIT) {
                        if let Some(p) = self.pending.get_mut(i) {
                            p.deadline = now_ms + self.config.timeout_ms;
                        }
                    } else {
                        let p = self.pending.remove(i);
                        self.events.push_back(Event::Failed {
                            muid: from,
                            request: Some(RequestId(p.id)),
                            inquiry: request_message(p.kind),
                            reason: Failure::Notify(status.unwrap_or(PeStatus::TERMINATE)),
                        });
                    }
                }
            }
            Body::Nak(n) if to_us => self.on_nak(from, &n),
            Body::ProfileInquiryReply(r) if to_us => {
                self.answered(from, MessageKind::ProfileInquiry);
                self.events.push_back(Event::Profiles {
                    muid: from,
                    device: env.device,
                    enabled: r.enabled.iter().collect(),
                    disabled: r.disabled.iter().collect(),
                });
            }
            Body::ProfileEnabled(p) | Body::ProfileDisabled(p) => {
                self.events.push_back(Event::ProfileState {
                    muid: from,
                    device: env.device,
                    profile: p.profile,
                    enabled: matches!(msg.body, Body::ProfileEnabled(_)),
                    channels: p.channels,
                });
            }
            Body::ProfileAdded(p) | Body::ProfileRemoved(p) => {
                self.events.push_back(Event::ProfileListChanged {
                    muid: from,
                    device: env.device,
                    profile: p.profile,
                    added: matches!(msg.body, Body::ProfileAdded(_)),
                });
            }
            Body::ProfileDetailsReply(r) if to_us => {
                self.answered(from, MessageKind::ProfileDetailsInquiry);
                self.events.push_back(Event::ProfileDetails {
                    muid: from,
                    profile: r.profile,
                    target: r.target,
                    data: r.data.to_vec(),
                });
            }
            Body::ProcessInquiryReply(r) if to_us => {
                self.answered(from, MessageKind::ProcessInquiryCapabilities);
                self.events.push_back(Event::ProcessCapabilities {
                    muid: from,
                    features: r.features,
                });
            }
            Body::MessageReportReply(r) if to_us => {
                self.answered(from, MessageKind::MidiMessageReportInquiry);
                self.events.push_back(Event::MessageReportBegin {
                    muid: from,
                    device: env.device,
                    selection: r.selection,
                });
            }
            Body::EndOfMessageReport(_) if to_us => {
                self.events.push_back(Event::MessageReportEnd {
                    muid: from,
                    device: env.device,
                });
            }
            Body::Ack(_) => {}
            _ if to_us => {
                // An inquiry to us: this role answers only Discovery.
                let nak = Nak::new(msg.kind(), NakStatus::NOT_SUPPORTED);
                let _ = self.send(self.envelope(env.device, from), Body::Nak(nak));
            }
            _ => return Handled::Ignored,
        }
        Handled::Processed
    }

    fn on_nak(&mut self, from: Muid, n: &Nak<'_>) {
        let Some(r) = n.report else {
            // A v1 NAK carries no original: fail the oldest inquiry.
            if let Some(i) = self.inquiries.iter().position(|q| q.peer == from) {
                let q = self.inquiries.remove(i);
                self.events.push_back(Event::Failed {
                    muid: from,
                    request: None,
                    inquiry: q.kind,
                    reason: Failure::Nak(NakStatus::NAK),
                });
            }
            return;
        };
        if r.original.category() == crate::wire::Category::PropertyExchange {
            let (rid, _) = r.details.as_property_exchange();
            if let Some(i) = self
                .pending
                .iter()
                .position(|p| p.peer == from && p.id == rid)
            {
                let p = self.pending.remove(i);
                self.events.push_back(Event::Failed {
                    muid: from,
                    request: Some(RequestId(p.id)),
                    inquiry: r.original,
                    reason: Failure::Nak(r.status),
                });
                return;
            }
        }
        if let Some(i) = self
            .inquiries
            .iter()
            .position(|q| q.peer == from && q.kind == r.original)
        {
            self.inquiries.remove(i);
            self.events.push_back(Event::Failed {
                muid: from,
                request: None,
                inquiry: r.original,
                reason: Failure::Nak(r.status),
            });
            return;
        }
        self.events.push_back(Event::Nak {
            muid: from,
            original: r.original,
            status: r.status,
            text: String::from(r.text.as_str()),
        });
    }

    fn reply_chunk(&mut self, now_ms: u64, from: Muid, c: &PeChunk<'_>) {
        let Some(i) = self
            .pending
            .iter()
            .position(|p| p.peer == from && p.id == c.request_id)
        else {
            return;
        };
        let timeout = self.config.timeout_ms;
        let progress = self.pending.get_mut(i).map(|p| {
            p.deadline = now_ms + timeout;
            p.asm.push(c, now_ms)
        });
        let reason = match progress {
            Some(Ok(Progress::Partial)) | None => return,
            Some(Ok(Progress::Complete { usable })) => {
                let p = self.pending.remove(i);
                self.complete(from, &p, usable);
                return;
            }
            Some(Err(AssembleError::TooLarge)) => Failure::TooLarge,
            Some(Err(_)) => Failure::OutOfSequence,
        };
        let p = self.pending.remove(i);
        self.events.push_back(Event::Failed {
            muid: from,
            request: Some(RequestId(p.id)),
            inquiry: request_message(p.kind),
            reason,
        });
    }

    fn complete(&mut self, from: Muid, p: &Pending, usable: bool) {
        let header = p.asm.header().to_vec();
        let raw = p.asm.data().to_vec();
        let h = Header::parse(&header).unwrap_or_default();
        let status = h.status.unwrap_or(PeStatus::BAD_DATA);
        let encoding = h.mutual_encoding.unwrap_or(p.encoding);
        let data = if usable {
            decode_body(encoding, &raw, self.config.body_limit)
        } else {
            None
        };
        let Some(data) = data else {
            self.events.push_back(Event::Failed {
                muid: from,
                request: Some(RequestId(p.id)),
                inquiry: request_message(p.kind),
                reason: Failure::BadData,
            });
            return;
        };
        if p.kind == RequestKind::Subscribe
            && status.is_success()
            && let Some(id) = h.subscribe_id
        {
            self.subscriptions.push((from, id.to_string()));
        }
        self.events.push_back(Event::Response {
            muid: from,
            request: RequestId(p.id),
            kind: p.kind,
            status,
            header,
            data,
        });
    }

    fn subscription_chunk(&mut self, now_ms: u64, from: Muid, device: DeviceId, c: &PeChunk<'_>) {
        let limit = self.config.body_limit;
        let pos = self
            .inbound
            .iter()
            .position(|(m, id, _)| *m == from && *id == c.request_id);
        let i = pos.unwrap_or_else(|| {
            self.inbound
                .push((from, c.request_id, Assembler::new(Vec::new(), limit)));
            self.inbound.len() - 1
        });
        let Some((_, _, asm)) = self.inbound.get_mut(i) else {
            return;
        };
        let result = asm.push(c, now_ms);
        let (status, done) = match result {
            Ok(Progress::Partial) => return,
            Ok(Progress::Complete { usable }) => (
                if usable {
                    PeStatus::OK
                } else {
                    PeStatus::BAD_DATA
                },
                true,
            ),
            Err(AssembleError::TooLarge) => (PeStatus::PAYLOAD_TOO_LARGE, false),
            Err(_) => (PeStatus::BAD_DATA, false),
        };
        let (_, _, asm) = self.inbound.remove(i);
        let h = Header::parse(asm.header()).unwrap_or_default();
        // Acknowledge (M2-103 §9.2).
        let reply = Header::reply(status).to_vec();
        let _ = self.send(
            self.envelope(device, from),
            Body::SubscriptionReply(PeChunk::single(c.request_id, &reply, &[])),
        );
        if !done {
            return;
        }
        let (Some(command), Some(id)) = (h.command, h.subscribe_id) else {
            return;
        };
        let id = id.to_string();
        let Some(data) = decode_body(h.encoding(), asm.data(), limit) else {
            return;
        };
        if command == Command::End {
            self.subscriptions
                .retain(|(m, s)| !(*m == from && *s == id));
        }
        self.events.push_back(Event::SubscriptionUpdate {
            muid: from,
            subscribe_id: id,
            command,
            data,
        });
    }
}

const fn request_message(kind: RequestKind) -> MessageKind {
    match kind {
        RequestKind::Get => MessageKind::GetPropertyData,
        RequestKind::Set => MessageKind::SetPropertyData,
        RequestKind::Subscribe | RequestKind::Unsubscribe => MessageKind::Subscription,
    }
}

fn decode_body(encoding: Encoding, raw: &[u8], limit: usize) -> Option<Vec<u8>> {
    match encoding {
        Encoding::Ascii => Some(raw.to_vec()),
        Encoding::Mcoded7 => mcoded7::decode_vec(raw).ok(),
        #[cfg(feature = "zlib")]
        Encoding::ZlibMcoded7 => crate::pe::zlib::decode(raw, limit).ok(),
        #[cfg(not(feature = "zlib"))]
        Encoding::ZlibMcoded7 => {
            let _ = limit;
            None
        }
    }
}

/// Results of the foundational sequence (M2-103 §7; M2-105; M2-117).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoundationReport {
    /// PE capabilities.
    pub capabilities: Option<PeCapabilities>,
    /// ResourceList.
    pub resources: Vec<ResourceInfo>,
    /// DeviceInfo.
    pub device_info: Option<DeviceInfo<String>>,
    /// ChannelList.
    pub channels: Option<Vec<ChannelEntry<String>>>,
    /// AllCtrlList.
    pub controllers: Option<Vec<Controller<String>>>,
    /// CtrlMapList per `ctrlMapId`.
    pub ctrl_maps: Vec<(String, Vec<CtrlMapEntry<String>>)>,
    /// ProgramList.
    pub programs: Option<Vec<ProgramEntry<String>>>,
    /// Steps that failed: resource name and reason.
    pub errors: Vec<(String, FoundationError)>,
}

/// Why one foundational step failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoundationError {
    /// The request failed.
    Failed(Failure),
    /// The reply status was not 2xx.
    Status(PeStatus),
    /// The reply did not parse.
    Parse(Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Capabilities,
    Get(String),
    Done,
}

/// Runs Discovery → PE Capabilities → ResourceList → DeviceInfo →
/// ChannelList → AllCtrlList (+ CtrlMapList per `ctrlMapId`) →
/// ProgramList against one peer, skipping resources the peer does not
/// list.
///
/// Feed every [`Event`] to [`Foundation::on_event`]; when it returns
/// `true` the sequence is done and [`Foundation::report`] has the results.
#[derive(Clone, Debug)]
pub struct Foundation {
    peer: Muid,
    step: Step,
    request: Option<RequestId>,
    queue: VecDeque<String>,
    report: FoundationReport,
}

impl Foundation {
    /// Starts the sequence against a discovered `peer`.
    ///
    /// # Errors
    ///
    /// Encoding errors from the first inquiry.
    pub fn start(initiator: &mut Initiator, now_ms: u64, peer: Muid) -> Result<Self, Error> {
        initiator.pe_capabilities(now_ms, peer, 1)?;
        Ok(Self {
            peer,
            step: Step::Capabilities,
            request: None,
            queue: VecDeque::new(),
            report: FoundationReport::default(),
        })
    }

    /// The peer.
    #[must_use]
    pub const fn peer(&self) -> Muid {
        self.peer
    }

    /// Whether the sequence is finished.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.step == Step::Done
    }

    /// The results so far.
    #[must_use]
    pub const fn report(&self) -> &FoundationReport {
        &self.report
    }

    /// The results.
    #[must_use]
    pub fn into_report(self) -> FoundationReport {
        self.report
    }

    fn listed(&self, name: &str) -> Option<&ResourceInfo> {
        self.report.resources.iter().find(|r| r.resource == name)
    }

    fn next(&mut self, ini: &mut Initiator, now_ms: u64) {
        while let Some(name) = self.queue.pop_front() {
            let (resource, res_id) = match name.split_once('#') {
                Some((r, id)) => (String::from(r), Some(String::from(id))),
                None => (name.clone(), None),
            };
            let h = Header {
                res_id: res_id.as_deref().map(JsonStr::plain),
                ..Header::request(&resource)
            };
            match ini.get(now_ms, self.peer, &h) {
                Ok(id) => {
                    self.request = Some(id);
                    self.step = Step::Get(name);
                    return;
                }
                Err(e) => self.report.errors.push((name, FoundationError::Parse(e))),
            }
        }
        self.step = Step::Done;
    }

    /// Feeds an event; returns `true` once the sequence has finished.
    #[allow(clippy::too_many_lines)]
    pub fn on_event(&mut self, ini: &mut Initiator, now_ms: u64, event: &Event) -> bool {
        match (&self.step, event) {
            (Step::Capabilities, Event::PeCapabilities { muid, caps }) if *muid == self.peer => {
                self.report.capabilities = Some(*caps);
                self.queue.push_back(String::from(names::RESOURCE_LIST));
                self.next(ini, now_ms);
            }
            (
                Step::Capabilities,
                Event::Failed {
                    muid,
                    request: None,
                    inquiry: MessageKind::PeCapabilitiesInquiry,
                    reason,
                },
            ) if *muid == self.peer => {
                self.report.errors.push((
                    String::from("PeCapabilities"),
                    FoundationError::Failed(*reason),
                ));
                self.queue.push_back(String::from(names::RESOURCE_LIST));
                self.next(ini, now_ms);
            }
            (
                Step::Get(name),
                Event::Response {
                    muid,
                    request,
                    status,
                    data,
                    ..
                },
            ) if *muid == self.peer && Some(*request) == self.request => {
                let name = name.clone();
                if status.is_success() {
                    if let Err(e) = self.store(&name, data) {
                        self.report
                            .errors
                            .push((name.clone(), FoundationError::Parse(e)));
                    }
                } else {
                    self.report
                        .errors
                        .push((name.clone(), FoundationError::Status(*status)));
                }
                if name == names::RESOURCE_LIST {
                    for r in [
                        names::DEVICE_INFO,
                        names::CHANNEL_LIST,
                        names::ALL_CTRL_LIST,
                    ] {
                        if self
                            .listed(r)
                            .is_some_and(|i| i.can_get && !i.require_res_id)
                        {
                            self.queue.push_back(String::from(r));
                        }
                    }
                    if self
                        .listed(names::PROGRAM_LIST)
                        .is_some_and(|i| i.can_get && !i.require_res_id)
                    {
                        self.queue.push_back(String::from(names::PROGRAM_LIST));
                    }
                }
                if name == names::ALL_CTRL_LIST && self.listed(names::CTRL_MAP_LIST).is_some() {
                    let mut ids: Vec<String> = self
                        .report
                        .controllers
                        .iter()
                        .flatten()
                        .filter_map(|c| c.ctrl_map_id.clone())
                        .collect();
                    ids.sort();
                    ids.dedup();
                    // Before ProgramList, keeping the documented order.
                    for id in ids.into_iter().rev() {
                        self.queue
                            .push_front(alloc::format!("{}#{id}", names::CTRL_MAP_LIST));
                    }
                }
                self.next(ini, now_ms);
            }
            (
                Step::Get(name),
                Event::Failed {
                    muid,
                    request,
                    reason,
                    ..
                },
            ) if *muid == self.peer && *request == self.request => {
                self.report
                    .errors
                    .push((name.clone(), FoundationError::Failed(*reason)));
                if name == names::RESOURCE_LIST {
                    self.queue.clear();
                }
                self.next(ini, now_ms);
            }
            (_, Event::Invalidated(m)) if *m == self.peer => self.step = Step::Done,
            _ => {}
        }
        self.is_done()
    }

    fn store(&mut self, name: &str, data: &[u8]) -> Result<(), Error> {
        let r = &mut self.report;
        if name == names::RESOURCE_LIST {
            r.resources = parse_resource_list(data)?;
        } else if name == names::DEVICE_INFO {
            r.device_info = Some(parse_device_info(data)?);
        } else if name == names::CHANNEL_LIST {
            r.channels = Some(parse_channel_list(data)?);
        } else if name == names::ALL_CTRL_LIST {
            r.controllers = Some(parse_controllers(data)?);
        } else if name == names::PROGRAM_LIST {
            r.programs = Some(parse_program_list(data)?);
        } else if let Some(id) = name.strip_prefix("CtrlMapList#") {
            r.ctrl_maps
                .push((String::from(id), parse_ctrl_map_list(data)?));
        }
        Ok(())
    }
}
