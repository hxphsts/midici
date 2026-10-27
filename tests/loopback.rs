//! Initiator ↔ Responder loopback conformance suite.
#![cfg(feature = "alloc")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::cast_possible_truncation
)]

use midici::engine::Handled;
use midici::engine::initiator::{
    Config as IConfig, Event, Failure, Foundation, Initiator, RequestKind,
};
use midici::engine::responder::{
    Config as RConfig, Device, GetMeta, ProfileOutcome, ProfileSet, Responder, Update,
};
use midici::identity::{DeviceIdentity, ManufacturerId};
use midici::message::{Body, Message};
use midici::mgmt::{Categories, Discovery, NakStatus};
use midici::muid::Muid;
use midici::pe::PeCapabilities;
use midici::pe::header::{Command, Encoding, Header};
use midici::pe::json::{JsonStr, JsonWriter};
use midici::pe::resource::{
    CanSet, ChannelEntry, Controller, CtrlMapEntry, CtrlType, DeviceInfo, ProgramEntry,
    ResourceDecl, names, write_list,
};
use midici::pe::sink::Sink;
use midici::pe::status::PeStatus;
use midici::process::{
    ChannelControllers, DataControl, MessageReportInquiry, ProcessFeatures, ReportSelection,
};
use midici::profile::{ChannelCount, ProfileId, TARGET_CHANNEL_COUNT};
use midici::wire::{CiVersion, DeviceId, Envelope, Framing, MessageKind};

const ID: DeviceIdentity = DeviceIdentity {
    manufacturer: ManufacturerId::NON_COMMERCIAL,
    family: 1,
    model: 2,
    revision: [0, 1, 0, 0],
};
const PROFILE: ProfileId = ProfileId::standard(0x21, 0x00, 0x01, 0x01);

static RESOURCES: &[ResourceDecl<'static>] = &[
    ResourceDecl::new(names::DEVICE_INFO),
    ResourceDecl {
        can_subscribe: true,
        ..ResourceDecl::new(names::CHANNEL_LIST)
    },
    ResourceDecl::new(names::ALL_CTRL_LIST),
    ResourceDecl {
        require_res_id: true,
        ..ResourceDecl::new(names::CTRL_MAP_LIST)
    },
    ResourceDecl::new(names::PROGRAM_LIST),
    ResourceDecl {
        can_set: CanSet::Full,
        require_res_id: true,
        media_types: &["application/octet-stream"],
        encodings: &["Mcoded7"],
        ..ResourceDecl::new(names::STATE)
    },
    ResourceDecl {
        can_set: CanSet::Partial,
        can_subscribe: true,
        ..ResourceDecl::new("X-Param")
    },
];

struct Synth {
    rng: u32,
    state: Vec<u8>,
    param: u32,
    profile_on: bool,
    subs: Vec<(String, bool)>,
    channels: usize,
}

impl Synth {
    fn new() -> Self {
        Self {
            rng: 0x1234_5678,
            state: (0..=255u8).cycle().take(600).collect(),
            param: 7,
            profile_on: false,
            subs: Vec::new(),
            channels: 16,
        }
    }
}

impl Device for Synth {
    fn random(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.rng
    }

    fn profiles(&self, target: DeviceId) -> ProfileSet<'_> {
        if target != DeviceId::FunctionBlock {
            return ProfileSet::default();
        }
        if self.profile_on {
            ProfileSet {
                enabled: core::slice::from_ref(&PROFILE),
                disabled: &[],
            }
        } else {
            ProfileSet {
                enabled: &[],
                disabled: core::slice::from_ref(&PROFILE),
            }
        }
    }

    fn set_profile(&mut self, _: DeviceId, _: ProfileId, enable: bool, _: u16) -> ProfileOutcome {
        self.profile_on = enable;
        if enable {
            ProfileOutcome::Enabled(0)
        } else {
            ProfileOutcome::Disabled(0)
        }
    }

    fn profile_details(&self, _: DeviceId, _: ProfileId, t: u8, out: &mut [u8]) -> Option<usize> {
        (t == TARGET_CHANNEL_COUNT).then(|| {
            let b = ChannelCount { in_use: 0, max: 16 }.to_bytes().unwrap();
            out[..4].copy_from_slice(&b);
            4
        })
    }

    fn resources(&self) -> &[ResourceDecl<'_>] {
        RESOURCES
    }

    fn get(&self, req: &Header<'_>, out: &mut dyn Sink) -> Result<GetMeta<'_>, PeStatus> {
        if req.is_resource(names::STATE) {
            out.put(&self.state);
            return Ok(GetMeta {
                media_type: Some("application/octet-stream"),
                state_rev: Some("rev1"),
                ..GetMeta::default()
            });
        }
        let mut w = JsonWriter::new(out);
        if req.is_resource(names::DEVICE_INFO) {
            DeviceInfo {
                manufacturer_id: [0x7D, 0, 0],
                family_id: [1, 0],
                model_id: [2, 0],
                version_id: [0, 1, 0, 0],
                manufacturer: "Educational Use",
                family: "Test",
                model: "Loopback Synth",
                version: "0.1",
                serial_number: Some("SN-1"),
            }
            .write(&mut w);
        } else if req.is_resource(names::CHANNEL_LIST) {
            let entries: Vec<ChannelEntry<String>> = (1..=self.channels)
                .map(|ch| ChannelEntry {
                    title: format!("Part {ch} with a fairly long descriptive title"),
                    channel: u16::try_from(ch).unwrap(),
                    program_title: Some(format!("Program number {ch}")),
                    bank_pc: Some([0, 0, u8::try_from(ch).unwrap()]),
                    ..ChannelEntry::default()
                })
                .collect();
            write_list(&entries, &mut w, ChannelEntry::write);
        } else if req.is_resource(names::ALL_CTRL_LIST) {
            let c = [
                Controller {
                    title: "Cutoff",
                    channel: Some(1),
                    ctrl_type: CtrlType::Cc,
                    ctrl_index: Some([74, 0]),
                    ctrl_index_len: 1,
                    param_path: Some("/Filter/Cutoff"),
                    ..Controller::default()
                },
                Controller {
                    title: "LFO Wave",
                    channel: Some(1),
                    ctrl_type: CtrlType::Nrpn,
                    ctrl_index: Some([0, 3]),
                    ctrl_index_len: 2,
                    type_hint: Some("valueSelect"),
                    ctrl_map_id: Some("lfoWave"),
                    ..Controller::default()
                },
            ];
            write_list(&c, &mut w, Controller::write);
        } else if req.is_resource(names::CTRL_MAP_LIST) {
            if !req.res_id.is_some_and(|r| r.eq_str("lfoWave")) {
                return Err(PeStatus::NOT_FOUND);
            }
            let m = [
                CtrlMapEntry {
                    value: 0,
                    title: "Sine",
                },
                CtrlMapEntry {
                    value: 2_147_483_648,
                    title: "Saw",
                },
            ];
            write_list(&m, &mut w, CtrlMapEntry::write);
        } else if req.is_resource(names::PROGRAM_LIST) {
            let p = [ProgramEntry {
                title: "Init",
                bank_pc: [0, 0, 0],
            }];
            write_list(&p, &mut w, ProgramEntry::write);
        } else if req.is_resource("X-Param") {
            w.begin_object()
                .field_uint("value", u64::from(self.param))
                .end_object();
        } else {
            return Err(PeStatus::NOT_FOUND);
        }
        w.finish().map_err(|_| PeStatus::INTERNAL_ERROR)?;
        Ok(GetMeta::default())
    }

    fn set(&mut self, req: &Header<'_>, data: &[u8]) -> Result<(), PeStatus> {
        if req.is_resource(names::STATE) {
            self.state = data.to_vec();
            Ok(())
        } else if req.is_resource("X-Param") {
            let s = core::str::from_utf8(data).map_err(|_| PeStatus::BAD_REQUEST)?;
            let v = s
                .trim_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .map_err(|_| PeStatus::BAD_REQUEST)?;
            self.param = v;
            Ok(())
        } else {
            Err(PeStatus::NOT_ALLOWED)
        }
    }

    fn subscription(&mut self, _: Muid, _: &Header<'_>, id: &str, active: bool) {
        self.subs.push((id.to_string(), active));
    }

    fn message_report(
        &mut self,
        _: Muid,
        _: DeviceId,
        q: &MessageReportInquiry,
    ) -> Option<ReportSelection> {
        let _ = q;
        Some(ReportSelection {
            channel: ChannelControllers::CONTROL_CHANGE | ChannelControllers::PROGRAM_CHANGE,
            ..ReportSelection::default()
        })
    }
}

fn rconfig() -> RConfig<'static> {
    RConfig {
        profiles: true,
        property_exchange: Some(PeCapabilities::default()),
        process: ProcessFeatures::MESSAGE_REPORT,
        product_instance_id: Some("LOOP-0001"),
        max_sysex: 512,
        max_frame: 128,
        ..RConfig::new(ID)
    }
}

type Resp = Responder<'static, 128, 1024, 2048, 4>;

struct Rig {
    ini: Initiator,
    resp: Resp,
    synth: Synth,
    now: u64,
    max_seen: usize,
    frames: usize,
}

impl Rig {
    fn new() -> Self {
        let synth = Synth::new();
        Self {
            ini: Initiator::new(IConfig::new(ID), 0xCAFE_F00D),
            resp: Responder::new(rconfig(), 0x0BAD_5EED),
            synth,
            now: 0,
            max_seen: 0,
            frames: 0,
        }
    }

    /// Moves frames both ways until quiet; returns initiator events.
    fn pump(&mut self) -> Vec<Event> {
        let mut buf = [0u8; 600];
        loop {
            let mut moved = false;
            while let Some(n) = self.resp.poll_transmit(&self.synth, &mut buf) {
                self.max_seen = self.max_seen.max(n);
                self.frames += 1;
                let h = self.ini.handle(self.now, &buf[..n]);
                assert!(matches!(h, Handled::Processed | Handled::Ignored), "{h:?}");
                moved = true;
            }
            while let Some(f) = self.ini.pop_transmit() {
                self.resp.handle(&mut self.synth, self.now, &f);
                moved = true;
            }
            if !moved {
                break;
            }
        }
        let mut evs = Vec::new();
        while let Some(e) = self.ini.pop_event() {
            evs.push(e);
        }
        evs
    }

    fn discover(&mut self) -> Muid {
        self.ini.discover(self.now).unwrap();
        let evs = self.pump();
        let Some(Event::Discovered(p)) = evs.first() else {
            panic!("{evs:?}")
        };
        assert_eq!(p.info.max_sysex, 512);
        assert!(p.info.categories.contains(Categories::PROPERTY_EXCHANGE));
        p.muid
    }

    fn response(&mut self) -> (PeStatus, Vec<u8>, Vec<u8>) {
        for e in self.pump() {
            if let Event::Response {
                status,
                header,
                data,
                ..
            } = e
            {
                return (status, header, data);
            }
        }
        panic!("no response")
    }
}

#[test]
fn full_foundational_sequence_at_128_bytes() {
    let mut rig = Rig::new();
    let peer = rig.discover();
    rig.now = 3000;
    rig.ini.poll(rig.now);
    assert_eq!(rig.ini.pop_event(), Some(Event::DiscoveryFinished));

    let mut f = Foundation::start(&mut rig.ini, rig.now, peer).unwrap();
    let mut done = false;
    for _ in 0..50 {
        for e in rig.pump() {
            done |= f.on_event(&mut rig.ini, rig.now, &e);
        }
        if done {
            break;
        }
    }
    assert!(done);
    let r = f.into_report();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(r.capabilities, Some(PeCapabilities::default()));
    assert_eq!(r.resources.len(), RESOURCES.len());
    assert_eq!(r.device_info.unwrap().model, "Loopback Synth");
    let ch = r.channels.unwrap();
    assert_eq!(ch.len(), 16);
    assert_eq!(ch[15].bank_pc, Some([0, 0, 16]));
    let c = r.controllers.unwrap();
    assert_eq!(c[0].param_path.as_deref(), Some("/Filter/Cutoff"));
    assert_eq!(r.ctrl_maps.len(), 1);
    assert_eq!(r.ctrl_maps[0].0, "lfoWave");
    assert_eq!(r.ctrl_maps[0].1[1].title, "Saw");
    assert_eq!(r.programs.unwrap()[0].title, "Init");
    // Every frame the device sent fit FMX's 128-byte limit, and the
    // ChannelList really was chunked.
    assert!(rig.max_seen <= 128, "{}", rig.max_seen);
    assert!(rig.frames > 15, "{}", rig.frames);
    assert_eq!(rig.resp.dropped(), 0);
}

#[test]
fn endpoint_profiles_and_process_inquiry() {
    let mut rig = Rig::new();
    let peer = rig.discover();
    rig.ini.endpoint_info(0, peer).unwrap();
    rig.ini
        .profile_inquiry(0, peer, DeviceId::FunctionBlock)
        .unwrap();
    rig.ini
        .profile_details(
            0,
            peer,
            DeviceId::FunctionBlock,
            PROFILE,
            TARGET_CHANNEL_COUNT,
        )
        .unwrap();
    rig.ini
        .set_profile(peer, DeviceId::FunctionBlock, PROFILE, true, 0)
        .unwrap();
    rig.ini.process_capabilities(0, peer).unwrap();
    rig.ini
        .message_report(
            0,
            peer,
            DeviceId::FunctionBlock,
            MessageReportInquiry {
                control: DataControl::FULL,
                selection: ReportSelection {
                    channel: ChannelControllers::CONTROL_CHANGE | ChannelControllers::PITCH_BEND,
                    ..ReportSelection::default()
                },
            },
        )
        .unwrap();
    let evs = rig.pump();
    assert!(evs.contains(&Event::Endpoint {
        muid: peer,
        status: 0,
        data: b"LOOP-0001".to_vec()
    }));
    assert!(evs.contains(&Event::Profiles {
        muid: peer,
        device: DeviceId::FunctionBlock,
        enabled: vec![],
        disabled: vec![PROFILE]
    }));
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::ProfileDetails { data, .. }
        if ChannelCount::from_bytes(data).unwrap().max == 16))
    );
    assert!(evs.contains(&Event::ProfileState {
        muid: peer,
        device: DeviceId::FunctionBlock,
        profile: PROFILE,
        enabled: true,
        channels: 0
    }));
    assert!(evs.contains(&Event::ProcessCapabilities {
        muid: peer,
        features: ProcessFeatures::MESSAGE_REPORT
    }));
    // The reply is the intersection of request and support (§9.6).
    assert!(evs.contains(&Event::MessageReportBegin {
        muid: peer,
        device: DeviceId::FunctionBlock,
        selection: ReportSelection {
            channel: ChannelControllers::CONTROL_CHANGE,
            ..ReportSelection::default()
        }
    }));
    rig.resp
        .end_message_report(rig.ini.muid(), DeviceId::FunctionBlock);
    assert!(rig.pump().contains(&Event::MessageReportEnd {
        muid: peer,
        device: DeviceId::FunctionBlock
    }));
    // Unsupported profile → NAK 0x04 (M2-102).
    let other = ProfileId::standard(0x22, 0, 1, 1);
    rig.ini
        .set_profile(peer, DeviceId::FunctionBlock, other, true, 0)
        .unwrap();
    assert!(rig.pump().iter().any(
        |e| matches!(e, Event::Nak { status, original: MessageKind::SetProfileOn, .. }
        if *status == NakStatus::PROFILE_NOT_SUPPORTED)
    ));
}

#[test]
fn mcoded7_state_get_and_chunked_set() {
    let mut rig = Rig::new();
    let peer = rig.discover();
    let h = Header {
        res_id: Some(JsonStr::plain("buffer")),
        mutual_encoding: Some(Encoding::Mcoded7),
        ..Header::request(names::STATE)
    };
    rig.ini.get(0, peer, &h).unwrap();
    let (status, header, data) = rig.response();
    assert_eq!(status, PeStatus::OK);
    let rh = Header::parse(&header).unwrap();
    assert_eq!(rh.encoding(), Encoding::Mcoded7);
    assert!(rh.state_rev.unwrap().eq_str("rev1"));
    assert_eq!(data, rig.synth.state);
    assert!(rig.max_seen <= 128);

    let new_state: Vec<u8> = (0..900u32).map(|i| (i * 7 % 256) as u8).collect();
    rig.ini.set(0, peer, &h, &new_state).unwrap();
    let (status, _, _) = rig.response();
    assert_eq!(status, PeStatus::OK);
    assert_eq!(rig.synth.state, new_state);

    // 8-bit data without Mcoded7 is refused locally.
    let plain = Header::request(names::STATE);
    assert!(rig.ini.set(0, peer, &plain, &[0xFF]).is_err());
    // Asking for State without resId: 400 (requireResId).
    rig.ini.get(0, peer, &plain).unwrap();
    assert_eq!(rig.response().0, PeStatus::BAD_REQUEST);
    // State in ASCII would need 8-bit bytes: 415.
    let ascii = Header {
        res_id: Some(JsonStr::plain("buffer")),
        ..Header::request(names::STATE)
    };
    rig.ini.get(0, peer, &ascii).unwrap();
    assert_eq!(rig.response().0, PeStatus::UNSUPPORTED_MEDIA_TYPE);
}

#[test]
fn subscription_lifecycle() {
    let mut rig = Rig::new();
    let peer = rig.discover();
    rig.ini.subscribe(0, peer, "X-Param", None).unwrap();
    let (status, header, _) = rig.response();
    assert_eq!(status, PeStatus::OK);
    let sub_id = Header::parse(&header)
        .unwrap()
        .subscribe_id
        .unwrap()
        .to_string();
    assert_eq!(rig.ini.subscriptions(), &[(peer, sub_id.clone())]);
    assert_eq!(rig.resp.subscriptions(), 1);

    // Partial update.
    assert_eq!(
        rig.resp
            .publish(&rig.synth, "X-Param", Update::Partial(br#"{"/value":9}"#)),
        1
    );
    let evs = rig.pump();
    assert!(evs.contains(&Event::SubscriptionUpdate {
        muid: peer,
        subscribe_id: sub_id.clone(),
        command: Command::Partial,
        data: br#"{"/value":9}"#.to_vec()
    }));
    // A Set from the initiator, then a full update reflecting it.
    let set = Header {
        set_partial: true,
        ..Header::request("X-Param")
    };
    rig.ini.set(0, peer, &set, br#"{"/value":42}"#).unwrap();
    assert_eq!(rig.response().0, PeStatus::OK);
    rig.resp.publish(&rig.synth, "X-Param", Update::Full);
    assert!(rig.pump().contains(&Event::SubscriptionUpdate {
        muid: peer,
        subscribe_id: sub_id.clone(),
        command: Command::Full,
        data: br#"{"value":42}"#.to_vec()
    }));
    rig.resp.publish(&rig.synth, "X-Param", Update::Notify);
    assert!(rig.pump().iter().any(|e| matches!(
        e,
        Event::SubscriptionUpdate {
            command: Command::Notify,
            ..
        }
    )));
    // Initiator ends it.
    rig.ini.unsubscribe(0, peer, &sub_id).unwrap();
    assert_eq!(rig.response().0, PeStatus::OK);
    assert_eq!(rig.resp.subscriptions(), 0);
    assert_eq!(
        rig.synth.subs,
        vec![(sub_id.clone(), true), (sub_id, false)]
    );

    // Responder-side end.
    rig.ini
        .subscribe(0, peer, names::CHANNEL_LIST, None)
        .unwrap();
    rig.response();
    // ChannelList does not fit one chunk: a full update becomes notify.
    rig.resp
        .publish(&rig.synth, names::CHANNEL_LIST, Update::Full);
    assert!(rig.pump().iter().any(|e| matches!(
        e,
        Event::SubscriptionUpdate {
            command: Command::Notify,
            ..
        }
    )));
    rig.resp
        .publish(&rig.synth, names::CHANNEL_LIST, Update::End);
    rig.pump();
    assert_eq!(rig.ini.subscriptions(), []);
    // Subscribing to a non-subscribable resource: 405.
    rig.ini
        .subscribe(0, peer, names::DEVICE_INFO, None)
        .unwrap();
    assert_eq!(rig.response().0, PeStatus::NOT_ALLOWED);
}

#[test]
fn errors_and_naks() {
    let mut rig = Rig::new();
    let peer = rig.discover();
    rig.ini.get(0, peer, &Header::request("X-Nope")).unwrap();
    assert_eq!(rig.response().0, PeStatus::NOT_FOUND);
    // Set on a Get-only resource: 405.
    rig.ini
        .set(0, peer, &Header::request(names::DEVICE_INFO), b"{}")
        .unwrap();
    assert_eq!(rig.response().0, PeStatus::NOT_ALLOWED);

    // Undefined Sub-ID#2 and a deprecated 1.1 message: NAK 0x01.
    let me = rig.ini.muid();
    let env = Envelope::new(me, rig.resp.muid());
    for body in [
        Body::Raw {
            kind: MessageKind::from_sub_id2(0x45),
            data: &[],
        },
        Body::ProtocolEstablished(midici::legacy::ProtocolEstablished { authority: 0x10 }),
    ] {
        let bytes = Message::new(env, body).to_vec(Framing::SysEx).unwrap();
        assert_eq!(
            rig.resp.handle(&mut rig.synth, 0, &bytes),
            Handled::Processed
        );
        let mut buf = [0u8; 128];
        let n = rig.resp.poll_transmit(&rig.synth, &mut buf).unwrap();
        let Body::Nak(nak) = Message::decode(&buf[..n]).unwrap().body else {
            panic!()
        };
        assert_eq!(nak.report.unwrap().status, NakStatus::NOT_SUPPORTED);
    }
    // Reserved version bits: NAK 0x02 (M2-101 §5.3).
    let mut bytes = Message::new(env, Body::ProfileInquiry(midici::profile::ProfileInquiry))
        .to_vec(Framing::SysEx)
        .unwrap();
    bytes[5] = 0x12;
    assert!(matches!(
        rig.resp.handle(&mut rig.synth, 0, &bytes),
        Handled::Malformed(_)
    ));
    let mut buf = [0u8; 128];
    let n = rig.resp.poll_transmit(&rig.synth, &mut buf).unwrap();
    let Body::Nak(nak) = Message::decode(&buf[..n]).unwrap().body else {
        panic!()
    };
    assert_eq!(nak.report.unwrap().status, NakStatus::VERSION_NOT_SUPPORTED);
    // PE on a channel Device ID: NAK; PE-disabled device: Failed(Nak).
    let mut plain: Responder<'static> = Responder::new(RConfig::new(ID), 5);
    let mut ini = Initiator::new(IConfig::new(ID), 9);
    ini.discover(0).unwrap();
    let mut buf = [0u8; 600];
    let d = ini.pop_transmit().unwrap();
    plain.handle(&mut rig.synth, 0, &d);
    let n = plain.poll_transmit(&rig.synth, &mut buf).unwrap();
    ini.handle(0, &buf[..n]);
    ini.get(0, plain.muid(), &Header::request(names::DEVICE_INFO))
        .unwrap();
    let g = ini.pop_transmit().unwrap();
    plain.handle(&mut rig.synth, 0, &g);
    let n = plain.poll_transmit(&rig.synth, &mut buf).unwrap();
    ini.handle(0, &buf[..n]);
    let mut failed = false;
    while let Some(e) = ini.pop_event() {
        failed |= matches!(e, Event::Failed { reason: Failure::Nak(s), .. } if s == NakStatus::NOT_SUPPORTED);
    }
    assert!(failed);
    // FMX's own SysEx coexists untouched.
    let fmx = [0xF0, 0x7D, b'F', b'X', 1, 1, 0xF7];
    assert_eq!(rig.resp.handle(&mut rig.synth, 0, &fmx), Handled::NotCi);
    assert_eq!(rig.ini.handle(0, &fmx), Handled::NotCi);
}

#[test]
fn timeouts() {
    let mut rig = Rig::new();
    let peer = rig.discover();
    // The device never answers: Failed(Timeout) and Notify 408 goes out.
    rig.ini
        .get(0, peer, &Header::request(names::DEVICE_INFO))
        .unwrap();
    rig.ini.pop_transmit();
    rig.ini.poll(3001);
    let notify = rig.ini.pop_transmit().unwrap();
    let Body::Notify(c) = Message::decode(&notify).unwrap().body else {
        panic!()
    };
    assert_eq!(
        Header::parse(c.header).unwrap().status,
        Some(PeStatus::TIMEOUT)
    );
    let evs: Vec<_> = core::iter::from_fn(|| rig.ini.pop_event()).collect();
    assert!(evs.contains(&Event::DiscoveryFinished));
    assert!(evs.iter().any(|e| matches!(
        e,
        Event::Failed {
            reason: Failure::Timeout,
            inquiry: MessageKind::GetPropertyData,
            ..
        }
    )));
    // Plain inquiries time out too.
    rig.ini.endpoint_info(5000, peer).unwrap();
    rig.ini.pop_transmit();
    rig.ini.poll(8001);
    assert!(matches!(
        rig.ini.pop_event(),
        Some(Event::Failed {
            reason: Failure::Timeout,
            inquiry: MessageKind::EndpointInquiry,
            request: None,
            ..
        })
    ));
    // A Set that stalls after its first chunk: the device sends Notify 408.
    let h = Header {
        res_id: Some(JsonStr::plain("buffer")),
        mutual_encoding: Some(Encoding::Mcoded7),
        ..Header::request(names::STATE)
    };
    rig.ini.set(10_000, peer, &h, &[0u8; 400]).unwrap();
    let first = rig.ini.pop_transmit().unwrap();
    while rig.ini.pop_transmit().is_some() {}
    rig.resp.handle(&mut rig.synth, 10_000, &first);
    rig.resp.poll(13_001);
    let mut buf = [0u8; 128];
    let n = rig.resp.poll_transmit(&rig.synth, &mut buf).unwrap();
    assert_eq!(
        Message::decode(&buf[..n]).unwrap().kind(),
        MessageKind::Notify
    );
}

#[test]
fn muid_collisions() {
    // Responder, unused MUID: option A, change and reply (§5.9.1).
    let mut synth = Synth::new();
    let mut resp: Resp = Responder::new(rconfig(), 77);
    let old = resp.muid();
    let disc = Discovery {
        identity: ID,
        categories: Categories::NONE,
        max_sysex: 512,
        output_path: 0,
    };
    let bytes = Message::new(Envelope::broadcast(old), Body::Discovery(disc))
        .to_vec(Framing::SysEx)
        .unwrap();
    resp.handle(&mut synth, 0, &bytes);
    assert_ne!(resp.muid(), old);
    let mut buf = [0u8; 128];
    let n = resp.poll_transmit(&synth, &mut buf).unwrap();
    let m = Message::decode(&buf[..n]).unwrap();
    assert_eq!(m.kind(), MessageKind::DiscoveryReply);
    assert_eq!(m.envelope.source, resp.muid());

    // Used MUID: option B, invalidate and change, no reply.
    let used = resp.muid();
    let inq = Message::new(
        Envelope::new(Muid::new(5).unwrap(), used),
        Body::EndpointInquiry(midici::mgmt::EndpointInquiry { status: 0 }),
    )
    .to_vec(Framing::SysEx)
    .unwrap();
    resp.handle(&mut synth, 0, &inq);
    while resp.poll_transmit(&synth, &mut buf).is_some() {}
    let bytes = Message::new(Envelope::broadcast(used), Body::Discovery(disc))
        .to_vec(Framing::SysEx)
        .unwrap();
    resp.handle(&mut synth, 0, &bytes);
    let n = resp.poll_transmit(&synth, &mut buf).unwrap();
    let Body::InvalidateMuid(inv) = Message::decode(&buf[..n]).unwrap().body else {
        panic!()
    };
    assert_eq!(inv.target, used);
    assert_ne!(resp.muid(), used);
    assert!(resp.poll_transmit(&synth, &mut buf).is_none());

    // Initiator: two Responders with the same MUID (§5.9.2).
    let mut ini = Initiator::new(IConfig::new(ID), 1);
    ini.discover(0).unwrap();
    let d = ini.pop_transmit().unwrap();
    let mut a: Resp = Responder::new(rconfig(), 42);
    let mut b: Resp = Responder::new(rconfig(), 42);
    for r in [&mut a, &mut b] {
        r.handle(&mut synth, 0, &d);
        let n = r.poll_transmit(&synth, &mut buf).unwrap();
        ini.handle(0, &buf[..n]);
    }
    let evs: Vec<_> = core::iter::from_fn(|| ini.pop_event()).collect();
    assert!(evs.contains(&Event::Collision(a.muid())));
    let inv = ini.pop_transmit().unwrap();
    let Body::InvalidateMuid(t) = Message::decode(&inv).unwrap().body else {
        panic!()
    };
    assert_eq!(t.target, a.muid());
    let before = a.muid();
    a.handle(&mut synth, 0, &inv);
    assert_ne!(a.muid(), before);
    assert_eq!(ini.peers(), []);
}

#[test]
fn v1_1_peers_interoperate() {
    let mut synth = Synth::new();
    let mut resp: Resp = Responder::new(rconfig(), 3);
    // A 1.1 Discovery (no Output Path Id).
    let mut env = Envelope::broadcast(Muid::new(0x100).unwrap());
    env.version = CiVersion::V1_1;
    let disc = Discovery {
        identity: ID,
        categories: Categories::NONE,
        max_sysex: 256,
        output_path: 0,
    };
    let bytes = Message::new(env, Body::Discovery(disc))
        .to_vec(Framing::SysEx)
        .unwrap();
    resp.handle(&mut synth, 0, &bytes);
    let mut buf = [0u8; 128];
    let n = resp.poll_transmit(&synth, &mut buf).unwrap();
    let m = Message::decode(&buf[..n]).unwrap();
    // We answer in 1.2; a 1.1 peer ignores the appended fields (§5.4).
    assert_eq!(m.envelope.version, CiVersion::V1_2);
    assert_eq!(m.kind(), MessageKind::DiscoveryReply);
    // 1.1 Get Property Data (same layout) works.
    let mut genv = Envelope::new(Muid::new(0x100).unwrap(), resp.muid());
    genv.version = CiVersion::V1_1;
    let hdr = Header::request(names::DEVICE_INFO).to_vec();
    let g = Message::new(
        genv,
        Body::GetPropertyData(midici::pe::PeChunk::single(1, &hdr, &[])),
    )
    .to_vec(Framing::SysEx)
    .unwrap();
    resp.handle(&mut synth, 0, &g);
    let n = resp.poll_transmit(&synth, &mut buf).unwrap();
    assert_eq!(
        Message::decode(&buf[..n]).unwrap().kind(),
        MessageKind::GetPropertyDataReply
    );

    // The Initiator accepts a 1.1 Reply to Discovery and a 1.1 short NAK.
    let mut ini = Initiator::new(IConfig::new(ID), 11);
    ini.discover(0).unwrap();
    ini.pop_transmit();
    let peer = Muid::new(0x200).unwrap();
    let mut renv = Envelope::new(peer, ini.muid());
    renv.version = CiVersion::V1_1;
    let reply = midici::mgmt::DiscoveryReply {
        identity: ID,
        categories: Categories::PROPERTY_EXCHANGE,
        max_sysex: 512,
        output_path: 0,
        function_block: None,
    };
    let b = Message::new(renv, Body::DiscoveryReply(reply))
        .to_vec(Framing::SysEx)
        .unwrap();
    assert_eq!(ini.handle(0, &b), Handled::Processed);
    assert!(matches!(ini.pop_event(), Some(Event::Discovered(p)) if p.version == CiVersion::V1_1));
    ini.endpoint_info(0, peer).unwrap();
    let nak = Message::new(renv, Body::Nak(midici::mgmt::Nak { report: None }))
        .to_vec(Framing::SysEx)
        .unwrap();
    ini.handle(0, &nak);
    assert!(matches!(
        ini.pop_event(),
        Some(Event::Failed {
            inquiry: MessageKind::EndpointInquiry,
            ..
        })
    ));
}

#[test]
fn ump_transport_round_trip() {
    let mut rig = Rig::new();
    rig.ini.discover(0).unwrap();
    let frame = rig.ini.pop_transmit().unwrap();
    let packets = midici::ump::to_ump(&frame, 3).unwrap();
    let mut asm = midici::ump::SysEx7Assembler::<512>::new();
    let mut out = None;
    for p in packets {
        if let Some(r) = asm.push(p) {
            out = Some((r.0, r.1.to_vec()));
        }
    }
    let (group, payload) = out.unwrap();
    assert_eq!(group, 3);
    assert_eq!(payload, frame[1..frame.len() - 1]);
    assert_eq!(
        rig.resp.handle(&mut rig.synth, 0, &payload),
        Handled::Processed
    );
    let _ = RequestKind::Get;
}
