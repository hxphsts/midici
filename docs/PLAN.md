# midici roadmap and status

Goal: a 1:1, reference-quality Rust implementation of MIDI Capability
Inquiry: M2-101-UM, and its Property Exchange companions M2-103, M2-105,
M2-111 and M2-117. It is `no_std`, needs no allocator, and is sans-io.
**Status: 0.0.1 implements the whole of M2-101 v1.2, and both roles.**

Specification text is copyrighted by the MMA and AMEI. This repo cites
sections (`M2-101 §5.6`) and never copies prose. Download the PDFs into
`specs/` (git-ignored) to read along. When reading table layouts, use
`pdftotext -raw`: `-layout` silently drops literal-value rows such as
`01 00`.

## Sources actually used

| Document | Version read | Notes |
|---|---|---|
| M2-101-UM MIDI-CI | **1.2** (2023-06-15) | 1.2.1 is not on the public AMEI mirror and was not found elsewhere. **Unverified: any 1.2.1 errata.** |
| M2-101-UM MIDI-CI | 1.1 (2020-02-20) | Legacy layouts: v1 NAK, Protocol Negotiation `0x10–0x15`, Has Property Data. |
| M2-102-U Common Rules for Profiles | 1.1 | Profile ID (§2.2); Profile Details target `0x00` (§2.5.1). |
| M2-103-UM Common Rules for PE | 1.1 | Mcoded7 (§4.3.1), Header (§5), status codes (§5.4.1, §10.1.1), pagination (§6.6.2), partial Set (§8), subscriptions (§9), Notify (§10), ResourceList (§12). |
| M2-104-UM UMP and MIDI 2.0 Protocol | 1.1.1 | SysEx7 packets (§7.7, Figure 84, Table 18). |
| M2-105-UM PE Foundational Resources | 1.1.1 | DeviceInfo, ChannelList, JSONSchema. |
| M2-111-UM Get and Set Device State | 1.0 | StateList, State. |
| M2-115-U Bit Scaling and Resolution | 1.0.1 | `scaling`. |
| M2-117-UM PE Controller Resources | 1.0 | AllCtrlList, ChCtrlList, CtrlMapList. |
| M2-107-UM ProgramList | **not public** | Probed on the AMEI mirror and searched; not available. `ProgramEntry` is minimal (`title`, `bankPC`, as used in M2-103/M2-105 examples). **Unverified.** |
| MIDI 1.0 Detailed Spec (Identity Request/Reply) | not available | `identity` follows the widely documented layout. M2-101 §5.5.1 confirms the field sizes. |

## Module map

```text
midici
├── wire        envelope, DeviceId, CiVersion, MessageKind (every Sub-ID#2), Framing
├── muid        28-bit Muid, broadcast/reserved, 7-bit codec, from_random(u32)
├── identity    ManufacturerId, DeviceIdentity, MIDI 1.0 Identity Request/Reply
├── message     Message / Body: one typed variant per message, Raw only for undefined Sub-ID#2
├── mgmt        Discovery(+Reply), Endpoint Info(+Reply), Invalidate MUID, ACK, NAK
├── profile     ProfileId, ProfileList, all §7 messages, ChannelCount (M2-102 §2.5.1)
├── pe          PeCapabilities, PeChunk (all chunked PE messages)
│   ├── chunk     ChunkPlan (split for a max SysEx size), Assembler (caller storage, caps, timeouts)
│   ├── mcoded7   buffer, streaming Encoder/Decoder, in-place decode
│   ├── zlib      zlib+Mcoded7 (feature `zlib`, needs alloc; miniz_oxide)
│   ├── json      no-alloc pull Parser (depth limit, 7-bit only) + streaming JsonWriter
│   ├── header    typed Header Data (request/reply/subscription/pagination/State props)
│   ├── status    PeStatus (reply + Notify codes)
│   ├── sink      Sink, Counter, SliceSink, Window, ArrayBuf
│   └── resource  ResourceList/DeviceInfo/ChannelList/Controller/CtrlMap/Program/State; no-alloc writers, alloc parsers
├── process     §9 Process Inquiry bodies and bitmaps
├── legacy      MIDI-CI 1.1 Protocol Negotiation (deprecated; parse/encode only)
├── ump         SysEx7 packetiser + reassembler (no alloc)
├── scaling     M2-115 Min-Center-Max, zero-extension, stepped values
└── engine
    ├── responder  no-alloc Responder<FRAME, OUT, SET, SUBS> + Device trait
    └── initiator  (alloc) Initiator + Foundation sequence helper
```

## Spec coverage matrix (M2-101 v1.2 Appendix D, plus 1.1-only messages)

"Typed" means encoded and decoded by `message::Body`, with golden frames
and property round-trips. "R" and "I" show what the Responder and
Initiator engines do with the message.

| Sub-ID#2 | Message | Section | Typed | Responder | Initiator |
|---|---|---|---|---|---|
| 0x10–0x15 | Protocol Negotiation (6 messages) | 1.1 §6 | yes (`legacy`) | NAK `0x01` (deprecated) | NAK `0x01` if addressed |
| 0x20 | Profile Inquiry | §7.2 | yes | answers from `Device::profiles` | `profile_inquiry` → `Event::Profiles` |
| 0x21 | Reply to Profile Inquiry | §7.3 | yes | n/a | parsed |
| 0x22 / 0x23 | Set Profile On / Off | §7.8, §7.9 | yes (v1 + v2) | `Device::set_profile` → report or NAK `0x04` | `set_profile` |
| 0x24 / 0x25 | Profile Enabled / Disabled Report | §7.10, §7.11 | yes (v1 + v2) | sent to broadcast | `Event::ProfileState` |
| 0x26 / 0x27 | Profile Added / Removed Report | §7.4, §7.5 | yes | `profile_added` / `profile_removed` | `Event::ProfileListChanged` |
| 0x28 / 0x29 | Profile Details Inquiry / Reply | §7.6, §7.7 | yes | `Device::profile_details` or NAK `0x04` | `profile_details` → `Event::ProfileDetails` |
| 0x2F | Profile Specific Data | §7.12 | yes | `Device::profile_data` | n/a |
| 0x30 / 0x31 | PE Capabilities | §8.5, §8.6 | yes (v1 + v2) | answers | `pe_capabilities` |
| 0x32 / 0x33 | Has Property Data | 1.1 §8 | yes (`PeChunk`) | NAK `0x01` (reserved in 1.2) | n/a |
| 0x34 / 0x35 | Get Property Data / Reply | §8.7, §8.8 | yes | chunked, Mcoded7, ResourceList automatic | `get`, reassembly, decoding |
| 0x36 / 0x37 | Set Property Data / Reply | §8.9, §8.10 | yes | reassembly into `SET` bytes, full/partial checks | `set`, chunked to the peer's size |
| 0x38 / 0x39 | Subscription / Reply | §8.11, §8.12 | yes | start/end, `publish` partial/full/notify/end | `subscribe`/`unsubscribe`, auto-replies |
| 0x3F | Notify | §8.13 | yes | 144/408 abort; sends 408 / 144 | 100 extends, 144/408 fail; sends 408 |
| 0x40 / 0x41 | Process Inquiry Capabilities | §9.2, §9.3 | yes | answers | `process_capabilities` |
| 0x42 / 0x43 | MIDI Message Report Inquiry / Reply | §9.5, §9.6 | yes | `Device::message_report` (intersection) | `message_report` |
| 0x44 | End of MIDI Message Report | §9.8 | yes | `end_message_report` | `Event::MessageReportEnd` |
| 0x70 / 0x71 | Discovery / Reply | §5.5, §5.6 | yes (v1 + v2) | answers; collision rules §5.9.1 | `discover`; duplicate-MUID rule §5.9.2 |
| 0x72 / 0x73 | Endpoint Information | §5.7, §5.8 | yes | Product Instance Id or NAK | `endpoint_info` |
| 0x7D | ACK | §5.10 | yes | n/a | accepted |
| 0x7E | Invalidate MUID | §5.9 | yes | renews / forgets peer | renews / forgets peer |
| 0x7F | NAK | §5.11 | yes (1.1 short + 1.2 full) | sends with typed status | maps to `Failure::Nak` |
| other | undefined | §5.1 | `Body::Raw` | NAK `0x01` | NAK `0x01` if addressed |

Also implemented:
- M2-103: Mcoded7, zlib+Mcoded7, Header Data, status codes, pagination
  header fields, partial Set, subscriptions, Notify.
- Resources: M2-105, M2-111, M2-117, and ProgramList (minimal).
- UMP SysEx7 transport, MIDI 1.0 Identity, M2-115 scaling.

### Deliberately left out of 0.0.1

- **zlib in the no-alloc Responder.** Deflate needs a heap. The Responder
  answers `zlib+Mcoded7` requests with 415; the Initiator decodes zlib
  with the `zlib` feature.
- **Typed JSON Schema documents.** The `JSONSchema` resource is passed
  through: the Responder app writes the schema JSON, and the Initiator
  gets the raw bytes.
- **ProgramList beyond `title` and `bankPC`.** M2-107 is not public.
- **Mode, Channel, LocalOn and ExternalSync resources (M2-106, M2-108–M2-110).**
  Not public, and not part of the foundational sequence. Apps can serve
  and get them generically through `Device::get`/`set` and
  `Initiator::get`/`set`.
- **Stage 3 of a MIDI Message Report** (the MIDI messages themselves,
  §9.7) is the application's job; the engine brackets it.
- **Profile Specific Data chunking over 512 bytes to broadcast** is defined
  per Profile (§7.12), so it is left to the Profile implementation.
- **Responder concurrency.** One incoming Set at a time (others get 343)
  and two concurrent Get transfers. Both bounds are deliberate for
  firmware.

## Verified answers (from the PDFs)

- **Property Exchange Device ID is `0x7F`.** M2-101 v1.2 §8.2 says
  "0x0F (whole Function Block)". I checked the rendered page: the PDF really
  does say 0x0F, so this is not a text-extraction artefact. It contradicts
  §5.2.1, where 0x0F is MIDI channel 16, 0x7F is the Function Block, and PE
  must not use channel or group IDs. It also contradicts every PE message
  table (Tables 30–39: `7F`) and Appendix D (PE rows: `0x7F`). We treat §8.2
  as an erratum: we emit `0x7F`. In the engine phase, receiving `0x0F` on a
  PE message will be accepted only behind a "lenient" option, never by
  default.
- **Message Version/Format byte:** Figure 3 shows `0 | reserved ×3 | minor ×4`,
  so `RESERVED_MASK = 0x70` and minor = `0x0F`. The prose in §5.2.1 says
  "bits 7..5 / bits 4..1", which only matches the figure if bits are
  numbered from one. Reserved bits set → NAK status 0x02 (§5.3, §5.4).
- **Discovery (0x70), Table 6:** manufacturer ×3, family ×2 (LSB first), model
  ×2 (LSB first), software revision ×4, category bitmap ×1, receivable max
  SysEx ×4 (LSB first), then v2 Output Path Id ×1.
- **Reply to Discovery (0x71), Table 8:** the same 16 bytes, then v2 Output
  Path Id ×1 (echoed, §5.6.1) and Function Block ×1 (`0x7F` = none, §5.6.2).
  Framed size is 33 bytes.
- **NAK (0x7F), Table 15:** v1 is the header only. v2 adds original Sub-ID#2
  ×1, status ×1, status data ×1, details ×5, text length ×2 (LSB first) and
  text. Status codes are in Table 16. Details are a Profile ID, or the PE
  stream ID plus a 14-bit chunk number (§5.11.3). At the 128-byte minimum,
  103 bytes of text fit exactly; the test `nak_with_103_bytes_of_text_is_128_bytes`
  checks this.
- **ACK (0x7D), Table 13:** same body as the v2 NAK. New in 1.2, so the
  fields are required at every version.
- **Endpoint Information (0x72/0x73), Tables 9/11:** the inquiry carries
  status ×1. The reply carries status ×1, length ×2 and data. Status 0x00 is
  the Product Instance Id: ASCII 32–126, at most 16 bytes (§5.8.3.1).

## MIDI-CI 1.1 vs 1.2, and our policy

Policy: **accept both, emit 1.2.** Receivers must parse all earlier versions
(§5.4). Senders always use their own version (§5.3). Encoders write v2 fields
only when the envelope's version declares them, so tests can produce 1.1
frames. Both engines always send `CiVersion::CURRENT`; the loopback suite checks 1.1 peers in both directions.

| Area | 1.1 | 1.2 | Handling |
|---|---|---|---|
| Version byte | 0x01 | 0x02 | `CiVersion`; v2 fields gated on `minor() >= 2` |
| Device ID 0x7F | whole MIDI port | Function Block | same byte |
| Device ID 0x7E | reserved | Group | accepted on parse |
| Discovery | n/a | + Output Path Id | decodes with `output_path = 0` on v1 |
| Reply to Discovery | n/a | + Output Path Id, Function Block | v1 → `0`, `None` |
| NAK | header only | + full report | `Nak { report: Option<_> }` |
| ACK, Endpoint Info | absent | new | parsed at any version |
| Invalidate MUID Sub-ID#2 | table typo 0x72 | 0x7E (1.2 §1.3 fix) | 0x7E only; 0x72 is Endpoint Inquiry |
| Protocol Negotiation 0x10–0x15 | defined | deprecated | typed (`legacy`), parse + encode; engines never send them, Responder NAKs `0x01` |
| Has Property Data 0x32/0x33 | defined | reserved | typed (`PeChunk`), Responder NAKs `0x01` |
| Set Profile On/Off, Enabled/Disabled Report | n/a | + 2 bytes (channels / reserved) | v2-gated |
| Profile Added/Removed/Details | absent | new | typed |
| PE Capabilities | n/a | + PE major/minor | v2-gated |
| PE Device ID | channel or port | 0x7F only | see §8.2 erratum above |
| Process Inquiry 0x40–0x44 | absent | new | typed; Responder answers |
| Newer minor versions | n/a | trailing fields allowed | trailing bytes ignored on decode |

## No-alloc design of the Responder

FMX's constraints set the bar: no allocator, and at most 128 bytes per
SysEx it sends.

- **Chunk budget.** A framed PE chunk has 24 bytes of overhead (`F0`,
  13-byte envelope, 9 chunk bytes, `F7`). At 128 bytes that leaves 104 for
  header plus data (`pe::chunk::FRAME_OVERHEAD`). Reply headers are never
  chunked (§8.3.2).
- **Pull-based output.** `handle()` queues short replies in a fixed
  `OUT`-byte frame queue. `poll_transmit()` drains one frame at a time,
  so the caller paces USB.
- **Get replies are never buffered.** The Responder calls `Device::get`
  once into a `Counter` to learn the length and plan the chunks (known
  `numChunks`), then once per chunk into a `Window` sink that keeps only
  that chunk's bytes. Mcoded7 is applied by a streaming `Encoder` in the
  same pipeline. If the length changes mid-transfer, it aborts with
  Notify 144 (M2-103 §10.2).
- **Sets** reassemble into an `ArrayBuf<SET>`. Mcoded7 is decoded in place.
- **JSON** goes through the borrowed pull `Parser` and the streaming
  `JsonWriter`. There is no DOM, and nothing beyond `core`.
- **The proof:** `cargo build --no-default-features --target
  riscv32imc-unknown-none-elf`, and the `examples/no_std_responder`
  crate built for the same target.

## Test strategy

1. **Golden frames** (`tests/golden.rs`). Every message type, with
   expected bytes derived by hand from the spec tables and cited by
   section and table. Checked in both framings and in v1/v2 forms.
2. **Property round-trips and never-panic properties**
   (`tests/properties.rs`). Covers every body, Mcoded7, UMP, the JSON
   parser, Header Data, the chunk assembler and the resource parsers.
3. **Loopback conformance** (`tests/loopback.rs`): Initiator ↔ Responder.
   - The full foundational sequence at 128-byte frames.
   - Subscription start, partial, full, notify and end, from both sides.
   - Chunked Mcoded7 State Get and Set.
   - MUID collisions: Responder options A and B, Initiator duplicate
     replies.
   - Timeouts in both directions, with Notify 408.
   - NAK paths (`0x01`, `0x02`, `0x04`) and PE statuses 400, 404, 405
     and 415.
   - v1.1 peers in both directions; UMP transport; FMX SysEx coexistence.
4. **cargo-fuzz** (`fuzz/`). Targets for every parser and for both engines.
5. **Spec numeric tables.** M2-115 Tables 5–10 as unit tests.
6. **Interop (after 0.0.1, needs tools and hardware).** Capture and replay
   against MIDI2.0Workbench, JUCE CapabilityInquiryDemo, ktmidi-ci-tool,
   midicci, and an FM-1 running FMX. Captured frames will become golden
   fixtures with provenance.

## What comes after 0.0.1

| Item | Notes |
|---|---|
| Interop captures against external implementations | Not done in this session: no tools or hardware attached. |
| More M2-10x resources | When their specs are public. |
| Owned/`serde` helpers for building and diffing partial Sets | JSON Pointer apply on the Initiator side. |
| Per-Group CI on UMP | Today one Responder per Function Block; a `SysEx7Assembler` per Group. |
| MSRV CI job | `rust-version = 1.94` is declared; the 1.94 toolchain was not installed here, so it is unverified. |

## Spec ambiguities and errata log

1. **M2-101 §8.2: PE Device ID "0x0F".** It contradicts §5.2.1, Tables
   30–39 and Appendix D, which all give `0x7F`. We emit `0x7F` and accept
   `0x0F` only with `Config::lenient_pe_device_id`.
2. **M2-101 §5.2.1: version bit numbering.** The prose says
   "bits 7..5 / 4..1", which disagrees with Figure 3 (3 reserved bits, 4
   minor bits) unless bits are numbered from one. We follow the figure.
3. **M2-101 Appendix D omits `0x45`.** The list jumps from 0x44 to
   "0x46-4F Reserved". We treat 0x45 as undefined.
4. **M2-101 Table 21** names `0x27` "Profile List Remove". Section §7.5
   calls it the Profile Removed Report.
5. **M2-101 Table 23** labels the Reply to Profile Details' Device ID
   "Destination", and prints the version byte as a literal `02`.
6. **M2-115 Table 9, last row.** 127 → 1019392 (0xF8E00) is not a 16-bit
   value and contradicts §4.2's own example (127 → 65024). We follow §4.2.
7. **M2-103 §12.2 example** spells the encoding `"MCoded7"`; §4.3
   defines `"Mcoded7"`. Encodings are matched ignoring ASCII case.
8. **M2-103 §6.6.2 example** has unquoted `link` keys, which is invalid
   JSON.
9. **M2-117 Table 26** uses `"requireId"` instead of `"requireResId"`. The
   parser accepts both.
10. **M2-117 Table 11** capitalises `"Title"`. The parser accepts both.
11. **M2-117 Table 20** has an unquoted `title` key (invalid JSON).
12. **M2-117 JSON Schema** (published separately) is reported to omit
    `paramPath`, which Table 5 defines. We follow the table. *The schema
    file itself was not available here; unverified.*
13. **M2-111 §2.4** points StateList's schema `$ref` at `M2-112-S`.
14. **M2-105 §4.3** says the Responder "shall return an object", but
    ChannelList is a List Resource whose Table 11 example is an array.
    We parse an array.
15. **Mcoded7 bit order** (M2-103 §4.3.1). The sign bit of byte *i* goes
    in bit 6−i, MSB first. Several manufacturer formats, FMX's own
    protocol among them, use the opposite order. This is easy to get
    wrong.
16. **NAK/ACK text** (§5.10.4): JSON `\u` escapes for non-ASCII. Text is
    kept verbatim.
17. **Discovery `max_sysex` below 128** (§5.5.3) is accepted. The engines
    use at least 128 when sizing frames.
18. **MIDI-CI 1.2.1** is not public. Anything it changed is unverified.
