//! A small demo device shared by the `responder` and `initiator` examples.

use midici::engine::responder::{Config, Device, GetMeta, ProfileOutcome, ProfileSet};
use midici::identity::{DeviceIdentity, ManufacturerId};
use midici::muid::Muid;
use midici::pe::PeCapabilities;
use midici::pe::header::Header;
use midici::pe::json::JsonWriter;
use midici::pe::resource::{
    CanSet, ChannelEntry, Controller, CtrlMapEntry, CtrlType, DeviceInfo, ProgramEntry,
    ResourceDecl, names, write_list,
};
use midici::pe::sink::Sink;
use midici::pe::status::PeStatus;
use midici::process::ProcessFeatures;
use midici::profile::ProfileId;
use midici::wire::DeviceId;

pub const IDENTITY: DeviceIdentity = DeviceIdentity {
    manufacturer: ManufacturerId::NON_COMMERCIAL,
    family: 1,
    model: 1,
    revision: [0, 0, 1, 0],
};

const PROFILES: [ProfileId; 1] = [ProfileId::standard(0x21, 0x00, 0x01, 0x01)];

static RESOURCES: &[ResourceDecl<'static>] = &[
    ResourceDecl::new(names::DEVICE_INFO),
    ResourceDecl {
        can_subscribe: true,
        ..ResourceDecl::new(names::CHANNEL_LIST)
    },
    ResourceDecl::new(names::ALL_CTRL_LIST),
    ResourceDecl {
        require_res_id: true,
        ..ResourceDecl::new(names::CTRL_MAP_LIST)
    },
    ResourceDecl::new(names::PROGRAM_LIST),
    ResourceDecl {
        can_set: CanSet::Partial,
        can_subscribe: true,
        ..ResourceDecl::new("X-Cutoff")
    },
];

pub fn config() -> Config<'static> {
    Config {
        profiles: true,
        property_exchange: Some(PeCapabilities::default()),
        process: ProcessFeatures::NONE,
        product_instance_id: Some("DEMO-0001"),
        max_frame: 128,
        ..Config::new(IDENTITY)
    }
}

pub struct DemoSynth {
    pub cutoff: u32,
    seed: u32,
}

impl DemoSynth {
    pub const fn new() -> Self {
        Self {
            cutoff: 64,
            seed: 0x5EED,
        }
    }
}

impl Device for DemoSynth {
    fn random(&mut self) -> u32 {
        // Demo only: use a real entropy source on hardware.
        self.seed = self.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        self.seed
    }

    fn profiles(&self, _: DeviceId) -> ProfileSet<'_> {
        ProfileSet {
            enabled: &[],
            disabled: &PROFILES,
        }
    }

    fn set_profile(&mut self, _: DeviceId, _: ProfileId, enable: bool, _: u16) -> ProfileOutcome {
        if enable {
            ProfileOutcome::Enabled(0)
        } else {
            ProfileOutcome::Disabled(0)
        }
    }

    fn resources(&self) -> &[ResourceDecl<'_>] {
        RESOURCES
    }

    fn get(&self, req: &Header<'_>, out: &mut dyn Sink) -> Result<GetMeta<'_>, PeStatus> {
        let mut w = JsonWriter::new(out);
        if req.is_resource(names::DEVICE_INFO) {
            DeviceInfo {
                manufacturer_id: [0x7D, 0, 0],
                family_id: [1, 0],
                model_id: [1, 0],
                version_id: [0, 0, 1, 0],
                manufacturer: "Educational Use",
                family: "midici",
                model: "Demo Synth",
                version: "0.0.1",
                serial_number: None,
            }
            .write(&mut w);
        } else if req.is_resource(names::CHANNEL_LIST) {
            let entries = [ChannelEntry {
                title: "Synth",
                channel: 1,
                program_title: Some("Init"),
                bank_pc: Some([0, 0, 0]),
                ..ChannelEntry::default()
            }];
            write_list(&entries, &mut w, ChannelEntry::write);
        } else if req.is_resource(names::ALL_CTRL_LIST) {
            let ctrls = [
                Controller {
                    title: "Cutoff",
                    channel: Some(1),
                    ctrl_type: CtrlType::Cc,
                    ctrl_index: Some([74, 0]),
                    ctrl_index_len: 1,
                    priority: Some(1),
                    param_path: Some("/Filter/Cutoff"),
                    ..Controller::default()
                },
                Controller {
                    title: "Wave",
                    channel: Some(1),
                    ctrl_type: CtrlType::Cc,
                    ctrl_index: Some([70, 0]),
                    ctrl_index_len: 1,
                    type_hint: Some("valueSelect"),
                    ctrl_map_id: Some("wave"),
                    step_count: Some(2),
                    ..Controller::default()
                },
            ];
            write_list(&ctrls, &mut w, Controller::write);
        } else if req.is_resource(names::CTRL_MAP_LIST) {
            let map = [
                CtrlMapEntry {
                    value: 0,
                    title: "Saw",
                },
                CtrlMapEntry {
                    value: u32::MAX,
                    title: "Square",
                },
            ];
            write_list(&map, &mut w, CtrlMapEntry::write);
        } else if req.is_resource(names::PROGRAM_LIST) {
            let programs = [
                ProgramEntry {
                    title: "Init",
                    bank_pc: [0, 0, 0],
                },
                ProgramEntry {
                    title: "Bass",
                    bank_pc: [0, 0, 1],
                },
            ];
            write_list(&programs, &mut w, ProgramEntry::write);
        } else if req.is_resource("X-Cutoff") {
            w.begin_object()
                .field_uint("value", u64::from(self.cutoff))
                .end_object();
        } else {
            return Err(PeStatus::NOT_FOUND);
        }
        w.finish().map_err(|_| PeStatus::INTERNAL_ERROR)?;
        Ok(GetMeta::default())
    }

    fn set(&mut self, req: &Header<'_>, data: &[u8]) -> Result<(), PeStatus> {
        if !req.is_resource("X-Cutoff") {
            return Err(PeStatus::NOT_ALLOWED);
        }
        // Accept {"value":n} or {"/value":n}.
        let text = std::str::from_utf8(data).map_err(|_| PeStatus::BAD_REQUEST)?;
        let digits: String = text.chars().filter(char::is_ascii_digit).collect();
        self.cutoff = digits.parse().map_err(|_| PeStatus::BAD_REQUEST)?;
        Ok(())
    }

    fn subscription(&mut self, peer: Muid, _: &Header<'_>, id: &str, active: bool) {
        println!(
            "subscription {id} from {peer}: {}",
            if active { "started" } else { "ended" }
        );
    }
}
