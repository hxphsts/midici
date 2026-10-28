//! Profile Configuration messages (Sub-ID#2 `0x20..=0x2F`, M2-101 §7) and
//! Profile IDs (M2-101 §7.3.1, M2-102 §2.2).

use core::fmt;

use crate::codec::{Reader, Writer};
use crate::error::Error;
use crate::identity::ManufacturerId;
use crate::mgmt::Codec;
use crate::wire::CiVersion;

/// A five-byte Profile ID (M2-101 Table 19, M2-102 Table 5).
///
/// Standard Defined Profiles start with `0x7E`, followed by bank, number,
/// version and level. Manufacturer Profiles start with the 3-byte SysEx ID
/// (a 1-byte ID padded with two zeros) and end with two bytes of
/// manufacturer-specific info.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProfileId([u8; 5]);

impl ProfileId {
    /// Byte 1 of a Standard Defined Profile.
    pub const STANDARD: u8 = 0x7E;

    /// A Profile ID from its five wire bytes.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] if a byte exceeds 7 bits.
    pub const fn new(bytes: [u8; 5]) -> Result<Self, Error> {
        let [b0, b1, b2, b3, b4] = bytes;
        if (b0 | b1 | b2 | b3 | b4) & 0x80 != 0 {
            Err(Error::OutOfRange("profile ID"))
        } else {
            Ok(Self(bytes))
        }
    }

    /// A Standard Defined Profile.
    #[must_use]
    pub const fn standard(bank: u8, number: u8, version: u8, level: u8) -> Self {
        Self([Self::STANDARD, bank, number, version, level])
    }

    /// A Manufacturer Specific Profile.
    #[must_use]
    pub const fn manufacturer(id: ManufacturerId, info: [u8; 2]) -> Self {
        let [a, b, c] = id.ci_bytes();
        Self([a, b, c, info[0], info[1]])
    }

    /// Whether byte 1 is `0x7E`.
    #[must_use]
    pub const fn is_standard(self) -> bool {
        self.0[0] == Self::STANDARD
    }

    /// The wire bytes.
    #[must_use]
    pub const fn bytes(self) -> [u8; 5] {
        self.0
    }

    fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(Self(r.array()?))
    }

    fn write(self, w: &mut Writer<'_>) -> Result<(), Error> {
        w.bytes(&self.0, "profile ID")
    }
}

impl fmt::Debug for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProfileId({:02X?})", self.0)
    }
}

/// A list of Profile IDs: either borrowed from received bytes, or from a
/// caller's slice when encoding. Compares by content.
#[derive(Clone, Copy)]
pub enum ProfileList<'a> {
    /// `5 × n` bytes as received.
    Wire(&'a [u8]),
    /// IDs supplied by the caller.
    Ids(&'a [ProfileId]),
}

impl Default for ProfileList<'_> {
    fn default() -> Self {
        Self::Ids(&[])
    }
}

impl<'a> ProfileList<'a> {
    /// Number of IDs.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Self::Wire(b) => b.len() / 5,
            Self::Ids(ids) => ids.len(),
        }
    }

    /// Whether the list is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The IDs in order.
    pub fn iter(&self) -> impl Iterator<Item = ProfileId> + 'a {
        let (wire, ids): (&'a [u8], &'a [ProfileId]) = match *self {
            Self::Wire(b) => (b, &[]),
            Self::Ids(ids) => (&[], ids),
        };
        wire.as_chunks::<5>()
            .0
            .iter()
            .map(|c| ProfileId(*c))
            .chain(ids.iter().copied())
    }

    fn read(r: &mut Reader<'a>) -> Result<Self, Error> {
        let n = usize::from(r.u14()?);
        Ok(Self::Wire(r.bytes(n * 5)?))
    }

    fn write(&self, w: &mut Writer<'_>) -> Result<(), Error> {
        let n = u16::try_from(self.len()).map_err(|_| Error::OutOfRange("profile count"))?;
        w.u14(n, "profile count")?;
        for id in self.iter() {
            id.write(w)?;
        }
        Ok(())
    }
}

impl PartialEq for ProfileList<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for ProfileList<'_> {}

impl fmt::Debug for ProfileList<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// Profile Inquiry (`0x20`, M2-101 §7.2, Table 17): no data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ProfileInquiry;

impl Codec<'_> for ProfileInquiry {
    fn read(_: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self)
    }
    fn write(&self, _: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        Ok(())
    }
}

/// Reply to Profile Inquiry (`0x21`, M2-101 §7.3, Table 18).
///
/// ```text
/// enabled count ×2, enabled IDs ×5n, disabled count ×2, disabled IDs ×5m
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfileInquiryReply<'a> {
    /// Supported and currently enabled.
    pub enabled: ProfileList<'a>,
    /// Supported and currently disabled.
    pub disabled: ProfileList<'a>,
}

impl<'a> Codec<'a> for ProfileInquiryReply<'a> {
    fn read(r: &mut Reader<'a>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self {
            enabled: ProfileList::read(r)?,
            disabled: ProfileList::read(r)?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        self.enabled.write(w)?;
        self.disabled.write(w)
    }
}

/// A Profile ID plus a v2 channel count.
///
/// Used by Set Profile On (`0x22`, §7.8, "Number Channels Requested"),
/// Profile Enabled Report (`0x24`, §7.10) and Profile Disabled Report
/// (`0x25`, §7.11). The count is `0` when addressing a Group or Function
/// Block, and `0` when decoded from version 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ProfileChannels {
    /// The Profile.
    pub profile: ProfileId,
    /// Number of channels (Manager + Member), 14 bits.
    pub channels: u16,
}

impl Codec<'_> for ProfileChannels {
    fn read(r: &mut Reader<'_>, v: CiVersion) -> Result<Self, Error> {
        let profile = ProfileId::read(r)?;
        let channels = if v.has_v2_fields() { r.u14()? } else { 0 };
        Ok(Self { profile, channels })
    }
    fn write(&self, w: &mut Writer<'_>, v: CiVersion) -> Result<(), Error> {
        self.profile.write(w)?;
        if v.has_v2_fields() {
            w.u14(self.channels, "channel count")?;
        }
        Ok(())
    }
}

/// A bare Profile ID: Profile Added Report (`0x26`, §7.4) and Profile
/// Removed Report (`0x27`, §7.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ProfileReport {
    /// The Profile.
    pub profile: ProfileId,
}

impl Codec<'_> for ProfileReport {
    fn read(r: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self {
            profile: ProfileId::read(r)?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        self.profile.write(w)
    }
}

/// Set Profile Off (`0x23`, §7.9, Table 25). Version 2 appends two reserved
/// bytes, sent as zero and ignored on receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct SetProfileOff {
    /// The Profile.
    pub profile: ProfileId,
}

impl Codec<'_> for SetProfileOff {
    fn read(r: &mut Reader<'_>, v: CiVersion) -> Result<Self, Error> {
        let profile = ProfileId::read(r)?;
        if v.has_v2_fields() {
            r.bytes(2)?;
        }
        Ok(Self { profile })
    }
    fn write(&self, w: &mut Writer<'_>, v: CiVersion) -> Result<(), Error> {
        self.profile.write(w)?;
        if v.has_v2_fields() {
            w.bytes(&[0, 0], "reserved")?;
        }
        Ok(())
    }
}

/// Profile Details Inquiry (`0x28`, §7.6, Table 22).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ProfileDetailsInquiry {
    /// The Profile.
    pub profile: ProfileId,
    /// Inquiry Target: `0x00..=0x3F` registered, `0x40..=0x7F` Profile
    /// specific (§7.6.1).
    pub target: u8,
}

/// Registered Inquiry Target `0x00`: number of MIDI channels
/// (M2-102 §2.5.1).
pub const TARGET_CHANNEL_COUNT: u8 = 0x00;

impl Codec<'_> for ProfileDetailsInquiry {
    fn read(r: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self {
            profile: ProfileId::read(r)?,
            target: r.u7()?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        self.profile.write(w)?;
        w.u7(self.target, "inquiry target")
    }
}

/// Reply to Profile Details Inquiry (`0x29`, §7.7, Table 23).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ProfileDetailsReply<'a> {
    /// The Profile.
    pub profile: ProfileId,
    /// Echo of the Inquiry Target.
    pub target: u8,
    /// Inquiry Target Data (14-bit length on the wire).
    pub data: &'a [u8],
}

/// Target `0x00` data: channels in use and maximum (M2-102 §2.5.1,
/// Table 8), each 14 bits LSB first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct ChannelCount {
    /// Channels currently used (0 if the Profile is disabled).
    pub in_use: u16,
    /// Maximum channels available to the Profile.
    pub max: u16,
}

impl ChannelCount {
    /// The four data bytes.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] if a count exceeds 14 bits.
    pub fn to_bytes(self) -> Result<[u8; 4], Error> {
        let mut out = [0; 4];
        let mut w = Writer::new(&mut out);
        w.u14(self.in_use, "channels in use")?;
        w.u14(self.max, "maximum channels")?;
        Ok(out)
    }

    /// Parses the four data bytes.
    ///
    /// # Errors
    ///
    /// [`Error::Truncated`] or [`Error::InvalidField`].
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(data);
        Ok(Self {
            in_use: r.u14()?,
            max: r.u14()?,
        })
    }
}

impl<'a> Codec<'a> for ProfileDetailsReply<'a> {
    fn read(r: &mut Reader<'a>, _: CiVersion) -> Result<Self, Error> {
        let profile = ProfileId::read(r)?;
        let target = r.u7()?;
        let len = r.u14()?;
        Ok(Self {
            profile,
            target,
            data: r.bytes(usize::from(len))?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        self.profile.write(w)?;
        w.u7(self.target, "inquiry target")?;
        let len = u16::try_from(self.data.len()).map_err(|_| Error::OutOfRange("target data"))?;
        w.u14(len, "target data length")?;
        w.bytes(self.data, "target data")
    }
}

/// Profile Specific Data (`0x2F`, §7.12, Table 28): Profile ID, 28-bit
/// length, data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ProfileSpecificData<'a> {
    /// The Profile.
    pub profile: ProfileId,
    /// Profile-defined data.
    pub data: &'a [u8],
}

impl<'a> Codec<'a> for ProfileSpecificData<'a> {
    fn read(r: &mut Reader<'a>, _: CiVersion) -> Result<Self, Error> {
        let profile = ProfileId::read(r)?;
        let len = usize::try_from(r.u28()?).map_err(|_| Error::Truncated)?;
        Ok(Self {
            profile,
            data: r.bytes(len)?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        self.profile.write(w)?;
        let len = u32::try_from(self.data.len()).map_err(|_| Error::OutOfRange("profile data"))?;
        w.u28(len, "profile data length")?;
        w.bytes(self.data, "profile data")
    }
}

/// Whether a Profile is (to be) enabled or disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ProfileState {
    /// Enabled ("on").
    Enabled,
    /// Disabled ("off").
    Disabled,
}

impl ProfileInquiry {
    /// A Profile Inquiry.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl<'a> ProfileInquiryReply<'a> {
    /// A reply listing `enabled` and `disabled` Profiles.
    #[must_use]
    pub const fn new(enabled: ProfileList<'a>, disabled: ProfileList<'a>) -> Self {
        Self { enabled, disabled }
    }
}

impl ProfileChannels {
    /// `profile` with a channel count of 0 (Group or Function Block).
    #[must_use]
    pub const fn new(profile: ProfileId) -> Self {
        Self {
            profile,
            channels: 0,
        }
    }

    /// Sets the channel count (v2 field, 14 bits).
    #[must_use]
    pub const fn with_channels(mut self, channels: u16) -> Self {
        self.channels = channels;
        self
    }
}

impl ProfileReport {
    /// A report about `profile`.
    #[must_use]
    pub const fn new(profile: ProfileId) -> Self {
        Self { profile }
    }
}

impl SetProfileOff {
    /// Disable `profile`.
    #[must_use]
    pub const fn new(profile: ProfileId) -> Self {
        Self { profile }
    }
}

impl ProfileDetailsInquiry {
    /// Ask about `target` of `profile`.
    #[must_use]
    pub const fn new(profile: ProfileId, target: u8) -> Self {
        Self { profile, target }
    }
}

impl<'a> ProfileDetailsReply<'a> {
    /// Answer `target` of `profile` with `data`.
    #[must_use]
    pub const fn new(profile: ProfileId, target: u8, data: &'a [u8]) -> Self {
        Self {
            profile,
            target,
            data,
        }
    }
}

impl ChannelCount {
    /// Channels in use and the maximum.
    #[must_use]
    pub const fn new(in_use: u16, max: u16) -> Self {
        Self { in_use, max }
    }
}

impl<'a> ProfileSpecificData<'a> {
    /// `data` for `profile`.
    #[must_use]
    pub const fn new(profile: ProfileId, data: &'a [u8]) -> Self {
        Self { profile, data }
    }
}
