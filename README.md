# midici

MIDI Capability Inquiry (MIDI-CI) for Rust: both roles, sans-io, `#![no_std]`,
no allocator required.

Implements M2-101 v1.2 and its Property Exchange companions: Discovery, Profiles,
Property Exchange (Get/Set/Subscribe, chunking, Mcoded7) and Process Inquiry, with a
no-alloc `Responder` for firmware and an `Initiator` for hosts.

Licensed under either of MIT or Apache-2.0 at your option.
