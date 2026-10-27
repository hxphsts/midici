//! Mcoded7: decoders never panic; encode→decode is the identity.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::pe::mcoded7;
use midici::pe::sink::SliceSink;

fuzz_target!(|data: &[u8]| {
    let mut out = vec![0u8; data.len() + 8];
    let buffered = mcoded7::decode(data, &mut out);
    let mut copy = data.to_vec();
    let in_place = mcoded7::decode_in_place(&mut copy);
    assert_eq!(buffered.is_ok(), in_place.is_ok());
    if let (Ok(a), Ok(b)) = (buffered, in_place) {
        assert_eq!(out[..a], copy[..b]);
    }
    // Streaming, split at a data-chosen point.
    let split = data
        .first()
        .map_or(0, |&b| usize::from(b) % (data.len() + 1));
    let mut sbuf = vec![0u8; data.len() + 8];
    let mut sink = SliceSink::new(&mut sbuf);
    let mut d = mcoded7::Decoder::new();
    let streamed = d
        .feed(&data[..split], &mut sink)
        .and_then(|()| d.feed(&data[split..], &mut sink))
        .and_then(|()| d.finish());
    assert_eq!(streamed.is_ok(), buffered.is_ok());
    // Identity.
    let enc = mcoded7::encode_vec(data);
    assert!(enc.iter().all(|b| b & 0x80 == 0));
    assert_eq!(
        mcoded7::decode_vec(&enc).expect("own encoding decodes"),
        data
    );
});
