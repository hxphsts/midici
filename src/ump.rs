//! Carrying CI messages in Universal MIDI Packets: System Exclusive (7-Bit)
//! messages, Message Type `0x3` (M2-104 §7.7, Figure 84, Table 18).
//!
//! ```text
//! word 0: mt=3 (4) | group (4) | status (4) | # of bytes (4) | d0 | d1
//! word 1: d2 | d3 | d4 | d5
//! ```
//!
//! Status `0x0` is a complete message in one packet; `0x1`/`0x2`/`0x3` are
//! start, continue and end. `F0`/`F7` are not carried. Unused bytes are
//! zero.

/// UMP Message Type for 7-bit System Exclusive.
pub const MT_SYSEX7: u8 = 0x3;

const STATUS_COMPLETE: u8 = 0x0;
const STATUS_START: u8 = 0x1;
const STATUS_CONTINUE: u8 = 0x2;
const STATUS_END: u8 = 0x3;

fn strip(bytes: &[u8]) -> &[u8] {
    let b = bytes.strip_prefix(&[0xF0]).unwrap_or(bytes);
    b.strip_suffix(&[0xF7]).unwrap_or(b)
}

/// Iterator over the 64-bit packets of one SysEx message.
#[derive(Clone, Debug)]
pub struct Packets<'a> {
    rest: &'a [u8],
    group: u8,
    first: bool,
    done: bool,
}

/// Packetises one SysEx message (with or without `F0`/`F7`) on `group`.
#[must_use]
pub fn packets(bytes: &[u8], group: u8) -> Packets<'_> {
    Packets {
        rest: strip(bytes),
        group: group & 0x0F,
        first: true,
        done: false,
    }
}

impl Iterator for Packets<'_> {
    type Item = [u32; 2];

    fn next(&mut self) -> Option<[u32; 2]> {
        if self.done {
            return None;
        }
        let n = self.rest.len().min(6);
        let (data, rest) = self.rest.split_at(n);
        let last = rest.is_empty();
        let status = match (self.first, last) {
            (true, true) => STATUS_COMPLETE,
            (true, false) => STATUS_START,
            (false, false) => STATUS_CONTINUE,
            (false, true) => STATUS_END,
        };
        self.first = false;
        self.done = last;
        self.rest = rest;
        let mut d = [0u8; 6];
        if let Some(dst) = d.get_mut(..n) {
            dst.copy_from_slice(data);
        }
        // `n <= 6`.
        #[allow(clippy::cast_possible_truncation)]
        let n = n as u8;
        let w0 = u32::from_be_bytes([(MT_SYSEX7 << 4) | self.group, (status << 4) | n, d[0], d[1]]);
        let w1 = u32::from_be_bytes([d[2], d[3], d[4], d[5]]);
        Some([w0, w1])
    }
}

/// Packetises into a `Vec`.
///
/// # Errors
///
/// [`crate::Error::NonDataByte`] if the payload has a byte `>= 0x80`.
#[cfg(feature = "alloc")]
pub fn to_ump(bytes: &[u8], group: u8) -> Result<alloc::vec::Vec<[u32; 2]>, crate::Error> {
    if let Some(i) = strip(bytes).iter().position(|b| b & 0x80 != 0) {
        return Err(crate::Error::NonDataByte { offset: i });
    }
    Ok(packets(bytes, group).collect())
}

/// Reassembles SysEx7 packets into a bare SysEx payload (no `F0`/`F7`).
///
/// Uses a fixed `N`-byte buffer. Tracks one message at a time; packets of
/// other message types are ignored, and a packet for another Group while a
/// message is in progress is ignored (use one assembler per Group).
#[derive(Clone, Debug)]
pub struct SysEx7Assembler<const N: usize> {
    buf: [u8; N],
    len: usize,
    group: Option<u8>,
    overflow: bool,
}

impl<const N: usize> Default for SysEx7Assembler<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> SysEx7Assembler<N> {
    /// An idle assembler.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            group: None,
            overflow: false,
        }
    }

    fn append(&mut self, data: &[u8]) {
        let end = self.len + data.len();
        match self.buf.get_mut(self.len..end) {
            Some(dst) => {
                dst.copy_from_slice(data);
                self.len = end;
            }
            None => self.overflow = true,
        }
    }

    /// Feeds one packet. Returns `(group, payload)` when a message
    /// completes; oversized or malformed messages are dropped.
    pub fn push(&mut self, packet: [u32; 2]) -> Option<(u8, &[u8])> {
        let [b0, b1, d0, d1] = packet[0].to_be_bytes();
        let [d2, d3, d4, d5] = packet[1].to_be_bytes();
        if b0 >> 4 != MT_SYSEX7 {
            return None;
        }
        let group = b0 & 0x0F;
        let status = b1 >> 4;
        let n = usize::from(b1 & 0x0F);
        let all = [d0, d1, d2, d3, d4, d5];
        let data = all.get(..n)?;
        if data.iter().any(|b| b & 0x80 != 0) {
            self.group = None;
            return None;
        }
        if self.group.is_some_and(|g| g != group) {
            return None;
        }
        match status {
            STATUS_COMPLETE | STATUS_START => {
                self.len = 0;
                self.overflow = false;
                self.append(data);
                if status == STATUS_START {
                    self.group = Some(group);
                    return None;
                }
            }
            STATUS_CONTINUE | STATUS_END => {
                self.group?;
                self.append(data);
                if status == STATUS_CONTINUE {
                    return None;
                }
                self.group = None;
            }
            _ => return None,
        }
        if self.overflow {
            return None;
        }
        Some((group, self.buf.get(..self.len)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_round_trip() {
        // 13-byte bare CI envelope → start(6) continue(6) end(1).
        let msg = [
            0xF0, 0x7E, 0x7F, 0x0D, 0x70, 2, 1, 2, 3, 4, 0x7F, 0x7F, 0x7F, 0x7F, 0xF7,
        ];
        let p: [[u32; 2]; 3] = {
            let mut it = packets(&msg, 5);
            [it.next().unwrap(), it.next().unwrap(), it.next().unwrap()]
        };
        assert_eq!(p[0], [0x3516_7E7F, 0x0D70_0201]);
        assert_eq!(p[1], [0x3526_0203, 0x047F_7F7F]);
        assert_eq!(p[2], [0x3531_7F00, 0x0000_0000]);
        let mut a = SysEx7Assembler::<32>::new();
        assert!(a.push(p[0]).is_none());
        assert!(a.push(p[1]).is_none());
        let (g, data) = a.push(p[2]).unwrap();
        assert_eq!(g, 5);
        assert_eq!(data, &msg[1..msg.len() - 1]);
    }

    #[test]
    fn short_message_is_one_complete_packet() {
        let mut it = packets(&[0x7E, 0x7F, 0x06, 0x01], 0);
        assert_eq!(it.next(), Some([0x3004_7E7F, 0x0601_0000]));
        assert_eq!(it.next(), None);
    }
}
