//! Typed resource parsers never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use midici::pe::resource;

fuzz_target!(|data: &[u8]| {
    let _ = resource::parse_resource_list(data);
    let _ = resource::parse_device_info(data);
    let _ = resource::parse_channel_list(data);
    let _ = resource::parse_controllers(data);
    let _ = resource::parse_ctrl_map_list(data);
    let _ = resource::parse_program_list(data);
    let _ = resource::parse_state_list(data);
});
