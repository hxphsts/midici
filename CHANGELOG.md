# Changelog

## [0.0.1] - 2026-10-28

First release: all of MIDI-CI 1.2 (M2-101-UM v1.2), both roles, sans-io,
with a `no_std` + no-alloc core. MIDI-CI 1.1 peers are accepted.

### Added

- **Wire format.**
  - `wire`: the envelope with or without `F0`/`F7`, `DeviceId`,
    `CiVersion` (reserved-bit detection), and `MessageKind` for every
    Sub-ID#2.
  - `muid`: 28-bit MUIDs and RNG-free generation.
  - `message`: a typed `Body` for every message; `Raw` is used only for
    undefined Sub-ID#2.
- **Management** (`mgmt`): Discovery, Endpoint Information, Invalidate
  MUID, ACK, and NAK in the 1.1 short form and the 1.2 full form.
- **Profiles** (`profile`): all §7 messages, Profile IDs, and the M2-102
  channel-count details target.
- **Property Exchange** (`pe`):
  - capabilities (v1 and v2) and the chunked PE body for all messages;
  - Mcoded7: buffer, streaming and in-place;
  - zlib+Mcoded7 (`zlib` feature);
  - a no-alloc JSON pull parser and streaming writer;
  - typed Header Data and PE status codes;
  - chunk planning and reassembly with caps and timeouts;
  - typed resources: ResourceList, DeviceInfo, ChannelList, AllCtrlList,
    ChCtrlList, CtrlMapList, StateList, and a minimal ProgramList.
    Responders write them with no allocator; Initiators parse them under
    `alloc`.
- **Process Inquiry** (`process`): capabilities, MIDI Message Report
  inquiry, reply and end.
- **Legacy** (`legacy`): MIDI-CI 1.1 Protocol Negotiation, parse and
  encode only.
- **Engines.**
  - `engine::responder::Responder`: no alloc, capacities as const
    generics. Handles the MUID lifecycle and collisions, every reply,
    chunked and Mcoded7 Get without buffering, Set, subscriptions, Notify
    and NAKs.
  - `engine::initiator::Initiator` (`alloc`): discovery, request IDs,
    reassembly, timeouts with Notify 408, subscriptions, Profiles, Process
    Inquiry.
  - `Foundation` runs the foundational resource sequence.
- **Transports and utilities.**
  - `ump`: SysEx7 packetiser and reassembler.
  - `identity`: MIDI 1.0 Identity Request/Reply.
  - `scaling`: M2-115 bit scaling.
- **An API built to grow with the spec.** Message bodies, engine
  configuration, PE headers and resource types are `#[non_exhaustive]`:
  build them with `new(..)` and `with_*` setters, and read their public
  fields. Enums that future versions may extend (`Handled`, `Failure`,
  `Encoding`, `CtrlType`, ...) are `#[non_exhaustive]` too. Wire newtypes
  (`NakStatus`, `ProfileId`, `Details`, ...) validate 7-bit values in
  `new` and expose them through `value()` / `bytes()`. Two-state choices
  use enums (`ProfileState`, `SubscriptionChange`) instead of `bool`.
- **Tests and examples.**
  - A spec conformance suite (`tests/conformance/`) with a case for every
    Sub-ID#2 and every erratum found, derived from the spec tables.
  - Golden frames for every message, property tests, an
    Initiator↔Responder loopback suite, and cargo-fuzz targets.
  - Examples: a std `responder`, a std `initiator`, and the
    `no_std_responder` firmware skeleton.
