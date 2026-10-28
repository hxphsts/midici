//! PE Header Data: parse never panics; whatever parses, re-writes and
//! re-parses to the same header.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::pe::header::Header;

fuzz_target!(|data: &[u8]| {
    let Ok(h) = Header::parse(data) else { return };
    let bytes = h.to_vec();
    let again = Header::parse(&bytes).expect("written header re-parses");
    assert_eq!(again, h);
});
