//! Any bytes: the CI decoders must return, never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::Message;
use midici::wire::Frame;

fuzz_target!(|data: &[u8]| {
    let _ = Frame::parse(data);
    let _ = Message::decode(data);
});
