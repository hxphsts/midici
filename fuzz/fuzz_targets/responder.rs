//! The no-alloc Responder never panics on arbitrary input, and everything
//! it sends decodes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::Message;
use midici::engine::responder::{Config, Device, GetMeta, ProfileOutcome, ProfileSet, Responder};
use midici::identity::{DeviceIdentity, ManufacturerId};
use midici::pe::PeCapabilities;
use midici::pe::header::Header;
use midici::pe::json::JsonWriter;
use midici::pe::resource::{CanSet, ResourceDecl, names};
use midici::pe::sink::Sink;
use midici::pe::status::PeStatus;
use midici::process::ProcessFeatures;
use midici::profile::{ProfileId, ProfileState};
use midici::wire::DeviceId;

const ID: DeviceIdentity = DeviceIdentity {
    manufacturer: ManufacturerId::NON_COMMERCIAL,
    family: 1,
    model: 2,
    revision: [0, 1, 0, 0],
};
const P: ProfileId = ProfileId::standard(0x21, 0, 1, 1);

static RES: &[ResourceDecl<'static>] = &[
    ResourceDecl::new(names::DEVICE_INFO),
    ResourceDecl::new("X-Param")
        .with_set(CanSet::Partial)
        .with_subscribe(),
    ResourceDecl::new(names::STATE)
        .with_set(CanSet::Full)
        .with_require_res_id()
        .with_encodings(&["Mcoded7"]),
];

struct Dev(u32, Vec<u8>);

impl Device for Dev {
    fn random(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
    fn profiles(&self, _: DeviceId) -> ProfileSet<'_> {
        ProfileSet::new(&[], core::slice::from_ref(&P))
    }
    fn set_profile(
        &mut self,
        _: DeviceId,
        _: ProfileId,
        state: ProfileState,
        n: u16,
    ) -> ProfileOutcome {
        if state == ProfileState::Enabled {
            ProfileOutcome::Enabled(n)
        } else {
            ProfileOutcome::Disabled(0)
        }
    }
    fn resources(&self) -> &[ResourceDecl<'_>] {
        RES
    }
    fn get(&self, req: &Header<'_>, out: &mut dyn Sink) -> Result<GetMeta<'_>, PeStatus> {
        if req.is_resource(names::STATE) {
            out.put(&self.1);
            return Ok(GetMeta::default());
        }
        let mut w = JsonWriter::new(out);
        w.begin_array();
        for i in 0..20u64 {
            w.begin_object()
                .field_uint("n", i)
                .field_str("t", "some text")
                .end_object();
        }
        w.end_array();
        w.finish().map_err(|_| PeStatus::INTERNAL_ERROR)?;
        Ok(GetMeta::default())
    }
    fn set(&mut self, _: &Header<'_>, data: &[u8]) -> Result<(), PeStatus> {
        self.1 = data.to_vec();
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    let mut config = Config::new(ID);
    config.profiles = true;
    config.property_exchange = Some(PeCapabilities::default());
    config.process = ProcessFeatures::MESSAGE_REPORT;
    config.product_instance_id = Some("FUZZ");
    let mut dev = Dev(7, vec![0x80, 0xFF, 0x00]);
    let mut r: Responder<'static, 128, 1024, 512, 2> = Responder::new(config, 99);
    let dst = r.muid().to_bytes();
    let mut out = [0u8; 256];
    let mut now = 0u64;
    // Messages separated by 0xF7; the first byte of each picks raw or
    // "CI-prefixed and addressed to the Responder".
    for (i, part) in data.split(|&b| b == 0xF7).enumerate() {
        now += 700;
        let bytes: Vec<u8> = match part.split_first() {
            Some((mode, rest)) if mode & 1 == 1 => {
                let (sub2, rest) = rest.split_first().unwrap_or((&0x34, &[]));
                let mut v = vec![0xF0, 0x7E, 0x7F, 0x0D, sub2 & 0x7F, 0x02, 1, 2, 3, 4];
                v.extend_from_slice(if mode & 2 == 2 { &[0x7F; 4] } else { &dst });
                v.extend(rest.iter().map(|b| b & 0x7F));
                v.push(0xF7);
                v
            }
            _ => part.to_vec(),
        };
        let _ = r.handle(&mut dev, now, &bytes);
        if i % 3 == 0 {
            r.poll(now);
        }
        let mut guard = 0;
        while let Some(n) = r.poll_transmit(&dev, &mut out) {
            assert!(n <= 128, "frame of {n} bytes exceeds FRAME");
            Message::decode(&out[..n]).expect("responder output decodes");
            guard += 1;
            assert!(guard < 10_000);
        }
    }
});
