//! Any bytes: the Identity Request/Reply decoders must return, never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::identity::{IdentityReply, IdentityRequest};

fuzz_target!(|data: &[u8]| {
    let _ = IdentityRequest::decode(data);
    let _ = IdentityReply::decode(data);
});
