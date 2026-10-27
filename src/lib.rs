//! MIDI Capability Inquiry (MIDI-CI), sans-io, `no_std`, no allocator.
//!
//! `midici` implements the wire format of MIDI-CI (MMA/AMEI M2-101-UM,
//! version 1.2) and its companion specifications. It does no I/O and reads
//! no clocks: hand it bytes, get typed messages; hand it messages and a
//! buffer, get bytes.
//!
//! # What is here
//!
//! * [`wire`], [`muid`], [`message`]: the envelope and a typed body for
//!   every MIDI-CI 1.2 message (plus the deprecated 1.1 ones, [`legacy`]).
//! * [`mgmt`], [`profile`], [`pe`], [`process`]: Management, Profile
//!   Configuration, Property Exchange (chunking, Mcoded7, JSON headers,
//!   resources) and Process Inquiry.
//! * [`engine`]: sans-io state machines. The no-alloc
//!   [`engine::responder::Responder`] is for devices; the
//!   `engine::initiator::Initiator` (feature `alloc`) is for hosts.
//! * [`ump`]: SysEx7 Universal MIDI Packets. [`identity`]: MIDI 1.0
//!   Identity Request/Reply. [`scaling`]: MIDI 2.0 bit scaling.
//!
//! Without default features the crate uses only `core`; see the feature
//! table in the README.
//!
//! # Example
//!
//! ```
//! use midici::identity::{DeviceIdentity, ManufacturerId};
//! use midici::mgmt::{Categories, Discovery};
//! use midici::muid::Muid;
//! use midici::wire::{Envelope, Framing};
//!
//! let me = Muid::from_random(0x1234_5678);
//! let discovery = Discovery {
//!     identity: DeviceIdentity {
//!         manufacturer: ManufacturerId::NON_COMMERCIAL,
//!         family: 1,
//!         model: 1,
//!         revision: [0, 1, 0, 0],
//!     },
//!     categories: Categories::PROPERTY_EXCHANGE,
//!     max_sysex: 512,
//!     output_path: 0,
//! };
//! let mut buf = [0u8; 128];
//! let n = discovery.encode(Envelope::broadcast(me), Framing::SysEx, &mut buf)?;
//! let (envelope, decoded) = Discovery::decode(&buf[..n])?;
//! assert_eq!(envelope.source, me);
//! assert_eq!(decoded, discovery);
//! # Ok::<(), midici::Error>(())
//! ```
//!
//! # Specification citations
//!
//! Comments cite sections such as `M2-101 §5.6`. The specifications are
//! published by the MIDI Association and AMEI; their text is not reproduced
//! here.
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::cast_possible_truncation
    )
)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod codec;
pub mod engine;
mod error;
mod flags;
pub mod identity;
pub mod legacy;
pub mod message;
pub mod mgmt;
pub mod muid;
pub mod pe;
pub mod process;
pub mod profile;
pub mod scaling;
pub mod ump;
pub mod wire;

pub use error::Error;

/// Compiles and runs the README example.
#[cfg(all(doctest, feature = "std"))]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
pub use message::{Body, Message};
pub use muid::Muid;
