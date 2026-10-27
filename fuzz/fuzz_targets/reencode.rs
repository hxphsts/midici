//! Whatever decodes must re-encode, and re-decode to the same message.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::Message;
use midici::wire::Framing;

fuzz_target!(|data: &[u8]| {
    let Ok(m) = Message::decode(data) else { return };
    let mut buf = vec![0u8; data.len() + 64];
    let n = m
        .encode(Framing::SysEx, &mut buf)
        .expect("a decoded message re-encodes");
    let again = Message::decode(&buf[..n]).expect("re-encoded bytes decode");
    assert_eq!(again, m);
});
