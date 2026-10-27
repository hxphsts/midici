//! MIDI 2.0 value scaling between resolutions (M2-115-U "MIDI 2.0 Bit
//! Scaling and Resolution", v1.0.1).
//!
//! * [`scale_up`] / [`scale_down`]: Min-Center-Max scaling (M2-115 §3), the
//!   default; minimum, centre and maximum map onto each other, and
//!   `scale_down(scale_up(x))` is `x`. Use it for Control Change, NRPN, and
//!   RPN with index LSB `32..=127` (M2-115 §3.1).
//! * [`zero_extend_up`] / [`round_down`]: Zero-Extension Scaling with
//!   Rounding (M2-115 §4), for fixed-point/absolute values; RPN index LSB
//!   `0..=31` (M2-115 §4.1).
//! * [`step_to_value`] / [`value_to_step`]: stepped values and enumerations
//!   (M2-115 §5).
//!
//! Bit widths are `1..=32`. The general functions return `None` for invalid
//! widths or for a value wider than its source width; the fixed-width
//! helpers are total.

const fn mask(bits: u32) -> u64 {
    (1u64 << bits) - 1
}

/// `v` as `u32`, saturating; callers only pass values already below 2^32.
const fn narrow(v: u64) -> u32 {
    if v > u32::MAX as u64 {
        u32::MAX
    } else {
        // Checked just above.
        #[allow(clippy::cast_possible_truncation)]
        {
            v as u32
        }
    }
}

const fn valid(value: u32, bits: u32) -> bool {
    bits >= 1 && bits <= 32 && (value as u64) <= mask(bits)
}

/// Min-Center-Max upscaling (M2-115 §3.3, §3.3.1).
///
/// Values up to the centre are shifted; values above it fill the new low
/// bits by repeating all but the top source bit. A 1-bit source maps
/// `1` to all-ones (M2-115 §3.3). Equal widths return `value`.
///
/// Returns `None` if `src_bits > dst_bits`, a width is outside `1..=32`, or
/// `value` does not fit in `src_bits`.
#[must_use]
pub const fn scale_up(value: u32, src_bits: u32, dst_bits: u32) -> Option<u32> {
    if !valid(value, src_bits) || dst_bits > 32 || src_bits > dst_bits {
        return None;
    }
    if src_bits == dst_bits {
        return Some(value);
    }
    let v = value as u64;
    if src_bits == 1 {
        return Some(if value == 0 {
            0
        } else {
            narrow(mask(dst_bits))
        });
    }
    let scale_bits = dst_bits - src_bits;
    let center = 1u64 << (src_bits - 1);
    let mut out = v << scale_bits;
    if v <= center {
        return Some(narrow(out));
    }
    let repeat_bits = src_bits - 1;
    let mut repeat = v & mask(repeat_bits);
    if scale_bits > repeat_bits {
        repeat <<= scale_bits - repeat_bits;
    } else {
        repeat >>= repeat_bits - scale_bits;
    }
    while repeat != 0 {
        out |= repeat;
        repeat >>= repeat_bits;
    }
    Some(narrow(out))
}

/// Min-Center-Max downscaling: drop the low bits (M2-115 §3.4).
///
/// Returns `None` if `dst_bits > src_bits`, a width is outside `1..=32`, or
/// `value` does not fit in `src_bits`.
#[must_use]
pub const fn scale_down(value: u32, src_bits: u32, dst_bits: u32) -> Option<u32> {
    if !valid(value, src_bits) || dst_bits == 0 || dst_bits > src_bits {
        return None;
    }
    Some(value >> (src_bits - dst_bits))
}

/// Zero-extension upscaling: shift left, fill with zeros (M2-115 §4.3).
/// Not for 1-bit values (M2-115 §4.3); use [`scale_up`] for those.
#[must_use]
pub const fn zero_extend_up(value: u32, src_bits: u32, dst_bits: u32) -> Option<u32> {
    if !valid(value, src_bits) || dst_bits > 32 || src_bits > dst_bits {
        return None;
    }
    Some(narrow((value as u64) << (dst_bits - src_bits)))
}

/// Downscaling with rounding and clamping (M2-115 §4.4, §4.4.1).
#[must_use]
pub const fn round_down(value: u32, src_bits: u32, dst_bits: u32) -> Option<u32> {
    if !valid(value, src_bits) || dst_bits == 0 || dst_bits > src_bits {
        return None;
    }
    let scale_bits = src_bits - dst_bits;
    if scale_bits == 0 {
        return Some(value);
    }
    let half = 1u64 << (scale_bits - 1);
    let shifted = (value as u64 + half) >> scale_bits;
    let max = mask(dst_bits);
    Some(narrow(if shifted > max { max } else { shifted }))
}

/// The message value for step `step` of `count` at `bits` resolution
/// (M2-115 §5.1): centred in its bin; with two steps, `0` and the maximum.
///
/// Returns `None` if `count` is 0 or exceeds the value range, `step >=
/// count`, or `bits` is outside `1..=32`.
#[must_use]
pub const fn step_to_value(step: u32, count: u32, bits: u32) -> Option<u32> {
    if bits == 0 || bits > 32 || count == 0 || step >= count {
        return None;
    }
    let range = 1u64 << bits;
    let (s, c) = (step as u64, count as u64);
    if c > range {
        return None;
    }
    if c == 2 {
        return Some(if step == 0 { 0 } else { narrow(mask(bits)) });
    }
    let v = (range * s).div_ceil(c) + range / (2 * c);
    Some(narrow(if v > mask(bits) { mask(bits) } else { v }))
}

/// The step selected by `value` at `bits` resolution, out of `count`
/// (M2-115 §5.2). Any value within a bin selects that bin.
#[must_use]
pub const fn value_to_step(value: u32, count: u32, bits: u32) -> Option<u32> {
    if !valid(value, bits) || count == 0 {
        return None;
    }
    Some(narrow((value as u64 * count as u64) >> bits))
}

/// 7 → 16 bit Min-Center-Max (masks `v` to 7 bits).
#[must_use]
pub const fn u7_to_u16(v: u8) -> u16 {
    // 16-bit result by construction.
    #[allow(clippy::cast_possible_truncation)]
    match scale_up((v & 0x7F) as u32, 7, 16) {
        Some(x) => x as u16,
        None => 0,
    }
}

/// 7 → 32 bit Min-Center-Max (masks `v` to 7 bits).
#[must_use]
pub const fn u7_to_u32(v: u8) -> u32 {
    match scale_up((v & 0x7F) as u32, 7, 32) {
        Some(x) => x,
        None => 0,
    }
}

/// 14 → 32 bit Min-Center-Max (masks `v` to 14 bits).
#[must_use]
pub const fn u14_to_u32(v: u16) -> u32 {
    match scale_up((v & 0x3FFF) as u32, 14, 32) {
        Some(x) => x,
        None => 0,
    }
}

/// 16 → 32 bit Min-Center-Max.
#[must_use]
pub const fn u16_to_u32(v: u16) -> u32 {
    match scale_up(v as u32, 16, 32) {
        Some(x) => x,
        None => 0,
    }
}

/// 16 → 7 bit Min-Center-Max.
#[must_use]
pub const fn u16_to_u7(v: u16) -> u8 {
    (v >> 9) as u8
}

/// 32 → 7 bit Min-Center-Max.
#[must_use]
pub const fn u32_to_u7(v: u32) -> u8 {
    // 7-bit result by construction.
    #[allow(clippy::cast_possible_truncation)]
    {
        (v >> 25) as u8
    }
}

/// 32 → 14 bit Min-Center-Max.
#[must_use]
pub const fn u32_to_u14(v: u32) -> u16 {
    // 14-bit result by construction.
    #[allow(clippy::cast_possible_truncation)]
    {
        (v >> 18) as u16
    }
}

/// 32 → 16 bit Min-Center-Max.
#[must_use]
pub const fn u32_to_u16(v: u32) -> u16 {
    // 16-bit result by construction.
    #[allow(clippy::cast_possible_truncation)]
    {
        (v >> 16) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M2-115 Table 5.
    #[test]
    fn table_5_upscale_7_to_16() {
        for (a, b) in [
            (0, 0x0000),
            (5, 0x0A00),
            (30, 0x3C00),
            (32, 0x4000),
            (64, 0x8000),
            (70, 0x8C30),
            (96, 0xC104),
            (120, 0xF1C7),
            (127, 0xFFFF),
        ] {
            assert_eq!(u7_to_u16(a), b, "{a}");
        }
    }

    /// M2-115 Table 6.
    #[test]
    fn table_6_upscale_7_to_32() {
        for (a, b) in [
            (0, 0x0000_0000),
            (5, 0x0A00_0000),
            (30, 0x3C00_0000),
            (32, 0x4000_0000),
            (64, 0x8000_0000),
            (70, 0x8C30_C30C),
            (96, 0xC104_1041),
            (120, 0xF1C7_1C71),
            (127, 0xFFFF_FFFF),
        ] {
            assert_eq!(u7_to_u32(a), b, "{a}");
        }
    }

    /// M2-115 Table 7.
    #[test]
    fn table_7_upscale_16_to_32() {
        for (a, b) in [
            (0, 0x0000_0000),
            (5, 0x0005_0000),
            (30, 0x001E_0000),
            (16384, 0x4000_0000),
            (32768, 0x8000_0000),
            (40000, 0x9C40_3880),
            (49152, 0xC000_8001),
            (65000, 0xFDE8_FBD1),
            (65535, 0xFFFF_FFFF),
        ] {
            assert_eq!(u16_to_u32(a), b, "{a}");
        }
    }

    /// M2-115 Table 8.
    #[test]
    fn table_8_downscale_16_to_7() {
        for (a, b) in [(5120, 10), (32768, 64), (44730, 87), (65535, 127)] {
            assert_eq!(u16_to_u7(a), b, "{a}");
        }
    }

    /// M2-115 Table 9 (rows 1–3; the last row prints 1019392/0xF8E00, which
    /// contradicts §4.2's own example of 127 → 65024; we follow §4.2).
    #[test]
    fn table_9_zero_extend_7_to_16() {
        for (a, b) in [(10, 5120), (64, 32768), (87, 44544), (127, 65024)] {
            assert_eq!(zero_extend_up(a, 7, 16), Some(b), "{a}");
        }
    }

    /// M2-115 Table 10.
    #[test]
    fn table_10_round_down_16_to_7() {
        for (a, b) in [
            (5120, 10),
            (5631, 11),
            (32768, 64),
            (44544, 87),
            (44730, 87),
            (44800, 88),
            (65024, 127),
            (65535, 127),
        ] {
            assert_eq!(round_down(a, 16, 7), Some(b), "{a}");
        }
    }

    #[test]
    fn one_bit_and_identity() {
        assert_eq!(scale_up(1, 1, 16), Some(0xFFFF));
        assert_eq!(scale_up(0, 1, 32), Some(0));
        assert_eq!(scale_up(1, 1, 32), Some(u32::MAX));
        assert_eq!(scale_up(5, 7, 7), Some(5));
        assert_eq!(scale_up(128, 7, 16), None);
        assert_eq!(scale_up(1, 16, 7), None);
        assert_eq!(scale_up(1, 0, 7), None);
        assert_eq!(scale_down(1, 7, 16), None);
    }

    #[test]
    fn round_trips_exhaustively_for_small_widths() {
        for src in 1..=14u32 {
            for dst in src..=32 {
                for v in 0..(1u32 << src) {
                    let up = scale_up(v, src, dst).unwrap();
                    assert_eq!(scale_down(up, dst, src), Some(v), "{v} {src}->{dst}");
                }
            }
        }
    }

    #[test]
    fn centre_maps_to_centre() {
        assert_eq!(u7_to_u32(0x40), 0x8000_0000);
        assert_eq!(u14_to_u32(0x2000), 0x8000_0000);
        assert_eq!(u32_to_u14(0x8000_0000), 0x2000);
    }

    #[test]
    fn steps() {
        // Two steps behave like a 1-bit value.
        assert_eq!(step_to_value(1, 2, 7), Some(127));
        for count in 1..=32u32 {
            for step in 0..count {
                for bits in [7, 14, 16, 32] {
                    let v = step_to_value(step, count, bits).unwrap();
                    assert_eq!(
                        value_to_step(v, count, bits),
                        Some(step),
                        "{step}/{count}@{bits}"
                    );
                }
            }
        }
        assert_eq!(step_to_value(3, 3, 7), None);
        assert_eq!(step_to_value(0, 0, 7), None);
    }
}
