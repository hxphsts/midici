# MIDI-CI conformance cases

These fixtures are exercised by `tests/spec_conformance.rs`:

```sh
cargo test --test spec_conformance
MIDICI_CONFORMANCE_REPORT=1 cargo test --test spec_conformance -- --nocapture
```

**Where the cases come from.** Unlike TOON or other formats with an official
test suite, MIDI-CI has no public official fixture set. Every case here was
derived by hand from the tables of the specification documents listed below
and records that in its `provenance` field (`"derived from M2-101 Table 6"`).
No specification text is reproduced: cases cite sections and tables, and
the frames are our reading of those tables. Interoperability with other
implementations is not proven by these cases; captured frames (see
[Adding a captured case](#adding-a-captured-case)) are how that gets added.

## Documents

| Document | Version | Used for |
|---|---|---|
| M2-101-UM MIDI-CI | 1.2 (2023-06-15) | every message layout |
| M2-101-UM MIDI-CI | 1.1 (2020-02-20) | `legacy-1.1` and the version 1 forms |
| M2-102-U Common Rules for Profiles | 1.1 | Profile ID, Profile Details target 0x00 |
| M2-103-UM Common Rules for PE | 1.1 | Mcoded7, Header Data, status codes, ResourceList |
| M2-104-UM UMP and MIDI 2.0 Protocol | 1.1.1 | SysEx7 packets (`ump`) |
| M2-105-UM PE Foundational Resources | 1.1.1 | DeviceInfo, ChannelList |
| M2-111-UM Get and Set Device State | 1.0 | StateList, State headers |
| M2-115-U Bit Scaling and Resolution | 1.0.1 | `scaling` |
| M2-117-UM PE Controller Resources | 1.0 | AllCtrlList, ChCtrlList, CtrlMapList |

**Unverified:**

- **M2-101 v1.2.1** is not public; any change it made is not reflected.
- **M2-107 (ProgramList)** is not public. `ProgramEntry` parses only
  `title` and `bankPC`, and the known failure
  `pe-resources/program-list-category` records the gap.
- **The MIDI 1.0 Identity Request/Reply layout** (`identity`) follows the
  widely documented form. M2-101 §5.5.1 confirms the field sizes.

## Areas and coverage

| Area (`fixtures/<area>.json`) | Cases | Covers |
|---|---|---|
| `management` | 20 | Discovery, Reply, Endpoint, Invalidate MUID, ACK, NAK (1.1 and 1.2), envelope errors, version handling |
| `profiles` | 15 | Sub-ID#2 0x20-0x29, 0x2F; v1 and v2 forms |
| `pe-capabilities` | 4 | 0x30, 0x31; v1 and v2 forms |
| `pe-chunks` | 13 | 0x34-0x39, 0x3F, chunk numbering, the §8.2 erratum |
| `process` | 6 | 0x40-0x44 |
| `legacy-1.1` | 8 | 0x10-0x15, 0x32, 0x33 |
| `identity` | 5 | MIDI 1.0 Identity Request/Reply |
| `mcoded7` | 7 | M2-103 §4.3.1 |
| `ump` | 4 | M2-104 §7.7 |
| `scaling` | 36 | M2-115 Tables 5-10 |
| `pe-headers` | 10 | M2-103 §5, §6.6.2, §8, §9, M2-111 §3 |
| `pe-resources` | 11 | M2-103 §12, M2-105, M2-111, M2-117 |

`every_message_kind_has_a_case` fails, naming the kind, if any Sub-ID#2 in
M2-101 Appendix D (or a MIDI-CI 1.1-only kind) has no frame case.

### How the crate handles each message

| Sub-ID#2 | Message | Section | Responder | Initiator |
|---|---|---|---|---|
| 0x10-0x15 | Protocol Negotiation | 1.1 §6 | NAK 0x01 (deprecated) | NAK 0x01 if addressed |
| 0x20 / 0x21 | Profile Inquiry / Reply | §7.2, §7.3 | answers from `Device::profiles` | `profile_inquiry`, `Event::Profiles` |
| 0x22 / 0x23 | Set Profile On / Off | §7.8, §7.9 | `Device::set_profile`, report or NAK 0x04 | `set_profile` |
| 0x24 / 0x25 | Profile Enabled / Disabled Report | §7.10, §7.11 | sent to broadcast | `Event::ProfileState` |
| 0x26 / 0x27 | Profile Added / Removed Report | §7.4, §7.5 | `profile_added` / `profile_removed` | `Event::ProfileListChanged` |
| 0x28 / 0x29 | Profile Details | §7.6, §7.7 | `Device::profile_details` or NAK 0x04 | `profile_details` |
| 0x2F | Profile Specific Data | §7.12 | `Device::profile_data` | n/a |
| 0x30 / 0x31 | PE Capabilities | §8.5, §8.6 | answers | `pe_capabilities` |
| 0x32 / 0x33 | Has Property Data | 1.1 §8 | NAK 0x01 (reserved in 1.2) | n/a |
| 0x34 / 0x35 | Get Property Data | §8.7, §8.8 | chunked, Mcoded7, ResourceList automatic | `get`, reassembly |
| 0x36 / 0x37 | Set Property Data | §8.9, §8.10 | reassembly, full/partial checks | `set`, chunked to the peer |
| 0x38 / 0x39 | Subscription | §8.11, §8.12 | start/end, `publish` | `subscribe` / `unsubscribe` |
| 0x3F | Notify | §8.13 | 144/408 abort | 100 extends, 144/408 fail |
| 0x40 / 0x41 | Process Inquiry Capabilities | §9.2, §9.3 | answers | `process_capabilities` |
| 0x42-0x44 | MIDI Message Report | §9.5-§9.8 | `Device::message_report`, `end_message_report` | `message_report` |
| 0x70 / 0x71 | Discovery | §5.5, §5.6 | answers; collisions §5.9.1 | `discover`; duplicates §5.9.2 |
| 0x72 / 0x73 | Endpoint Information | §5.7, §5.8 | Product Instance Id or NAK | `endpoint_info` |
| 0x7D / 0x7F | ACK / NAK | §5.10, §5.11 | NAKs with typed status | maps to `Failure::Nak` |
| 0x7E | Invalidate MUID | §5.9 | renews or forgets | renews or forgets |
| other | undefined | §5.1 | NAK 0x01 | NAK 0x01 if addressed |

## Implementation status

What midici 0.0.1 implements, by area:

| Area | Spec | Status |
|---|---|---|
| Envelope, Device ID, version, every Sub-ID#2 | M2-101 §5.1–5.4, App. D | done |
| MUID: generation, broadcast, reserved range, collisions | §3.3, §5.9 | done |
| Discovery, Endpoint Information, Invalidate MUID, ACK, NAK | §5.5–5.11 | done |
| Profile Configuration (all 11 messages, Profile Details) | §7, M2-102 | done |
| Property Exchange: capabilities, Get, Set, Subscribe, Notify | §8, M2-103 | done |
| Chunking (any max SysEx size ≥ 25), reassembly, 3 s timeouts | §8.3, M2-103 §10 | done |
| Mcoded7; zlib+Mcoded7 (`zlib` feature) | M2-103 §4.3 | done |
| JSON header parser/writer (no alloc), status codes | M2-103 §5 | done |
| ResourceList, DeviceInfo, ChannelList, JSONSchema (raw) | M2-103 §12, M2-105 | done |
| AllCtrlList, ChCtrlList, CtrlMapList | M2-117 | done |
| StateList, State | M2-111 | done |
| ProgramList | M2-107 (not public) | minimal: `title`, `bankPC` |
| Process Inquiry | §9 | done |
| MIDI-CI 1.1 Protocol Negotiation | 1.1 §6 | parse/encode; NAKed (deprecated) |
| Responder engine (no alloc) / Initiator engine (alloc) | | done |
| UMP SysEx7 transport | M2-104 §7.7 | done |
| MIDI 1.0 Identity Request/Reply, M2-115 bit scaling | | done |

## Errata

Each contradiction the crate resolves is pinned by a case with an `erratum`
note; `every_erratum_has_a_case` checks they all exist.

| Erratum | Our reading | Case |
|---|---|---|
| M2-101 §8.2 gives PE Device ID 0x0F; §5.2.1, Tables 30-39 and Appendix D give 0x7F | 0x7F; 0x0F is NAKed unless `lenient_pe_device_id` | `pe-chunks/erratum-pe-device-id-0x0f-rejected`, `pe-chunks/erratum-pe-device-id-0x0f-lenient` |
| M2-101 §5.2.1 numbers the version bits differently from Figure 3 | reserved mask 0x70, minor 0x0F | `management/erratum-version-reserved-bits` |
| M2-101 Appendix D omits Sub-ID#2 0x45 | undefined, NAK 0x01 | `management/erratum-undefined-sub-id-0x45` |
| M2-101 Table 21 calls 0x27 "Profile List Remove" | Profile Removed Report (§7.5) | `profiles/profile-removed-report` |
| M2-101 Table 23 labels the reply's Device ID "Destination" and prints version `02` | ordinary reply envelope | `profiles/profile-details-reply-channel-count` |
| M2-115 Table 9, last row, is not a 16-bit value | §4.2: 127 maps to 65024 | `scaling/erratum-table-9-last-row` |
| M2-103 §12.2 spells `"MCoded7"` | encodings match ignoring ASCII case | `pe-resources/erratum-resource-list-mcoded7-spelling`, `pe-headers/erratum-mcoded7-spelling-in-header` |
| M2-117 Table 26 writes `"requireId"` | accepted as `requireResId` | `pe-resources/erratum-ctrl-map-list-require-id` |
| M2-117 Table 11 capitalises `"Title"` | accepted as `title` | `pe-resources/erratum-ch-ctrl-list-capital-title` |
| M2-117 Table 20 has an unquoted key | invalid JSON, rejected | `pe-resources/erratum-ctrl-map-list-table-20-unquoted-key` |
| M2-105 §4.3 calls the ChannelList reply "an object" | it is an array (Table 11) | `pe-resources/channel-list-table-11` |
| Mcoded7 bit order (M2-103 §4.3.1) | high bit of byte i in bit 6-i | `mcoded7/erratum-mcoded7-bit-order-short-group` |

Noted without a case, because they need no code decision:

- **M2-103 §6.6.2's example** has unquoted `link` keys.
- **M2-111 §2.4** points StateList's schema `$ref` at `M2-112-S`.
- **M2-117's published JSON Schema** is reported to omit `paramPath`,
  which Table 5 defines. The schema file was not available, so this is
  unverified; we follow the table.

## Fixture format

Every file is `{ "spec", "version", "description", "cases": [...] }`. Each
case has a unique `name`, a `specSection` and a `provenance`, and may have
an `erratum` note.

- **Frame areas** (`management`, `profiles`, `pe-capabilities`, `pe-chunks`,
  `process`, `legacy-1.1`):
  - `hex` is the framed message (`F0 ... F7`) and `ciVersion` its version
    byte.
  - `expected` is
    `{ "kind", "device", "version", "source", "destination", "body" }`, or
    `{ "error": "..." }`. MUIDs are integers: `19088743` is `0x01234567`
    ("A") and `124076833` is `0x07654321` ("B").
  - `"roundTrip": false` skips the re-encode check, for inputs that are not
    canonical, such as appended newer-version fields.
  - `responderReply`, if present, is the single frame the reference
    Responder must send in reply.
- **identity**: `hex`, and `expected` `{ "type": "request" | "reply", ... }`
  or an error.
- **mcoded7**: `decoded` and `encoded` hex, checked both ways and in place;
  error cases give only `encoded`.
- **ump**: `payload` hex (no `F0`/`F7`), `group`, and `packets` as pairs of
  32-bit hex words.
- **scaling**: `function` (a `midici::scaling` function in camelCase),
  `args` and `expected`.
- **pe-headers**: `input` JSON, `expected` properties (absent means not
  present), and an optional `write`, the compact form written back.
- **pe-resources**: `resource`, `input` Property Data, and `expected`.

An `error` matches the `midici::Error` variant name, or its whole `Debug`
text when the expectation contains `(` or `{`.

### The reference Responder

`responderReply` is checked against a `Responder` with:
- MUID `0x07654321` (B);
- identity: manufacturer `7D 00 00`, family 1, model 2, revision
  `00 01 00 00`;
- max SysEx 512, frames of at most 128 bytes;
- Profiles enabled, with none declared;
- PE capabilities `1, 0, 0` and no resources (so ResourceList is `[]`);
- Process Inquiry features `0x01`;
- Product Instance Id `CONF-01`.

A case may set `"responderConfig": { "lenientPeDeviceId": true }`.

## Known failures

`known_failures.txt` lists case ids (`<area>/<name>`) that may fail. It can
only shrink: a listed case that passes, or an id that matches no case, fails
the run.

## Adding a captured case

Frames captured from another implementation are the strongest evidence and
are welcome:

1. Capture the exchange, e.g. from MIDI2.0Workbench, JUCE's
   CapabilityInquiryDemo, ktmidi-ci-tool, midicci, or a hardware device.
   Use a MIDI monitor or the tool's own log.
2. Add a case to the matching area file:
   - `hex` is the frame exactly as captured;
   - `expected` is what it should decode to;
   - `responderReply` is set if you captured the reply to one of our
     frames;
   - `provenance` names the tool and its version, e.g.
     `"captured from MIDI2.0Workbench 1.x.y, macOS, 2026-11-02"`.
3. If the capture disagrees with a derived case, keep both. Open an issue
   quoting the section numbers, not the spec text.
