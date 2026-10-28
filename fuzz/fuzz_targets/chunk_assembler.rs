//! Chunk reassembly over byte-driven sequences of PE chunks never panics
//! and never exceeds its limit.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::pe::PeChunk;
use midici::pe::chunk::{Assembler, ChunkPlan};
use midici::pe::sink::ArrayBuf;

fuzz_target!(|data: &[u8]| {
    let mut a = Assembler::new(ArrayBuf::<1024>::new(), 900);
    let mut rest = data;
    let mut now = 0u64;
    while let [rid, num, this, hlen, dlen, dt, tail @ ..] = rest {
        let hl = usize::from(*hlen & 0x3F).min(tail.len());
        let (header, tail) = tail.split_at(hl);
        let dl = usize::from(*dlen).min(tail.len());
        let (body, tail) = tail.split_at(dl);
        rest = tail;
        now += u64::from(*dt) * 20;
        let c = PeChunk::new(
            rid & 0x03,
            header,
            u16::from(num & 0x07),
            u16::from(this & 0x07),
            body,
        );
        let _ = a.push(&c, now);
        assert!(a.header().len() + a.data().len() <= 900);
        let _ = a.expired(now, 3000);
    }
    // Planning with arbitrary sizes is total too.
    if let [m0, m1, h, t0, t1, ..] = data {
        let max = u32::from(u16::from_le_bytes([*m0, *m1]));
        let total = usize::from(u16::from_le_bytes([*t0, *t1]));
        if let Ok(plan) = ChunkPlan::new(max, usize::from(*h), total) {
            let mut sum = 0;
            for n in 1..=plan.num_chunks() {
                sum += plan.range(n).len();
            }
            assert_eq!(sum, total);
        }
    }
});
