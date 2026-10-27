//! Process Inquiry messages (Sub-ID#2 `0x40..=0x44`, M2-101 §9), new in
//! MIDI-CI 1.2.

use crate::codec::{Reader, Writer};
use crate::error::Error;
use crate::flags::flags;
use crate::mgmt::Codec;
use crate::wire::CiVersion;

flags! {
    /// Process Inquiry Supported Features (M2-101 §9.3, Table 42).
    pub struct ProcessFeatures {
        /// D0: MIDI Message Report.
        const MESSAGE_REPORT = 1 << 0;
    }
}

flags! {
    /// Bitmap of System Messages (M2-101 Table 43).
    pub struct SystemMessages {
        /// D0: MTC Quarter Frame.
        const MTC_QUARTER_FRAME = 1 << 0;
        /// D1: Song Position.
        const SONG_POSITION = 1 << 1;
        /// D2: Song Select.
        const SONG_SELECT = 1 << 2;
    }
}

flags! {
    /// Bitmap of Channel Controller Messages (M2-101 Table 43).
    pub struct ChannelControllers {
        /// D0: Pitch Bend.
        const PITCH_BEND = 1 << 0;
        /// D1: Control Change.
        const CONTROL_CHANGE = 1 << 1;
        /// D2: RPN / Registered Controller.
        const RPN = 1 << 2;
        /// D3: NRPN / Assignable Controller.
        const NRPN = 1 << 3;
        /// D4: Program Change.
        const PROGRAM_CHANGE = 1 << 4;
        /// D5: Channel Pressure.
        const CHANNEL_PRESSURE = 1 << 5;
    }
}

flags! {
    /// Bitmap of Note Data Messages (M2-101 Table 43).
    pub struct NoteData {
        /// D0: Notes.
        const NOTES = 1 << 0;
        /// D1: Poly Pressure.
        const POLY_PRESSURE = 1 << 1;
        /// D2: Per-Note Pitch Bend.
        const PER_NOTE_PITCH_BEND = 1 << 2;
        /// D3: Registered Per-Note Controller.
        const REGISTERED_PER_NOTE = 1 << 3;
        /// D4: Assignable Per-Note Controller.
        const ASSIGNABLE_PER_NOTE = 1 << 4;
    }
}

/// Inquiry: Process Inquiry Capabilities (`0x40`, §9.2): no data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcessInquiry;

impl Codec<'_> for ProcessInquiry {
    fn read(_: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self)
    }
    fn write(&self, _: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        Ok(())
    }
}

/// Reply to Process Inquiry Capabilities (`0x41`, §9.3, Table 41).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcessInquiryReply {
    /// Supported features.
    pub features: ProcessFeatures,
}

impl Codec<'_> for ProcessInquiryReply {
    fn read(r: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self {
            features: ProcessFeatures::from_bits(r.u7()?),
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        w.u7(self.features.bits(), "process features")
    }
}

/// Message Data Control (M2-101 §9.5.1, Table 44).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataControl(pub u8);

impl DataControl {
    /// `0x00`: begin/end replies only, no data.
    pub const NO_DATA: Self = Self(0x00);
    /// `0x01`: only non-default controllers and active notes.
    pub const NON_DEFAULT: Self = Self(0x01);
    /// `0x7F`: full report.
    pub const FULL: Self = Self(0x7F);
}

/// The message bitmaps shared by the report inquiry and its reply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReportSelection {
    /// System messages.
    pub system: SystemMessages,
    /// Channel controller messages.
    pub channel: ChannelControllers,
    /// Note data messages.
    pub note: NoteData,
}

impl ReportSelection {
    /// `system ×1, reserved ×1 (0x00), channel ×1, note ×1`.
    fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let system = SystemMessages::from_bits(r.u7()?);
        let _reserved = r.u7()?;
        Ok(Self {
            system,
            channel: ChannelControllers::from_bits(r.u7()?),
            note: NoteData::from_bits(r.u7()?),
        })
    }

    fn write(self, w: &mut Writer<'_>) -> Result<(), Error> {
        w.u7(self.system.bits(), "system bitmap")?;
        w.u7(0, "reserved")?;
        w.u7(self.channel.bits(), "channel bitmap")?;
        w.u7(self.note.bits(), "note bitmap")
    }

    /// Only what both selections contain: a Responder replies with the
    /// intersection of the request and what it supports (§9.6).
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self {
            system: self.system.intersection(other.system),
            channel: self.channel.intersection(other.channel),
            note: self.note.intersection(other.note),
        }
    }
}

/// Inquiry: MIDI Message Report (`0x42`, §9.5, Table 43).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MessageReportInquiry {
    /// How much to report.
    pub control: DataControl,
    /// What to report.
    pub selection: ReportSelection,
}

impl Codec<'_> for MessageReportInquiry {
    fn read(r: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self {
            control: DataControl(r.u7()?),
            selection: ReportSelection::read(r)?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        w.u7(self.control.0, "message data control")?;
        self.selection.write(w)
    }
}

/// Reply to MIDI Message Report (`0x43`, §9.6, Table 45): what the
/// Responder will report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MessageReportReply {
    /// What will be reported.
    pub selection: ReportSelection,
}

impl Codec<'_> for MessageReportReply {
    fn read(r: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self {
            selection: ReportSelection::read(r)?,
        })
    }
    fn write(&self, w: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        self.selection.write(w)
    }
}

/// End of MIDI Message Report (`0x44`, §9.8, Table 46): no data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EndOfMessageReport;

impl Codec<'_> for EndOfMessageReport {
    fn read(_: &mut Reader<'_>, _: CiVersion) -> Result<Self, Error> {
        Ok(Self)
    }
    fn write(&self, _: &mut Writer<'_>, _: CiVersion) -> Result<(), Error> {
        Ok(())
    }
}
