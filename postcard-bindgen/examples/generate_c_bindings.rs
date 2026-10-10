use postcard_bindgen::{c, generate_bindings, PackageInfo, PostcardBindings};
use serde::Serialize;

#[derive(Serialize, PostcardBindings)]
struct Packet {
    sequence: u32,
    label: String,
    samples: Vec<i16>,
    status: Status,
}

#[derive(Serialize, PostcardBindings)]
#[allow(dead_code)]
enum Status {
    Idle,
    Reading(u16),
    Failed { code: u8 },
}

fn main() -> std::io::Result<()> {
    c::build_package(
        std::env::current_dir()?.as_path(),
        PackageInfo {
            name: "c-bindings".into(),
            version: "0.1.0".try_into().unwrap(),
        },
        c::GenerationSettings::enable_all().max_sequence_length(16),
        generate_bindings!(Packet, Status),
    )
}
