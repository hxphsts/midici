//! Property tests: encode→decode round trips over arbitrary valid values,
//! and decoders that never panic on arbitrary or mutated input.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use midici::identity::{DeviceIdentity, IdentityReply, IdentityRequest, ManufacturerId};
use midici::legacy::{
    ProtocolEstablished, ProtocolNegotiation, ProtocolType, SetNewProtocol, TestNewProtocol,
};
use midici::mgmt::{
    Ack, AckStatus, Categories, Details, Discovery, DiscoveryReply, EndpointInquiry, EndpointReply,
    InvalidateMuid, Nak, NakStatus, Report, Text,
};
use midici::pe::chunk::Assembler;
use midici::pe::header::Header;
use midici::pe::json::{Event as JsonEvent, Parser};
use midici::pe::mcoded7;
use midici::pe::resource;
use midici::pe::sink::{ArrayBuf, SliceSink};
use midici::pe::{PeCapabilities, PeChunk};
use midici::process::{
    ChannelControllers, DataControl, EndOfMessageReport, MessageReportInquiry, MessageReportReply,
    NoteData, ProcessFeatures, ProcessInquiry, ProcessInquiryReply, ReportSelection,
    SystemMessages,
};
use midici::profile::{
    ProfileChannels, ProfileDetailsInquiry, ProfileDetailsReply, ProfileId, ProfileInquiry,
    ProfileInquiryReply, ProfileList, ProfileReport, ProfileSpecificData, SetProfileOff,
};
use midici::ump::{SysEx7Assembler, packets};
use midici::wire::{Channel, CiVersion, DeviceId, Envelope, Frame, Framing, MessageKind};
use midici::{Body, Message, Muid};
use proptest::prelude::*;

fn u7() -> impl Strategy<Value = u8> {
    0u8..0x80
}

fn muid() -> impl Strategy<Value = Muid> {
    (0u32..=Muid::MAX).prop_map(|v| Muid::new(v).unwrap())
}

fn device() -> impl Strategy<Value = DeviceId> {
    prop_oneof![
        (0u8..16).prop_map(|c| DeviceId::Channel(Channel::new(c).unwrap())),
        Just(DeviceId::Group),
        Just(DeviceId::FunctionBlock),
    ]
}

fn version() -> impl Strategy<Value = CiVersion> {
    prop_oneof![Just(CiVersion::V1_1), Just(CiVersion::V1_2)]
}

fn envelope() -> impl Strategy<Value = Envelope> {
    (device(), version(), muid(), muid()).prop_map(|(device, version, source, destination)| {
        Envelope {
            device,
            version,
            source,
            destination,
        }
    })
}

fn identity() -> impl Strategy<Value = DeviceIdentity> {
    (
        prop_oneof![
            (1u8..=0x7D).prop_map(|b| ManufacturerId::short(b).unwrap()),
            (u7(), u7()).prop_map(|(a, b)| ManufacturerId::extended(a, b).unwrap()),
        ],
        0u16..0x4000,
        0u16..0x4000,
        prop::array::uniform4(u7()),
    )
        .prop_map(|(manufacturer, family, model, revision)| DeviceIdentity {
            manufacturer,
            family,
            model,
            revision,
        })
}

/// Sub-ID#2 values with no defined message (M2-101 Appendix D, plus 0x45).
fn undefined_sub_ids() -> Vec<u8> {
    (0x00..=0x0F)
        .chain(0x16..=0x1F)
        .chain(0x2A..=0x2E)
        .chain(0x3A..=0x3E)
        .chain(0x45..=0x6F)
        .chain(0x74..=0x7C)
        .collect()
}

fn ascii(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(0u8..0x80, 0..max).prop_map(|v| String::from_utf8(v).unwrap())
}

fn encode(m: &Message<'_>, framing: Framing) -> Vec<u8> {
    let mut buf = vec![0u8; 1024];
    let n = m.encode(framing, &mut buf).unwrap();
    buf.truncate(n);
    buf
}

/// Round-trips `m`, both framings. For version 1, v2-only fields come back
/// at their defaults, so compare after normalising with `v1`.
fn round_trip(m: &Message<'_>, v1: impl Fn(Body<'_>) -> Body<'_>) {
    for framing in [Framing::SysEx, Framing::Bare] {
        let bytes = encode(m, framing);
        let back = Message::decode(&bytes).unwrap();
        let mut expected = *m;
        if !m.envelope.version.has_v2_fields() {
            expected.body = v1(expected.body);
        }
        assert_eq!(back, expected);
        // And re-encoding the decoded message is byte-identical.
        assert_eq!(encode(&back, framing), bytes);
    }
}

proptest! {
    #[test]
    fn discovery(env in envelope(), identity in identity(), cats in u7(), max in 0u32..=0x0FFF_FFFF, path in u7()) {
        let d = Discovery { identity, categories: Categories::from_bits(cats), max_sysex: max, output_path: path };
        round_trip(&Message { envelope: env, body: Body::Discovery(d) }, |b| match b {
            Body::Discovery(d) => Body::Discovery(Discovery { output_path: 0, ..d }),
            other => other,
        });
    }

    #[test]
    fn discovery_reply(env in envelope(), identity in identity(), cats in u7(), max in 0u32..=0x0FFF_FFFF, path in u7(), fb in prop::option::of(0u8..0x7F)) {
        let d = DiscoveryReply { identity, categories: Categories::from_bits(cats), max_sysex: max, output_path: path, function_block: fb };
        round_trip(&Message { envelope: env, body: Body::DiscoveryReply(d) }, |b| match b {
            Body::DiscoveryReply(d) => Body::DiscoveryReply(DiscoveryReply { output_path: 0, function_block: None, ..d }),
            other => other,
        });
    }

    #[test]
    fn endpoint(env in envelope(), status in u7(), data in prop::collection::vec(u7(), 0..300)) {
        round_trip(&Message { envelope: env, body: Body::EndpointInquiry(EndpointInquiry { status }) }, |b| b);
        round_trip(&Message { envelope: env, body: Body::EndpointReply(EndpointReply { status, data: &data }) }, |b| b);
    }

    #[test]
    fn invalidate(env in envelope(), target in muid()) {
        round_trip(&Message { envelope: env, body: Body::InvalidateMuid(InvalidateMuid { target }) }, |b| b);
    }

    #[test]
    fn ack_and_nak(env in envelope(), orig in u7(), status in u7(), data in u7(), details in prop::array::uniform5(u7()), text in ascii(200)) {
        let text = Text::new(&text).unwrap();
        let ack = Ack { original: MessageKind::from_sub_id2(orig), status: AckStatus(status), status_data: data, details: Details(details), text };
        round_trip(&Message { envelope: env, body: Body::Ack(ack) }, |b| b);
        let nak = Nak { report: Some(Report { original: MessageKind::from_sub_id2(orig), status: NakStatus(status), status_data: data, details: Details(details), text }) };
        round_trip(&Message { envelope: env, body: Body::Nak(nak) }, |_| Body::Nak(Nak { report: None }));
    }

    #[test]
    fn raw(env in envelope(), sub2 in prop::sample::select(undefined_sub_ids()), data in prop::collection::vec(u7(), 0..300)) {
        let kind = MessageKind::from_sub_id2(sub2);
        prop_assert!(matches!(kind, MessageKind::Undefined(_)));
        let m = Message { envelope: env, body: Body::Raw { kind, data: &data } };
        round_trip(&m, |b| b);
    }

    #[test]
    fn identity_reply(dev in u7(), identity in identity()) {
        let r = IdentityReply { device_id: dev, identity };
        let mut buf = [0u8; 32];
        for framing in [Framing::SysEx, Framing::Bare] {
            let n = r.encode(framing, &mut buf).unwrap();
            prop_assert_eq!(IdentityReply::decode(&buf[..n]), Ok(r));
        }
        let q = IdentityRequest { device_id: dev };
        let n = q.encode(Framing::SysEx, &mut buf).unwrap();
        prop_assert_eq!(IdentityRequest::decode(&buf[..n]), Ok(q));
    }

    #[test]
    fn muid_bytes(m in muid()) {
        prop_assert_eq!(Muid::from_bytes(m.to_bytes()), Ok(m));
        prop_assert!(m.to_bytes().iter().all(|b| b & 0x80 == 0));
    }

    #[test]
    fn random_muids_are_assignable(r in any::<u32>()) {
        prop_assert!(Muid::from_random(r).is_assignable());
    }

    /// Arbitrary bytes never panic any decoder.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = Message::decode(&bytes);
        let _ = Frame::parse(&bytes);
        let _ = IdentityReply::decode(&bytes);
        let _ = IdentityRequest::decode(&bytes);
    }

    /// Bytes that start like CI never panic, whatever follows.
    #[test]
    fn ci_prefixed_bytes_never_panic(framed in any::<bool>(), dev in any::<u8>(), sub2 in any::<u8>(), rest in prop::collection::vec(any::<u8>(), 0..200)) {
        let mut bytes = Vec::new();
        if framed { bytes.push(0xF0); }
        bytes.extend([0x7E, dev, 0x0D, sub2]);
        bytes.extend(rest);
        if framed { bytes.push(0xF7); }
        let _ = Message::decode(&bytes);
    }

    /// Every prefix and every single-byte corruption of a valid message
    /// decodes or errors, never panics.
    #[test]
    fn mutated_valid_frames_never_panic(env in envelope(), identity in identity(), at in any::<prop::sample::Index>(), byte in any::<u8>()) {
        let d = DiscoveryReply { identity, categories: Categories::PROPERTY_EXCHANGE, max_sysex: 512, output_path: 0, function_block: Some(1) };
        let bytes = encode(&Message { envelope: env, body: Body::DiscoveryReply(d) }, Framing::SysEx);
        for n in 0..bytes.len() {
            let _ = Message::decode(&bytes[..n]);
        }
        let mut m = bytes;
        let i = at.index(m.len());
        m[i] = byte;
        let _ = Message::decode(&m);
    }
}

fn profile_id() -> impl Strategy<Value = ProfileId> {
    prop::array::uniform5(u7()).prop_map(ProfileId)
}

fn bytes7(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(u7(), 0..max)
}

fn selection() -> impl Strategy<Value = ReportSelection> {
    (u7(), u7(), u7()).prop_map(|(s, c, n)| ReportSelection {
        system: SystemMessages::from_bits(s),
        channel: ChannelControllers::from_bits(c),
        note: NoteData::from_bits(n),
    })
}

/// v1 normalisation: v2-only fields read back as defaults.
const fn v1_norm(b: Body<'_>) -> Body<'_> {
    match b {
        Body::SetProfileOn(p) => Body::SetProfileOn(ProfileChannels { channels: 0, ..p }),
        Body::ProfileEnabled(p) => Body::ProfileEnabled(ProfileChannels { channels: 0, ..p }),
        Body::ProfileDisabled(p) => Body::ProfileDisabled(ProfileChannels { channels: 0, ..p }),
        Body::PeCapabilitiesInquiry(c) => Body::PeCapabilitiesInquiry(PeCapabilities {
            major: 0,
            minor: 0,
            ..c
        }),
        Body::PeCapabilitiesReply(c) => Body::PeCapabilitiesReply(PeCapabilities {
            major: 0,
            minor: 0,
            ..c
        }),
        other => other,
    }
}

proptest! {
    #[test]
    fn profile_bodies(
        env in envelope(),
        enabled in prop::collection::vec(profile_id(), 0..8),
        disabled in prop::collection::vec(profile_id(), 0..8),
        p in profile_id(),
        channels in 0u16..0x4000,
        target in u7(),
        data in bytes7(300),
    ) {
        let pc = ProfileChannels { profile: p, channels };
        let bodies = [
            Body::ProfileInquiry(ProfileInquiry),
            Body::ProfileInquiryReply(ProfileInquiryReply {
                enabled: ProfileList::Ids(&enabled),
                disabled: ProfileList::Ids(&disabled),
            }),
            Body::SetProfileOn(pc),
            Body::SetProfileOff(SetProfileOff { profile: p }),
            Body::ProfileEnabled(pc),
            Body::ProfileDisabled(pc),
            Body::ProfileAdded(ProfileReport { profile: p }),
            Body::ProfileRemoved(ProfileReport { profile: p }),
            Body::ProfileDetailsInquiry(ProfileDetailsInquiry { profile: p, target }),
            Body::ProfileDetailsReply(ProfileDetailsReply { profile: p, target, data: &data }),
            Body::ProfileSpecificData(ProfileSpecificData { profile: p, data: &data }),
        ];
        for body in bodies {
            round_trip(&Message::new(env, body), v1_norm);
        }
    }

    #[test]
    fn pe_bodies(
        env in envelope(),
        sim in u7(), major in u7(), minor in u7(),
        rid in u7(),
        header in bytes7(200),
        data in bytes7(300),
        num in 0u16..0x4000,
        this in 0u16..0x4000,
    ) {
        let caps = PeCapabilities { simultaneous: sim, major, minor };
        round_trip(&Message::new(env, Body::PeCapabilitiesInquiry(caps)), v1_norm);
        round_trip(&Message::new(env, Body::PeCapabilitiesReply(caps)), v1_norm);
        let c = PeChunk { request_id: rid, header: &header, num_chunks: num, this_chunk: this, data: &data };
        let bodies = [
            Body::HasPropertyData(c),
            Body::HasPropertyDataReply(c),
            Body::GetPropertyData(c),
            Body::GetPropertyDataReply(c),
            Body::SetPropertyData(c),
            Body::SetPropertyDataReply(c),
            Body::Subscription(c),
            Body::SubscriptionReply(c),
            Body::Notify(c),
        ];
        for body in bodies {
            let m = Message::new(env, body);
            round_trip(&m, |b| b);
            prop_assert_eq!(m.body.pe_chunk(), Some(&c));
        }
    }

    #[test]
    fn process_bodies(env in envelope(), f in u7(), control in u7(), sel in selection()) {
        let bodies = [
            Body::ProcessInquiry(ProcessInquiry),
            Body::ProcessInquiryReply(ProcessInquiryReply { features: ProcessFeatures::from_bits(f) }),
            Body::MessageReportInquiry(MessageReportInquiry { control: DataControl(control), selection: sel }),
            Body::MessageReportReply(MessageReportReply { selection: sel }),
            Body::EndOfMessageReport(EndOfMessageReport),
        ];
        for body in bodies {
            round_trip(&Message::new(env, body), |b| b);
        }
    }

    #[test]
    fn legacy_bodies(
        env in envelope(),
        auth in u7(),
        protos in prop::collection::vec(prop::array::uniform5(u7()), 0..10),
        test in prop::collection::vec(u7(), 48..=48),
    ) {
        let flat: Vec<u8> = protos.concat();
        let n = ProtocolNegotiation { authority: auth, protocols: &flat };
        let first = protos.first().copied().unwrap_or_default();
        let t = TestNewProtocol { authority: auth, data: &test };
        let bodies = [
            Body::InitiateProtocolNegotiation(n),
            Body::InitiateProtocolNegotiationReply(n),
            Body::SetNewProtocol(SetNewProtocol { authority: auth, protocol: ProtocolType(first) }),
            Body::TestNewProtocolInitiator(t),
            Body::TestNewProtocolResponder(t),
            Body::ProtocolEstablished(ProtocolEstablished { authority: auth }),
        ];
        for body in bodies {
            round_trip(&Message::new(env, body), |b| b);
        }
        prop_assert_eq!(n.iter().count(), protos.len());
    }

    #[test]
    fn mcoded7_round_trip(data in prop::collection::vec(any::<u8>(), 0..500)) {
        let mut enc = vec![0u8; mcoded7::encoded_len(data.len())];
        let n = mcoded7::encode(&data, &mut enc).unwrap();
        prop_assert_eq!(n, enc.len());
        prop_assert!(enc.iter().all(|b| b & 0x80 == 0));
        prop_assert_eq!(mcoded7::decoded_len(n), Some(data.len()));
        let mut dec = vec![0u8; data.len()];
        prop_assert_eq!(mcoded7::decode(&enc, &mut dec), Ok(data.len()));
        prop_assert_eq!(&dec, &data);
        let m = mcoded7::decode_in_place(&mut enc).unwrap();
        prop_assert_eq!(&enc[..m], &data[..]);
    }

    #[test]
    fn mcoded7_decoders_never_panic(data in prop::collection::vec(any::<u8>(), 0..300), split in any::<prop::sample::Index>()) {
        let mut out = [0u8; 400];
        let _ = mcoded7::decode(&data, &mut out);
        let mut copy = data.clone();
        let _ = mcoded7::decode_in_place(&mut copy);
        let mut d = mcoded7::Decoder::new();
        let mut sink = SliceSink::new(&mut out);
        let at = if data.is_empty() { 0 } else { split.index(data.len()) };
        let _ = d.feed(&data[..at], &mut sink);
        let _ = d.feed(&data[at..], &mut sink);
        let _ = d.finish();
    }

    #[test]
    fn header_parse_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = Header::parse(&bytes);
    }

    #[test]
    fn header_parse_json_like_never_panics(s in r#"[{}\[\]":,a-z0-9\\ .eE+-]{0,120}"#) {
        let _ = Header::parse(s.as_bytes());
    }

    #[test]
    fn json_parser_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let mut p = Parser::new(&bytes);
        for _ in 0..10_000 {
            match p.next() {
                Ok(Some(JsonEvent::Str(s) | JsonEvent::Key(s))) => {
                    let _ = s.chars().count();
                }
                Ok(Some(JsonEvent::Number(n))) => {
                    let _ = (n.as_u64(), n.as_i64(), n.as_f64());
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            }
        }
    }

    #[test]
    fn json_like_parser_never_panics(s in r#"[{}\[\]":,a-zA-Z0-9\\ .eE+-]{0,200}"#) {
        let mut p = Parser::new(s.as_bytes());
        for _ in 0..10_000 {
            match p.next() {
                Ok(Some(JsonEvent::Str(s) | JsonEvent::Key(s))) => {
                    let mut buf = [0u8; 256];
                    let _ = s.unescape_into(&mut buf);
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            }
        }
    }

    #[test]
    fn resource_parsers_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = resource::parse_resource_list(&bytes);
        let _ = resource::parse_device_info(&bytes);
        let _ = resource::parse_channel_list(&bytes);
        let _ = resource::parse_controllers(&bytes);
        let _ = resource::parse_ctrl_map_list(&bytes);
        let _ = resource::parse_program_list(&bytes);
        let _ = resource::parse_state_list(&bytes);
    }

    #[test]
    fn resource_parsers_never_panic_json_like(s in r#"[{}\[\]":,a-zA-Z0-9 ]{0,200}"#) {
        let b = s.as_bytes();
        let _ = resource::parse_resource_list(b);
        let _ = resource::parse_device_info(b);
        let _ = resource::parse_channel_list(b);
        let _ = resource::parse_controllers(b);
        let _ = resource::parse_ctrl_map_list(b);
        let _ = resource::parse_program_list(b);
        let _ = resource::parse_state_list(b);
    }

    #[test]
    fn assembler_never_panics(
        chunks in prop::collection::vec(
            (0u8..4, 0u16..6, 0u16..6, bytes7(40), bytes7(80), any::<u16>()),
            0..20,
        ),
    ) {
        let mut a = Assembler::new(ArrayBuf::<256>::new(), 200);
        let mut now = 0u64;
        for (rid, num, this, header, data, dt) in &chunks {
            now += u64::from(*dt);
            let c = PeChunk { request_id: *rid, header, num_chunks: *num, this_chunk: *this, data };
            let _ = a.push(&c, now);
            let _ = (a.header(), a.data(), a.expired(now, 3000));
            prop_assert!(a.header().len() + a.data().len() <= 200);
        }
    }

    #[test]
    fn ump_round_trip(group in 0u8..16, payload in bytes7(400)) {
        let mut asm = SysEx7Assembler::<512>::new();
        let mut got = None;
        for p in packets(&payload, group) {
            if let Some((g, d)) = asm.push(p) {
                got = Some((g, d.to_vec()));
            }
        }
        prop_assert_eq!(got, Some((group, payload)));
    }

    #[test]
    fn ump_assembler_never_panics(ps in prop::collection::vec(any::<[u32; 2]>(), 0..60)) {
        let mut asm = SysEx7Assembler::<64>::new();
        for p in ps {
            if let Some((g, d)) = asm.push(p) {
                prop_assert!(g < 16);
                prop_assert!(d.len() <= 64);
            }
        }
    }
}
