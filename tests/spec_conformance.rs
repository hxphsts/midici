//! Data-driven conformance runner for the fixtures in `tests/conformance/`.
//!
//! MIDI-CI has no public official fixture set, so every case here was derived
//! by hand from the specification tables (see `tests/conformance/README.md`,
//! which also says where each document version comes from). A case is
//! identified by `<area>/<name>`, for example `management/discovery-v2`.
//!
//! What is asserted, per area:
//!
//! - **Frame areas** (management, profiles, pe-capabilities, pe-chunks,
//!   process, legacy-1.1): the `hex` frame decodes with
//!   [`midici::Message::decode`] to exactly the `expected` envelope and body
//!   (or to the `expected` error), and re-encodes to the identical bytes unless
//!   the case says `"roundTrip": false`. When `responderReply` is present, a
//!   [`Responder`] configured as described in the README is fed the frame and
//!   must emit exactly that one reply frame.
//! - **identity**: MIDI 1.0 Identity Request/Reply decode and re-encode.
//! - **mcoded7**: encode and decode in both directions, buffered and in place.
//! - **ump**: SysEx7 packetisation and reassembly.
//! - **scaling**: the M2-115 functions against the spec's numeric tables.
//! - **pe-headers** and **pe-resources**: parsed properties, and for headers
//!   the compact form written back.
//!
//! Two structural tests make the coverage matrix executable: every Sub-ID#2
//! that M2-101 Appendix D defines (plus the MIDI-CI 1.1-only kinds) has at
//! least one frame case, and every erratum the crate pins has a named case.
//!
//! What is not asserted: anything about bytes the specification leaves to the
//! implementation (e.g. NAK message text), and interoperability with other
//! implementations, which needs captured cases (see the README).
//!
//! # Known failures
//!
//! `tests/conformance/known_failures.txt` lists case ids that are allowed to
//! fail. The list can only shrink: a listed case that now passes fails the
//! run with a request to remove it, as does an id that matches no case.
//!
//! Set `MIDICI_CONFORMANCE_REPORT=1` to print per-area counts to stderr.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::cast_possible_truncation
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use midici::engine::responder::{Config, Device, Responder};
use midici::identity::{DeviceIdentity, IdentityReply, IdentityRequest, ManufacturerId};
use midici::message::{Body, Message};
use midici::mgmt::{NakStatus, Report};
use midici::pe::header::Header;
use midici::pe::resource::{
    parse_channel_list, parse_controllers, parse_ctrl_map_list, parse_device_info,
    parse_program_list, parse_resource_list, parse_state_list,
};
use midici::pe::{PeCapabilities, PeChunk, mcoded7};
use midici::process::ProcessFeatures;
use midici::profile::ProfileId;
use midici::wire::{DeviceId, Framing, MessageKind};
use midici::{Error, scaling, ump};
use serde_json::{Map, Value as Json, json};

const KNOWN_FAILURES: &str = "tests/conformance/known_failures.txt";

const FRAME_AREAS: &[&str] = &[
    "management",
    "profiles",
    "pe-capabilities",
    "pe-chunks",
    "process",
    "legacy-1.1",
];

/// Every erratum the crate pins, by case id.
const ERRATA: &[&str] = &[
    "pe-chunks/erratum-pe-device-id-0x0f-rejected",
    "pe-chunks/erratum-pe-device-id-0x0f-lenient",
    "management/erratum-version-reserved-bits",
    "management/erratum-undefined-sub-id-0x45",
    "profiles/profile-removed-report",
    "profiles/profile-details-reply-channel-count",
    "scaling/erratum-table-9-last-row",
    "pe-headers/erratum-mcoded7-spelling-in-header",
    "pe-resources/erratum-resource-list-mcoded7-spelling",
    "pe-resources/erratum-ctrl-map-list-require-id",
    "pe-resources/erratum-ch-ctrl-list-capital-title",
    "pe-resources/erratum-ctrl-map-list-table-20-unquoted-key",
    "pe-resources/channel-list-table-11",
    "mcoded7/erratum-mcoded7-bit-order-short-group",
];

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

struct Area {
    name: String,
    cases: Vec<Json>,
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance/fixtures")
}

fn load() -> Vec<Area> {
    let mut areas: Vec<Area> = fs::read_dir(dir())
        .expect("fixtures directory")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let text = fs::read_to_string(&p).expect("readable fixture");
            let file: Json = serde_json::from_str(&text).expect("valid fixture JSON");
            for key in ["spec", "version", "description", "cases"] {
                assert!(file.get(key).is_some(), "{} lacks {key}", p.display());
            }
            Area {
                name: p.file_stem().unwrap().to_string_lossy().into_owned(),
                cases: file["cases"].as_array().expect("cases array").clone(),
            }
        })
        .collect();
    areas.sort_by(|a, b| a.name.cmp(&b.name));
    areas
}

fn id(area: &Area, case: &Json) -> String {
    format!(
        "{}/{}",
        area.name,
        case["name"].as_str().expect("case name")
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|t| u8::from_str_radix(t, 16).expect("hex byte"))
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn ascii(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Compares an error against a fixture's `{"error": "..."}`: a bare variant
/// name matches the variant; anything with `(` or `{` must match the whole
/// `Debug` text.
fn error_matches(e: &Error, expected: &str) -> bool {
    let debug = format!("{e:?}");
    if expected.contains('(') || expected.contains('{') {
        debug == expected
    } else {
        debug.split(['(', ' ']).next() == Some(expected)
    }
}

fn expect_error(case: &Json) -> Option<&str> {
    case.get("expected")
        .and_then(|e| e.get("error"))
        .and_then(Json::as_str)
}

fn round_trip(case: &Json) -> bool {
    case.get("roundTrip")
        .and_then(Json::as_bool)
        .unwrap_or(true)
}

fn diff(what: &str, got: &Json, want: &Json) -> Result<(), String> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{what} mismatch\n  got:  {got}\n  want: {want}"))
    }
}

// ---------------------------------------------------------------------------
// Decoded message → JSON (the fixture's `expected` shape)
// ---------------------------------------------------------------------------

fn device_json(d: DeviceId) -> Json {
    match d {
        DeviceId::FunctionBlock => json!("functionBlock"),
        DeviceId::Group => json!("group"),
        DeviceId::Channel(c) => json!({ "channel": c.index() }),
    }
}

fn identity_json(i: &DeviceIdentity) -> Json {
    json!({
        "manufacturer": i.manufacturer.ci_bytes(),
        "family": i.family,
        "model": i.model,
        "revision": i.revision,
    })
}

fn profile(p: ProfileId) -> Json {
    json!(p.bytes())
}

fn report_json<S>(r: &Report<'_, S>, status: u8) -> Json {
    json!({
        "original": r.original.sub_id2(),
        "status": status,
        "statusData": r.status_data,
        "details": r.details.bytes(),
        "text": r.text.as_str(),
    })
}

fn chunk_json(c: &PeChunk<'_>) -> Json {
    json!({
        "requestId": c.request_id,
        "header": ascii(c.header),
        "numChunks": c.num_chunks,
        "thisChunk": c.this_chunk,
        "data": ascii(c.data),
    })
}

fn caps_json(c: PeCapabilities) -> Json {
    json!({ "simultaneous": c.simultaneous, "major": c.major, "minor": c.minor })
}

fn body_json(body: &Body<'_>) -> Json {
    match body {
        Body::Discovery(d) => json!({
            "identity": identity_json(&d.identity),
            "categories": d.categories.bits(),
            "maxSysex": d.max_sysex,
            "outputPath": d.output_path,
        }),
        Body::DiscoveryReply(d) => json!({
            "identity": identity_json(&d.identity),
            "categories": d.categories.bits(),
            "maxSysex": d.max_sysex,
            "outputPath": d.output_path,
            "functionBlock": d.function_block,
        }),
        Body::EndpointInquiry(q) => json!({ "status": q.status }),
        Body::EndpointReply(r) => json!({ "status": r.status, "data": ascii(r.data) }),
        Body::InvalidateMuid(i) => json!({ "target": i.target.value() }),
        Body::Ack(a) => report_json(a, a.status.value()),
        Body::Nak(n) => {
            json!({ "report": n.report.as_ref().map(|r| report_json(r, r.status.value())) })
        }
        Body::ProfileInquiry(_) | Body::ProcessInquiry(_) | Body::EndOfMessageReport(_) => {
            json!({})
        }
        Body::ProfileInquiryReply(r) => json!({
            "enabled": r.enabled.iter().map(profile).collect::<Vec<_>>(),
            "disabled": r.disabled.iter().map(profile).collect::<Vec<_>>(),
        }),
        Body::SetProfileOn(p) | Body::ProfileEnabled(p) | Body::ProfileDisabled(p) => {
            json!({ "profile": profile(p.profile), "channels": p.channels })
        }
        Body::SetProfileOff(p) => json!({ "profile": profile(p.profile) }),
        Body::ProfileAdded(p) | Body::ProfileRemoved(p) => json!({ "profile": profile(p.profile) }),
        Body::ProfileDetailsInquiry(q) => {
            json!({ "profile": profile(q.profile), "target": q.target })
        }
        Body::ProfileDetailsReply(r) => {
            json!({ "profile": profile(r.profile), "target": r.target, "data": hex(r.data) })
        }
        Body::ProfileSpecificData(d) => {
            json!({ "profile": profile(d.profile), "data": hex(d.data) })
        }
        Body::PeCapabilitiesInquiry(c) | Body::PeCapabilitiesReply(c) => caps_json(*c),
        Body::HasPropertyData(c)
        | Body::HasPropertyDataReply(c)
        | Body::GetPropertyData(c)
        | Body::GetPropertyDataReply(c)
        | Body::SetPropertyData(c)
        | Body::SetPropertyDataReply(c)
        | Body::Subscription(c)
        | Body::SubscriptionReply(c)
        | Body::Notify(c) => chunk_json(c),
        Body::ProcessInquiryReply(r) => json!({ "features": r.features.bits() }),
        Body::MessageReportInquiry(q) => json!({
            "control": q.control.value(),
            "system": q.selection.system.bits(),
            "channel": q.selection.channel.bits(),
            "note": q.selection.note.bits(),
        }),
        Body::MessageReportReply(r) => json!({
            "system": r.selection.system.bits(),
            "channel": r.selection.channel.bits(),
            "note": r.selection.note.bits(),
        }),
        Body::InitiateProtocolNegotiation(p) | Body::InitiateProtocolNegotiationReply(p) => json!({
            "authority": p.authority,
            "protocols": p.iter().map(midici::legacy::ProtocolType::bytes).collect::<Vec<_>>(),
        }),
        Body::SetNewProtocol(p) => {
            json!({ "authority": p.authority, "protocol": p.protocol.bytes() })
        }
        Body::TestNewProtocolInitiator(t) | Body::TestNewProtocolResponder(t) => {
            json!({ "authority": t.authority, "data": hex(t.data) })
        }
        Body::ProtocolEstablished(p) => json!({ "authority": p.authority }),
        Body::Raw { data, .. } => json!({ "data": hex(data) }),
        other => json!({ "unmapped": format!("{other:?}") }),
    }
}

fn message_json(m: &Message<'_>) -> Json {
    let e = &m.envelope;
    json!({
        "kind": format!("{:?}", m.kind()),
        "device": device_json(e.device),
        "version": e.version.byte(),
        "source": e.source.value(),
        "destination": e.destination.value(),
        "body": body_json(&m.body),
    })
}

// ---------------------------------------------------------------------------
// The Responder used for `responderReply` (documented in the README)
// ---------------------------------------------------------------------------

struct ConformanceDevice;

impl Device for ConformanceDevice {
    fn random(&mut self) -> u32 {
        0x0ABC_DEF0
    }
}

fn responder_frames(frame: &[u8], case: &Json) -> Vec<String> {
    let identity = DeviceIdentity {
        manufacturer: ManufacturerId::NON_COMMERCIAL,
        family: 1,
        model: 2,
        revision: [0, 1, 0, 0],
    };
    let mut config = Config::new(identity);
    config.max_sysex = 512;
    config.max_frame = 128;
    config.profiles = true;
    config.property_exchange = Some(PeCapabilities::new(1));
    config.process = ProcessFeatures::MESSAGE_REPORT;
    config.product_instance_id = Some("CONF-01");
    config.lenient_pe_device_id = case
        .get("responderConfig")
        .and_then(|c| c.get("lenientPeDeviceId"))
        .and_then(Json::as_bool)
        .unwrap_or(false);
    // MUID 0x0765_4321 ("B" in the fixtures).
    let mut responder: Responder<'_, 128> = Responder::new(config, 0x0765_4321);
    let mut device = ConformanceDevice;
    responder.handle(&mut device, 0, frame);
    let mut out = Vec::new();
    let mut buf = [0u8; 128];
    while let Some(n) = responder.poll_transmit(&device, &mut buf) {
        out.push(hex(&buf[..n]));
    }
    out
}

// ---------------------------------------------------------------------------
// Runners
// ---------------------------------------------------------------------------

fn run_frame(case: &Json) -> Result<(), String> {
    let bytes = parse_hex(case["hex"].as_str().ok_or("hex missing")?);
    match (Message::decode(&bytes), expect_error(case)) {
        (Err(e), Some(want)) if error_matches(&e, want) => {}
        (Err(e), Some(want)) => return Err(format!("error {e:?}, want {want}")),
        (Err(e), None) => return Err(format!("decode failed: {e:?}")),
        (Ok(m), Some(want)) => return Err(format!("decoded {m:?}, want error {want}")),
        (Ok(m), None) => {
            diff("decoded message", &message_json(&m), &case["expected"])?;
            if round_trip(case) {
                let mut out = [0u8; 512];
                let n = m
                    .encode(Framing::SysEx, &mut out)
                    .map_err(|e| format!("re-encode failed: {e:?}"))?;
                if out[..n] != bytes[..] {
                    return Err(format!(
                        "re-encoded\n  got:  {}\n  want: {}",
                        hex(&out[..n]),
                        hex(&bytes)
                    ));
                }
            }
        }
    }
    if let Some(want) = case.get("responderReply").and_then(Json::as_str) {
        let got = responder_frames(&bytes, case);
        if got != [want.to_string()] {
            return Err(format!(
                "responder reply\n  got:  {got:?}\n  want: [{want:?}]"
            ));
        }
    }
    Ok(())
}

fn run_identity(case: &Json) -> Result<(), String> {
    let bytes = parse_hex(case["hex"].as_str().ok_or("hex missing")?);
    if let Some(want) = expect_error(case) {
        return match IdentityReply::decode(&bytes) {
            Err(e) if error_matches(&e, want) => Ok(()),
            other => Err(format!("got {other:?}, want error {want}")),
        };
    }
    let expected = &case["expected"];
    let mut out = [0u8; 64];
    let n = if expected["type"] == "request" {
        let r = IdentityRequest::decode(&bytes).map_err(|e| format!("{e:?}"))?;
        diff(
            "request",
            &json!({ "type": "request", "deviceId": r.device_id }),
            expected,
        )?;
        r.encode(Framing::SysEx, &mut out)
            .map_err(|e| format!("{e:?}"))?
    } else {
        let r = IdentityReply::decode(&bytes).map_err(|e| format!("{e:?}"))?;
        let got = json!({ "type": "reply", "deviceId": r.device_id, "identity": identity_json(&r.identity) });
        diff("reply", &got, expected)?;
        r.encode(Framing::SysEx, &mut out)
            .map_err(|e| format!("{e:?}"))?
    };
    if round_trip(case) && out[..n] != bytes[..] {
        return Err(format!("re-encoded {}", hex(&out[..n])));
    }
    Ok(())
}

fn run_mcoded7(case: &Json) -> Result<(), String> {
    let encoded = parse_hex(case["encoded"].as_str().ok_or("encoded missing")?);
    let mut out = vec![0u8; encoded.len() + 8];
    if let Some(want) = expect_error(case) {
        return match mcoded7::decode(&encoded, &mut out) {
            Err(e) if error_matches(&e, want) => Ok(()),
            other => Err(format!("got {other:?}, want error {want}")),
        };
    }
    let decoded = parse_hex(case["decoded"].as_str().ok_or("decoded missing")?);
    let mut enc = vec![0u8; mcoded7::encoded_len(decoded.len())];
    let n = mcoded7::encode(&decoded, &mut enc).map_err(|e| format!("{e:?}"))?;
    if enc[..n] != encoded[..] {
        return Err(format!("encoded {}", hex(&enc[..n])));
    }
    let n = mcoded7::decode(&encoded, &mut out).map_err(|e| format!("{e:?}"))?;
    if out[..n] != decoded[..] {
        return Err(format!("decoded {}", hex(&out[..n])));
    }
    let mut in_place = encoded;
    let n = mcoded7::decode_in_place(&mut in_place).map_err(|e| format!("{e:?}"))?;
    if in_place[..n] != decoded[..] {
        return Err(format!("decoded in place {}", hex(&in_place[..n])));
    }
    Ok(())
}

fn run_ump(case: &Json) -> Result<(), String> {
    let payload = parse_hex(case["payload"].as_str().ok_or("payload missing")?);
    let group = case["group"].as_u64().ok_or("group missing")? as u8;
    let want: Vec<[u32; 2]> = case["packets"]
        .as_array()
        .ok_or("packets missing")?
        .iter()
        .map(|p| {
            let w = |i: usize| u32::from_str_radix(p[i].as_str().unwrap(), 16).unwrap();
            [w(0), w(1)]
        })
        .collect();
    let got: Vec<[u32; 2]> = ump::packets(&payload, group).collect();
    if got != want {
        return Err(format!("packets {got:08X?}"));
    }
    let mut asm = ump::SysEx7Assembler::<64>::new();
    let mut done = None;
    for p in got {
        if let Some((g, data)) = asm.push(p) {
            done = Some((g, data.to_vec()));
        }
    }
    if done != Some((group, payload)) {
        return Err(format!("reassembled {done:?}"));
    }
    Ok(())
}

fn run_scaling(case: &Json) -> Result<(), String> {
    let args: Vec<u32> = case["args"]
        .as_array()
        .ok_or("args missing")?
        .iter()
        .map(|a| a.as_u64().unwrap() as u32)
        .collect();
    let f = case["function"].as_str().ok_or("function missing")?;
    let got = match f {
        "scaleUp" => scaling::scale_up(args[0], args[1], args[2]),
        "scaleDown" => scaling::scale_down(args[0], args[1], args[2]),
        "zeroExtendUp" => scaling::zero_extend_up(args[0], args[1], args[2]),
        "roundDown" => scaling::round_down(args[0], args[1], args[2]),
        "stepToValue" => scaling::step_to_value(args[0], args[1], args[2]),
        "valueToStep" => scaling::value_to_step(args[0], args[1], args[2]),
        other => return Err(format!("unknown function {other}")),
    };
    diff("result", &json!(got), &case["expected"])
}

fn header_json(h: &Header<'_>) -> Json {
    let mut m = Map::new();
    let mut put = |k: &str, v: Json| {
        m.insert(k.to_string(), v);
    };
    if let Some(v) = h.resource {
        put("resource", json!(v.to_string()));
    }
    if let Some(v) = h.res_id {
        put("resId", json!(v.to_string()));
    }
    if let Some(v) = h.command {
        put("command", json!(v.as_str()));
    }
    if let Some(v) = h.subscribe_id {
        put("subscribeId", json!(v.to_string()));
    }
    if let Some(v) = h.status {
        put("status", json!(v.value()));
    }
    if let Some(v) = h.message {
        put("message", json!(v.to_string()));
    }
    if let Some(v) = h.mutual_encoding {
        put("mutualEncoding", json!(v.as_str()));
    }
    if let Some(v) = h.media_type {
        put("mediaType", json!(v.to_string()));
    }
    for (k, v) in [
        ("offset", h.offset),
        ("limit", h.limit),
        ("totalCount", h.total_count),
        ("cacheTime", h.cache_time),
    ] {
        if let Some(v) = v {
            put(k, json!(v));
        }
    }
    if let Some(v) = h.state_rev {
        put("stateRev", json!(v.to_string()));
    }
    if let Some(v) = h.timestamp {
        put("timestamp", json!(v));
    }
    if h.set_partial {
        put("setPartial", json!(true));
    }
    Json::Object(m)
}

fn run_header(case: &Json) -> Result<(), String> {
    let input = case["input"].as_str().ok_or("input missing")?;
    match (Header::parse(input.as_bytes()), expect_error(case)) {
        (Err(e), Some(want)) if error_matches(&e, want) => Ok(()),
        (Err(e), Some(want)) => Err(format!("error {e:?}, want {want}")),
        (Err(e), None) => Err(format!("parse failed: {e:?}")),
        (Ok(h), Some(want)) => Err(format!("parsed {h:?}, want error {want}")),
        (Ok(h), None) => {
            diff("header", &header_json(&h), &case["expected"])?;
            if let Some(want) = case.get("write").and_then(Json::as_str) {
                let mut buf = [0u8; 256];
                let n = h.write_to(&mut buf).map_err(|e| format!("{e:?}"))?;
                if &buf[..n] != want.as_bytes() {
                    return Err(format!("wrote {}", ascii(&buf[..n])));
                }
            }
            Ok(())
        }
    }
}

/// Inserts `k: v` when `v` is `Some`.
macro_rules! opt {
    ($m:expr, $k:expr, $v:expr $(,)?) => {
        if let Some(v) = $v {
            $m.insert($k.to_string(), json!(v));
        }
    };
}

fn resource_json(resource: &str, input: &[u8]) -> Result<Json, Error> {
    Ok(match resource {
        "ResourceList" => Json::Array(
            parse_resource_list(input)?
                .iter()
                .map(|r| {
                    json!({
                        "resource": r.resource,
                        "canGet": r.can_get,
                        "canSet": r.can_set.as_str(),
                        "canSubscribe": r.can_subscribe,
                        "requireResId": r.require_res_id,
                        "canPaginate": r.can_paginate,
                        "mediaTypes": r.media_types,
                        "encodings": r.encodings,
                        "supportsMcoded7": r.supports_encoding("Mcoded7"),
                    })
                })
                .collect(),
        ),
        "DeviceInfo" => {
            let d = parse_device_info(input)?;
            let mut m = Map::new();
            for (k, v) in [
                ("manufacturerId", json!(d.manufacturer_id)),
                ("familyId", json!(d.family_id)),
                ("modelId", json!(d.model_id)),
                ("versionId", json!(d.version_id)),
                ("manufacturer", json!(d.manufacturer)),
                ("family", json!(d.family)),
                ("model", json!(d.model)),
                ("version", json!(d.version)),
            ] {
                m.insert(k.to_string(), v);
            }
            opt!(m, "serialNumber", d.serial_number);
            Json::Object(m)
        }
        "ChannelList" => Json::Array(
            parse_channel_list(input)?
                .into_iter()
                .map(|c| {
                    let mut m = Map::new();
                    m.insert("title".into(), json!(c.title));
                    m.insert("channel".into(), json!(c.channel));
                    opt!(m, "programTitle", c.program_title);
                    opt!(m, "bankPC", c.bank_pc);
                    opt!(m, "clusterChannelStart", c.cluster_channel_start);
                    opt!(m, "clusterLength", c.cluster_length);
                    opt!(m, "clusterMidiMode", c.cluster_midi_mode);
                    opt!(m, "clusterType", c.cluster_type);
                    Json::Object(m)
                })
                .collect(),
        ),
        "AllCtrlList" | "ChCtrlList" => Json::Array(
            parse_controllers(input)?
                .into_iter()
                .map(|c| {
                    let mut m = Map::new();
                    m.insert("title".into(), json!(c.title));
                    opt!(m, "description", c.description);
                    opt!(m, "channel", c.channel);
                    m.insert("ctrlType".into(), json!(c.ctrl_type.as_str()));
                    opt!(
                        m,
                        "ctrlIndex",
                        c.ctrl_index
                            .map(|i| i[..usize::from(c.ctrl_index_len)].to_vec()),
                    );
                    opt!(m, "priority", c.priority);
                    opt!(m, "default", c.default);
                    opt!(m, "transmit", c.transmit);
                    opt!(m, "recognize", c.recognize);
                    opt!(m, "numSigBits", c.num_sig_bits);
                    opt!(m, "paramPath", c.param_path);
                    opt!(m, "typeHint", c.type_hint);
                    opt!(m, "ctrlMapId", c.ctrl_map_id);
                    opt!(m, "stepCount", c.step_count);
                    opt!(m, "minMax", c.min_max);
                    Json::Object(m)
                })
                .collect(),
        ),
        "CtrlMapList" => Json::Array(
            parse_ctrl_map_list(input)?
                .into_iter()
                .map(|c| json!({ "value": c.value, "title": c.title }))
                .collect(),
        ),
        "ProgramList" => Json::Array(
            parse_program_list(input)?
                .into_iter()
                .map(|p| json!({ "title": p.title, "bankPC": p.bank_pc }))
                .collect(),
        ),
        "StateList" => Json::Array(
            parse_state_list(input)?
                .into_iter()
                .map(|s| {
                    let mut m = Map::new();
                    m.insert("title".into(), json!(s.title));
                    m.insert("stateId".into(), json!(s.state_id));
                    opt!(m, "stateRev", s.state_rev);
                    opt!(m, "timestamp", s.timestamp);
                    opt!(m, "description", s.description);
                    opt!(m, "size", s.size);
                    Json::Object(m)
                })
                .collect(),
        ),
        other => panic!("unknown resource {other}"),
    })
}

fn run_resource(case: &Json) -> Result<(), String> {
    let resource = case["resource"].as_str().ok_or("resource missing")?;
    let input = case["input"].as_str().ok_or("input missing")?;
    match (
        resource_json(resource, input.as_bytes()),
        expect_error(case),
    ) {
        (Err(e), Some(want)) if error_matches(&e, want) => Ok(()),
        (Err(e), Some(want)) => Err(format!("error {e:?}, want {want}")),
        (Err(e), None) => Err(format!("parse failed: {e:?}")),
        (Ok(v), Some(want)) => Err(format!("parsed {v}, want error {want}")),
        (Ok(v), None) => diff(resource, &v, &case["expected"]),
    }
}

fn run_case(area: &str, case: &Json) -> Result<(), String> {
    match area {
        a if FRAME_AREAS.contains(&a) => run_frame(case),
        "identity" => run_identity(case),
        "mcoded7" => run_mcoded7(case),
        "ump" => run_ump(case),
        "scaling" => run_scaling(case),
        "pe-headers" => run_header(case),
        "pe-resources" => run_resource(case),
        other => Err(format!("no runner for area {other}")),
    }
}

// ---------------------------------------------------------------------------
// Verdict
// ---------------------------------------------------------------------------

fn known_failures() -> BTreeSet<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(KNOWN_FAILURES);
    fs::read_to_string(path)
        .expect("known_failures.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().next().unwrap().to_string())
        .collect()
}

#[test]
fn spec_conformance_fixtures() {
    let known = known_failures();
    let mut all = BTreeSet::new();
    let mut failed = Vec::new();
    let mut fixed = Vec::new();
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for area in load() {
        for case in &area.cases {
            let id = id(&area, case);
            assert!(all.insert(id.clone()), "duplicate case id {id}");
            let result = run_case(&area.name, case);
            let entry = counts.entry(area.name.clone()).or_default();
            entry.0 += 1;
            match (result, known.contains(&id)) {
                (Ok(()), false) => entry.1 += 1,
                (Ok(()), true) => fixed.push(id),
                (Err(_), true) => {}
                (Err(why), false) => failed.push(format!("{id}: {why}")),
            }
        }
    }
    let stale: Vec<_> = known.iter().filter(|k| !all.contains(*k)).collect();
    if std::env::var_os("MIDICI_CONFORMANCE_REPORT").is_some() {
        let mut report = String::from("\nMIDI-CI conformance\n");
        for (area, (total, passed)) in &counts {
            let _ = writeln!(report, "  {area:<16} {passed:>3}/{total:<3}");
        }
        let _ = writeln!(
            report,
            "  total            {}/{}",
            counts.values().map(|c| c.1).sum::<usize>(),
            all.len()
        );
        eprintln!("{report}");
    }
    assert!(
        failed.is_empty(),
        "{} failing case(s):\n{}",
        failed.len(),
        failed.join("\n")
    );
    assert!(
        fixed.is_empty(),
        "NOW PASSING: remove from {KNOWN_FAILURES}:\n{}",
        fixed.join("\n")
    );
    assert!(
        stale.is_empty(),
        "UNKNOWN IDS in {KNOWN_FAILURES}: {stale:?}"
    );
}

/// Every Sub-ID#2 of M2-101 Appendix D, plus the MIDI-CI 1.1-only kinds,
/// has at least one frame case.
#[test]
fn every_message_kind_has_a_case() {
    let mut covered = BTreeSet::new();
    for area in load() {
        if !FRAME_AREAS.contains(&area.name.as_str()) {
            continue;
        }
        for case in &area.cases {
            let bytes = parse_hex(case["hex"].as_str().unwrap());
            if let Some(&sub2) = bytes.get(4) {
                covered.insert(sub2);
            }
        }
    }
    let missing: Vec<String> = MessageKind::ALL
        .iter()
        .filter(|k| !covered.contains(&k.sub_id2()))
        .map(|k| format!("{k:?} (0x{:02X})", k.sub_id2()))
        .collect();
    assert!(
        missing.is_empty(),
        "message kinds without a conformance case: {missing:?}"
    );
}

/// Every erratum the crate pins is a named case carrying an `erratum` note.
#[test]
fn every_erratum_has_a_case() {
    let mut noted = BTreeSet::new();
    for area in load() {
        for case in &area.cases {
            if case.get("erratum").is_some() {
                noted.insert(id(&area, case));
            }
        }
    }
    let missing: Vec<_> = ERRATA.iter().filter(|e| !noted.contains(**e)).collect();
    assert!(missing.is_empty(), "errata without a case: {missing:?}");
}

/// The NAK status the §8.2 erratum case expects is the crate's named
/// constant, so the fixture and the code cannot drift apart silently.
#[test]
fn erratum_pe_device_id_nak_status_is_malformed() {
    assert_eq!(NakStatus::MALFORMED.value(), 0x41);
}
