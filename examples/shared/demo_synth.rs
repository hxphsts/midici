//! A small demo device shared by the `responder` and `initiator` examples.

use midici::engine::responder::{
    Config, Device, GetMeta, ProfileOutcome, ProfileSet, SubscriptionChange,
};
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
use midici::profile::{ProfileId, ProfileState};
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
    ResourceDecl::new(names::CHANNEL_LIST).with_subscribe(),
    ResourceDecl::new(names::ALL_CTRL_LIST),
    ResourceDecl::new(names::CTRL_MAP_LIST).with_require_res_id(),
    ResourceDecl::new(names::PROGRAM_LIST),
    ResourceDecl::new("X-Cutoff")
        .with_set(CanSet::Partial)
        .with_subscribe(),
];

pub fn config() -> Config<'static> {
    let mut config = Config::new(IDENTITY);
    config.profiles = true;
    config.property_exchange = Some(PeCapabilities::default());
    config.process = ProcessFeatures::NONE;
    config.product_instance_id = Some("DEMO-0001");
    config.max_frame = 128;
    config
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
        ProfileSet::new(&[], &PROFILES)
    }

    fn set_profile(
        &mut self,
        _: DeviceId,
        _: ProfileId,
        state: ProfileState,
        _: u16,
    ) -> ProfileOutcome {
        if state == ProfileState::Enabled {
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
            DeviceInfo::from_identity(
                &IDENTITY,
                "Educational Use",
                "midici",
                "Demo Synth",
                "0.0.1",
            )
            .write(&mut w);
        } else if req.is_resource(names::CHANNEL_LIST) {
            let entries = [ChannelEntry::new("Synth", 1).with_program("Init", [0, 0, 0])];
            write_list(&entries, &mut w, ChannelEntry::write);
        } else if req.is_resource(names::ALL_CTRL_LIST) {
            let ctrls = [
                Controller::new("Cutoff", CtrlType::Cc)
                    .with_channel(1)
                    .with_index(74)
                    .with_priority(1)
                    .with_param_path("/Filter/Cutoff"),
                Controller::new("Wave", CtrlType::Cc)
                    .with_channel(1)
                    .with_index(70)
                    .with_type_hint("valueSelect")
                    .with_ctrl_map_id("wave")
                    .with_step_count(2),
            ];
            write_list(&ctrls, &mut w, Controller::write);
        } else if req.is_resource(names::CTRL_MAP_LIST) {
            let map = [
                CtrlMapEntry::new(0, "Saw"),
                CtrlMapEntry::new(u32::MAX, "Square"),
            ];
            write_list(&map, &mut w, CtrlMapEntry::write);
        } else if req.is_resource(names::PROGRAM_LIST) {
            let programs = [
                ProgramEntry::new("Init", [0, 0, 0]),
                ProgramEntry::new("Bass", [0, 0, 1]),
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

    fn subscription(&mut self, peer: Muid, _: &Header<'_>, id: &str, change: SubscriptionChange) {
        let what = if change == SubscriptionChange::Started {
            "started"
        } else {
            "ended"
        };
        println!("subscription {id} from {peer}: {what}");
    }
}
