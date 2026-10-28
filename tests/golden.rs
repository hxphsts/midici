//! Hand-derived frames that do not fit the fixture format of
//! `tests/conformance/`: encoder range checks, the 128-byte NAK budget,
//! envelope-only parsing, and typed-decode mismatches. Every per-message
//! golden frame lives in `tests/conformance/fixtures/` instead.
//!
//! MUIDs used throughout (7-bit, LSB first):
//! * `A = 0x0123_4567` → `67 0A 0D 09`
//! * `B = 0x0765_4321` → `21 06 15 3B`
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use midici::identity::{DeviceIdentity, IdentityReply, ManufacturerId};
use midici::mgmt::{Categories, Discovery, DiscoveryReply, Nak, NakStatus, Text};
use midici::profile::ProfileId;
use midici::wire::{Envelope, Frame, Framing, MessageKind};
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

const fn nc_identity() -> DeviceIdentity {
    DeviceIdentity {
        manufacturer: ManufacturerId::NON_COMMERCIAL,
        family: 0x0102,
        model: 0x0003,
        revision: [0, 1, 2, 3],
    }
}

fn discovery() -> Discovery {
    Discovery::new(
        nc_identity(),
        Categories::PROFILE_CONFIGURATION | Categories::PROPERTY_EXCHANGE,
        512,
    )
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
    let mut r =
        DiscoveryReply::new(nc_identity(), Categories::NONE, 128).with_function_block(Some(0x7F));
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
        Nak::v1()
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

#[test]
fn profile_id_constructors() {
    // M2-102 Table 5: a Standard Defined Profile and a manufacturer one.
    const P: [u8; 5] = [0x7E, 0x21, 0x00, 0x01, 0x01];
    const Q: [u8; 5] = [0x7D, 0x00, 0x00, 0x12, 0x34];
    assert_eq!(ProfileId::standard(0x21, 0x00, 0x01, 0x01).bytes(), P);
    let m = ProfileId::manufacturer(ManufacturerId::NON_COMMERCIAL, [0x12, 0x34]);
    assert_eq!(m.bytes(), Q);
    assert!(ProfileId::new(P).unwrap().is_standard());
    assert!(!m.is_standard());
}
