//! A std Responder driven by a callback transport.
//!
//! Reads SysEx messages as hex lines on stdin (`F0 7E 7F 0D 70 …`) and
//! prints reply frames as hex on stdout. With `--demo`, it plays a scripted
//! Initiator (Discovery, then Get DeviceInfo) through the same callback.
//!
//! ```sh
//! cargo run --example responder -- --demo
//! ```

// Example code: panicking on setup errors keeps it short.
#![allow(clippy::expect_used, clippy::indexing_slicing)]

#[path = "shared/demo_synth.rs"]
mod demo_synth;

use std::io::BufRead;

use midici::engine::Handled;
use midici::engine::responder::Responder;
use midici::mgmt::{Categories, Discovery};
use midici::muid::Muid;
use midici::pe::PeChunk;
use midici::pe::header::Header;
use midici::wire::{Envelope, Framing};
use midici::{Body, Message};

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_hex(line: &str) -> Option<Vec<u8>> {
    line.split_whitespace()
        .map(|t| u8::from_str_radix(t, 16).ok())
        .collect()
}

/// Feeds one message and sends every resulting frame through `send`.
fn exchange(
    responder: &mut Responder<'static, 128>,
    synth: &mut demo_synth::DemoSynth,
    now_ms: u64,
    bytes: &[u8],
    send: &mut impl FnMut(&[u8]),
) -> Handled {
    let handled = responder.handle(synth, now_ms, bytes);
    let mut buf = [0u8; 128];
    while let Some(n) = responder.poll_transmit(synth, &mut buf) {
        send(&buf[..n]);
    }
    handled
}

fn main() {
    let mut synth = demo_synth::DemoSynth::new();
    let mut responder: Responder<'static, 128> = Responder::new(demo_synth::config(), 0xC0FF_EE00);
    let mut print = |frame: &[u8]| println!("{}", hex(frame));
    eprintln!("responder MUID {}", responder.muid());

    if std::env::args().any(|a| a == "--demo") {
        let me = Muid::new(0x0101_0101).expect("valid MUID");
        let discovery = Discovery::new(demo_synth::IDENTITY, Categories::PROPERTY_EXCHANGE, 512);
        let d = Message::new(Envelope::broadcast(me), Body::Discovery(discovery))
            .to_vec(Framing::SysEx)
            .expect("encode");
        eprintln!("> Discovery");
        exchange(&mut responder, &mut synth, 0, &d, &mut print);
        let header = Header::request("DeviceInfo").to_vec();
        let get = Message::new(
            Envelope::new(me, responder.muid()),
            Body::GetPropertyData(PeChunk::single(1, &header, &[])),
        )
        .to_vec(Framing::SysEx)
        .expect("encode");
        eprintln!("> Get DeviceInfo");
        exchange(&mut responder, &mut synth, 10, &get, &mut print);
        return;
    }

    let start = std::time::Instant::now();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Some(bytes) = parse_hex(&line) else {
            eprintln!("not hex: {line}");
            continue;
        };
        let now = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let handled = exchange(&mut responder, &mut synth, now, &bytes, &mut print);
        eprintln!("{handled:?}");
        responder.poll(now);
    }
}
