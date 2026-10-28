# midici

MIDI Capability Inquiry (MIDI-CI) for Rust: both roles, sans-io,
`#![no_std]`, and no allocator required.

[![Crates.io](https://img.shields.io/crates/v/midici.svg)](https://crates.io/crates/midici) [![Documentation](https://docs.rs/midici/badge.svg)](https://docs.rs/midici) [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

`midici` implements the MMA/AMEI MIDI-CI specification (M2-101-UM v1.2)
and its Property Exchange companions. In practice:
- **Find devices.** Discovery, Endpoint Information, MUID management.
- **Answer as a device.** A no-alloc `Responder` that firmware embeds,
  sending at most 128-byte SysEx frames if asked.
- **Read and write properties.** Property Exchange Get, Set and Subscribe,
  with chunking, Mcoded7, zlib and typed foundational resources. An
  `Initiator` (host side) runs the whole foundational sequence for you.
- **Profiles and Process Inquiry.** Every message is typed.

It does no I/O and reads no clocks. You feed it bytes and a millisecond
timestamp, then drain frames and events. Parsers borrow from your input,
return typed errors, never panic, and are fuzzed.

## Status (0.0.1)

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

The full coverage matrix, design notes and errata are in
[`docs/SPEC.md`](docs/SPEC.md).

## Quick start: answer as a device (no_std, no alloc)

```rust
use midici::engine::responder::{Config, Device, GetMeta, Responder};
use midici::identity::{DeviceIdentity, ManufacturerId};
use midici::pe::{PeCapabilities, header::Header, json::JsonWriter, sink::Sink, status::PeStatus};
use midici::pe::resource::{DeviceInfo, ResourceDecl, names};

struct Synth;
static RESOURCES: [ResourceDecl<'static>; 1] = [ResourceDecl::new(names::DEVICE_INFO)];

impl Device for Synth {
    fn random(&mut self) -> u32 { 0x1234_5678 /* use a hardware RNG */ }
    fn resources(&self) -> &[ResourceDecl<'_>] { &RESOURCES }
    fn get(&self, req: &Header<'_>, out: &mut dyn Sink) -> Result<GetMeta<'_>, PeStatus> {
        if !req.is_resource(names::DEVICE_INFO) { return Err(PeStatus::NOT_FOUND); }
        let mut w = JsonWriter::new(out);
        DeviceInfo {
            manufacturer_id: [0x7D, 0, 0], family_id: [1, 0], model_id: [1, 0],
            version_id: [0, 0, 1, 0], manufacturer: "Educational Use", family: "Demo",
            model: "Synth", version: "1.0", serial_number: None,
        }.write(&mut w);
        w.finish().map_err(|_| PeStatus::INTERNAL_ERROR)?;
        Ok(GetMeta::default())
    }
}

let identity = DeviceIdentity {
    manufacturer: ManufacturerId::NON_COMMERCIAL, family: 1, model: 1, revision: [0, 0, 1, 0],
};
let config = Config {
    property_exchange: Some(PeCapabilities::default()),
    max_frame: 128, // never send a frame larger than 128 bytes
    ..Config::new(identity)
};
// 128-byte frames, 512 bytes of output queue, 256-byte Sets, 2 subscriptions.
let mut ci: Responder<'_, 128, 512, 256, 2> = Responder::new(config, 0xC0FFEE);
let mut synth = Synth;

// For every SysEx received (with or without F0/F7):
let incoming: &[u8] = &[0xF0, 0x7D, b'F', b'X', 1, 1, 0xF7]; // e.g. another protocol
let _ = ci.handle(&mut synth, /* now_ms */ 0, incoming);   // → Handled::NotCi
// From the main loop, one frame at a time:
let mut frame = [0u8; 128];
while let Some(n) = ci.poll_transmit(&synth, &mut frame) {
    let _send = &frame[..n];
}
```

[`examples/no_std_responder`](examples/no_std_responder) is a complete
firmware-style skeleton (static capacities, the 128-byte limit, sharing
the port with `F0 7D 'F' 'X'` frames). It builds for
`riscv32imc-unknown-none-elf`.

## Quick start: talk to devices (host, `alloc`)

```rust,no_run
use midici::engine::initiator::{Config, Event, Foundation, Initiator};
use midici::identity::{DeviceIdentity, ManufacturerId};

// Your transport and clock.
fn send(_frame: &[u8]) {}
fn receive() -> Vec<Vec<u8>> { Vec::new() }
fn now_ms() -> u64 { 0 }

fn main() -> Result<(), midici::Error> {
    let identity = DeviceIdentity {
        manufacturer: ManufacturerId::NON_COMMERCIAL, family: 2, model: 1, revision: [0; 4],
    };
    let mut ini = Initiator::new(Config::new(identity), /* random seed */ 42);
    ini.discover(now_ms())?;
    let mut sequence = None;
    loop {
        for frame in receive() {
            ini.handle(now_ms(), &frame);
        }
        ini.poll(now_ms());
        while let Some(frame) = ini.pop_transmit() {
            send(&frame);
        }
        while let Some(event) = ini.pop_event() {
            if let (None, Event::Discovered(peer)) = (&sequence, &event) {
                sequence = Some(Foundation::start(&mut ini, now_ms(), peer.muid)?);
            } else if let Some(seq) = &mut sequence {
                if seq.on_event(&mut ini, now_ms(), &event) {
                    println!("{:?}", seq.report().device_info);
                    println!("{:?}", seq.report().controllers);
                    return Ok(());
                }
            }
        }
    }
}
```

`cargo run --example initiator` runs that sequence, plus a subscription
and a partial Set, against an in-process demo Responder.
`cargo run --example responder -- --demo` prints the frames the device
sends.

## no_std and allocation

| Feature | Default | Adds |
|---|---|---|
| (none) | | Codec, PE layer, JSON, UMP, scaling and the **Responder**. `core` only. |
| `alloc` | | **Initiator**, `Foundation`, resource parsers into owned types, `Message::to_vec` |
| `std` | yes | implies `alloc` |
| `serde` | | `Serialize`/`Deserialize` for plain-data and resource types |
| `zlib` | | `zlib+Mcoded7` bodies (implies `alloc`; uses `miniz_oxide`) |

`cargo build --no-default-features --target riscv32imc-unknown-none-elf`
is part of the release checks. MSRV is Rust 1.94; edition 2024.

## Specification versions

- **MIDI-CI M2-101-UM v1.2.** We emit Message Version `0x02` and accept
  `0x01`. Version 1.2.1 is not publicly available and was not verified.
- **Common Rules for PE, M2-103-UM v1.1.**
- **PE resources:** M2-105-UM v1.1.1, M2-111-UM v1.0, M2-117-UM v1.0.
- **Profiles:** M2-102-U v1.1.
- **UMP, M2-104-UM v1.1.1** (SysEx7 only).
- **Bit scaling, M2-115-U v1.0.1.**

Code comments cite sections (e.g. `M2-101 §5.6`); no specification text
is reproduced. Get the PDFs from the MIDI Association or AMEI.

## Errata found along the way

- **M2-101 §8.2.** It says PE messages use Device ID `0x0F`, but every PE
  table and Appendix D use `0x7F` (Function Block). `midici` uses `0x7F`.
  `0x0F` is accepted only with `Config::lenient_pe_device_id`.
- **Figure 3** (version byte) and §5.2.1 prose number bits differently.
  Reserved is `0x70`, minor is `0x0F`.
- **Appendix D** skips Sub-ID#2 `0x45`.
- **M2-115 Table 9's last row** contradicts §4.2.
- **M2-103 §12.2** writes `"MCoded7"`. **M2-117 Table 26** uses
  `requireId` and **Table 11** uses `Title`. Parsers accept both
  spellings.
- **Mcoded7 puts the high bit of byte *i* in bit 6−i** (M2-103 §4.3.1),
  the reverse of several manufacturer packings.

The full list is in [`docs/SPEC.md`](docs/SPEC.md).

## Testing

```sh
cargo test --all-features                 # unit, golden, property and loopback tests
cargo build --no-default-features --target riscv32imc-unknown-none-elf
cargo +nightly fuzz run decode_message    # see fuzz/ for all targets
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
