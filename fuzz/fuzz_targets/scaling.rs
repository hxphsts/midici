//! Scaling never panics, and Min-Center-Max up-then-down is the identity.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::scaling::{
    round_down, scale_down, scale_up, step_to_value, value_to_step, zero_extend_up,
};

fuzz_target!(|input: (u32, u8, u8, u32)| {
    let (v, a, b, n) = input;
    let (a, b) = (u32::from(a % 34), u32::from(b % 34));
    if let Some(up) = scale_up(v, a, b) {
        assert_eq!(scale_down(up, b, a), Some(v));
    }
    let _ = zero_extend_up(v, a, b);
    let _ = round_down(v, a, b);
    if let Some(x) = step_to_value(v, n, a) {
        assert_eq!(value_to_step(x, n, a), Some(v));
    }
});
