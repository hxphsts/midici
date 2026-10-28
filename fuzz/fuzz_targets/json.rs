//! JSON pull parser never panics; a string document round-trips through
//! the writer.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::pe::json::{Event, JsonWriter, Parser};

fuzz_target!(|data: &[u8]| {
    let mut p = Parser::new(data);
    let mut first = None;
    let mut count = 0usize;
    for _ in 0..100_000 {
        match p.next() {
            Ok(Some(ev)) => {
                if count == 0 {
                    first = Some(ev);
                }
                count += 1;
                if let Event::Str(s) | Event::Key(s) = ev {
                    let _ = s.chars().count();
                    let mut buf = [0u8; 64];
                    let _ = s.unescape_into(&mut buf);
                }
                if let Event::Number(n) = ev {
                    let _ = (n.as_u64(), n.as_i64(), n.as_f64());
                }
            }
            Ok(None) => {
                if let (1, Some(Event::Str(s))) = (count, first) {
                    let mut w = JsonWriter::new(Vec::new());
                    w.json_str(s);
                    let out = w.finish().expect("one string is a complete document");
                    let mut q = Parser::new(&out);
                    match q.next() {
                        Ok(Some(Event::Str(t))) => assert!(t.chars().eq(s.chars())),
                        other => panic!("re-parse: {other:?}"),
                    }
                }
                break;
            }
            Err(_) => break,
        }
    }
});
