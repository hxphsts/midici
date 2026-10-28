//! `zlib+Mcoded7` Property Data (M2-103 §4.3, §4.4.2): zlib (RFC 1950)
//! compression, then Mcoded7. Requires the `zlib` feature (and `alloc`).

use alloc::vec::Vec;

use crate::error::Error;
use crate::pe::mcoded7;

/// Default compression level.
pub const LEVEL: u8 = 6;

/// Compresses with zlib and encodes with Mcoded7.
#[must_use]
pub fn encode(data: &[u8]) -> Vec<u8> {
    let z = miniz_oxide::deflate::compress_to_vec_zlib(data, LEVEL);
    mcoded7::encode_vec(&z)
}

/// Decodes Mcoded7 and inflates zlib, refusing output beyond `limit` bytes.
///
/// # Errors
///
/// [`Error::InvalidField`] for bad Mcoded7 or zlib data, or output larger
/// than `limit`.
pub fn decode(data: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let z = mcoded7::decode_vec(data)?;
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&z, limit)
        .map_err(|_| Error::InvalidField("zlib data"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let data = br#"{"title":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
        let enc = super::encode(data);
        assert!(enc.iter().all(|b| b & 0x80 == 0));
        assert_eq!(super::decode(&enc, 1024).unwrap(), data);
        assert!(super::decode(&enc, 10).is_err());
    }
}
