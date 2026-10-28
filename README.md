# midici

MIDI Capability Inquiry (MIDI-CI) for Rust: find devices, answer as a device, and read and write their properties, with a `no_std` core that never allocates.

[![Crates.io](https://img.shields.io/crates/v/midici.svg)](https://crates.io/crates/midici) [![Documentation](https://docs.rs/midici/badge.svg)](https://docs.rs/midici) [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

## What is MIDI-CI?

MIDI-CI is the MIDI 2.0 specification for capability inquiry: Universal SysEx messages that let devices discover each other, switch Profiles and read and write Property Exchange properties. midici implements both the device side (the Responder) and the host side (the Initiator).

## Quick start

A device that answers Discovery, never sending more than 128 bytes per SysEx:

```rust
use midici::engine::responder::{Config, Device, Responder};
use midici::identity::{DeviceIdentity, ManufacturerId};

struct Synth;
impl Device for Synth {
    fn random(&mut self) -> u32 { 0x2468_ACE0 } // use a hardware RNG
}

let identity = DeviceIdentity {
    manufacturer: ManufacturerId::NON_COMMERCIAL, family: 1, model: 1, revision: [0, 0, 1, 0],
};
let mut ci: Responder<'_, 128> = Responder::new(Config::new(identity), 0x1357_9BDF);

// For every SysEx received from the transport:
let incoming: &[u8] = &[0xF0, 0x7E, 0x7F, 0x0D, 0x70, 0x02, 1, 1, 1, 1, 0x7F, 0x7F, 0x7F, 0x7F,
    0x7D, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0xF7]; // a Discovery
ci.handle(&mut Synth, 0, incoming);

// From the main loop: one outgoing frame at a time.
let mut frame = [0u8; 128];
while let Some(n) = ci.poll_transmit(&Synth, &mut frame) {
    let reply = &frame[..n]; // a Reply to Discovery: send it
    assert_eq!(reply[4], 0x71);
}
```

Implement more of the `Device` trait to add Profiles, Property Exchange resources and Process Inquiry. For the host side, `cargo run --example initiator` discovers the demo device and runs the whole foundational sequence (capabilities, ResourceList, DeviceInfo, ChannelList, controllers, programs), then subscribes to a property and sets it. [`examples/no_std_responder`](examples/no_std_responder) shows a firmware embedding that shares the port with another SysEx protocol.

## Features

- Every MIDI-CI 1.2 message typed, decoded from borrowed bytes and encoded into caller buffers, plus the 1.1 forms for older peers
- Sans-io engines: a no-alloc `Responder` with const-generic capacities, and an `Initiator` with request IDs, timeouts and subscriptions
- Property Exchange in full: chunking for any SysEx size, Mcoded7, zlib, a no-alloc JSON header codec, typed foundational resources
- SysEx7 Universal MIDI Packets, MIDI 1.0 Identity Request/Reply, and MIDI 2.0 bit scaling
- No unsafe code, no panics on input, fuzzed parsers; MSRV 1.94

## no_std and allocation

| Feature | Default | Adds |
|---|---|---|
| (none) | | Codec, PE layer, JSON, UMP, scaling and the **Responder**. `core` only. |
| `alloc` | | **Initiator**, `Foundation`, resource parsers into owned types, `Message::to_vec` |
| `std` | yes | implies `alloc` |
| `serde` | | `Serialize`/`Deserialize` for plain-data and resource types |
| `zlib` | | `zlib+Mcoded7` bodies (implies `alloc`; uses `miniz_oxide`) |

## Specification

midici implements MIDI-CI M2-101-UM v1.2 (and accepts v1.1 peers), with Common Rules for Property Exchange M2-103-UM v1.1, the resources of M2-105, M2-111 and M2-117, Profiles M2-102-U v1.1, UMP SysEx7 from M2-104-UM v1.1.1 and bit scaling M2-115-U v1.0.1. Version 1.2.1 is not public and was not verified.

One erratum matters in practice: M2-101 §8.2 says Property Exchange uses Device ID `0x0F`, but every PE table says `0x7F`, which is what midici sends.

The conformance suite in [`tests/conformance/`](tests/conformance/) holds a case for every message and every erratum, derived from the spec tables. Code comments cite sections such as `M2-101 §5.6`; no specification text is reproduced.

## Documentation

<https://docs.rs/midici>

## License

MIT OR Apache-2.0
