//! A small JSON pull parser and streaming writer, no allocator required.
//!
//! Property Exchange JSON is 7-bit ASCII: non-ASCII characters travel as
//! `\uXXXX` escapes (M2-103 §4.1.1). The [`Parser`] therefore rejects bytes
//! `>= 0x80`, borrows every string and number from the input, and enforces a
//! nesting limit. The [`JsonWriter`] escapes non-ASCII output the same way.

use core::fmt;

use crate::error::Error;
use crate::pe::sink::Sink;

/// Maximum nesting depth the parser and writer accept.
pub const MAX_DEPTH: usize = 32;

/// A JSON string as it appears in the input (escapes not yet resolved), or
/// a plain string supplied by the caller.
#[derive(Clone, Copy)]
pub struct JsonStr<'a> {
    raw: &'a str,
    escaped: bool,
}

impl<'a> JsonStr<'a> {
    /// A caller string; escaped when written.
    #[must_use]
    pub const fn plain(s: &'a str) -> Self {
        Self {
            raw: s,
            escaped: false,
        }
    }

    /// The text between the quotes, escapes unresolved (for an escaped
    /// string) or as given (for a plain one).
    #[must_use]
    pub const fn raw(&self) -> &'a str {
        self.raw
    }

    /// The text if it needs no unescaping: no escape sequences at all.
    #[must_use]
    pub fn as_plain(&self) -> Option<&'a str> {
        (!self.escaped || !self.raw.contains('\\')).then_some(self.raw)
    }

    /// The unescaped characters. Unpaired surrogates become U+FFFD.
    #[must_use]
    pub fn chars(&self) -> Chars<'a> {
        Chars {
            rest: self.raw.as_bytes(),
            escaped: self.escaped,
            plain: self.raw.chars(),
        }
    }

    /// Whether the unescaped text equals `s`.
    #[must_use]
    pub fn eq_str(&self, s: &str) -> bool {
        self.chars().eq(s.chars())
    }

    /// Whether the unescaped text equals `s`, ignoring ASCII case.
    #[must_use]
    pub fn eq_ignore_ascii_case(&self, s: &str) -> bool {
        self.chars()
            .map(|c| c.to_ascii_lowercase())
            .eq(s.chars().map(|c| c.to_ascii_lowercase()))
    }

    /// Unescaped length in UTF-8 bytes.
    #[must_use]
    pub fn unescaped_len(&self) -> usize {
        self.chars().map(char::len_utf8).sum()
    }

    /// Copies the unescaped text into `buf`.
    ///
    /// # Errors
    ///
    /// [`Error::BufferTooSmall`] if it does not fit.
    pub fn unescape_into<'b>(&self, buf: &'b mut [u8]) -> Result<&'b str, Error> {
        let mut n = 0;
        for c in self.chars() {
            let mut tmp = [0u8; 4];
            let s = c.encode_utf8(&mut tmp);
            let end = n + s.len();
            buf.get_mut(n..end)
                .ok_or(Error::BufferTooSmall)?
                .copy_from_slice(s.as_bytes());
            n = end;
        }
        let bytes = buf.get(..n).ok_or(Error::BufferTooSmall)?;
        core::str::from_utf8(bytes).map_err(|_| Error::InvalidField("JSON string"))
    }
}

impl PartialEq for JsonStr<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.chars().eq(other.chars())
    }
}

impl Eq for JsonStr<'_> {}

impl core::hash::Hash for JsonStr<'_> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        for c in self.chars() {
            c.hash(state);
        }
    }
}

impl fmt::Debug for JsonStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"")?;
        for c in self.chars() {
            fmt::Write::write_char(f, c)?;
        }
        f.write_str("\"")
    }
}

impl fmt::Display for JsonStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.chars() {
            fmt::Write::write_char(f, c)?;
        }
        Ok(())
    }
}

/// Iterator over the unescaped characters of a [`JsonStr`].
#[derive(Clone, Debug)]
pub struct Chars<'a> {
    rest: &'a [u8],
    escaped: bool,
    plain: core::str::Chars<'a>,
}

fn hex4(b: &[u8]) -> Option<u16> {
    let mut v = 0u16;
    for &d in b.get(..4)? {
        let n = match d {
            b'0'..=b'9' => d - b'0',
            b'a'..=b'f' => d - b'a' + 10,
            b'A'..=b'F' => d - b'A' + 10,
            _ => return None,
        };
        v = (v << 4) | u16::from(n);
    }
    Some(v)
}

impl Iterator for Chars<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        if !self.escaped {
            return self.plain.next();
        }
        // Escaped strings were validated by the parser: ASCII only, escapes
        // well-formed. Anything else degrades to U+FFFD, never panics.
        let (&b, rest) = self.rest.split_first()?;
        self.rest = rest;
        if b != b'\\' {
            return Some(char::from(b));
        }
        let (&e, rest) = self.rest.split_first()?;
        self.rest = rest;
        Some(match e {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let hi = hex4(self.rest).unwrap_or(0xFFFD);
                self.rest = self.rest.get(4..).unwrap_or_default();
                if (0xD800..0xDC00).contains(&hi) {
                    let lo = match self.rest {
                        [b'\\', b'u', tail @ ..] => hex4(tail),
                        _ => None,
                    };
                    match lo {
                        Some(lo) if (0xDC00..0xE000).contains(&lo) => {
                            self.rest = self.rest.get(6..).unwrap_or_default();
                            let c = 0x1_0000
                                + ((u32::from(hi) - 0xD800) << 10)
                                + (u32::from(lo) - 0xDC00);
                            char::from_u32(c).unwrap_or('\u{FFFD}')
                        }
                        _ => '\u{FFFD}',
                    }
                } else {
                    char::from_u32(u32::from(hi)).unwrap_or('\u{FFFD}')
                }
            }
            _ => '\u{FFFD}',
        })
    }
}

/// A JSON number, borrowed and validated against the JSON grammar.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Number<'a>(&'a str);

impl<'a> Number<'a> {
    /// The literal text.
    #[must_use]
    pub const fn as_str(&self) -> &'a str {
        self.0
    }

    /// As an unsigned integer, if it is one that fits.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        self.0.parse().ok()
    }

    /// As a signed integer, if it is one that fits.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        self.0.parse().ok()
    }

    /// As a `u32`.
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        self.as_u64().and_then(|v| u32::try_from(v).ok())
    }

    /// As a float.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        self.0.parse().ok()
    }
}

impl fmt::Debug for Number<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// One parsing event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event<'a> {
    /// `{`
    ObjectStart,
    /// `}`
    ObjectEnd,
    /// `[`
    ArrayStart,
    /// `]`
    ArrayEnd,
    /// An object key; the next event is its value.
    Key(JsonStr<'a>),
    /// A string value.
    Str(JsonStr<'a>),
    /// A number value.
    Number(Number<'a>),
    /// `true` / `false`.
    Bool(bool),
    /// `null`.
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    Value,
    ValueOrEnd,
    KeyOrEnd,
    Key,
    CommaOrEnd,
    Done,
}

/// A pull parser over one JSON document.
#[derive(Clone, Debug)]
pub struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    /// `true` = object, `false` = array, per level.
    stack: [bool; MAX_DEPTH],
    depth: usize,
    expect: Expect,
}

const fn err(what: &'static str) -> Error {
    Error::InvalidField(what)
}

impl<'a> Parser<'a> {
    /// A parser over `src`.
    #[must_use]
    pub const fn new(src: &'a [u8]) -> Self {
        Self {
            src,
            pos: 0,
            stack: [false; MAX_DEPTH],
            depth: 0,
            expect: Expect::Value,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    const fn after_value(&mut self) {
        self.expect = if self.depth == 0 {
            Expect::Done
        } else {
            Expect::CommaOrEnd
        };
    }

    fn push(&mut self, object: bool) -> Result<(), Error> {
        let slot = self
            .stack
            .get_mut(self.depth)
            .ok_or(err("JSON nesting too deep"))?;
        *slot = object;
        self.depth += 1;
        Ok(())
    }

    fn top_is_object(&self) -> Option<bool> {
        self.depth
            .checked_sub(1)
            .and_then(|i| self.stack.get(i).copied())
    }

    fn pop(&mut self, object: bool) -> Result<(), Error> {
        if self.top_is_object() != Some(object) {
            return Err(err("JSON brackets"));
        }
        self.depth -= 1;
        self.after_value();
        Ok(())
    }

    fn string(&mut self) -> Result<JsonStr<'a>, Error> {
        // At the opening quote.
        self.pos += 1;
        let start = self.pos;
        let mut escaped = false;
        loop {
            let b = self.peek().ok_or(err("unterminated JSON string"))?;
            match b {
                b'"' => break,
                b'\\' => {
                    escaped = true;
                    let e = self
                        .src
                        .get(self.pos + 1)
                        .copied()
                        .ok_or(err("JSON escape"))?;
                    match e {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.pos += 2,
                        b'u' => {
                            let digits = self.src.get(self.pos + 2..).unwrap_or_default();
                            hex4(digits).ok_or(err("JSON \\u escape"))?;
                            self.pos += 6;
                        }
                        _ => return Err(err("JSON escape")),
                    }
                }
                0x00..=0x1F | 0x80..=0xFF => return Err(err("JSON string character")),
                _ => self.pos += 1,
            }
        }
        let raw = self.src.get(start..self.pos).ok_or(err("JSON string"))?;
        self.pos += 1;
        let raw = core::str::from_utf8(raw).map_err(|_| err("JSON string"))?;
        Ok(JsonStr { raw, escaped })
    }

    fn number(&mut self) -> Result<Number<'a>, Error> {
        let start = self.pos;
        let digits = |p: &mut Self| {
            let s = p.pos;
            while matches!(p.peek(), Some(b'0'..=b'9')) {
                p.pos += 1;
            }
            p.pos > s
        };
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        if self.peek() == Some(b'0') {
            self.pos += 1;
        } else if !digits(self) {
            return Err(err("JSON number"));
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !digits(self) {
                return Err(err("JSON number"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !digits(self) {
                return Err(err("JSON number"));
            }
        }
        let s = self.src.get(start..self.pos).ok_or(err("JSON number"))?;
        core::str::from_utf8(s)
            .map(Number)
            .map_err(|_| err("JSON number"))
    }

    fn literal(&mut self, word: &[u8]) -> Result<(), Error> {
        let end = self.pos + word.len();
        if self.src.get(self.pos..end) == Some(word) {
            self.pos = end;
            Ok(())
        } else {
            Err(err("JSON literal"))
        }
    }

    fn value(&mut self, c: u8) -> Result<Event<'a>, Error> {
        Ok(match c {
            b'{' => {
                self.pos += 1;
                self.push(true)?;
                self.expect = Expect::KeyOrEnd;
                return Ok(Event::ObjectStart);
            }
            b'[' => {
                self.pos += 1;
                self.push(false)?;
                self.expect = Expect::ValueOrEnd;
                return Ok(Event::ArrayStart);
            }
            b'"' => {
                let s = self.string()?;
                self.after_value();
                Event::Str(s)
            }
            b't' => {
                self.literal(b"true")?;
                self.after_value();
                Event::Bool(true)
            }
            b'f' => {
                self.literal(b"false")?;
                self.after_value();
                Event::Bool(false)
            }
            b'n' => {
                self.literal(b"null")?;
                self.after_value();
                Event::Null
            }
            b'-' | b'0'..=b'9' => {
                let n = self.number()?;
                self.after_value();
                Event::Number(n)
            }
            _ => return Err(err("JSON value")),
        })
    }

    fn key(&mut self) -> Result<Event<'a>, Error> {
        if self.peek() != Some(b'"') {
            return Err(err("JSON key"));
        }
        let k = self.string()?;
        self.skip_ws();
        if self.peek() != Some(b':') {
            return Err(err("JSON colon"));
        }
        self.pos += 1;
        self.expect = Expect::Value;
        Ok(Event::Key(k))
    }

    /// The next event, or `None` at the end of a complete document.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for malformed JSON, non-ASCII input, or
    /// nesting deeper than [`MAX_DEPTH`].
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Event<'a>>, Error> {
        self.skip_ws();
        let Some(c) = self.peek() else {
            return if self.expect == Expect::Done {
                Ok(None)
            } else {
                Err(err("JSON ends early"))
            };
        };
        let ev = match self.expect {
            Expect::Done => return Err(err("JSON trailing data")),
            Expect::Value => self.value(c)?,
            Expect::ValueOrEnd => {
                if c == b']' {
                    self.pos += 1;
                    self.pop(false)?;
                    Event::ArrayEnd
                } else {
                    self.value(c)?
                }
            }
            Expect::KeyOrEnd => {
                if c == b'}' {
                    self.pos += 1;
                    self.pop(true)?;
                    Event::ObjectEnd
                } else {
                    self.key()?
                }
            }
            Expect::Key => self.key()?,
            Expect::CommaOrEnd => match c {
                b',' => {
                    self.pos += 1;
                    self.expect = if self.top_is_object() == Some(true) {
                        Expect::Key
                    } else {
                        Expect::Value
                    };
                    return self.next();
                }
                b'}' => {
                    self.pos += 1;
                    self.pop(true)?;
                    Event::ObjectEnd
                }
                b']' => {
                    self.pos += 1;
                    self.pop(false)?;
                    Event::ArrayEnd
                }
                _ => return Err(err("JSON separator")),
            },
        };
        Ok(Some(ev))
    }

    /// The next event; end of input is an error.
    ///
    /// # Errors
    ///
    /// As [`Parser::next`], plus [`Error::Truncated`] at the end.
    pub fn expect_event(&mut self) -> Result<Event<'a>, Error> {
        self.next()?.ok_or(Error::Truncated)
    }

    /// Skips the value that starts with `first` (already returned by
    /// [`Parser::next`]); for containers, consumes through the matching end.
    ///
    /// # Errors
    ///
    /// As [`Parser::next`].
    pub fn skip(&mut self, first: Event<'a>) -> Result<(), Error> {
        let mut depth = match first {
            Event::ObjectStart | Event::ArrayStart => 1usize,
            _ => return Ok(()),
        };
        while depth > 0 {
            match self.expect_event()? {
                Event::ObjectStart | Event::ArrayStart => depth += 1,
                Event::ObjectEnd | Event::ArrayEnd => depth -= 1,
                _ => {}
            }
        }
        Ok(())
    }

    /// Validates that `src` is one well-formed JSON document.
    ///
    /// # Errors
    ///
    /// As [`Parser::next`].
    pub fn validate(src: &'a [u8]) -> Result<(), Error> {
        let mut p = Self::new(src);
        while p.next()?.is_some() {}
        Ok(())
    }
}

/// A streaming JSON writer. Methods never fail; structural misuse or depth
/// overflow is reported by [`JsonWriter::finish`].
#[derive(Debug)]
pub struct JsonWriter<S: Sink> {
    out: S,
    /// Bit `d` set = a value has been written at depth `d` (needs a comma).
    has_items: u64,
    depth: usize,
    after_key: bool,
    error: bool,
}

impl<S: Sink> JsonWriter<S> {
    /// A writer into `out`.
    pub const fn new(out: S) -> Self {
        Self {
            out,
            has_items: 0,
            depth: 0,
            after_key: false,
            error: false,
        }
    }

    /// The sink, mutably (e.g. to write raw pre-rendered JSON).
    pub const fn sink(&mut self) -> &mut S {
        &mut self.out
    }

    fn sep(&mut self) {
        if self.after_key {
            self.after_key = false;
            return;
        }
        let bit = 1u64 << self.depth.min(63);
        if self.has_items & bit != 0 {
            self.out.put(b",");
        }
        self.has_items |= bit;
    }

    fn open(&mut self, c: &[u8]) -> &mut Self {
        self.sep();
        self.out.put(c);
        if self.depth >= MAX_DEPTH {
            self.error = true;
        } else {
            self.depth += 1;
            self.has_items &= !(1u64 << self.depth);
        }
        self
    }

    fn close(&mut self, c: &[u8]) -> &mut Self {
        if self.depth == 0 || self.after_key {
            self.error = true;
        } else {
            self.depth -= 1;
        }
        self.out.put(c);
        self
    }

    /// `{`
    pub fn begin_object(&mut self) -> &mut Self {
        self.open(b"{")
    }

    /// `}`
    pub fn end_object(&mut self) -> &mut Self {
        self.close(b"}")
    }

    /// `[`
    pub fn begin_array(&mut self) -> &mut Self {
        self.open(b"[")
    }

    /// `]`
    pub fn end_array(&mut self) -> &mut Self {
        self.close(b"]")
    }

    /// An object key.
    pub fn key(&mut self, k: &str) -> &mut Self {
        self.sep();
        self.escaped(JsonStr::plain(k));
        self.out.put(b":");
        self.after_key = true;
        self
    }

    fn escaped(&mut self, s: JsonStr<'_>) {
        self.out.put(b"\"");
        if s.escaped {
            self.out.put(s.raw.as_bytes());
        } else {
            for c in s.raw.chars() {
                match c {
                    '"' => self.out.put(b"\\\""),
                    '\\' => self.out.put(b"\\\\"),
                    '\n' => self.out.put(b"\\n"),
                    '\r' => self.out.put(b"\\r"),
                    '\t' => self.out.put(b"\\t"),
                    ' '..='~' => {
                        let mut tmp = [0u8; 1];
                        self.out.put(c.encode_utf8(&mut tmp).as_bytes());
                    }
                    _ => {
                        let mut units = [0u16; 2];
                        for u in c.encode_utf16(&mut units) {
                            self.u_escape(*u);
                        }
                    }
                }
            }
        }
        self.out.put(b"\"");
    }

    fn u_escape(&mut self, u: u16) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut b = *b"\\u0000";
        for (i, slot) in b.iter_mut().skip(2).enumerate() {
            let nib = (u >> (12 - 4 * i)) & 0xF;
            *slot = HEX.get(usize::from(nib)).copied().unwrap_or(b'0');
        }
        self.out.put(&b);
    }

    /// A string value (escaped as M2-103 §4.1.1 requires).
    pub fn string(&mut self, s: &str) -> &mut Self {
        self.sep();
        self.escaped(JsonStr::plain(s));
        self
    }

    /// A string value from a [`JsonStr`] (raw escapes are passed through).
    pub fn json_str(&mut self, s: JsonStr<'_>) -> &mut Self {
        self.sep();
        self.escaped(s);
        self
    }

    /// An unsigned number.
    pub fn uint(&mut self, v: u64) -> &mut Self {
        self.sep();
        let mut buf = [0u8; 20];
        let mut i = buf.len();
        let mut v = v;
        loop {
            i -= 1;
            if let Some(slot) = buf.get_mut(i) {
                // `v % 10 < 10`.
                #[allow(clippy::cast_possible_truncation)]
                {
                    *slot = b'0' + (v % 10) as u8;
                }
            }
            v /= 10;
            if v == 0 || i == 0 {
                break;
            }
        }
        self.out.put(buf.get(i..).unwrap_or_default());
        self
    }

    /// A signed number.
    pub fn int(&mut self, v: i64) -> &mut Self {
        if v < 0 {
            self.sep();
            self.out.put(b"-");
            self.after_key = true;
        }
        self.uint(v.unsigned_abs())
    }

    /// `true`/`false`.
    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.sep();
        self.out.put(if v { b"true" } else { b"false" });
        self
    }

    /// `null`.
    pub fn null(&mut self) -> &mut Self {
        self.sep();
        self.out.put(b"null");
        self
    }

    /// A pre-rendered JSON value, written verbatim. The caller is
    /// responsible for its validity.
    pub fn raw_value(&mut self, json: &[u8]) -> &mut Self {
        self.sep();
        self.out.put(json);
        self
    }

    /// `"key": "value"`.
    pub fn field_str(&mut self, k: &str, v: &str) -> &mut Self {
        self.key(k).string(v)
    }

    /// `"key": n`.
    pub fn field_uint(&mut self, k: &str, v: u64) -> &mut Self {
        self.key(k).uint(v)
    }

    /// `"key": bool`.
    pub fn field_bool(&mut self, k: &str, v: bool) -> &mut Self {
        self.key(k).bool(v)
    }

    /// `"key": [n, …]`.
    pub fn field_uints(&mut self, k: &str, vs: &[u8]) -> &mut Self {
        self.key(k).begin_array();
        for &v in vs {
            self.uint(u64::from(v));
        }
        self.end_array()
    }

    /// Returns the sink if the document was well-formed and complete.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for unbalanced containers, a dangling key,
    /// or depth overflow.
    pub fn finish(self) -> Result<S, Error> {
        if self.error || self.depth != 0 || self.after_key {
            Err(err("JSON writer misuse"))
        } else {
            Ok(self.out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pe::sink::SliceSink;

    fn events(src: &str) -> Result<usize, Error> {
        let mut p = Parser::new(src.as_bytes());
        let mut n = 0;
        while p.next()?.is_some() {
            n += 1;
        }
        Ok(n)
    }

    #[test]
    fn parses_documents() {
        assert_eq!(events(r#"{"a":[1,2,{"b":null}],"c":true}"#), Ok(13));
        assert_eq!(events(" [ ] "), Ok(2));
        assert_eq!(events("-0.5e+3"), Ok(1));
        assert_eq!(events(r#""\u30d4\ud83c\udfb9""#), Ok(1));
    }

    #[test]
    fn rejects_malformed() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{,}",
            r#"{"a"}"#,
            r#"{"a":1,}"#,
            "01",
            "1.",
            "tru",
            "[1] 2",
            r#""\x""#,
            "\"\u{e9}\"",
            "]",
            r#"{"a":1]"#,
        ] {
            assert!(events(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(MAX_DEPTH + 1);
        assert!(events(&deep).is_err());
    }

    #[test]
    fn unescapes() {
        let mut p = Parser::new(br#""a\"b\u00e9\ud83c\udfb9\/""#);
        let Some(Event::Str(s)) = p.next().unwrap() else {
            panic!()
        };
        assert!(s.eq_str("a\"b\u{e9}\u{1F3B9}/"));
        let mut buf = [0u8; 16];
        assert_eq!(s.unescape_into(&mut buf), Ok("a\"b\u{e9}\u{1F3B9}/"));
    }

    #[test]
    fn writes_and_round_trips() {
        let mut buf = [0u8; 128];
        let mut w = JsonWriter::new(SliceSink::new(&mut buf));
        w.begin_object()
            .field_str("title", "ピアノ \"x\"")
            .field_uint("n", 42)
            .key("neg")
            .int(-7)
            .field_uints("bankPC", &[0, 0, 76])
            .key("e")
            .begin_array()
            .end_array()
            .end_object();
        let n = w.finish().unwrap().finish().unwrap();
        let out = core::str::from_utf8(&buf[..n]).unwrap();
        assert_eq!(
            out,
            r#"{"title":"\u30d4\u30a2\u30ce \"x\"","n":42,"neg":-7,"bankPC":[0,0,76],"e":[]}"#
        );
        assert_eq!(events(out), Ok(17));
    }

    #[test]
    fn writer_detects_misuse() {
        let mut buf = [0u8; 8];
        let mut w = JsonWriter::new(SliceSink::new(&mut buf));
        w.begin_object();
        assert!(w.finish().is_err());
    }
}
