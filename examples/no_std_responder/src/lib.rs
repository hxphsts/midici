//! How firmware such as FMX (M-VAVE FM-1) embeds the midici Responder:
//! `#![no_std]`, no allocator, static capacities, at most 128 bytes per
//! SysEx frame, sharing the port with a manufacturer protocol on
//! `F0 7D 'F' 'X' …`.
//!
//! Build it for a bare-metal target to prove it needs neither `std` nor
//! `alloc`:
//!
//! ```sh
//! cargo build --manifest-path examples/no_std_responder/Cargo.toml \
//!     --target riscv32imc-unknown-none-elf
//! ```
#![no_std]

use midici::engine::Handled;
use midici::engine::responder::{Config, Device, GetMeta, Responder};
use midici::identity::{DeviceIdentity, ManufacturerId};
use midici::pe::PeCapabilities;
use midici::pe::header::Header;
use midici::pe::json::JsonWriter;
use midici::pe::resource::{DeviceInfo, ResourceDecl, names};
use midici::pe::sink::Sink;
use midici::pe::status::PeStatus;

/// The SDK sends at most 128 bytes per SysEx (`fmx-proto` `MAX_FRAME`).
pub const MAX_FRAME: usize = 128;

/// The Responder with firmware-sized capacities: 128-byte frames, 512
/// bytes of outbound queue, 256-byte Sets, 2 subscriptions. All static
/// memory; no heap.
pub type FmxResponder = Responder<'static, MAX_FRAME, 512, 256, 2>;

const IDENTITY: DeviceIdentity = DeviceIdentity {
    manufacturer: ManufacturerId::NON_COMMERCIAL,
    family: 0x0001,
    model: 0x0001,
    revision: [0, 0, 1, 0],
};

static RESOURCES: [ResourceDecl<'static>; 1] = [ResourceDecl::new(names::DEVICE_INFO)];

/// Device state the Responder queries. Replace `random` with the MCU's
/// TRNG (or ADC noise) in real firmware.
pub struct Fm1 {
    entropy: u32,
}

impl Device for Fm1 {
    fn random(&mut self) -> u32 {
        self.entropy ^= self.entropy << 13;
        self.entropy ^= self.entropy >> 17;
        self.entropy ^= self.entropy << 5;
        self.entropy
    }

    fn resources(&self) -> &[ResourceDecl<'_>] {
        &RESOURCES
    }

    fn get(&self, req: &Header<'_>, out: &mut dyn Sink) -> Result<GetMeta<'_>, PeStatus> {
        if !req.is_resource(names::DEVICE_INFO) {
            return Err(PeStatus::NOT_FOUND);
        }
        let mut w = JsonWriter::new(out);
        DeviceInfo::from_identity(&IDENTITY, "Educational Use", "FMX", "FM-1", "0.1.0")
            .write(&mut w);
        w.finish().map_err(|_| PeStatus::INTERNAL_ERROR)?;
        Ok(GetMeta::default())
    }
}

/// The firmware's SysEx endpoint.
pub struct Firmware {
    device: Fm1,
    ci: FmxResponder,
}

impl Firmware {
    /// Boot: seed the MUID from hardware entropy (M2-101 §3.3.1).
    #[must_use]
    pub fn new(entropy: u32) -> Self {
        let mut config = Config::new(IDENTITY);
        config.property_exchange = Some(PeCapabilities::default());
        config.max_sysex = 512;
        config.max_frame = 128;
        Self {
            device: Fm1 {
                entropy: entropy | 1,
            },
            ci: Responder::new(config, entropy),
        }
    }

    /// Called for every complete SysEx received on USB-MIDI.
    pub fn on_sysex(&mut self, now_ms: u64, bytes: &[u8], fmx: &mut impl FnMut(&[u8])) {
        if bytes.starts_with(&[0xF0, 0x7D, b'F', b'X']) {
            // The FMX control protocol: not ours.
            fmx(bytes);
            return;
        }
        // `Handled` says whether the message was CI at all; firmware that
        // only shares the port can ignore it.
        let _: Handled = self.ci.handle(&mut self.device, now_ms, bytes);
    }

    /// Called from the main loop: sends at most one frame per call, so
    /// large Property Exchange replies are paced by the USB endpoint.
    pub fn poll(&mut self, now_ms: u64, send: &mut impl FnMut(&[u8])) {
        self.ci.poll(now_ms);
        let mut frame = [0u8; MAX_FRAME];
        if let Some(n) = self.ci.poll_transmit(&self.device, &mut frame) {
            if let Some(f) = frame.get(..n) {
                send(f);
            }
        }
    }
}
