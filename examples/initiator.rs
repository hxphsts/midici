//! A std Initiator running the whole foundational sequence (Discovery →
//! PE Capabilities → ResourceList → DeviceInfo → ChannelList → AllCtrlList
//! → CtrlMapList → ProgramList) against the demo Responder over an
//! in-process loopback, with 128-byte frames from the device.
//!
//! ```sh
//! cargo run --example initiator
//! ```

// Example code: panicking on setup errors keeps it short.
#![allow(clippy::expect_used, clippy::indexing_slicing)]

#[path = "shared/demo_synth.rs"]
mod demo_synth;

use midici::engine::initiator::{Config, Event, Foundation, Initiator};
use midici::engine::responder::{Responder, Update};
use midici::identity::{DeviceIdentity, ManufacturerId};
use midici::pe::header::Header;

fn main() {
    let host = DeviceIdentity {
        manufacturer: ManufacturerId::NON_COMMERCIAL,
        family: 2,
        model: 1,
        revision: [0, 0, 1, 0],
    };
    let mut ini = Initiator::new(Config::new(host), 0x1357_9BDF);
    let mut synth = demo_synth::DemoSynth::new();
    let mut device: Responder<'static, 128> = Responder::new(demo_synth::config(), 0x2468_ACE0);

    // The "transport": move frames both ways until quiet.
    let pump = |ini: &mut Initiator,
                device: &mut Responder<'static, 128>,
                synth: &mut demo_synth::DemoSynth,
                now: u64| {
        let mut events = Vec::new();
        let mut buf = [0u8; 128];
        loop {
            let mut moved = false;
            while let Some(n) = device.poll_transmit(synth, &mut buf) {
                ini.handle(now, &buf[..n]);
                moved = true;
            }
            while let Some(f) = ini.pop_transmit() {
                device.handle(synth, now, &f);
                moved = true;
            }
            if !moved {
                break;
            }
        }
        while let Some(e) = ini.pop_event() {
            events.push(e);
        }
        events
    };

    ini.discover(0).expect("discover");
    let mut now = 0;
    let peer = pump(&mut ini, &mut device, &mut synth, now)
        .into_iter()
        .find_map(|e| match e {
            Event::Discovered(p) => Some(p),
            _ => None,
        })
        .expect("a responder");
    println!("found {} ({:?})", peer.muid, peer.info.categories);

    let mut seq = Foundation::start(&mut ini, now, peer.muid).expect("start");
    while !seq.is_done() {
        now += 1;
        for e in pump(&mut ini, &mut device, &mut synth, now) {
            seq.on_event(&mut ini, now, &e);
        }
    }
    let report = seq.into_report();
    println!(
        "resources: {:?}",
        report
            .resources
            .iter()
            .map(|r| &r.resource)
            .collect::<Vec<_>>()
    );
    if let Some(d) = &report.device_info {
        println!("device: {} {} {}", d.manufacturer, d.model, d.version);
    }
    for c in report.controllers.iter().flatten() {
        println!(
            "control: {} ({:?} {:?})",
            c.title, c.ctrl_type, c.ctrl_index
        );
    }
    for (id, map) in &report.ctrl_maps {
        println!(
            "map {id}: {:?}",
            map.iter().map(|m| &m.title).collect::<Vec<_>>()
        );
    }
    for p in report.programs.iter().flatten() {
        println!("program: {} {:?}", p.title, p.bank_pc);
    }

    // Subscribe, change the value on the device, receive the update.
    ini.subscribe(now, peer.muid, "X-Cutoff", None)
        .expect("subscribe");
    pump(&mut ini, &mut device, &mut synth, now);
    synth.cutoff = 99;
    device.publish(&synth, "X-Cutoff", Update::Full);
    for e in pump(&mut ini, &mut device, &mut synth, now) {
        if let Event::SubscriptionUpdate { command, data, .. } = e {
            println!("update {command:?}: {}", String::from_utf8_lossy(&data));
        }
    }
    // And set it from the host.
    let set = Header {
        set_partial: true,
        ..Header::request("X-Cutoff")
    };
    ini.set(now, peer.muid, &set, br#"{"/value":12}"#)
        .expect("set");
    pump(&mut ini, &mut device, &mut synth, now);
    println!("cutoff now {}", synth.cutoff);
}
