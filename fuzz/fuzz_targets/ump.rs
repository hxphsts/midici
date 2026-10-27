//! UMP SysEx7: packets→assembler is the identity on 7-bit payloads;
//! arbitrary packets never panic the assembler.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::ump::{SysEx7Assembler, packets};

fuzz_target!(|data: &[u8]| {
    let payload: Vec<u8> = data.iter().map(|b| b & 0x7F).collect();
    let group = u8::try_from(data.len() % 16).unwrap_or(0);
    let mut asm = SysEx7Assembler::<2048>::new();
    let mut got = None;
    for p in packets(&payload, group) {
        if let Some((g, d)) = asm.push(p) {
            got = Some((g, d.to_vec()));
        }
    }
    if payload.len() <= 2048 {
        assert_eq!(got, Some((group, payload)));
    }
    let mut asm = SysEx7Assembler::<64>::new();
    for w in data.chunks_exact(8) {
        let p = [
            u32::from_be_bytes([w[0], w[1], w[2], w[3]]),
            u32::from_be_bytes([w[4], w[5], w[6], w[7]]),
        ];
        if let Some((g, d)) = asm.push(p) {
            assert!(g < 16 && d.len() <= 64);
        }
    }
});
