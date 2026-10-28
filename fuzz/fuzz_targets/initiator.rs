//! The Initiator never panics on arbitrary input, and everything it sends
//! decodes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::Message;
use midici::engine::initiator::{Config, Initiator};
use midici::identity::{DeviceIdentity, ManufacturerId};
use midici::muid::Muid;
use midici::pe::header::Header;

const ID: DeviceIdentity = DeviceIdentity {
    manufacturer: ManufacturerId::NON_COMMERCIAL,
    family: 1,
    model: 2,
    revision: [0, 1, 0, 0],
};

fuzz_target!(|data: &[u8]| {
    let mut ini = Initiator::new(Config::new(ID), 0x5EED);
    let peer = Muid::new(0x100).unwrap_or(Muid::BROADCAST);
    let _ = ini.discover(0);
    let _ = ini.get(0, peer, &Header::request("DeviceInfo"));
    let _ = ini.subscribe(0, peer, "X-Param", None);
    let _ = ini.endpoint_info(0, peer);
    let dst = ini.muid().to_bytes();
    let src = peer.to_bytes();
    let mut now = 0u64;
    for part in data.split(|&b| b == 0xF7) {
        now += 500;
        let bytes: Vec<u8> = match part.split_first() {
            Some((mode, rest)) if mode & 1 == 1 => {
                let (sub2, rest) = rest.split_first().unwrap_or((&0x35, &[]));
                let mut v = vec![0xF0, 0x7E, 0x7F, 0x0D, sub2 & 0x7F, 0x02];
                v.extend_from_slice(&src);
                v.extend_from_slice(if mode & 2 == 2 { &[0x7F; 4] } else { &dst });
                v.extend(rest.iter().map(|b| b & 0x7F));
                v.push(0xF7);
                v
            }
            _ => part.to_vec(),
        };
        let _ = ini.handle(now, &bytes);
        ini.poll(now);
        while let Some(f) = ini.pop_transmit() {
            Message::decode(&f).expect("initiator output decodes");
        }
        while ini.pop_event().is_some() {}
    }
});
