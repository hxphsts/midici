//! Golden frames: expected bytes derived by hand from the message tables in
//! M2-101-UM v1.2 (cited per test), not from this crate's encoder.
//!
//! MUIDs used throughout (7-bit, LSB first):
//! * `A = 0x0123_4567` → `67 0A 0D 09`
//! * `B = 0x0765_4321` → `21 06 15 3B`
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use midici::identity::{DeviceIdentity, IdentityReply, IdentityRequest, ManufacturerId};
use midici::legacy::{
    ProtocolEstablished, ProtocolNegotiation, ProtocolType, SetNewProtocol, TestNewProtocol,
};
use midici::mgmt::{
    Ack, AckStatus, Categories, Details, Discovery, DiscoveryReply, EndpointInquiry, EndpointReply,
    InvalidateMuid, Nak, NakStatus, Report, Text,
};
use midici::pe::{PeCapabilities, PeChunk};
use midici::process::{
    ChannelControllers, DataControl, EndOfMessageReport, MessageReportInquiry, MessageReportReply,
    NoteData, ProcessFeatures, ProcessInquiry, ProcessInquiryReply, ReportSelection,
    SystemMessages,
};
use midici::profile::{
    ChannelCount, ProfileChannels, ProfileDetailsInquiry, ProfileDetailsReply, ProfileId,
    ProfileInquiry, ProfileInquiryReply, ProfileList, ProfileReport, ProfileSpecificData,
    SetProfileOff, TARGET_CHANNEL_COUNT,
};
use midici::wire::{Channel, CiVersion, DeviceId, Envelope, Frame, Framing, MessageKind};
use midici::{Body, Error, Message, Muid};

const A: [u8; 4] = [0x67, 0x0A, 0x0D, 0x09];
const B: [u8; 4] = [0x21, 0x06, 0x15, 0x3B];
const BCAST: [u8; 4] = [0x7F; 4];

fn a() -> Muid {
    Muid::new(0x0123_4567).unwrap()
}
fn b() -> Muid {
    Muid::new(0x0765_4321).unwrap()
}

/// Concatenates byte groups.
fn frame(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// Encodes `m` and checks it equals `expected`, then decodes `expected` and
/// checks it equals `m`; repeats unframed.
fn check(m: &Message<'_>, expected: &[u8]) {
    let mut buf = [0u8; 512];
    let n = m.encode(Framing::SysEx, &mut buf).unwrap();
    assert_eq!(&buf[..n], expected, "encode {:?}", m.kind());
    assert_eq!(Message::decode(expected).unwrap(), *m, "decode");

    let bare = &expected[1..expected.len() - 1];
    let n = m.encode(Framing::Bare, &mut buf).unwrap();
    assert_eq!(&buf[..n], bare, "encode bare");
    assert_eq!(Message::decode(bare).unwrap(), *m, "decode bare");
}

const fn nc_identity() -> DeviceIdentity {
    DeviceIdentity {
        manufacturer: ManufacturerId::NON_COMMERCIAL,
        family: 0x0102,
        model: 0x0003,
        revision: [0, 1, 2, 3],
    }
}

fn discovery() -> Discovery {
    Discovery {
        identity: nc_identity(),
        categories: Categories::PROFILE_CONFIGURATION | Categories::PROPERTY_EXCHANGE,
        max_sysex: 512,
        output_path: 0,
    }
}

/// M2-101 §5.5, Table 6 (version 2: Output Path Id appended).
#[test]
fn discovery_v2() {
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x70, 0x02],
        &A,
        &BCAST,
        &[0x7D, 0x00, 0x00], // manufacturer, 1-byte ID padded with zeros (§5.5.1)
        &[0x02, 0x02],       // family 0x0102, LSB first
        &[0x03, 0x00],       // model 3
        &[0x00, 0x01, 0x02, 0x03],
        &[0x0C],                   // categories: D2 profiles | D3 PE (§5.5.2)
        &[0x00, 0x04, 0x00, 0x00], // max SysEx 512
        &[0x00],                   // output path id
        &[0xF7],
    ]);
    assert_eq!(expected.len(), 32);
    check(
        &Message {
            envelope: Envelope::broadcast(a()),
            body: Body::Discovery(discovery()),
        },
        &expected,
    );
}

/// M2-101 v1.1 Discovery: no Output Path Id. Decodes with `output_path = 0`.
#[test]
fn discovery_v1() {
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x70, 0x01],
        &A,
        &BCAST,
        &[
            0x7D, 0x00, 0x00, 0x02, 0x02, 0x03, 0x00, 0x00, 0x01, 0x02, 0x03,
        ],
        &[0x0C, 0x00, 0x04, 0x00, 0x00],
        &[0xF7],
    ]);
    let mut env = Envelope::broadcast(a());
    env.version = CiVersion::V1_1;
    check(
        &Message {
            envelope: env,
            body: Body::Discovery(discovery()),
        },
        &expected,
    );
}

/// M2-101 §5.6, Table 8: 3-byte manufacturer, no Function Block (`7F`).
#[test]
fn discovery_reply_v2() {
    let reply = DiscoveryReply {
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::extended(0x20, 0x6B).unwrap(),
            family: 1,
            model: 2,
            revision: [1, 0, 0, 0],
        },
        categories: Categories::PROPERTY_EXCHANGE,
        max_sysex: 128,
        output_path: 0,
        function_block: None,
    };
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x71, 0x02],
        &B,
        &A,
        &[0x00, 0x20, 0x6B],
        &[0x01, 0x00, 0x02, 0x00],
        &[0x01, 0x00, 0x00, 0x00],
        &[0x08],
        &[0x00, 0x01, 0x00, 0x00], // 128
        &[0x00],                   // output path echoed (§5.6.1)
        &[0x7F],                   // no Function Block (§5.6.2)
        &[0xF7],
    ]);
    assert_eq!(expected.len(), 33);
    let msg = Message {
        envelope: Envelope::new(b(), a()),
        body: Body::DiscoveryReply(reply),
    };
    check(&msg, &expected);

    // With Function Block 3.
    let mut with_fb = expected;
    with_fb[31] = 0x03;
    let mut reply3 = reply;
    reply3.function_block = Some(3);
    check(
        &Message {
            envelope: Envelope::new(b(), a()),
            body: Body::DiscoveryReply(reply3),
        },
        &with_fb,
    );
}

/// A version 1 Reply to Discovery ends after the max SysEx size.
#[test]
fn discovery_reply_v1() {
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x71, 0x01],
        &B,
        &A,
        &[
            0x7D, 0x00, 0x00, 0x02, 0x02, 0x03, 0x00, 0x00, 0x01, 0x02, 0x03,
        ],
        &[0x00, 0x00, 0x01, 0x00, 0x00],
        &[0xF7],
    ]);
    let (env, reply) = DiscoveryReply::decode(&expected).unwrap();
    assert_eq!(env.version, CiVersion::V1_1);
    assert_eq!(reply.function_block, None);
    assert_eq!(reply.output_path, 0);
    assert_eq!(reply.max_sysex, 128);
    assert_eq!(reply.categories, Categories::NONE);
}

/// M2-101 §5.7, Table 9.
#[test]
fn endpoint_inquiry() {
    let expected = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, 0x72, 0x02], &A, &B, &[0x00, 0xF7]]);
    check(
        &Message {
            envelope: Envelope::new(a(), b()),
            body: Body::EndpointInquiry(EndpointInquiry {
                status: EndpointInquiry::PRODUCT_INSTANCE_ID,
            }),
        },
        &expected,
    );
}

/// M2-101 §5.8, Table 11, with a Product Instance Id (§5.8.3.1).
#[test]
fn endpoint_reply() {
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x73, 0x02],
        &B,
        &A,
        &[0x00],       // status
        &[0x08, 0x00], // length 8, LSB first
        b"FM1-0042",
        &[0xF7],
    ]);
    let reply = EndpointReply::product_instance_id("FM1-0042").unwrap();
    check(
        &Message {
            envelope: Envelope::new(b(), a()),
            body: Body::EndpointReply(reply),
        },
        &expected,
    );
    let (_, decoded) = EndpointReply::decode(&expected).unwrap();
    assert_eq!(decoded.as_product_instance_id(), Ok("FM1-0042"));
}

/// M2-101 §5.9, Table 12.
#[test]
fn invalidate_muid() {
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x7E, 0x02],
        &A,
        &BCAST,
        &B,
        &[0xF7],
    ]);
    check(
        &Message {
            envelope: Envelope::broadcast(a()),
            body: Body::InvalidateMuid(InvalidateMuid { target: b() }),
        },
        &expected,
    );
}

/// M2-101 §5.10, Tables 13–14: Timeout Wait of 1 s on a PE Get, chunk 2.
#[test]
fn ack_timeout_wait() {
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x7D, 0x02],
        &B,
        &A,
        &[0x34],                         // original: Inquiry: Get Property Data
        &[0x10],                         // Timeout Wait
        &[0x0A],                         // 10 × 100 ms
        &[0x01, 0x02, 0x00, 0x00, 0x00], // PE: stream 1, chunk 2 (§5.10.3)
        &[0x04, 0x00],
        b"wait",
        &[0xF7],
    ]);
    let ack = Ack {
        original: MessageKind::GetPropertyData,
        status: AckStatus::TIMEOUT_WAIT,
        status_data: 10,
        details: Details::property_exchange(1, 2).unwrap(),
        text: Text::new("wait").unwrap(),
    };
    check(
        &Message {
            envelope: Envelope::new(b(), a()),
            body: Body::Ack(ack),
        },
        &expected,
    );
}

/// M2-101 §5.11, Tables 15–16: version 2 NAK, profile not supported on
/// channel 6, details carrying the Profile ID.
#[test]
fn nak_v2_profile_on_channel() {
    let profile = [0x7E, 0x31, 0x00, 0x00, 0x01];
    let expected = frame(&[
        &[0xF0, 0x7E, 0x05, 0x0D, 0x7F, 0x02],
        &B,
        &A,
        &[0x22], // original: Set Profile On
        &[0x04], // Profile not supported on the requested channel
        &[0x00],
        &profile,
        &[0x00, 0x00], // no text
        &[0xF7],
    ]);
    let nak = Nak {
        report: Some(Report {
            original: MessageKind::SetProfileOn,
            status: NakStatus::PROFILE_NOT_SUPPORTED,
            status_data: 0,
            details: Details(profile),
            text: Text::EMPTY,
        }),
    };
    let mut env = Envelope::new(b(), a());
    env.device = DeviceId::Channel(Channel::new(5).unwrap());
    check(
        &Message {
            envelope: env,
            body: Body::Nak(nak),
        },
        &expected,
    );
}

/// MIDI-CI 1.1 NAK: header only (M2-101 v1.1 §5.7; v1.2 Table 15 marks the
/// data as added in version 2).
#[test]
fn nak_v1_short_form() {
    let expected = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, 0x7F, 0x01], &B, &A, &[0xF7]]);
    let mut env = Envelope::new(b(), a());
    env.version = CiVersion::V1_1;
    check(
        &Message {
            envelope: env,
            body: Body::Nak(Nak { report: None }),
        },
        &expected,
    );
}

/// At the 128-byte minimum Receivable Maximum SysEx Message Size, a NAK has
/// room for exactly 103 bytes of text (M2-101 §5.11.4).
#[test]
fn nak_with_103_bytes_of_text_is_128_bytes() {
    let text = "x".repeat(103);
    let nak = Nak::new(MessageKind::GetPropertyData, NakStatus::MALFORMED)
        .with_text(Text::new(&text).unwrap());
    let mut buf = [0u8; 128];
    let n = nak
        .encode(Envelope::new(b(), a()), Framing::SysEx, &mut buf)
        .unwrap();
    assert_eq!(n, 128);
    let mut small = [0u8; 127];
    assert_eq!(
        nak.encode(Envelope::new(b(), a()), Framing::SysEx, &mut small),
        Err(Error::BufferTooSmall)
    );
}

/// M2-101 §8.7, Table 33: Inquiry: Get Property Data. In v1.2 the chunk
/// fields are fixed at `01 00 01 00 00 00`.
#[test]
fn pe_get_inquiry() {
    let header = br#"{"resource":"DeviceInfo"}"#;
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x34, 0x02],
        &A,
        &B,
        &[0x01],       // request ID
        &[0x19, 0x00], // header length 25
        header,
        &[0x01, 0x00, 0x01, 0x00, 0x00, 0x00], // 1 chunk, chunk 1, no property data
        &[0xF7],
    ]);
    let m = Message::decode(&expected).unwrap();
    assert_eq!(m.kind(), MessageKind::GetPropertyData);
    let Body::GetPropertyData(c) = m.body else {
        panic!("expected a typed Get body");
    };
    assert_eq!(
        c,
        PeChunk {
            request_id: 1,
            header,
            num_chunks: 1,
            this_chunk: 1,
            data: &[],
        }
    );
    check(&m, &expected);
}

/// MIDI 1.0 Identity Request, all-call.
#[test]
fn identity_request() {
    let expected = [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7];
    let mut buf = [0u8; 8];
    let n = IdentityRequest::ALL
        .encode(Framing::SysEx, &mut buf)
        .unwrap();
    assert_eq!(&buf[..n], &expected);
    assert_eq!(IdentityRequest::decode(&expected), Ok(IdentityRequest::ALL));
    assert_eq!(
        IdentityRequest::decode(&expected[1..5]),
        Ok(IdentityRequest::ALL)
    );
}

/// MIDI 1.0 Identity Reply with a 1-byte manufacturer ID.
#[test]
fn identity_reply_short_manufacturer() {
    let expected = [
        0xF0, 0x7E, 0x10, 0x06, 0x02, 0x7D, 0x02, 0x02, 0x03, 0x00, 0x00, 0x01, 0x02, 0x03, 0xF7,
    ];
    let reply = IdentityReply {
        device_id: 0x10,
        identity: nc_identity(),
    };
    let mut buf = [0u8; 32];
    let n = reply.encode(Framing::SysEx, &mut buf).unwrap();
    assert_eq!(&buf[..n], &expected);
    assert_eq!(IdentityReply::decode(&expected), Ok(reply));
}

/// MIDI 1.0 Identity Reply with a 3-byte manufacturer ID; trailing extra
/// bytes some devices append are ignored.
#[test]
fn identity_reply_extended_manufacturer() {
    let expected = [
        0xF0, 0x7E, 0x7F, 0x06, 0x02, 0x00, 0x20, 0x6B, 0x04, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00,
        0x00, 0xF7,
    ];
    let reply = IdentityReply {
        device_id: 0x7F,
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::extended(0x20, 0x6B).unwrap(),
            family: 4,
            model: 2,
            revision: [1, 0, 0, 0],
        },
    };
    let mut buf = [0u8; 32];
    let n = reply.encode(Framing::SysEx, &mut buf).unwrap();
    assert_eq!(&buf[..n], &expected);
    assert_eq!(IdentityReply::decode(&expected), Ok(reply));

    let mut extra = expected.to_vec();
    extra.insert(extra.len() - 1, 0x55);
    assert_eq!(IdentityReply::decode(&extra), Ok(reply));
}

/// M2-101 §5.3: reserved version bits → the envelope still parses (so the
/// Responder can NAK with status 0x02), but the message does not decode.
#[test]
fn reserved_version_bits() {
    let bytes = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, 0x70, 0x12], &A, &BCAST, &[0xF7]]);
    assert_eq!(
        Message::decode(&bytes),
        Err(Error::UnsupportedVersion(0x12))
    );
    let f = Frame::parse(&bytes).unwrap();
    assert!(f.header.envelope.version.has_reserved_bits());
    assert_eq!(f.header.envelope.source, a());
}

/// M2-101 §5.3/§5.4: a newer minor version may append fields; they are
/// ignored.
#[test]
fn newer_minor_version_appended_fields_are_ignored() {
    let mut bytes = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x70, 0x03],
        &A,
        &BCAST,
        &[
            0x7D, 0x00, 0x00, 0x02, 0x02, 0x03, 0x00, 0x00, 0x01, 0x02, 0x03,
        ],
        &[0x0C, 0x00, 0x04, 0x00, 0x00, 0x05],
        &[0x11, 0x22], // unknown v3 fields
        &[0xF7],
    ]);
    let (env, d) = Discovery::decode(&bytes).unwrap();
    assert_eq!(env.version.minor(), 3);
    assert_eq!(d.output_path, 5);
    // A v2 message missing its v2 field is truncated.
    bytes.truncate(bytes.len() - 4);
    bytes.push(0xF7);
    bytes[5] = 0x02;
    assert_eq!(Discovery::decode(&bytes), Err(Error::Truncated));
}

/// The FMX control protocol (`F0 7D 'F' 'X' …`) shares the port; it is
/// `Foreign`, never an error to NAK.
#[test]
fn coexists_with_manufacturer_sysex() {
    let fmx = [0xF0, 0x7D, b'F', b'X', 0x01, 0x01, 0xF7];
    assert_eq!(Message::decode(&fmx), Err(Error::Foreign));
    assert!(!midici::wire::is_midi_ci(&fmx));
    assert_eq!(IdentityReply::decode(&fmx), Err(Error::Foreign));
}

#[test]
fn structural_errors() {
    let good = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, 0x72, 0x02], &A, &B, &[0x00, 0xF7]]);
    // Missing F7.
    assert_eq!(
        Message::decode(&good[..good.len() - 1]),
        Err(Error::Unterminated)
    );
    // A status byte inside.
    let mut bad = good.clone();
    bad[7] = 0x90;
    assert_eq!(Message::decode(&bad), Err(Error::NonDataByte { offset: 7 }));
    // Reserved Device ID.
    let mut bad = good.clone();
    bad[2] = 0x20;
    assert_eq!(Message::decode(&bad), Err(Error::ReservedDeviceId(0x20)));
    // Wrong typed decode.
    assert_eq!(
        Discovery::decode(&good),
        Err(Error::UnexpectedKind(MessageKind::EndpointInquiry))
    );
    // Undefined Sub-ID#2 decodes as Raw so it can be NAKed "not supported".
    let mut undefined = good;
    undefined[4] = 0x45;
    let m = Message::decode(&undefined).unwrap();
    assert!(matches!(m.kind(), MessageKind::Undefined(u) if u.sub_id2() == 0x45));
}

#[test]
fn encoder_rejects_out_of_range_fields() {
    let mut d = discovery();
    d.identity.family = 0x4000;
    let mut buf = [0u8; 64];
    assert_eq!(
        d.encode(Envelope::broadcast(a()), Framing::SysEx, &mut buf),
        Err(Error::OutOfRange("device family"))
    );
    let mut d = discovery();
    d.max_sysex = 0x1000_0000;
    assert!(matches!(
        d.encode(Envelope::broadcast(a()), Framing::SysEx, &mut buf),
        Err(Error::OutOfRange(_))
    ));
    let mut r = DiscoveryReply {
        identity: nc_identity(),
        categories: Categories::NONE,
        max_sysex: 128,
        output_path: 0,
        function_block: Some(0x7F),
    };
    assert!(
        r.encode(Envelope::new(b(), a()), Framing::SysEx, &mut buf)
            .is_err()
    );
    r.function_block = None;
    assert!(
        r.encode(Envelope::new(b(), a()), Framing::SysEx, &mut buf)
            .is_ok()
    );
    // A version 2 NAK must carry a report.
    assert!(
        Nak { report: None }
            .encode(Envelope::new(b(), a()), Framing::SysEx, &mut buf)
            .is_err()
    );
    assert!(Text::new("café").is_err());
}

#[cfg(feature = "alloc")]
#[test]
fn to_vec_matches_encode() {
    let m = Message {
        envelope: Envelope::broadcast(a()),
        body: Body::Discovery(discovery()),
    };
    let v = m.to_vec(Framing::SysEx).unwrap();
    let mut buf = [0u8; 64];
    let n = m.encode(Framing::SysEx, &mut buf).unwrap();
    assert_eq!(v, &buf[..n]);
}

// ---------------------------------------------------------------------------
// Profile Configuration (M2-101 §7)
// ---------------------------------------------------------------------------

/// Standard Profile `7E 21 00 01 01` (M2-101 Table 19 / M2-102 Table 5).
const P: [u8; 5] = [0x7E, 0x21, 0x00, 0x01, 0x01];
/// Manufacturer Profile: `7D 00 00` + specific info `12 34`.
const Q: [u8; 5] = [0x7D, 0x00, 0x00, 0x12, 0x34];
/// Another standard Profile.
const R: [u8; 5] = [0x7E, 0x31, 0x00, 0x01, 0x01];

const fn env_v(source: Muid, destination: Muid, device: DeviceId, version: CiVersion) -> Envelope {
    Envelope {
        device,
        version,
        source,
        destination,
    }
}

const fn ch(n: u8) -> DeviceId {
    DeviceId::Channel(Channel::new(n).unwrap())
}

#[test]
fn profile_id_constructors() {
    assert_eq!(ProfileId::standard(0x21, 0x00, 0x01, 0x01).bytes(), P);
    let m = ProfileId::manufacturer(ManufacturerId::NON_COMMERCIAL, [0x12, 0x34]);
    assert_eq!(m.bytes(), Q);
    assert!(ProfileId(P).is_standard());
    assert!(!m.is_standard());
}

/// M2-101 §7.2, Table 17: Profile Inquiry to channel 6.
#[test]
fn profile_inquiry() {
    let expected = frame(&[&[0xF0, 0x7E, 0x05, 0x0D, 0x20, 0x02], &A, &B, &[0xF7]]);
    check(
        &Message::new(
            env_v(a(), b(), ch(5), CiVersion::V1_2),
            Body::ProfileInquiry(ProfileInquiry),
        ),
        &expected,
    );
}

/// M2-101 §7.3, Table 18: 1 enabled, 2 disabled.
#[test]
fn profile_inquiry_reply() {
    let expected = frame(&[
        &[0xF0, 0x7E, 0x05, 0x0D, 0x21, 0x02],
        &B,
        &A,
        &[0x01, 0x00], // cep = 1
        &P,
        &[0x02, 0x00], // cdp = 2
        &Q,
        &R,
        &[0xF7],
    ]);
    let enabled = [ProfileId(P)];
    let disabled = [ProfileId(Q), ProfileId(R)];
    let m = Message::new(
        env_v(b(), a(), ch(5), CiVersion::V1_2),
        Body::ProfileInquiryReply(ProfileInquiryReply {
            enabled: ProfileList::Ids(&enabled),
            disabled: ProfileList::Ids(&disabled),
        }),
    );
    check(&m, &expected);
    let (_, r) = ProfileInquiryReply::decode(&expected).unwrap();
    assert!(matches!(r.enabled, ProfileList::Wire(_)));
    assert_eq!(r.disabled.iter().collect::<Vec<_>>(), disabled);
    // §7.13: no profiles at all.
    let empty = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x21, 0x02],
        &B,
        &A,
        &[0x00, 0x00, 0x00, 0x00, 0xF7],
    ]);
    check(
        &Message::new(
            Envelope::new(b(), a()),
            Body::ProfileInquiryReply(ProfileInquiryReply::default()),
        ),
        &empty,
    );
}

/// M2-101 §7.8, Table 24: Set Profile On, v2 (channels requested) and v1.
#[test]
fn set_profile_on_v1_v2() {
    let v2 = frame(&[
        &[0xF0, 0x7E, 0x00, 0x0D, 0x22, 0x02],
        &A,
        &B,
        &P,
        &[0x03, 0x00], // 3 channels requested
        &[0xF7],
    ]);
    let body = Body::SetProfileOn(ProfileChannels {
        profile: ProfileId(P),
        channels: 3,
    });
    check(
        &Message::new(env_v(a(), b(), ch(0), CiVersion::V1_2), body),
        &v2,
    );
    let v1 = frame(&[&[0xF0, 0x7E, 0x00, 0x0D, 0x22, 0x01], &A, &B, &P, &[0xF7]]);
    let body1 = Body::SetProfileOn(ProfileChannels {
        profile: ProfileId(P),
        channels: 0,
    });
    check(
        &Message::new(env_v(a(), b(), ch(0), CiVersion::V1_1), body1),
        &v1,
    );
}

/// M2-101 §7.9, Table 25: Set Profile Off; v2 adds 2 reserved bytes.
#[test]
fn set_profile_off_v1_v2() {
    let v2 = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x23, 0x02],
        &A,
        &B,
        &P,
        &[0x00, 0x00],
        &[0xF7],
    ]);
    let body = Body::SetProfileOff(SetProfileOff {
        profile: ProfileId(P),
    });
    check(&Message::new(Envelope::new(a(), b()), body), &v2);
    let v1 = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, 0x23, 0x01], &A, &B, &P, &[0xF7]]);
    check(
        &Message::new(
            env_v(a(), b(), DeviceId::FunctionBlock, CiVersion::V1_1),
            body,
        ),
        &v1,
    );
    // Non-zero reserved bytes are ignored on receipt.
    let mut odd = v2;
    odd[19] = 0x55;
    let (_, off) = SetProfileOff::decode(&odd).unwrap();
    assert_eq!(off.profile, ProfileId(P));
}

/// M2-101 §7.10/§7.11, Tables 26/27: Enabled/Disabled Reports go to the
/// Broadcast MUID; v2 appends a channel count.
#[test]
fn profile_enabled_disabled_reports() {
    for (sub2, enabled) in [(0x24u8, true), (0x25u8, false)] {
        let v2 = frame(&[
            &[0xF0, 0x7E, 0x02, 0x0D, sub2, 0x02],
            &B,
            &BCAST,
            &P,
            &[0x01, 0x00], // 1 channel
            &[0xF7],
        ]);
        let pc = ProfileChannels {
            profile: ProfileId(P),
            channels: 1,
        };
        let body = if enabled {
            Body::ProfileEnabled(pc)
        } else {
            Body::ProfileDisabled(pc)
        };
        check(
            &Message::new(env_v(b(), Muid::BROADCAST, ch(2), CiVersion::V1_2), body),
            &v2,
        );
        let v1 = frame(&[
            &[0xF0, 0x7E, 0x02, 0x0D, sub2, 0x01],
            &B,
            &BCAST,
            &P,
            &[0xF7],
        ]);
        let pc0 = ProfileChannels {
            profile: ProfileId(P),
            channels: 0,
        };
        let body1 = if enabled {
            Body::ProfileEnabled(pc0)
        } else {
            Body::ProfileDisabled(pc0)
        };
        check(
            &Message::new(env_v(b(), Muid::BROADCAST, ch(2), CiVersion::V1_1), body1),
            &v1,
        );
    }
}

/// M2-101 §7.4/§7.5, Tables 20/21: Added/Removed Reports.
#[test]
fn profile_added_removed_reports() {
    for (sub2, added) in [(0x26u8, true), (0x27u8, false)] {
        let expected = frame(&[
            &[0xF0, 0x7E, 0x7F, 0x0D, sub2, 0x02],
            &B,
            &BCAST,
            &Q,
            &[0xF7],
        ]);
        let r = ProfileReport {
            profile: ProfileId(Q),
        };
        let body = if added {
            Body::ProfileAdded(r)
        } else {
            Body::ProfileRemoved(r)
        };
        check(&Message::new(Envelope::broadcast(b()), body), &expected);
    }
}

/// M2-101 §7.6/§7.7, Tables 22/23, with M2-102 §2.5.1 Table 8 data.
#[test]
fn profile_details_inquiry_and_reply() {
    let inq = frame(&[
        &[0xF0, 0x7E, 0x00, 0x0D, 0x28, 0x02],
        &A,
        &B,
        &P,
        &[0x00, 0xF7],
    ]);
    check(
        &Message::new(
            env_v(a(), b(), ch(0), CiVersion::V1_2),
            Body::ProfileDetailsInquiry(ProfileDetailsInquiry {
                profile: ProfileId(P),
                target: TARGET_CHANNEL_COUNT,
            }),
        ),
        &inq,
    );
    let reply = frame(&[
        &[0xF0, 0x7E, 0x00, 0x0D, 0x29, 0x02],
        &B,
        &A,
        &P,
        &[0x00],       // target: number of channels
        &[0x04, 0x00], // data length 4
        &[0x01, 0x00], // in use: 1
        &[0x10, 0x00], // max: 16
        &[0xF7],
    ]);
    let data = ChannelCount { in_use: 1, max: 16 }.to_bytes().unwrap();
    assert_eq!(data, [0x01, 0x00, 0x10, 0x00]);
    check(
        &Message::new(
            env_v(b(), a(), ch(0), CiVersion::V1_2),
            Body::ProfileDetailsReply(ProfileDetailsReply {
                profile: ProfileId(P),
                target: 0,
                data: &data,
            }),
        ),
        &reply,
    );
    let (_, r) = ProfileDetailsReply::decode(&reply).unwrap();
    assert_eq!(
        ChannelCount::from_bytes(r.data),
        Ok(ChannelCount { in_use: 1, max: 16 })
    );
}

/// M2-101 §7.12, Table 28: 4-byte length.
#[test]
fn profile_specific_data() {
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x2F, 0x02],
        &A,
        &B,
        &P,
        &[0x03, 0x00, 0x00, 0x00],
        &[0x01, 0x02, 0x03],
        &[0xF7],
    ]);
    check(
        &Message::new(
            Envelope::new(a(), b()),
            Body::ProfileSpecificData(ProfileSpecificData {
                profile: ProfileId(P),
                data: &[1, 2, 3],
            }),
        ),
        &expected,
    );
}

// ---------------------------------------------------------------------------
// Property Exchange (M2-101 §8)
// ---------------------------------------------------------------------------

/// M2-101 §8.5/§8.6, Tables 30/32: v2 adds major/minor (Table 31: 0, 0).
#[test]
fn pe_capabilities_v1_v2() {
    let caps = PeCapabilities {
        simultaneous: 4,
        major: 0,
        minor: 0,
    };
    for (sub2, reply) in [(0x30u8, false), (0x31u8, true)] {
        let (src, dst, s, d) = if reply {
            (b(), a(), B, A)
        } else {
            (a(), b(), A, B)
        };
        let body = if reply {
            Body::PeCapabilitiesReply(caps)
        } else {
            Body::PeCapabilitiesInquiry(caps)
        };
        let v2 = frame(&[
            &[0xF0, 0x7E, 0x7F, 0x0D, sub2, 0x02],
            &s,
            &d,
            &[0x04, 0x00, 0x00, 0xF7],
        ]);
        check(&Message::new(Envelope::new(src, dst), body), &v2);
        let v1 = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, sub2, 0x01], &s, &d, &[0x04, 0xF7]]);
        check(
            &Message::new(
                env_v(src, dst, DeviceId::FunctionBlock, CiVersion::V1_1),
                body,
            ),
            &v1,
        );
    }
}

fn pe_frame(sub2: u8, s: [u8; 4], d: [u8; 4], fields: &[&[u8]]) -> Vec<u8> {
    let mut parts: Vec<&[u8]> = vec![
        &[0xF0, 0x7E, 0x7F, 0x0D],
        std::slice::from_ref(&sub2),
        &[0x02],
        &s,
        &d,
    ];
    parts.extend_from_slice(fields);
    parts.push(&[0xF7]);
    parts.concat()
}

/// M2-101 §8.8, Table 34, with the M2-103 §6.7 `LocalOn` reply (`true`).
#[test]
fn pe_get_reply() {
    let header = br#"{"status":200}"#;
    let expected = pe_frame(
        0x35,
        B,
        A,
        &[
            &[0x01],
            &[0x0E, 0x00], // 14
            header,
            &[0x01, 0x00, 0x01, 0x00],
            &[0x04, 0x00],
            b"true",
        ],
    );
    check(
        &Message::new(
            Envelope::new(b(), a()),
            Body::GetPropertyDataReply(PeChunk::single(1, header, b"true")),
        ),
        &expected,
    );
}

/// M2-101 §8.3 / Table 34: chunk 2 of 3 carries no header (§8.3.1).
#[test]
fn pe_get_reply_middle_chunk() {
    let expected = pe_frame(
        0x35,
        B,
        A,
        &[
            &[0x05],
            &[0x00, 0x00], // no header after chunk 1
            &[0x03, 0x00], // 3 chunks
            &[0x02, 0x00], // this is chunk 2
            &[0x05, 0x00],
            br#","b":"#,
        ],
    );
    let c = PeChunk {
        request_id: 5,
        header: &[],
        num_chunks: 3,
        this_chunk: 2,
        data: br#","b":"#,
    };
    assert!(!c.is_last());
    check(
        &Message::new(Envelope::new(b(), a()), Body::GetPropertyDataReply(c)),
        &expected,
    );
}

/// M2-101 §8.9, Table 35, with M2-103 §8.2's partial Set.
#[test]
fn pe_set_inquiry() {
    let header = br#"{"resource":"X-ProgramEdit","resId":"abcd","setPartial":true}"#;
    let data = br#"{"/lfoSpeed":10}"#;
    let expected = pe_frame(
        0x36,
        A,
        B,
        &[
            &[0x02],
            &[0x3D, 0x00], // 61
            header,
            &[0x01, 0x00, 0x01, 0x00],
            &[0x10, 0x00], // 16
            data,
        ],
    );
    check(
        &Message::new(
            Envelope::new(a(), b()),
            Body::SetPropertyData(PeChunk::single(2, header, data)),
        ),
        &expected,
    );
}

/// M2-101 §8.10, Table 36: fixed `01 00 01 00 00 00`.
#[test]
fn pe_set_reply() {
    let header = br#"{"status":200}"#;
    let expected = pe_frame(
        0x37,
        B,
        A,
        &[
            &[0x02],
            &[0x0E, 0x00],
            header,
            &[0x01, 0x00, 0x01, 0x00, 0x00, 0x00],
        ],
    );
    check(
        &Message::new(
            Envelope::new(b(), a()),
            Body::SetPropertyDataReply(PeChunk::single(2, header, &[])),
        ),
        &expected,
    );
}

/// M2-101 §8.11/§8.12, Tables 37/38, with M2-103 §9.3.1's example.
#[test]
fn pe_subscription_and_reply() {
    let start = br#"{"command":"start","resource":"CurrentMode"}"#;
    let expected = pe_frame(
        0x38,
        A,
        B,
        &[
            &[0x03],
            &[0x2C, 0x00],
            start,
            &[0x01, 0x00, 0x01, 0x00, 0x00, 0x00],
        ],
    );
    check(
        &Message::new(
            Envelope::new(a(), b()),
            Body::Subscription(PeChunk::single(3, start, &[])),
        ),
        &expected,
    );
    let reply = br#"{"status":200,"subscribeId":"sub32847623"}"#;
    let expected = pe_frame(
        0x39,
        B,
        A,
        &[
            &[0x03],
            &[0x2A, 0x00],
            reply,
            &[0x01, 0x00, 0x01, 0x00, 0x00, 0x00],
        ],
    );
    check(
        &Message::new(
            Envelope::new(b(), a()),
            Body::SubscriptionReply(PeChunk::single(3, reply, &[])),
        ),
        &expected,
    );
}

/// M2-101 §8.13, Table 39, with M2-103 §10.2's Terminate (144).
#[test]
fn pe_notify() {
    let header = br#"{"status":144}"#;
    let expected = pe_frame(
        0x3F,
        A,
        B,
        &[
            &[0x07],
            &[0x0E, 0x00],
            header,
            &[0x01, 0x00, 0x01, 0x00, 0x00, 0x00],
        ],
    );
    check(
        &Message::new(
            Envelope::new(a(), b()),
            Body::Notify(PeChunk::single(7, header, &[])),
        ),
        &expected,
    );
}

/// M2-101 v1.1 §8.6/§8.7: Has Property Data (0x32/0x33), same chunk layout
/// as the other PE messages; reserved in 1.2.
#[test]
fn legacy_has_property_data() {
    let header = br#"{"resource":"DeviceInfo"}"#;
    for (sub2, reply) in [(0x32u8, false), (0x33u8, true)] {
        let expected = frame(&[
            &[0xF0, 0x7E, 0x7F, 0x0D, sub2, 0x01],
            &A,
            &B,
            &[0x01, 0x19, 0x00],
            header,
            &[0x01, 0x00, 0x01, 0x00, 0x00, 0x00],
            &[0xF7],
        ]);
        let c = PeChunk::single(1, header, &[]);
        let body = if reply {
            Body::HasPropertyDataReply(c)
        } else {
            Body::HasPropertyData(c)
        };
        let m = Message::new(
            env_v(a(), b(), DeviceId::FunctionBlock, CiVersion::V1_1),
            body,
        );
        check(&m, &expected);
        assert!(m.kind().is_deprecated());
    }
}

// ---------------------------------------------------------------------------
// Process Inquiry (M2-101 §9)
// ---------------------------------------------------------------------------

/// M2-101 §9.2/§9.3, Tables 40–42.
#[test]
fn process_capabilities() {
    let inq = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, 0x40, 0x02], &A, &B, &[0xF7]]);
    check(
        &Message::new(
            Envelope::new(a(), b()),
            Body::ProcessInquiry(ProcessInquiry),
        ),
        &inq,
    );
    let reply = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, 0x41, 0x02], &B, &A, &[0x01, 0xF7]]);
    check(
        &Message::new(
            Envelope::new(b(), a()),
            Body::ProcessInquiryReply(ProcessInquiryReply {
                features: ProcessFeatures::MESSAGE_REPORT,
            }),
        ),
        &reply,
    );
}

/// M2-101 §9.5/§9.6/§9.8, Tables 43–46: full report of pitch bend, CC and
/// program change plus notes and poly pressure; the reply drops pitch bend
/// and poly pressure.
#[test]
fn midi_message_report() {
    let inq = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x42, 0x02],
        &A,
        &B,
        &[0x7F], // full report (Table 44)
        &[0x07], // MTC QF | song position | song select
        &[0x00], // reserved
        &[0x13], // pitch bend | CC | program change
        &[0x03], // notes | poly pressure
        &[0xF7],
    ]);
    let request = ReportSelection {
        system: SystemMessages::MTC_QUARTER_FRAME
            | SystemMessages::SONG_POSITION
            | SystemMessages::SONG_SELECT,
        channel: ChannelControllers::PITCH_BEND
            | ChannelControllers::CONTROL_CHANGE
            | ChannelControllers::PROGRAM_CHANGE,
        note: NoteData::NOTES | NoteData::POLY_PRESSURE,
    };
    check(
        &Message::new(
            Envelope::new(a(), b()),
            Body::MessageReportInquiry(MessageReportInquiry {
                control: DataControl::FULL,
                selection: request,
            }),
        ),
        &inq,
    );
    let reply = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x43, 0x02],
        &B,
        &A,
        &[0x07, 0x00, 0x12, 0x01],
        &[0xF7],
    ]);
    let granted = ReportSelection {
        system: request.system,
        channel: ChannelControllers::CONTROL_CHANGE | ChannelControllers::PROGRAM_CHANGE,
        note: NoteData::NOTES,
    };
    assert_eq!(granted.intersection(request), granted);
    check(
        &Message::new(
            Envelope::new(b(), a()),
            Body::MessageReportReply(MessageReportReply { selection: granted }),
        ),
        &reply,
    );
    let end = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, 0x44, 0x02], &B, &A, &[0xF7]]);
    check(
        &Message::new(
            Envelope::new(b(), a()),
            Body::EndOfMessageReport(EndOfMessageReport),
        ),
        &end,
    );
}

// ---------------------------------------------------------------------------
// MIDI-CI 1.1 Protocol Negotiation (M2-101 v1.1 §6), deprecated
// ---------------------------------------------------------------------------

const MIDI2: [u8; 5] = [0x02, 0x00, 0x00, 0x00, 0x00];
const MIDI1: [u8; 5] = [0x01, 0x00, 0x00, 0x00, 0x00];

const fn v11(src: Muid, dst: Muid) -> Envelope {
    env_v(src, dst, DeviceId::FunctionBlock, CiVersion::V1_1)
}

/// v1.1 §6.4/§6.5: authority, count, 5-byte protocols (preferred first).
#[test]
fn legacy_protocol_negotiation() {
    let protocols: Vec<u8> = [MIDI2, MIDI1].concat();
    for (sub2, reply) in [(0x10u8, false), (0x11u8, true)] {
        let expected = frame(&[
            &[0xF0, 0x7E, 0x7F, 0x0D, sub2, 0x01],
            &A,
            &B,
            &[0x10, 0x02],
            &MIDI2,
            &MIDI1,
            &[0xF7],
        ]);
        let n = ProtocolNegotiation {
            authority: 0x10,
            protocols: &protocols,
        };
        let body = if reply {
            Body::InitiateProtocolNegotiationReply(n)
        } else {
            Body::InitiateProtocolNegotiation(n)
        };
        let m = Message::new(v11(a(), b()), body);
        check(&m, &expected);
        assert!(m.kind().is_deprecated());
        assert_eq!(
            n.iter().collect::<Vec<_>>(),
            [ProtocolType(MIDI2), ProtocolType(MIDI1)]
        );
    }
}

/// v1.1 §6.6: Set New Selected Protocol.
#[test]
fn legacy_set_new_protocol() {
    let expected = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x12, 0x01],
        &A,
        &B,
        &[0x10],
        &MIDI2,
        &[0xF7],
    ]);
    check(
        &Message::new(
            v11(a(), b()),
            Body::SetNewProtocol(SetNewProtocol {
                authority: 0x10,
                protocol: ProtocolType(MIDI2),
            }),
        ),
        &expected,
    );
}

/// v1.1 §6.7/§6.8: Test New Protocol, 48 bytes `0x00..=0x2F`.
#[test]
fn legacy_test_new_protocol() {
    let test: Vec<u8> = (0x00..=0x2F).collect();
    assert_eq!(test.len(), 48);
    for (sub2, responder) in [(0x13u8, false), (0x14u8, true)] {
        let expected = frame(&[
            &[0xF0, 0x7E, 0x7F, 0x0D, sub2, 0x01],
            &A,
            &B,
            &[0x10],
            &test,
            &[0xF7],
        ]);
        let t = TestNewProtocol {
            authority: 0x10,
            data: &test,
        };
        let body = if responder {
            Body::TestNewProtocolResponder(t)
        } else {
            Body::TestNewProtocolInitiator(t)
        };
        check(&Message::new(v11(a(), b()), body), &expected);
    }
    // Short test data is truncated.
    let short = frame(&[
        &[0xF0, 0x7E, 0x7F, 0x0D, 0x13, 0x01],
        &A,
        &B,
        &[0x10, 0x00, 0x01],
        &[0xF7],
    ]);
    assert_eq!(Message::decode(&short), Err(Error::Truncated));
}

/// v1.1 §6.9: Confirmation New Protocol Established.
#[test]
fn legacy_protocol_established() {
    let expected = frame(&[&[0xF0, 0x7E, 0x7F, 0x0D, 0x15, 0x01], &A, &B, &[0x10, 0xF7]]);
    check(
        &Message::new(
            v11(a(), b()),
            Body::ProtocolEstablished(ProtocolEstablished { authority: 0x10 }),
        ),
        &expected,
    );
}
